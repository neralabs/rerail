//! Capture environments as SQL values.
//!
//! A rule threads one environment of metavariable bindings through its terms. In SQL an
//! environment is an integer handle into the batch's arena:
//! - `NULL`: the term did not match;
//! - `>= 0`: it matched, with the environment at that handle (`0` is the empty environment);
//! - `-1`: its verdict is unknown (three-valued rules only).
//!
//! Environments are immutable once stored; a term that binds stores a new one. Nodes are preorder
//! ordinals of the document the statement is evaluating (a batch file, or a derived document: a
//! literal parsed by a sub-grammar).
//!
//! An environment also carries the binding sub-rules being matched (`following`): a chain of
//! bindings that comes back to a node its sub-rule is matching fails there, as the matcher's
//! `FOLLOWING` stack does. It travels with the environment through every term, utility statement
//! and frame, and is never a capture.

use crate::policy_dsl::tape::TapeDoc;
use crate::policy_dsl::transducer::sql_backend::BatchFile;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use std::sync::{Arc, Mutex};

pub(super) const UNKNOWN: i64 = -1;

/// One binding of a metavariable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Binding {
    Single(u32),
    Multi(Vec<u32>),
    /// Text a transform or a parsed sub-grammar exported.
    Text(Vec<u8>),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::policy_dsl) struct Env {
    /// Sorted by name; a name holds at most one binding of each kind.
    bindings: Vec<(Arc<str>, Binding)>,
    /// A three-valued `all` in progress: the terms (by position) whose captures are withheld,
    /// and whether a term was unknown. Empty outside one (`tq_all_end` clears them).
    pub(super) withheld: u64,
    pub(super) unknown: bool,
    /// The binding sub-rules being matched, innermost last: `(target node, sub-rule key)`.
    pub(super) following: Vec<(u32, u64)>,
}

impl Env {
    pub(super) fn single(&self, name: &str) -> Option<u32> {
        self.bindings
            .iter()
            .find_map(|(key, binding)| match binding {
                Binding::Single(node) if &**key == name => Some(*node),
                _ => None,
            })
    }

    pub(super) fn binds(&self, name: &str) -> bool {
        self.bindings.iter().any(|(key, _)| &**key == name)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.withheld == 0 && !self.unknown && self.following.is_empty()
    }

    /// This environment's bindings with `state`'s three-valued state and followed bindings.
    pub(super) fn with_state_of(&self, state: &Env) -> Self {
        Self {
            bindings: self.bindings.clone(),
            withheld: state.withheld,
            unknown: state.unknown,
            following: state.following.clone(),
        }
    }

    /// This environment's bindings, outside any three-valued `all`.
    pub(super) fn from_bindings(state: &Env) -> Self {
        Self {
            bindings: state.bindings.clone(),
            following: state.following.clone(),
            ..Self::default()
        }
    }

    /// An environment with no bindings that keeps `state`'s followed bindings: what a fresh
    /// environment (a stop rule's, a parsed tree's, a frame's) starts from.
    pub(super) fn following_of(state: &Env) -> Self {
        Self {
            following: state.following.clone(),
            ..Self::default()
        }
    }

    /// Whether `(node, key)` is a binding sub-rule being matched.
    pub(super) fn follows(&self, node: u32, key: u64) -> bool {
        self.following.contains(&(node, key))
    }

    /// This environment with `(node, key)` followed.
    pub(super) fn follow(&self, node: u32, key: u64) -> Self {
        let mut following = self.following.clone();
        following.push((node, key));
        Self {
            following,
            ..self.clone()
        }
    }

    /// This environment with `name` bound, where it holds nothing of that kind yet.
    pub(super) fn with(&self, name: Arc<str>, binding: Binding) -> Self {
        let mut bindings = self.bindings.clone();
        let position = bindings
            .binary_search_by(|(key, existing)| {
                key.cmp(&name)
                    .then_with(|| kind_order(existing).cmp(&kind_order(&binding)))
            })
            .unwrap_or_else(|position| position);
        bindings.insert(position, (name, binding));
        Self {
            bindings,
            ..self.clone()
        }
    }

