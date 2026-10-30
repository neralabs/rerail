//! Ingest as narrow fact families with derivation rows (05 Phase 2; 03 §1, §2, §14; 06 S3). Per
//! file of a scope there are four derivations, and no per-node rows anywhere:
//!
//! - `{scope}#tape:{path}` reads the path's name (the blob observed there), encodes the blob's tape
//!   and stores it (`tape_objects`), and outputs the tape's canonical hash. Content another scope
//!   already encoded is decoded from the store, not parsed again.
//! - `{scope}#core:{path}` reads the tape name and outputs the digest of the file's core facts
//!   (`core::digest`), built from that tape.
//! - `{scope}#prefilter:{path}` reads the tape name, publishes the tape's prefilter rows
//!   (`prefilter::image`, keyed by `tape_id`), and outputs a digest of the tape and the rows' image
//!   version: what `prefilter::select` reads, unchanged while the tape is.
//! - `{scope}#kinds:{path}` reads the core name and outputs the digest of the file's kind and class
//!   histogram. An edit that keeps every kind's count (a rename, whitespace, a literal) rebinds
//!   nothing here, so readers of the histogram never run.
//!
//! A tape encoded in the tape wave is kept, by its hash, until its two readers (core, prefilter)
//! take it; facts are kept, by core digest, from the core wave until the kinds wave takes them. A
//! reader that finds neither decodes the stored tape. New paths are pushed in chunks (`push`), so
//! a cold scan holds a chunk's tapes, not the scope's.

use super::Graph;
use super::core::{self, Facts};
use super::observe::{Push, observe_root, path_name};
use super::propagate::propagate;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label, label_of_path, tape_tag};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::tape::{self, FileImage, TAPE_FORMAT_VERSION, TapeDoc, TapeImage};
use crate::code_rule_dsl::workspace::grammar_version;
use crate::content_store::prefilter::{PREFILTER_VERSION, TapePrefilter};
use crate::content_store::{
    ABSENT, Derivation, DerivationInput, DerivationRecord, HashKey, SourceId, StoreError, StoreResult, TapeId,
};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const PATH: &str = "#path:";
const TAPE: &str = "#tape:";
const CORE: &str = "#core:";
const PREFILTER: &str = "#prefilter:";
const KINDS: &str = "#kinds:";
/// A tape's readers in the next wave: core and prefilter.
const TAPE_READERS: u8 = 2;
/// New paths per chunk of a push: a chunk's tapes and facts are resident at once.
const CHUNK: usize = 256;

pub(crate) fn tape_name(scope: &str, path: &str) -> String {
    format!("{scope}{TAPE}{path}")
}

pub(crate) fn prefilter_name(scope: &str, path: &str) -> String {
    format!("{scope}{PREFILTER}{path}")
}

pub(crate) fn core_name(scope: &str, path: &str) -> String {
    format!("{scope}{CORE}{path}")
}

pub(crate) fn kinds_name(scope: &str, path: &str) -> String {
    format!("{scope}{KINDS}{path}")
}

/// The producer of tape derivations: the tape format and the grammars that encode it.
pub(crate) fn tape_producer() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.tape/v1\0");
    hash.update(TAPE_FORMAT_VERSION.to_le_bytes());
    hash.update(grammar_version().to_le_bytes());
    hash.finalize().into()
}

/// The producer of core derivations: the tape format the facts are read from.
pub(crate) fn core_producer() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.core/v2\0");
    hash.update(TAPE_FORMAT_VERSION.to_le_bytes());
    hash.finalize().into()
}

/// The producer of prefilter derivations: the image version of the rows.
pub(crate) fn prefilter_producer() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.prefilter/v1\0");
    hash.update(PREFILTER_VERSION.to_le_bytes());
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

/// The source a path name's blob (a Git blob id in its first 20 bytes) is stored as.
fn blob_source_id(graph: &(impl Graph + ?Sized), digest: [u8; 32]) -> StoreResult<Option<SourceId>> {
    let blob: [u8; 20] = digest[..20].try_into().expect("twenty bytes");
    graph.store().source_by_alias(HashKey::git_sha1(blob))
}

/// The source of the blob a path name binds, as stored.
pub(crate) fn blob_source(graph: &(impl Graph + ?Sized), digest: [u8; 32]) -> StoreResult<Option<Arc<String>>> {
    let Some(source) = blob_source_id(graph, digest)? else {
        return Ok(None);
    };
    let bytes = graph.store().get_source(source)?;
    Ok(String::from_utf8(bytes).ok().map(Arc::new))
}

