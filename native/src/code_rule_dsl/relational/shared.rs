//! Full producer definitions identify shared work. Names and hashes alone never establish equality.
use super::{Plan, Stage, syntax};
use crate::code_rule_dsl::model::{CompiledCodeRule, FileInput, Snapshot};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[cfg(feature = "code-rule-profile")]
mod capture;
#[cfg(feature = "code-rule-profile")]
pub(super) use capture::save as capture;

type EvidenceVersion = (String, String, [u8; 32], Option<String>, Option<[u8; 32]>);

/// Authoritative producer context is independent of whether source strings are resident.
#[derive(Clone, Eq, PartialEq, Hash)]
pub(in crate::code_rule_dsl) struct Versions(Arc<[EvidenceVersion]>);
impl Versions {
    pub fn new<'a>(files: impl Iterator<Item = (&'a FileInput, [u8; 32])>) -> Self {
        let mut versions = files
            .map(|(f, digest)| {
                (
                    f.path.clone(),
                    f.language.clone(),
                    digest,
                    f.module.as_ref().map(|m| format!("{m:?}")),
                    f.package.as_ref().map(|p| *p.digest()),
                )
            })
            .collect::<Vec<_>>();
        versions.sort();
        Self(versions.into())
    }
    pub fn snapshot(snapshot: &Snapshot) -> Self {
        Self::new(
            snapshot
                .files
                .iter()
                .map(|f| (f, Sha256::digest(f.source.as_bytes()).into())),
        )
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
enum Key {
    Syntax {
        family: u8,
        files: Vec<(String, u32, [u8; 32])>,
        excluded: Vec<String>,
        empty: Vec<String>,
        owners: Vec<(String, String, bool)>,
    },
    Evidence {
        definition: Arc<str>,
        clause: String,
        name: String,
        versions: Versions,
        domain: Option<Vec<String>>,
        nonce: usize,
    },
    Sql {
        query: Arc<str>,
        schema: Arc<str>,
        deferred: Vec<(String, String)>,
        inputs: Vec<(String, usize)>,
        regex_limit: usize,
        indexes: Vec<Vec<String>>,
    },
    Reach {
        roots: String,
        edges: String,
        forest: bool,
        expand: bool,
        inputs: Vec<(String, usize)>,
    },
}
#[derive(Default)]
pub(super) struct Bindings {
    definitions: HashMap<Key, usize>,
    next_lease: usize,
    next_id: usize,
    bytes: usize,
}
impl Bindings {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn forget(&mut self) {
        self.definitions.clear();
        self.bytes = 0;
    }
    pub fn lease(&mut self) -> usize {
        self.next_lease += 1;
        self.next_lease
    }
    fn intern(&mut self, key: Key) -> usize {
        if let Some(id) = self.definitions.get(&key) {
            return *id;
        }
        self.bytes = self.bytes.saturating_add(
            match &key {
                Key::Sql {
                    query,
                    schema,
                    inputs,
                    indexes,
                    deferred,
                    ..
                } => {
                    query.len()
                        + schema.len()
                        + inputs.iter().map(|(n, _)| n.len() + 48).sum::<usize>()
                        + indexes
                            .iter()
                            .flatten()
                            .map(|n| n.len() + 24)
                            .sum::<usize>()
                        + deferred
                            .iter()
                            .map(|(n, v)| n.len() + v.len() + 48)
                            .sum::<usize>()
                }
                Key::Syntax {
                    files,
                    excluded,
                    empty,
                    owners,
                    ..
                } => {
                    files.iter().map(|(n, _, _)| n.len() + 80).sum::<usize>()
                        + excluded
                            .iter()
                            .chain(empty)
                            .map(|n| n.len() + 24)
                            .sum::<usize>()
                        + owners
                            .iter()
                            .map(|(n, k, _)| n.len() + k.len() + 64)
                            .sum::<usize>()
                }
                Key::Evidence {
                    versions,
                    domain,
                    clause,
                    name,
                    definition,
                    ..
                } => {
                    definition.len()
                        + clause.len()
                        + name.len()
                        + domain
                            .as_ref()
                            .map_or(0, |paths| paths.iter().map(|p| p.len() + 24).sum::<usize>())
                        + versions
                            .0
                            .iter()
                            .map(|(p, l, _, m, _)| {
                                p.len() + l.len() + m.as_ref().map_or(0, String::len) + 160
                            })
                            .sum::<usize>()
                }
                Key::Reach {
                    roots,
                    edges,
                    inputs,
                    ..
                } => {
                    roots.len()
                        + edges.len()
                        + inputs.iter().map(|(n, _)| n.len() + 48).sum::<usize>()
                }
            } + std::mem::size_of::<Key>()
                + 64,
        );
        self.next_id += 1;
        let id = self.next_id;
        self.definitions.insert(key, id);
        id
    }
    pub fn source(&mut self, spec: &syntax::Spec, files: &syntax::Files, family: u8) -> usize {
        let mut files = files
            .iter()
            .map(|(path, facts)| {
                let (lang, digest) = facts.version();
                (path.clone(), lang, digest)
            })
            .collect::<Vec<_>>();
        files.sort();
        let mut excluded = spec.exclude_kinds.clone();
        excluded.sort();
        let mut empty = spec.empty_text_kinds.clone();
        empty.sort();
        self.intern(Key::Syntax {
            family,
            files,
            excluded,
            empty,
            owners: spec
                .owners
                .iter()
                .map(|(name, o)| (name.clone(), o.kind.clone(), o.outermost))
                .collect(),
        })
    }
    pub fn evidence(
        &mut self,
        rule: &CompiledCodeRule,
        clause: &str,
        name: &str,
        versions: &Versions,
        domain: &Option<Vec<String>>,
        nonce: usize,
    ) -> usize {
        self.intern(Key::Evidence {
            definition: Arc::clone(&rule.canonical),
            clause: clause.into(),
            name: name.into(),
            versions: versions.clone(),
            domain: domain.clone(),
            nonce,
        })
    }
    pub fn stages(
        &mut self,
        plan: &Plan,
        sources: &BTreeMap<String, usize>,
        regex_limit: usize,
    ) -> BTreeMap<String, usize> {
        let mut bindings = sources.clone();
        for stage in &plan.stages {
            let mut inputs = stage
                .inputs
                .iter()
                .map(|name| (name.clone(), bindings[name]))
                .collect::<Vec<_>>();
            if stage.reads_source {
                inputs.push(("__source_text".into(), sources["__source_text"]));
            }
            let key = if let Some(spec) = &stage.reachability {
                if let Some(syntax) = &spec.syntax {
                    inputs.push(("__syntax_tree".into(), sources[syntax]));
                }
                Key::Reach {
                    roots: spec.roots.clone(),
                    edges: spec.edges.clone(),
                    forest: spec.forest,
                    expand: spec.expand,
                    inputs,
                }
            } else {
                Key::Sql {
                    query: Arc::from(stage.query.as_str()),
                    schema: Arc::from(
                        stage
                            .declaration
                            .split_once('(')
                            .expect("generated declaration")
                            .1,
                    ),
                    deferred: stage
                        .deferred
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    inputs,
                    regex_limit,
                    indexes: stage.indexes.clone(),
                }
            };
            let id = self.intern(key);
            bindings.insert(stage.name.clone(), id);
        }
        bindings
    }
}

pub(super) fn physical(id: usize) -> String {
    format!("__fact_{id}")
}

pub(super) fn resolve(query: &str, aliases: &BTreeMap<String, String>) -> String {
    if aliases.is_empty() {
        return query.to_owned();
    }
    let declarations = aliases
        .iter()
        .map(|(name, physical)| {
            format!(
                "{} AS NOT MATERIALIZED (SELECT * FROM {})",
                super::quote(name),
                super::quote(physical)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("WITH RECURSIVE {declarations} SELECT * FROM ({query})")
}

pub(super) fn stage(stage: &Stage, name: &str, aliases: &BTreeMap<String, String>) -> Stage {
    let mut out = stage.clone();
    out.name = name.into();
    out.query = resolve(&stage.query, aliases);
    let columns = stage
        .declaration
        .split_once('(')
        .expect("generated declaration")
        .1;
    out.declaration = format!(
        "CREATE TEMP TABLE {} ({columns}",
        super::quote(&super::deferred::physical_name(&out))
    )
    .into();
    if let Some(spec) = &mut out.reachability {
        spec.roots = resolve(&spec.roots, aliases);
        if spec.syntax.is_none() {
            spec.edges = resolve(&spec.edges, aliases);
        }
    }
    out
}
