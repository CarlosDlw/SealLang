//! Deterministic graph queries over a loaded project.
//!
//! The graphs are derived data: packages come from dependency resolution,
//! targets from the target model, actions from the compile relationship, and
//! modules from the source declarations. Every node and edge is sorted so two
//! runs over the same project produce byte-identical output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use seal_manifest::TargetKind;
use serde::Serialize;

use crate::{Package, Project};

/// The kind of graph requested by `seal graph`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphKind {
    /// Packages and resolved dependency edges.
    Packages,
    /// Package targets and the targets they use.
    Targets,
    /// Build actions and their ordering edges.
    Actions,
    /// Source modules and their imports.
    Modules,
}

impl GraphKind {
    /// Parses a command-line graph kind.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "packages" | "package" => Some(Self::Packages),
            "targets" | "target" => Some(Self::Targets),
            "actions" | "action" => Some(Self::Actions),
            "modules" | "module" => Some(Self::Modules),
            _ => None,
        }
    }

    /// Returns the stable name used by `--format` consumers.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Targets => "targets",
            Self::Actions => "actions",
            Self::Modules => "modules",
        }
    }
}

/// One node in a project graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphNode {
    /// The stable node identity.
    pub id: String,
    /// The short display label.
    pub label: String,
    /// The node kind, such as `package`, `target`, `action`, or `module`.
    pub kind: String,
    /// The package index the node belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<usize>,
    /// The package version, when the node names one instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// One directed edge in a project graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphEdge {
    /// The source node identity.
    pub from: String,
    /// The destination node identity.
    pub to: String,
    /// An optional edge label, such as a dependency alias.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A deterministic graph projection of a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectGraph {
    /// The JSON schema version.
    pub version: u32,
    /// Which graph was requested.
    pub kind: GraphKind,
    /// Nodes sorted by identity.
    pub nodes: Vec<GraphNode>,
    /// Edges sorted by source and destination.
    pub edges: Vec<GraphEdge>,
}

impl ProjectGraph {
    /// Builds the requested graph.
    pub fn build(project: &Project, kind: GraphKind) -> Self {
        let mut graph = match kind {
            GraphKind::Packages => packages(project),
            GraphKind::Targets => targets(project),
            GraphKind::Actions => actions(project),
            GraphKind::Modules => modules(project),
        };
        graph.normalize();
        graph
    }

    /// Restricts the graph to a node and its descendants (or ancestors when
    /// `reverse` is set), keeping the edges between retained nodes.
    pub fn focused(&self, focus: &str, reverse: bool) -> Option<Self> {
        let id = self
            .nodes
            .iter()
            .find(|node| node.id == focus || node.label == focus)
            .map(|node| node.id.clone())?;
        let mut retained: BTreeSet<String> = BTreeSet::new();
        retained.insert(id.clone());
        let mut frontier = vec![id];
        while let Some(current) = frontier.pop() {
            for edge in &self.edges {
                let next = if reverse {
                    (edge.to == current).then_some(&edge.from)
                } else {
                    (edge.from == current).then_some(&edge.to)
                };
                if let Some(next) = next {
                    if retained.insert(next.clone()) {
                        frontier.push(next.clone());
                    }
                }
            }
        }
        let mut graph = Self {
            version: self.version,
            kind: self.kind,
            nodes: self
                .nodes
                .iter()
                .filter(|node| retained.contains(&node.id))
                .cloned()
                .collect(),
            edges: self
                .edges
                .iter()
                .filter(|edge| retained.contains(&edge.from) && retained.contains(&edge.to))
                .cloned()
                .collect(),
        };
        graph.normalize();
        Some(graph)
    }

    /// Reverses every edge, keeping nodes unchanged.
    pub fn reversed(mut self) -> Self {
        for edge in &mut self.edges {
            std::mem::swap(&mut edge.from, &mut edge.to);
        }
        self.normalize();
        self
    }

    fn normalize(&mut self) {
        self.nodes.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.kind.cmp(&right.kind))
        });
        self.nodes.dedup_by(|left, right| left.id == right.id);
        self.edges.sort_by(|left, right| {
            left.from
                .cmp(&right.from)
                .then_with(|| left.to.cmp(&right.to))
                .then_with(|| left.label.cmp(&right.label))
        });
        self.edges.dedup();
    }
}