/// A path's dialect, where it has one with a tape.
fn dialect(path: &str) -> Option<GrammarDialect> {
    label_of_path(path).and_then(dialect_by_label).filter(|language| tape_tag(*language).is_some())
}

/// A stored tape, decoded.
pub(crate) struct Tape {
    pub id: TapeId,
    pub doc: TapeDoc,
    pub lines: Arc<LineTable>,
}

/// What one push's recomputation of the families did: tapes encoded from source, stored tapes
/// decoded, and tapes or facts a later wave took from an earlier one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Parses {
    pub parsed: u64,
    pub decoded: u64,
    pub reused: u64,
}

/// What one ingest push did: the observer and propagation (`Push`), and the derivations created
/// for paths no derivation read before.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Ingested {
    pub push: Push,
    pub created: u64,
}

/// Recomputes the ingest families for one push. Drop the value after the push.
#[derive(Default)]
pub(crate) struct Families {
    tapes: HashMap<[u8; 32], (Arc<Tape>, u8)>,
    facts: HashMap<[u8; 32], Arc<Facts>>,
    /// Prefilter rows built in this recomputation, published together.
    rows: Vec<TapePrefilter>,
    pub parses: Parses,
}

impl Families {
    /// One push of `scope`'s root `new`: observe it, propagate every rebound name through the
    /// derivations that read it, then create the four derivations of each new path with a tape
    /// dialect, a chunk of paths at a time, wave by wave (tape; core and prefilter; kinds). Cold
    /// and warm are this same call: cold every path is new. `others` re-runs each wave's
    /// derivations of other producers (the readers of these families).
    pub(crate) fn push(
        &mut self,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        new: HashKey,
        max_waves: u32,
        mut others: impl FnMut(&[Derivation]) -> StoreResult<Vec<DerivationRecord>>,
    ) -> StoreResult<Ingested> {
        graph.begin_push()?;
        let (changed, trees_read) = observe_root(graph, scope, new)?;
        let rebound = changed.len();
        let paths = changed
            .iter()
            .filter_map(|name| name.strip_prefix(scope)?.strip_prefix(PATH))
            .filter(|path| dialect(path).is_some())
            .collect::<Vec<_>>();
        let read = graph
            .derivations_reading(&changed)?
            .into_iter()
            .filter(|derivation| derivation.producer == tape_producer())
            .filter_map(|derivation| Some(derivation.output.strip_prefix(scope)?.strip_prefix(TAPE)?.to_owned()))
            .collect::<HashSet<_>>();
        let fresh = paths.into_iter().filter(|path| !read.contains(*path)).map(str::to_owned).collect::<Vec<_>>();
        let work = propagate(graph, changed, max_waves, |frontier| {
            let mut records = self.recompute(graph, frontier)?;
            records.extend(others(frontier)?);
            Ok(records)
        })?;
        let mut created = 0;
        for chunk in fresh.chunks(CHUNK) {
            let tapes = chunk.iter().map(|path| self.tape(graph, scope, path)).collect::<StoreResult<Vec<_>>>()?;
            graph.record_derivations(&tapes)?;
            let mut readers = Vec::with_capacity(2 * chunk.len());
            for path in chunk {
                readers.push(self.core(graph, scope, path)?.0);
                readers.push(self.prefilter(graph, scope, path)?);
            }
            self.publish(graph)?;
            graph.record_derivations(&readers)?;
            let kinds = chunk.iter().map(|path| self.kinds(graph, scope, path)).collect::<StoreResult<Vec<_>>>()?;
            graph.record_derivations(&kinds)?;
            created += (tapes.len() + readers.len() + kinds.len()) as u64;
        }
        graph.end_push()?;
        Ok(Ingested { push: Push { trees_read, rebound, work }, created })
    }

    /// Re-runs the ingest derivations of `frontier`; other producers' are left out.
    pub(crate) fn recompute(
        &mut self,
        graph: &(impl Graph + ?Sized),
        frontier: &[Derivation],
    ) -> StoreResult<Vec<DerivationRecord>> {
        let (tape, core, prefilter, kinds) = (tape_producer(), core_producer(), prefilter_producer(), kinds_producer());
        let mut records = Vec::new();
        for derivation in frontier {
            if derivation.producer == tape {
                let (scope, path) = split(&derivation.output, TAPE)?;
                records.push(self.tape(graph, scope, path)?);
            } else if derivation.producer == core {
                let (scope, path) = split(&derivation.output, CORE)?;
                records.push(self.core(graph, scope, path)?.0);
            } else if derivation.producer == prefilter {
                let (scope, path) = split(&derivation.output, PREFILTER)?;
                records.push(self.prefilter(graph, scope, path)?);
            } else if derivation.producer == kinds {
                let (scope, path) = split(&derivation.output, KINDS)?;
                records.push(self.kinds(graph, scope, path)?);
            }
        }
        self.publish(graph)?;
        Ok(records)
    }