    /// The environment ast-grep's matchers read, with nodes of `file`.
    pub(in crate::policy_dsl) fn to_meta<'a>(
        &self,
        file: &'a BatchFile,
    ) -> MetaVarEnv<'a, TapeDoc> {
        self.meta_with(|node| file.node(node))
    }

    /// The environment ast-grep's matchers read, with nodes `node` gives by ordinal.
    pub(super) fn meta_with<'a, D>(&self, node: impl Fn(u32) -> Node<'a, D>) -> MetaVarEnv<'a, D>
    where
        D: Doc,
        D::Source: Content<Underlying = u8>,
    {
        let mut env = MetaVarEnv::new();
        for (name, binding) in &self.bindings {
            match binding {
                Binding::Single(ordinal) => {
                    env.insert(name, node(*ordinal));
                }
                Binding::Multi(ordinals) => {
                    env.insert_multi(
                        name,
                        ordinals.iter().map(|&ordinal| node(ordinal)).collect(),
                    );
                }
                Binding::Text(bytes) => {
                    env.insert_transformation(&MetaVariable::Multiple, name, bytes.clone());
                }
            }
        }
        env
    }

    #[cfg(test)]
    pub(super) fn from_meta(env: &MetaVarEnv<'_, TapeDoc>) -> Self {
        Self::from_meta_with(env, |id| id as u32)
    }

    /// The bindings of `env`, each node by the ordinal `ordinal` gives its node id.
    pub(super) fn from_meta_with<D>(env: &MetaVarEnv<'_, D>, ordinal: impl Fn(usize) -> u32) -> Self
    where
        D: Doc,
        D::Source: Content<Underlying = u8>,
    {
        let mut bindings: Vec<(Arc<str>, Binding)> = Vec::new();
        for variable in env.get_matched_variables() {
            match variable {
                MetaVariable::Capture(name, _) => {
                    if let Some(node) = env.get_match(&name) {
                        bindings.push((Arc::from(name), Binding::Single(ordinal(node.node_id()))));
                    } else if let Some(bytes) = env.get_transformed(&name) {
                        bindings.push((Arc::from(name), Binding::Text(bytes.clone())));
                    }
                }
                MetaVariable::MultiCapture(name) => {
                    let nodes = env
                        .get_multiple_matches(&name)
                        .iter()
                        .map(|node| ordinal(node.node_id()))
                        .collect();
                    bindings.push((Arc::from(name), Binding::Multi(nodes)));
                }
                MetaVariable::Dropped(_) | MetaVariable::Multiple => {}
            }
        }
        bindings.sort_by(|(left, a), (right, b)| {
            left.cmp(right)
                .then_with(|| kind_order(a).cmp(&kind_order(b)))
        });
        bindings.dedup();
        Self {
            bindings,
            ..Self::default()
        }
    }
}

fn kind_order(binding: &Binding) -> u8 {
    match binding {
        Binding::Single(_) => 0,
        Binding::Multi(_) => 1,
        Binding::Text(_) => 2,
    }
}

/// A batch's environments. Handle `0` is the empty environment.
pub(super) struct EnvArena {
    envs: Mutex<Vec<Arc<Env>>>,
}

impl Default for EnvArena {
    fn default() -> Self {
        Self {
            envs: Mutex::new(vec![Arc::new(Env::default())]),
        }
    }
}

impl EnvArena {
    pub(super) fn get(&self, handle: i64) -> Option<Arc<Env>> {
        let index = usize::try_from(handle).ok()?;
        self.envs
            .lock()
            .expect("environment arena mutex poisoned")
            .get(index)
            .cloned()
    }

    pub(super) fn put(&self, env: Env) -> i64 {
        if env.is_empty() {
            return 0;
        }
        let mut envs = self.envs.lock().expect("environment arena mutex poisoned");
        envs.push(Arc::new(env));
        (envs.len() - 1) as i64
    }
}
