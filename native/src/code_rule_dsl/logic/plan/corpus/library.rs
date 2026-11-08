//! A library's cross-file rows as a derivation (decision 0059, t21 E5).
//!
//! After the requests converge (`world`), the program runs once over the loaded documents alone, the library
//! stage (`world: "library"`): no workspace document, the engine's demand off, `stage("library")` given to a program
//! that declares it. The rows of every relation its cross-file part derives are its output ([`Rows`]), stored in the
//! content store under a key of what they depend on ([`key`]): the engine, the program, the settings, and each loaded
//! document's path and content. A later run with the same inputs reads them back.
//!
//! The workspace run binds each relation's library rows as an input `<rel>__library`, adds
//! `rel(…) :- rel__library(…)` to its cross-file part, and derives no row of that relation whose first node lies in a
//! library document ([`replay`]): those are the stage's, made once. A library never imports the workspace, so the
//! workspace adds no row about a library node that the stage, its demand open, has not made.
//!
//! Rows are replayable as they are: a document's key is a function of its path (`memo::doc_keys`), a node is
//! `key << 32 | index`, and a symbol is an atom of the persistent store (decision 0049). The rows carry the keys they
//! were made under; a run whose keys differ (a collision moved one) does not replay them.

use super::{Error, Result, front};
use crate::content_store::{ContentStore, HashKey};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The option naming the file of rows a workspace run replays.
pub(super) const OPTION: &str = "libraryRows";
/// The file a library stage writes its rows to, in its output directory.
pub(super) const FILE: &str = "library.rows";
/// The suffix of a relation's library rows, an input of the workspace run's cross-file part.
pub(super) const SUFFIX: &str = "__library";
/// The input naming the library documents' keys, which the head guards read.
pub(super) const DOCS: &str = "rr_library_doc";
/// The fact a library stage gives a program that declares it.
pub(super) const STAGE: &str = "stage";
/// The setting that turns replay off (`library.rows=off`): the run derives the library's rows itself.
pub(super) const SETTING: &str = "library.rows";
/// The encoding's version: bumped when its meaning changes.
const SCHEMA: u32 = 1;

/// A library stage's output: each library document's key, and each relation's width and rows (flat).
#[derive(Default, Debug, PartialEq)]
pub(super) struct Rows {
    pub(super) keys: BTreeMap<String, i64>,
    pub(super) rels: BTreeMap<String, (usize, Vec<i64>)>,
}

impl Rows {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let put_str = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        b.extend_from_slice(&SCHEMA.to_le_bytes());
        b.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        for (path, key) in &self.keys {
            put_str(&mut b, path);
            b.extend_from_slice(&key.to_le_bytes());
        }
        b.extend_from_slice(&(self.rels.len() as u32).to_le_bytes());
        for (rel, (width, values)) in &self.rels {
            put_str(&mut b, rel);
            b.extend_from_slice(&(*width as u32).to_le_bytes());
            b.extend_from_slice(&(values.len() as u64).to_le_bytes());
            for v in values {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        zstd::encode_all(b.as_slice(), 3).expect("zstd over memory")
    }

    pub(super) fn decode(bytes: &[u8]) -> Option<Rows> {
        let b = zstd::decode_all(bytes).ok()?;
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().expect("4 bytes"));
        if u32_(take(4)?) != SCHEMA {
            return None;
        }
        let mut rows = Rows::default();
        for _ in 0..u32_(take(4)?) {
            let n = u32_(take(4)?) as usize;
            let path = String::from_utf8(take(n)?.to_vec()).ok()?;
            let key = i64::from_le_bytes(take(8)?.try_into().ok()?);
            rows.keys.insert(path, key);
        }
        for _ in 0..u32_(take(4)?) {
            let n = u32_(take(4)?) as usize;
            let rel = String::from_utf8(take(n)?.to_vec()).ok()?;
            let width = u32_(take(4)?) as usize;
            let len = u64::from_le_bytes(take(8)?.try_into().ok()?) as usize;
            let raw = take(len.checked_mul(8)?)?;
            let values = raw.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().expect("8 bytes"))).collect();
            rows.rels.insert(rel, (width, values));
        }
        Some(rows)
    }
}