    /// The tape derivation of `path` as its name is bound now. The output is `ABSENT` where the
    /// path is unbound, its blob is not stored as UTF-8, its language has no tape, or the parser
    /// stopped.
    pub(crate) fn tape(&mut self, graph: &(impl Graph + ?Sized), scope: &str, path: &str) -> StoreResult<DerivationRecord> {
        let name = path_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        let tape = match (read != ABSENT, dialect(path)) {
            (true, Some(language)) => self.encode(graph, read, language)?,
            _ => None,
        };
        let digest = tape.as_ref().map_or(ABSENT, |(digest, _)| *digest);
        if let Some((digest, tape)) = tape {
            self.tapes.insert(digest, (Arc::new(tape), TAPE_READERS));
        }
        Ok(DerivationRecord {
            output: tape_name(scope, path),
            producer: tape_producer(),
            inputs: vec![DerivationInput { name: Some(name), digest: read }],
            digest,
            status: 0,
            cost: 1,
        })
    }

    /// The tape of the blob `blob` binds, stored, and its canonical hash: decoded where the store
    /// has it, else encoded and stored.
    fn encode(
        &mut self,
        graph: &(impl Graph + ?Sized),
        blob: [u8; 32],
        language: GrammarDialect,
    ) -> StoreResult<Option<([u8; 32], Tape)>> {
        let store = graph.store();
        let Some(source_id) = blob_source_id(graph, blob)? else {
            return Ok(None);
        };
        let tag = i64::from(tape_tag(language).expect("a tape dialect"));
        let version = i64::from(TAPE_FORMAT_VERSION);
        if let Some(id) = store.tape_by_source(source_id, tag, grammar_version(), version)? {
            return self.decode(graph, source_id, id, language);
        }
        let Ok(source) = String::from_utf8(store.get_source(source_id)?) else {
            return Ok(None);
        };
        let source = Arc::new(source);
        let grammar = tape::Grammar::new(language).map_err(StoreError::Invalid)?;
        self.parses.parsed += 1;
        let Some(image) = TapeImage::parse(&grammar, &source, &|| false) else {
            return Ok(None);
        };
        let image = image.map_err(StoreError::Invalid)?;
        // A derived document's tape is not stored here: the slice's dialect (Python) has none.
        let bytes = image.primary.to_bytes().map_err(StoreError::Invalid)?;
        let id = store.put_tape(source_id, tag, grammar_version(), version, &bytes)?;
        let lines = Arc::new(LineTable::new(&source, Terminators::of(language)));
        let doc = TapeDoc::restore_with_lines(source, grammar, &image, Arc::clone(&lines), &|| false)
            .map_err(StoreError::Invalid)?;
        Ok(Some((Sha256::digest(&bytes).into(), Tape { id, doc, lines })))
    }

    /// The stored tape `id` of source `source_id`, decoded, and its canonical hash.
    fn decode(
        &mut self,
        graph: &(impl Graph + ?Sized),
        source_id: SourceId,
        id: TapeId,
        language: GrammarDialect,
    ) -> StoreResult<Option<([u8; 32], Tape)>> {
        let store = graph.store();
        let Ok(source) = String::from_utf8(store.get_source(source_id)?) else {
            return Ok(None);
        };
        let source = Arc::new(source);
        let Some((bytes, _)) = store.get_tapes_bulk(&[id])?.remove(&id) else {
            return Ok(None);
        };
        let grammar = tape::Grammar::new(language).map_err(StoreError::Invalid)?;
        self.parses.decoded += 1;
        let primary = TapeImage::from_bytes(&bytes, &grammar).map_err(StoreError::Invalid)?;
        let image = FileImage { primary, derived: Vec::new() };
        let lines = Arc::new(LineTable::new(&source, Terminators::of(language)));
        let doc = TapeDoc::restore_with_lines(source, grammar, &image, Arc::clone(&lines), &|| false)
            .map_err(StoreError::Invalid)?;
        Ok(Some((Sha256::digest(&bytes[..]).into(), Tape { id, doc, lines })))
    }

