//! Full producer definitions identify shared work. Names and hashes alone never establish equality.
use super::{Plan, Stage, syntax};
use crate::code_rule_dsl::model::{CompiledCodeRule, FileInput, Snapshot};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[cfg(feature = "code-rule-profile")]
mod capture;
#[cfg(feature = "code-rule-profile")]
pub(super) use capture::save as capture;

type EvidenceVersion = (String, String, [u8; 32], Option<String>, Option<[u8; 32]>);

/// Authoritative producer context is independent of whether source strings are resident.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Versions(Arc<VersionSet>);
#[derive(Eq, PartialEq)]
struct VersionSet {
    files: Box<[EvidenceVersion]>,
    fingerprint: [u8; 32],
    bytes: usize,
}
impl PartialEq for Versions {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}
impl Eq for Versions {}
impl Hash for Versions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.fingerprint.hash(state);
    }
}
#[derive(Default)]
struct IdentityHasher(Sha256);
impl Hasher for IdentityHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    fn finish(&self) -> u64 {
        u64::from_le_bytes(self.0.clone().finalize()[..8].try_into().unwrap())
    }
}
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
        let bytes = versions
            .iter()
            .map(|(p, l, _, m, _)| {
                p.len()
                    + l.len()
                    + m.as_ref().map_or(0, String::len)
                    + std::mem::size_of::<EvidenceVersion>()
            })
            .sum::<usize>()
            + std::mem::size_of::<VersionSet>();
        let mut hash = IdentityHasher::default();
        versions.hash(&mut hash);
        Self(Arc::new(VersionSet {
            files: versions.into(),
            fingerprint: hash.0.finalize().into(),
            bytes,
        }))
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

#[derive(Clone, Eq, PartialEq)]
enum Key {
    Layout(String),
    Syntax {
        family: u8,
        files: Vec<(String, u32, [u8; 32])>,
        excluded: Vec<String>,
        empty: Vec<String>,
        owners: Vec<(String, String, bool)>,
    },
    Evidence {
        definition: Arc<str>,
        definition_digest: String,
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
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Layout(value) => value.hash(state),
            Self::Syntax {
                family,
                files,
                excluded,
                empty,
                owners,
            } => (family, files, excluded, empty, owners).hash(state),
            Self::Evidence {
                definition_digest,
                clause,
                name,
                versions,
                domain,
                nonce,
                ..
            } => (definition_digest, clause, name, versions, domain, nonce).hash(state),
            Self::Sql {
                query,
                schema,
                deferred,
                inputs,
                regex_limit,
                indexes,
            } => (query, schema, deferred, inputs, regex_limit, indexes).hash(state),
            Self::Reach {
                roots,
                edges,
                forest,
                expand,
                inputs,
            } => (roots, edges, forest, expand, inputs).hash(state),
        }
    }
}
#[derive(Default)]
pub(super) struct Bindings {
    templates: Option<Box<Bindings>>,
    definitions: HashMap<Key, usize>,
    retained: HashSet<(u8, usize)>,
    next_lease: usize,
    next_id: usize,
    bytes: usize,
}
impl Bindings {
    pub fn bytes(&self) -> usize {
        self.bytes + self.templates.as_ref().map_or(0, |t| t.bytes())
    }
    pub fn data_bytes(&self) -> usize {
        self.bytes
    }
    pub fn forget(&mut self) {
        self.definitions.clear();
        self.retained.clear();
        self.bytes = 0;
    }
    pub fn lease(&mut self) -> usize {
        self.next_lease += 1;
        self.next_lease
    }
    fn owner(&mut self, tag: u8, pointer: usize, bytes: usize) -> usize {
        if self.retained.insert((tag, pointer)) {
            bytes + 48
        } else {
            0
        }
    }
    fn intern(&mut self, key: Key) -> usize {
        if let Some(id) = self.definitions.get(&key) {
            return *id;
        }
        let added = match &key {
            Key::Layout(value) => value.len(),
            Key::Sql {
                query,
                schema,
                inputs,
                indexes,
                deferred,
                ..
            } => {
                self.owner(0, Arc::as_ptr(query) as *const () as usize, query.len())
                    + self.owner(0, Arc::as_ptr(schema) as *const () as usize, schema.len())
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
                definition_digest,
                ..
            } => {
                self.owner(
                    0,
                    Arc::as_ptr(definition) as *const () as usize,
                    definition.len(),
                ) + self.owner(1, Arc::as_ptr(&versions.0) as usize, versions.0.bytes)
                    + definition_digest.len()
                    + clause.len()
                    + name.len()
                    + domain
                        .as_ref()
                        .map_or(0, |paths| paths.iter().map(|p| p.len() + 24).sum::<usize>())
            }
            Key::Reach {
                roots,
                edges,
                inputs,
                ..
            } => {
                roots.len() + edges.len() + inputs.iter().map(|(n, _)| n.len() + 48).sum::<usize>()
            }
        } + std::mem::size_of::<Key>()
            + 64;
        self.bytes = self.bytes.saturating_add(added);
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
            definition_digest: rule.digest.clone(),
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
    pub fn layouts(
        &mut self,
        rule: &CompiledCodeRule,
        clause: &str,
        plan: &Plan,
        limit: usize,
    ) -> BTreeMap<String, usize> {
        let t = self.templates.get_or_insert_with(Default::default);
        let versions = Versions::new(std::iter::empty());
        let mut sources = BTreeMap::new();
        for (name, _) in &plan.inputs {
            sources.insert(
                name.clone(),
                t.evidence(rule, clause, name, &versions, &None, 0),
            );
        }
        if plan.references.is_some() {
            sources.insert(
                "reference_edges".into(),
                t.evidence(rule, clause, "reference_edges", &versions, &None, 0),
            );
        }
        if plan.reads_source {
            sources.insert(
                "__source_text".into(),
                t.evidence(rule, clause, "__source_text", &versions, &None, 0),
            );
        }
        if let Some(spec) = &plan.syntax {
            let owners = spec
                .owners
                .iter()
                .map(|(n, o)| (n, &o.kind, o.outermost))
                .collect::<Vec<_>>();
            for (family, name) in [
                (0, Some(&spec.name)),
                (1, spec.edges.as_ref()),
                (2, spec.ownership.as_ref()),
            ] {
                if let Some(name) = name {
                    let definition = serde_json::json!([
                        family,
                        spec.fields,
                        spec.columns,
                        spec.exclude_kinds,
                        spec.empty_text_kinds,
                        owners
                    ])
                    .to_string();
                    sources.insert(name.clone(), t.intern(Key::Layout(definition)));
                }
            }
        }
        t.stages(plan, &sources, limit)
    }
}

pub(super) fn layout(id: usize) -> String {
    format!("__context_{id}")
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

#[cfg(test)]
#[path = "shared/identity_tests.rs"]
mod identity_tests;