/// Returns a node identity that distinguishes same-name package instances.
///
/// Unique names keep the plain package name, which keeps focus, tests, and
/// scripts readable. A duplicated name gains its version, and a duplicated
/// name and version gain the stable project index.
fn package_id(project: &Project, index: usize) -> String {
    let package = project.package(index);
    let same_name = project
        .packages
        .iter()
        .filter(|candidate| candidate.name == package.name)
        .count();
    if same_name == 1 {
        return package.name.clone();
    }
    let same_version = project
        .packages
        .iter()
        .filter(|candidate| candidate.name == package.name && candidate.version == package.version)
        .count();
    if same_version == 1 {
        format!("{}@{}", package.name, package.version)
    } else {
        format!("{}@{}#{index}", package.name, package.version)
    }
}

fn package_node(project: &Project, index: usize) -> GraphNode {
    let package = project.package(index);
    GraphNode {
        id: package_id(project, index),
        label: format!("{} {}", package.name, package.version),
        kind: "package".to_owned(),
        package: Some(index),
        version: Some(package.version.clone()),
    }
}

fn packages(project: &Project) -> ProjectGraph {
    let nodes = (0..project.packages.len())
        .map(|index| package_node(project, index))
        .collect();
    let mut edges = Vec::new();
    for (index, package) in project.packages.iter().enumerate() {
        for dependency in &package.dependencies {
            edges.push(GraphEdge {
                from: package_id(project, index),
                to: package_id(project, dependency.package),
                label: Some(dependency.alias.clone()),
            });
        }
    }
    ProjectGraph {
        version: 1,
        kind: GraphKind::Packages,
        nodes,
        edges,
    }
}

fn targets(project: &Project) -> ProjectGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (index, package) in project.packages.iter().enumerate() {
        let id = package_id(project, index);
        for target in &package.targets {
            nodes.push(target_node(index, package, &id, &target.name, &target.kind));
        }
    }
    for (index, package) in project.packages.iter().enumerate() {
        let id = package_id(project, index);
        for target in &package.targets {
            for generated in &target.generated {
                edges.push(GraphEdge {
                    from: target_id(&id, generated),
                    to: target_id(&id, &target.name),
                    label: Some("generates".to_owned()),
                });
            }
            for dependency in &package.dependencies {
                let dependency_package = project.package(dependency.package);
                if let Some(library) = dependency_package.library() {
                    edges.push(GraphEdge {
                        from: target_id(&id, &target.name),
                        to: target_id(&package_id(project, dependency.package), &library.name),
                        label: Some(dependency.alias.clone()),
                    });
                }
            }
        }
    }
    ProjectGraph {
        version: 1,
        kind: GraphKind::Targets,
        nodes,
        edges,
    }
}

fn actions(project: &Project) -> ProjectGraph {
    let targets = targets(project);
    let mut graph = ProjectGraph {
        version: 1,
        kind: GraphKind::Actions,
        nodes: targets
            .nodes
            .into_iter()
            .map(|node| GraphNode {
                id: format!("build:{}", node.id),
                label: node.label,
                kind: "action".to_owned(),
                package: node.package,
                version: node.version,
            })
            .collect(),
        edges: targets
            .edges
            .into_iter()
            .map(|edge| GraphEdge {
                from: format!("build:{}", edge.from),
                to: format!("build:{}", edge.to),
                label: edge.label,
            })
            .collect(),
    };
    graph.normalize();
    graph
}

fn target_node(
    package_index: usize,
    package: &Package,
    package_id: &str,
    name: &str,
    kind: &TargetKind,
) -> GraphNode {
    GraphNode {
        id: target_id(package_id, name),
        label: format!("{}:{} ({})", package.name, name, kind.as_str()),
        kind: "target".to_owned(),
        package: Some(package_index),
        version: Some(package.version.clone()),
    }
}

fn target_id(package: &str, target: &str) -> String {
    format!("{package}:{target}")
}