    /// The tape `read` names, for one of its readers: taken from the tape wave, else decoded from
    /// the store. A tape that is not the one `read` names means the tape derivation is itself
    /// stale; it re-runs and rebinds, and the reader re-runs after it.
    fn take(&mut self, graph: &(impl Graph + ?Sized), scope: &str, path: &str, read: [u8; 32]) -> StoreResult<Option<Arc<Tape>>> {
        if read == ABSENT {
            return Ok(None);
        }
        if let Some((tape, readers)) = self.tapes.get_mut(&read) {
            let tape = Arc::clone(tape);
            *readers -= 1;
            if *readers == 0 {
                self.tapes.remove(&read);
            }
            self.parses.reused += 1;
            return Ok(Some(tape));
        }
        let (Some(blob), Some(language)) = (graph.binding(&path_name(scope, path))?, dialect(path)) else {
            return Ok(None);
        };
        let Some(source_id) = blob_source_id(graph, blob)? else {
            return Ok(None);
        };
        let tag = i64::from(tape_tag(language).expect("a tape dialect"));
        let Some(id) = graph.store().tape_by_source(source_id, tag, grammar_version(), i64::from(TAPE_FORMAT_VERSION))? else {
            return Ok(None);
        };
        Ok(self
            .decode(graph, source_id, id, language)?
            .filter(|(digest, _)| *digest == read)
            .map(|(_, tape)| Arc::new(tape)))
    }

    /// The tape `path`'s tape name binds now, decoded from the store: what a reader of the
    /// families outside a push (a wave) reads.
    pub(crate) fn stored(graph: &(impl Graph + ?Sized), scope: &str, path: &str) -> StoreResult<Option<Arc<Tape>>> {
        let read = graph.binding(&tape_name(scope, path))?.unwrap_or(ABSENT);
        Self::default().take(graph, scope, path, read)
    }

    /// The core derivation of `path`, from the tape its tape name binds now, and its facts
    /// (`None` where there is no tape: the output is then `ABSENT`).
    pub(crate) fn core(
        &mut self,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        path: &str,
    ) -> StoreResult<(DerivationRecord, Option<Arc<Facts>>)> {
        let name = tape_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        let facts = match self.take(graph, scope, path, read)? {
            Some(tape) => Some(Arc::new(core::from_tape(&tape.doc, Arc::clone(&tape.lines)).map_err(StoreError::Invalid)?)),
            None => None,
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

    /// The prefilter derivation of `path`: the rows of the tape its tape name binds now, built
    /// here and published by `publish` before the record is.
    pub(crate) fn prefilter(&mut self, graph: &(impl Graph + ?Sized), scope: &str, path: &str) -> StoreResult<DerivationRecord> {
        let name = tape_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        let digest = match self.take(graph, scope, path, read)? {
            Some(tape) => {
                let budget = crate::code_rule_dsl::budget::matcher_budget_exhausted;
                self.rows.push(crate::code_rule_dsl::prefilter::image(tape.id, &tape.doc, &budget)?);
                let mut hash = Sha256::new();
                hash.update(b"rerail.prefilter-rows/v1\0");
                hash.update(PREFILTER_VERSION.to_le_bytes());
                hash.update(read);
                hash.finalize().into()
            }
            None => ABSENT,
        };
        Ok(DerivationRecord {
            output: prefilter_name(scope, path),
            producer: prefilter_producer(),
            inputs: vec![DerivationInput { name: Some(name), digest: read }],
            digest,
            status: 0,
            cost: 1,
        })
    }

    /// Publishes the prefilter rows built since the last call, 64 tapes per transaction.
    fn publish(&mut self, graph: &(impl Graph + ?Sized)) -> StoreResult<()> {
        for sets in self.rows.chunks(64) {
            graph.store().put_prefilters_bulk(sets)?;
        }
        self.rows.clear();
        Ok(())
    }

    /// The kinds derivation of `path`, from the facts its core name binds now.
    pub(crate) fn kinds(&mut self, graph: &(impl Graph + ?Sized), scope: &str, path: &str) -> StoreResult<DerivationRecord> {
        let name = core_name(scope, path);
        let read = graph.binding(&name)?.unwrap_or(ABSENT);
        // Kinds is the core facts' only reader, so it takes them: the cache holds a wave's facts,
        // not a push's, and a cold scan stays within the memory budget.
        let facts = match self.facts.remove(&read) {
            Some(facts) => {
                self.parses.reused += 1;
                Some(facts)
            }
            None if read == ABSENT => None,
            // Not built in this push: rebuild from the stored tape. Facts that are not the ones the
            // core name binds mean the core derivation is itself stale; it re-runs and rebinds, and
            // this derivation re-runs after it.
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
