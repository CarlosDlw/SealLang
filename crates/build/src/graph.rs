//! The action graph and its executor.
//!
//! Nodes are keyed by their content digest. Edges express ordering only: the
//! key already commits to every input digest and hashed environment value, so a
//! dependency cannot change without changing the dependent key. Execution is
//! level-synchronous: every ready node of a level runs in parallel up to the job
//! limit, and the next level starts only after the whole level succeeded. That
//! keeps cancellation and error aggregation simple and makes the order of
//! results deterministic.

use std::collections::BTreeMap;
use std::sync::Arc;

use seal_cas::{ActionCache, ActionRecord, Digest};

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::{BuildError, BuildResult};

#[derive(Clone)]
struct Node {
    action: Action,
    dependencies: Vec<Digest>,
}

/// A directed acyclic graph of actions.
#[derive(Default)]
pub(crate) struct ActionGraph {
    nodes: BTreeMap<Digest, Node>,
}

impl ActionGraph {
    /// Creates an empty graph.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds an action and returns its key.
    ///
    /// Every dependency must already be in the graph.
    pub(crate) fn add(
        &mut self,
        action: Action,
        mut dependencies: Vec<Digest>,
    ) -> BuildResult<Digest> {
        dependencies.sort();
        dependencies.dedup();
        for dependency in &dependencies {
            if !self.nodes.contains_key(dependency) {
                return Err(BuildError::action(format!(
                    "the action `{}` depends on the unknown key `{dependency}`",
                    action.label()
                )));
            }
        }
        let mut command = action.command();
        command.extend(dependencies.iter().map(|key| format!("dep:{key}")));
        let key = ActionCache::key(
            action.kind(),
            action.inputs(),
            &action.environment(),
            &command,
        );
        if let Some(existing) = self.nodes.get(&key) {
            if existing.action.label() != action.label() {
                return Err(BuildError::action(format!(
                    "the actions `{}` and `{}` have the same content key `{key}`",
                    existing.action.label(),
                    action.label()
                )));
            }
            return Ok(key);
        }
        self.nodes.insert(
            key,
            Node {
                action,
                dependencies,
            },
        );
        Ok(key)
    }

    /// Returns the number of nodes.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns whether the graph is empty.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Executes every node and returns the records keyed by action key.
    pub(crate) fn execute(
        &self,
        context: &Arc<ActionContext>,
        jobs: usize,
    ) -> BuildResult<BTreeMap<Digest, ActionRecord>> {
        let jobs = jobs.max(1);
        let mut pending: BTreeMap<Digest, usize> = self
            .nodes
            .iter()
            .map(|(key, node)| (*key, node.dependencies.len()))
            .collect();
        let mut dependents: BTreeMap<Digest, Vec<Digest>> = BTreeMap::new();
        for (key, node) in &self.nodes {
            for dependency in &node.dependencies {
                dependents.entry(*dependency).or_default().push(*key);
            }
        }
        let mut ready: Vec<Digest> = pending
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(key, _)| *key)
            .collect();
        sort_ready(&mut ready, &self.nodes);
        let mut results = BTreeMap::new();
        while !ready.is_empty() {
            let count = ready.len().min(jobs);
            let batch: Vec<Digest> = ready.drain(..count).collect();
            let outcomes = execute_batch(context, &self.nodes, &batch);
            let mut failed: Vec<BuildError> = Vec::new();
            let mut succeeded = Vec::new();
            for (key, outcome) in batch.into_iter().zip(outcomes) {
                match outcome {
                    Ok(record) => {
                        results.insert(key, record);
                        succeeded.push(key);
                    }
                    Err(error) => failed.push(error),
                }
            }
            if !failed.is_empty() {
                // Every action in the level had already started, so report all
                // of their failures instead of hiding the later ones behind the
                // first error.
                let code = failed[0].code();
                let message = failed
                    .iter()
                    .map(BuildError::message)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(BuildError::new(code, message));
            }
            for key in succeeded {
                if let Some(children) = dependents.get(&key) {
                    for child in children {
                        if let Some(remaining) = pending.get_mut(child) {
                            *remaining = remaining.saturating_sub(1);
                            if *remaining == 0 {
                                ready.push(*child);
                            }
                        }
                    }
                }
            }
            sort_ready(&mut ready, &self.nodes);
        }
        if results.len() != self.nodes.len() {
            return Err(BuildError::action(
                "the action graph contains a dependency cycle",
            ));
        }
        Ok(results)
    }
}

fn sort_ready(ready: &mut [Digest], nodes: &BTreeMap<Digest, Node>) {
    ready.sort_by(|left, right| {
        let left = &nodes[left];
        let right = &nodes[right];
        left.action
            .kind()
            .cmp(right.action.kind())
            .then_with(|| left.action.label().cmp(right.action.label()))
            .then_with(|| left.action.command().cmp(&right.action.command()))
    });
}

fn execute_batch(
    context: &Arc<ActionContext>,
    nodes: &BTreeMap<Digest, Node>,
    batch: &[Digest],
) -> Vec<BuildResult<ActionRecord>> {
    if batch.len() == 1 {
        let key = batch[0];
        return vec![context.run(key, &nodes[&key].action)];
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = batch
            .iter()
            .map(|key| {
                let context = Arc::clone(context);
                let action = nodes[key].action.clone();
                let key = *key;
                scope.spawn(move || context.run(key, &action))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(BuildError::action("an action worker panicked")))
            })
            .collect()
    })
}