fn modules(project: &Project) -> ProjectGraph {
    let mut nodes = Vec::new();
    let mut edges = BTreeSet::new();
    for (index, package) in project.packages.iter().enumerate() {
        let owner = package_id(project, index);
        let aliases: BTreeMap<String, String> = package
            .dependencies
            .iter()
            .map(|dependency| {
                (
                    dependency.alias.clone(),
                    package_id(project, dependency.package),
                )
            })
            .collect();
        let mut modules: BTreeMap<String, String> = BTreeMap::new();
        modules.insert("root".to_owned(), format!("{owner}::root"));
        for file in &package.files {
            let Some((module, imports)) = module_info(file) else {
                continue;
            };
            let id = format!("{owner}::{module}");
            modules.insert(module.clone(), id.clone());
            for parent in module_parents(&module) {
                let parent_id = format!("{owner}::{parent}");
                edges.insert((parent_id, id.clone()));
            }
            for import in imports {
                let local = import
                    .strip_prefix("root::")
                    .or_else(|| import.strip_prefix("root"));
                if let Some(local) = local {
                    let local = if local.is_empty() { "root" } else { local };
                    if local != module {
                        edges.insert((id.clone(), format!("{owner}::{local}")));
                    }
                } else {
                    let root = import.split("::").next().unwrap_or_default();
                    if let Some(dependency) = aliases.get(root) {
                        // An import can name a function directly, so the graph
                        // links the package root rather than inventing a module
                        // that may not exist.
                        let target = format!("{dependency}::root");
                        edges.insert((id.clone(), target));
                    }
                }
            }
        }
        for (module, id) in &modules {
            nodes.push(GraphNode {
                id: id.clone(),
                label: format!("{owner}::{module}"),
                kind: "module".to_owned(),
                package: Some(index),
                version: Some(package.version.clone()),
            });
        }
    }
    let ids: BTreeSet<String> = nodes.iter().map(|node| node.id.clone()).collect();
    ProjectGraph {
        version: 1,
        kind: GraphKind::Modules,
        nodes,
        edges: edges
            .into_iter()
            .filter(|(from, to)| ids.contains(from) && ids.contains(to))
            .map(|(from, to)| GraphEdge {
                from,
                to,
                label: None,
            })
            .collect(),
    }
}

fn module_parents(module: &str) -> Vec<String> {
    if module == "root" {
        return Vec::new();
    }
    let mut parents = Vec::new();
    let mut current = String::new();
    for segment in module.split("::") {
        let parent = if current.is_empty() {
            "root".to_owned()
        } else {
            current.clone()
        };
        parents.push(parent);
        if current.is_empty() {
            current.push_str(segment);
        } else {
            current.push_str("::");
            current.push_str(segment);
        }
    }
    parents
}

/// Reads a file's module declaration and absolute imports.
///
/// This is deliberately a lexical query, not a compiler parse: the graph must
/// remain available for projects with errors. A file without a declaration
/// contributes to the package root, and comments are ignored.
pub(crate) fn module_info(path: &Path) -> Option<(String, Vec<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut module = "root".to_owned();
    let mut imports = Vec::new();
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            let path = rest.strip_suffix(';').unwrap_or(rest).trim();
            if !path.is_empty() {
                module = path.to_owned();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            let path = rest
                .strip_suffix(';')
                .unwrap_or(rest)
                .split(" as ")
                .next()
                .unwrap_or(rest)
                .trim()
                .replace(' ', "");
            if !path.is_empty() && !path.contains('{') {
                imports.push(path);
            }
        }
    }
    Some((module, imports))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_parents_start_at_the_root() {
        assert!(module_parents("root").is_empty());
        assert_eq!(module_parents("app"), ["root"]);
        assert_eq!(module_parents("app::util"), ["root", "app"]);
    }

    #[test]
    fn graph_kinds_parse_and_round_trip() {
        for kind in [
            GraphKind::Packages,
            GraphKind::Targets,
            GraphKind::Actions,
            GraphKind::Modules,
        ] {
            assert_eq!(GraphKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(GraphKind::parse("nope"), None);
    }

    #[test]
    fn a_focus_keeps_transitive_descendants_and_reverse_keeps_ancestors() {
        let graph = ProjectGraph {
            version: 1,
            kind: GraphKind::Packages,
            nodes: ["a", "b", "c"]
                .into_iter()
                .map(|id| GraphNode {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    kind: "package".to_owned(),
                    package: None,
                    version: None,
                })
                .collect(),
            edges: vec![
                GraphEdge {
                    from: "a".to_owned(),
                    to: "b".to_owned(),
                    label: None,
                },
                GraphEdge {
                    from: "b".to_owned(),
                    to: "c".to_owned(),
                    label: None,
                },
            ],
        };
        let forward = graph.focused("a", false).expect("focus exists");
        assert_eq!(forward.nodes.len(), 3);
        let reverse = graph.focused("c", true).expect("focus exists");
        assert_eq!(reverse.nodes.len(), 3);
        assert_eq!(reverse.edges.len(), 2);
        assert!(graph.focused("missing", false).is_none());
    }
}
