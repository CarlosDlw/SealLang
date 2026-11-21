//! Graph projection and focus tests.

use seal_project::GraphKind;

use super::{load, monorepo};

#[test]
fn package_graphs_are_deterministic_and_contain_dependency_edges() {
    let fixture = monorepo("graph-packages");
    let project = load(&fixture);
    let first = project.graph(GraphKind::Packages);
    let second = project.graph(GraphKind::Packages);
    assert_eq!(first, second);
    assert_eq!(first.nodes.len(), 3);
    assert_eq!(first.edges.len(), 2);
    assert!(first.edges.iter().any(|edge| {
        edge.from == "app" && edge.to == "beta" && edge.label.as_deref() == Some("beta")
    }));
}

#[test]
fn target_and_action_graphs_follow_library_dependencies() {
    let fixture = monorepo("graph-actions");
    let project = load(&fixture);
    let targets = project.graph(GraphKind::Targets);
    assert!(
        targets
            .edges
            .iter()
            .any(|edge| edge.from == "app:app" && edge.to == "beta:beta")
    );
    let actions = project.graph(GraphKind::Actions);
    assert!(
        actions
            .nodes
            .iter()
            .all(|node| node.id.starts_with("build:"))
    );
}

#[test]
fn focus_and_reverse_keep_reachable_nodes() {
    let fixture = monorepo("graph-focus");
    let project = load(&fixture);
    let graph = project.graph(GraphKind::Packages);
    let descendants = graph.focused("app", false).expect("app exists");
    assert_eq!(descendants.nodes.len(), 3);
    let ancestors = graph.focused("alpha", true).expect("alpha exists");
    assert_eq!(ancestors.nodes.len(), 3);
    assert!(graph.focused("missing", false).is_none());
}

#[test]
fn module_graphs_link_imports() {
    let fixture = monorepo("graph-modules");
    let project = load(&fixture);
    let graph = project.graph(GraphKind::Modules);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == "beta::root" && edge.to == "alpha::root")
    );
}