/// The stage's key: the engine build and grammar, the program's text (its settings included), and each loaded
/// document's path and content digest, in order.
pub(super) fn key(programs: &[&str], loaded: &[String]) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(b"rerail.library-rows/v1\0");
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    for p in programs {
        hash.update((p.len() as u64).to_le_bytes());
        hash.update(p.as_bytes());
    }
    let mut loaded = loaded.to_vec();
    loaded.sort();
    for path in &loaded {
        let bytes = std::fs::read(path).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(Sha256::digest(&bytes));
    }
    Ok(hash.finalize().into())
}

/// The derivation's output name for a key.
fn name(key: &[u8; 32]) -> String {
    format!("library-rows/{}", key.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The stored rows of `key`, if a stage stored them and the store still holds them.
pub(super) fn lookup(store: &ContentStore, key: &[u8; 32]) -> Option<Vec<u8>> {
    lookup_named(store, &name(key))
}

/// The stored bytes a derivation's output `name` binds.
pub(super) fn lookup_named(store: &ContentStore, name: &str) -> Option<Vec<u8>> {
    lookup_source(store, name).map(|(_, bytes)| bytes)
}

/// The stored source a derivation's output `name` binds, and its bytes.
pub(super) fn lookup_source(store: &ContentStore, name: &str) -> Option<(crate::content_store::SourceId, Vec<u8>)> {
    let digest = store.binding(name).ok()??;
    let source = store.source_by_hash(HashKey::from_sha256_digest(digest)).ok()??;
    store.get_source(source).ok().map(|bytes| (source, bytes))
}

/// Stores a stage's encoded rows under `key`, as the output of a derivation that read the loaded documents.
/// Stores `bytes` as the derivation `name` of `producer`, with no named input: `name` is a function of its inputs.
pub(super) fn record_named(store: &ContentStore, name: &str, producer: &[u8; 32], bytes: &[u8]) -> Result<crate::content_store::SourceId> {
    use crate::content_store::DerivationRecord;
    let plan = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let source = store.put_source(bytes, &[]).map_err(plan)?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let record = DerivationRecord { output: name.to_owned(), producer: *producer, inputs: Vec::new(), digest, status: 0, cost: 0 };
    store.record_derivations(&[record]).map_err(plan)?;
    Ok(source)
}

pub(super) fn record(store: &ContentStore, key: &[u8; 32], bytes: &[u8], loaded: &[String]) -> Result<()> {
    use crate::content_store::{DerivationInput, DerivationRecord};
    let plan = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    store.put_source(bytes, &[]).map_err(plan)?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let inputs = loaded.iter().map(|path| {
        let bytes = std::fs::read(path).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        Ok(DerivationInput { name: None, digest: Sha256::digest(&bytes).into() })
    });
    let inputs = inputs.collect::<Result<Vec<_>>>()?;
    let record = DerivationRecord { output: name(key), producer: *key, inputs, digest, status: 0, cost: 0 };
    store.record_derivations(&[record]).map_err(plan)?;
    Ok(())
}

/// The relations a program says a workspace may add rows about library nodes to (`.pragma "library-open" "a b …"`):
/// those its uses feed, such as the attributes a workspace looks up on a library class. Their rules stay unguarded.
pub(super) fn open_relations(prog: &front::Program) -> std::collections::BTreeSet<String> {
    prog.settings
        .iter()
        .filter(|(key, _)| key == "pragma")
        .filter_map(|(_, text)| {
            let words: Vec<&str> = text.split('"').map(str::trim).filter(|w| !w.is_empty()).collect();
            match words.as_slice() {
                ["library-open", rels] => Some(rels.split_whitespace().map(str::to_owned).collect::<Vec<_>>()),
                _ => None,
            }
        })
        .flatten()
        .collect()
}

/// Adds the library's rows to `cross`, the workspace run's cross-file part: for each relation it derives that has
/// rows, an input `<rel>__library` and the rule `rel(…) :- rel__library(…)`. Each other rule of such a relation, but
/// those of `open`, derives no row whose first and last node columns both lie in library documents: guarded by
/// `d = @doc(v), !rr_library_doc(d)` on its first node, or, with a last node of its own, split in two (first node in
/// the workspace; first in a library and last in the workspace), so `sub(k, s)` keeps a workspace subclass `s` of a
/// library class `k`. Returns the inputs to bind: name, width and rows, `rr_library_doc` (the documents' keys) first.
pub(super) fn replay(cross: &mut front::Program, rows: &Rows, open: &std::collections::BTreeSet<String>) -> Vec<(String, usize, Vec<i64>)> {
    use front::{Atom, Literal, Rule, Term};
    let derived: std::collections::BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.clone()).collect();
    let mut bind = vec![(DOCS.to_owned(), 1, rows.keys.values().copied().collect())];
    cross.decls.insert(DOCS.to_owned(), vec![("d".to_owned(), "int".to_owned())]);
    cross.inputs.push(DOCS.to_owned());
    let mut replayed = std::collections::BTreeSet::new();
    for (rel, (width, values)) in &rows.rels {
        let Some(cols) = cross.decls.get(rel).cloned() else { continue };
        if !derived.contains(rel) || cols.len() != *width || values.is_empty() {
            continue;
        }
        let input = format!("{rel}{SUFFIX}");
        cross.decls.insert(input.clone(), cols.clone());
        cross.inputs.push(input.clone());
        let vars: Vec<Term> = (0..*width).map(|i| Term::Var(format!("rr_l{i}"))).collect();
        cross.rules.push(Rule {
            head: Atom { rel: rel.clone(), args: vars.clone(), neg: false, gated: false },
            body: vec![Literal::Atom(Atom { rel: input.clone(), args: vars, neg: false, gated: false })],
        });
        bind.push((input, *width, values.clone()));
        replayed.insert(rel.clone());
    }
    let doc_of = |v: &str, d: &str| Literal::Cmp { l: Term::Var(d.to_owned()), op: "=", r: Term::Call("doc".to_owned(), vec![Term::Var(v.to_owned())]) };
    let library = |d: &str, neg: bool| Literal::Atom(Atom { rel: DOCS.to_owned(), args: vec![Term::Var(d.to_owned())], neg, gated: false });
    let mut out = Vec::with_capacity(cross.rules.len());
    for rule in std::mem::take(&mut cross.rules) {
        let copy = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.ends_with(SUFFIX)));
        if !replayed.contains(&rule.head.rel) || open.contains(&rule.head.rel) || copy {
            out.push(rule);
            continue;
        }
        let cols = &cross.decls[&rule.head.rel];
        let nodes: Vec<String> = cols
            .iter()
            .zip(&rule.head.args)
            .filter(|((_, t), _)| t == "node")
            .filter_map(|(_, a)| if let Term::Var(v) = a { Some(v.clone()) } else { None })
            .collect();
        match (nodes.first(), nodes.last()) {
            (Some(first), Some(last)) if first != last => {
                let (df, dl) = ("rr_ld_first".to_owned(), "rr_ld_last".to_owned());
                let mut a = rule.clone();
                a.body.extend([doc_of(first, &df), library(&df, true)]);
                let mut b = rule;
                b.body.extend([doc_of(first, &df), library(&df, false), doc_of(last, &dl), library(&dl, true)]);
                out.extend([a, b]);
            }
            (Some(first), _) => {
                let d = "rr_ld_first".to_owned();
                let mut a = rule;
                a.body.extend([doc_of(first, &d), library(&d, true)]);
                out.push(a);
            }
            _ => out.push(rule),
        }
    }
    cross.rules = out;
    bind
}

/// Writes a stage's rows to `dir`'s [`FILE`].
pub(super) fn write(dir: &Path, rows: &Rows) -> Result<()> {
    std::fs::write(dir.join(FILE), rows.encode()).map_err(|e| Error::Plan(format!("{}: {e}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip_through_their_encoding() {
        let mut rows = Rows::default();
        rows.keys.insert("/ts/stdlib/os/path.pyi".into(), 7);
        rows.rels.insert("found".into(), (3, vec![1, 2, 3, 4, 5, 6]));
        rows.rels.insert("empty".into(), (2, vec![]));
        assert_eq!(Rows::decode(&rows.encode()), Some(rows));
    }
}
