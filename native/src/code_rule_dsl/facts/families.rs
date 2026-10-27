//! Core facts as narrow fact families with derivation rows (05 Phase 2; 03 §1, §2, §14). Per file
//! of a scope there are two derivations, and no per-node rows anywhere:
//!
//! - `{scope}#core:{path}` reads the path's name (the blob observed there) and outputs the digest of
//!   the file's core facts (`core::digest`): the columns, sidecars and source, a function of the
//!   content alone, so it changes on every content change.
//! - `{scope}#kinds:{path}` reads the core name and outputs the digest of the file's kind and class
//!   histogram. An edit that keeps every kind's count (a rename, whitespace, a literal) rebinds
//!   nothing here, so readers of the histogram (the prefilter, Phase 3) never run.
//!
//! The facts themselves are rebuilt from the tape on demand (`core::build`) and cached for one
//! push, by core digest, so the kinds wave reads the facts the core wave built.

use super::Graph;
use super::core::{self, Built, Facts};
use super::observe::path_name;
use crate::code_rule_dsl::language::label_of_path;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION;
use crate::content_store::{ABSENT, Derivation, DerivationInput, DerivationRecord, HashKey, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

const CORE: &str = "#core:";
const KINDS: &str = "#kinds:";

pub(crate) fn core_name(scope: &str, path: &str) -> String {
    format!("{scope}{CORE}{path}")
}

pub(crate) fn kinds_name(scope: &str, path: &str) -> String {
    format!("{scope}{KINDS}{path}")
}

/// The producer of core derivations: the tape format the facts are read from.
pub(crate) fn core_producer() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.core/v1\0");
    hash.update(TAPE_FORMAT_VERSION.to_le_bytes());
    hash.finalize().into()
}

pub(crate) fn kinds_producer() -> [u8; 32] {
    Sha256::digest(b"rerail.kinds/v1\0").into()
}

/// A file's (kind atom, count) and (class atom, count), each sorted by atom.
pub(crate) fn histogram(f: &Facts) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
    let counts = |keys: &[u32], offsets: &[u32]| {
        keys.iter().enumerate().map(|(i, &key)| (key, offsets[i + 1] - offsets[i])).collect::<Vec<_>>()
    };
    (counts(&f.kind_keys, &f.kind_offsets), counts(&f.class_keys, &f.class_offsets))
}

pub(crate) fn histogram_digest(f: &Facts) -> [u8; 32] {
    let (kinds, classes) = histogram(f);
    let mut hash = Sha256::new();
    hash.update(b"rerail.kind-histogram/v1\0");
    hash.update([u8::from(f.syntax_errors)]);
    for table in [kinds, classes] {
        hash.update((table.len() as u64).to_le_bytes());
        for (key, count) in table {
            hash.update(key.to_le_bytes());
            hash.update(count.to_le_bytes());
        }
    }
    hash.finalize().into()
}

/// The source of the blob a path name binds (a Git blob id in its first 20 bytes), as stored.
pub(crate) fn blob_source(graph: &(impl Graph + ?Sized), digest: [u8; 32]) -> StoreResult<Option<Arc<String>>> {
    let store = graph.store();
    let blob: [u8; 20] = digest[..20].try_into().expect("twenty bytes");
    let Some(source) = store.source_by_alias(HashKey::git_sha1(blob))? else {
        return Ok(None);
    };
    let bytes = store.get_source(source)?;
    Ok(String::from_utf8(bytes).ok().map(Arc::new))
}

/// What one push's recomputation of the families did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Parses {
    pub parsed: u64,
    pub reused: u64,
}

/// Recomputes core and kinds derivations for one push. Facts built by the core wave are kept, by
/// core digest, for the kinds wave; drop the value after the push.
#[derive(Default)]
pub(crate) struct Families {
    facts: HashMap<[u8; 32], Arc<Facts>>,
    pub parses: Parses,
}

impl Families {
    /// Re-runs the core and kinds derivations of `frontier`; other producers' are left out.
    pub(crate) fn recompute(
        &mut self,
        graph: &(impl Graph + ?Sized),
        frontier: &[Derivation],
    ) -> StoreResult<Vec<DerivationRecord>> {
        let (core, kinds) = (core_producer(), kinds_producer());
        let mut records = Vec::new();
        for derivation in frontier {
            if derivation.producer == core {
                let (scope, path) = split(&derivation.output, CORE)?;
                records.push(self.core(graph, scope, path)?.0);
            } else if derivation.producer == kinds {
                let (scope, path) = split(&derivation.output, KINDS)?;
                records.push(self.kinds(graph, scope, path)?);
            }
        }
        Ok(records)
    }

    /// The core derivation of `path` as its name is bound now, and its facts (`None` where the
    /// path is unbound, its blob is not stored as UTF-8, its language is unknown, or the parser
    /// stopped: the output is then `ABSENT`).
    pub(crate) fn core(
        &mut self,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        path: &str,
    ) -> StoreResult<(DerivationRecord, Option<Arc<Facts>>)> {
        let name = path_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        let facts = match (read != ABSENT, label_of_path(path)) {
            (true, Some(language)) => match blob_source(graph, read)? {
                Some(source) => {
                    let file = FileInput::new(path.to_owned(), language.to_owned(), source, None, None, "core")
                        .map_err(|e| StoreError::Invalid(e.reason.to_string()))?;
                    self.parses.parsed += 1;
                    match core::build(&file).map_err(StoreError::Invalid)? {
                        Built::Facts(facts) => Some(Arc::new(facts)),
                        Built::Unparsed => None,
                    }
                }
                None => None,
            },
            _ => None,
        };
        let digest = facts.as_deref().map_or(ABSENT, core::digest);
        if let Some(facts) = &facts {
            self.facts.insert(digest, Arc::clone(facts));
        }
        let record = DerivationRecord {
            output: core_name(scope, path),
            producer: core_producer(),
            inputs: vec![DerivationInput { name: Some(name), digest: read }],
            digest,
            status: 0,
            cost: 1,
        };
        Ok((record, facts))
    }

    /// The kinds derivation of `path`, from the facts its core name binds now.
    pub(crate) fn kinds(&mut self, graph: &(impl Graph + ?Sized), scope: &str, path: &str) -> StoreResult<DerivationRecord> {
        let name = core_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        let facts = match self.facts.get(&read) {
            Some(facts) => {
                self.parses.reused += 1;
                Some(Arc::clone(facts))
            }
            None if read == ABSENT => None,
            // Not built in this push: rebuild from the path. Facts that are not the ones the core
            // name binds mean the core derivation is itself stale; it re-runs and rebinds, and this
            // derivation re-runs after it.
            None => {
                let (_, facts) = self.core(graph, scope, path)?;
                facts.filter(|facts| core::digest(facts) == read)
            }
        };
        Ok(DerivationRecord {
            output: kinds_name(scope, path),
            producer: kinds_producer(),
            inputs: vec![DerivationInput { name: Some(name), digest: read }],
            digest: facts.as_deref().map_or(ABSENT, histogram_digest),
            status: 0,
            cost: 1,
        })
    }
}

fn split<'a>(output: &'a str, family: &str) -> StoreResult<(&'a str, &'a str)> {
    output
        .split_once(family)
        .ok_or_else(|| StoreError::Corrupt(format!("{output} is not a {family} derivation")))
}

#[cfg(test)]
mod tests;
