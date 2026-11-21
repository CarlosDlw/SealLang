//! `seal graph`: package, target, action, and module graphs.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use seal_project::{GraphEdge, GraphKind, ProjectGraph};
use serde::Serialize;

use crate::cli::{Cli, GraphArgs, GraphFormat};
use crate::ui;

pub(crate) fn graph(cli: &Cli, root: &Path, args: &GraphArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let Some(kind) = GraphKind::parse(&args.kind) else {
        return ui::error(
            "E8013",
            &format!(
                "unknown graph kind `{}`; use packages, targets, actions, or modules",
                args.kind
            ),
        );
    };
    let graph = project.graph(kind);
    let graph = match &args.focus {
        Some(focus) => match graph.focused(focus, args.reverse) {
            Some(graph) => graph,
            None => {
                return ui::error("E8013", &format!("the graph has no node named `{focus}`"));
            }
        },
        None => graph,
    };
    let graph = if args.reverse && args.focus.is_none() {
        graph.reversed()
    } else {
        graph
    };
    let output = match args.format {
        GraphFormat::Ascii => ascii(&graph),
        GraphFormat::Dot => dot(&graph),
        GraphFormat::Mermaid => mermaid(&graph),
        GraphFormat::Json => match json(&graph) {
            Ok(text) => text,
            Err(error) => return ui::error("E8013", &error),
        },
    };
    print!("{output}");
    ExitCode::SUCCESS
}

fn ascii(graph: &ProjectGraph) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "{} ({} node(s), {} edge(s))",
        graph.kind.as_str(),
        graph.nodes.len(),
        graph.edges.len()
    );
    for node in &graph.nodes {
        let _ = writeln!(output, "  {}", node.label);
        for edge in graph.edges.iter().filter(|edge| edge.from == node.id) {
            let label = edge
                .label
                .as_ref()
                .map(|label| format!(" [{label}]"))
                .unwrap_or_default();
            let target = graph
                .nodes
                .iter()
                .find(|candidate| candidate.id == edge.to)
                .map(|candidate| candidate.label.as_str())
                .unwrap_or(&edge.to);
            let _ = writeln!(output, "    -> {target}{label}");
        }
    }
    output
}

fn dot(graph: &ProjectGraph) -> String {
    let mut output = String::from("digraph seal {\n  rankdir=LR;\n");
    for node in &graph.nodes {
        let _ = writeln!(
            output,
            "  \"{}\" [label=\"{}\", shape=\"box\"];",
            escape(&node.id),
            escape(&node.label)
        );
    }
    for edge in &graph.edges {
        let label = edge
            .label
            .as_ref()
            .map(|label| format!(" [label=\"{}\"]", escape(label)))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "  \"{}\" -> \"{}\"{label};",
            escape(&edge.from),
            escape(&edge.to)
        );
    }
    output.push_str("}\n");
    output
}

fn mermaid(graph: &ProjectGraph) -> String {
    let mut output = String::from("flowchart LR\n");
    for node in &graph.nodes {
        let _ = writeln!(
            output,
            "  {}[\"{}\"]",
            mermaid_id(&node.id),
            escape(&node.label)
        );
    }
    for edge in &graph.edges {
        let label = edge
            .label
            .as_ref()
            .map(|label| format!("|{}|", escape(label)))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "  {} -->{label} {}",
            mermaid_id(&edge.from),
            mermaid_id(&edge.to)
        );
    }
    output
}

#[derive(Serialize)]
struct JsonGraph<'a> {
    version: u32,
    kind: &'a str,
    nodes: &'a [seal_project::GraphNode],
    edges: &'a [GraphEdge],
}

fn json(graph: &ProjectGraph) -> Result<String, String> {
    let document = JsonGraph {
        version: graph.version,
        kind: graph.kind.as_str(),
        nodes: &graph.nodes,
        edges: &graph.edges,
    };
    serde_json::to_string_pretty(&document)
        .map(|text| format!("{text}\n"))
        .map_err(|error| format!("cannot serialize the graph: {error}"))
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn mermaid_id(id: &str) -> String {
    let mut output = String::from("n_");
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            output.push(byte as char);
        } else {
            let _ = write!(output, "_{byte:02x}");
        }
    }
    output
}

/// Returns the set of nodes reached from `focus` in either direction.
#[allow(dead_code)]
pub(crate) fn reachable(graph: &ProjectGraph, focus: &str, reverse: bool) -> BTreeSet<String> {
    let mut nodes = BTreeSet::new();
    let Some(focused) = graph.focused(focus, reverse) else {
        return nodes;
    };
    nodes.extend(focused.nodes.into_iter().map(|node| node.id));
    nodes
}
