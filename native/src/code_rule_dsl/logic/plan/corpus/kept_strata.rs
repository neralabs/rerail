//! The cross-file part's strata kept across processes (decision 0077 §4), for a run with no session: what a session
//! keeps in memory (`retain`), a run from the command line keeps in the tape cache.
//!
//! A run after an edit reads every file's exports from the memo, but its cross-file part derived everything again:
//! 23 s of the Python pack's 51 s after a one-line edit. Most strata read a relation the edit changed (`node`, the
//! edited file's exports), so a cut made before the run (0077 §2: a stratum runs when an input it reads changed, or a
//! relation of a stratum that runs) runs 815 of the pack's 1,556 strata, 21.7 of 24.4 s. But most strata that run
//! derive the rows they had: an appended line changes no import, no class, no call of another file. So the cut is
//! made as the run goes, stratum by stratum (`plan::Hooks::strata`), with each stratum's rows' digest: a stratum
//! runs when a relation it reads has rows other than the last run's (an input's digest, or a relation of a stratum
//! that ran and came out different), and a stratum that comes out the same leaves its readers clean (early cutoff).
//! On the pack that is 351 strata run, 16.6 s.
//!
//! Kept as the store's frames (`content_store::frames`, decision 0104), in the corpus root's cross-file fact space:
//! each derived relation under its identity (`plan::identities`: its rules, and what they read, down to the
//! inputs), with its digest, its rows and the digest each relation its stratum read had then. A stratum is clean
//! when its relations are kept under their identities and each read has the digest it had. So a relation two
//! programs share is one frame, whichever ran last, and a rule edit makes only the relations it reaches new. A clean
//! relation's rows are read back only when a stratum that runs reads it, or the run's end keeps it (an output): on
//! the pack, 68 k rows of 695 relations, against 3.0 M rows derived. A run writes the relations whose rows changed.

use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::plan::KeptStrata;
use crate::content_store::{ContentStore, FramePut, FrameRel, FrameRows, FrameStored};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// Rows (values) buffered before a write.
const FLUSH_VALUES: usize = 1 << 22;

/// The cross-file fact space of the corpus at `root`.
pub(in crate::code_rule_dsl) fn space(root: &Path) -> [u8; 32] {
    part_space(root, "cross")
}

/// The fact space of a part of the run over `root` (`cross`, `cone`: the cone of requests answered in the fixpoint).
pub(in crate::code_rule_dsl) fn part_space(root: &Path, part: &str) -> [u8; 32] {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut h = Sha256::new();
    h.update(format!("rerail.space/v1\0{part}\0").as_bytes());
    h.update(root.to_string_lossy().as_bytes());
    h.finalize().into()
}

pub(in crate::code_rule_dsl) struct Kept<'s> {
    store: &'s ContentStore,
    space: [u8; 32],
    /// Each relation of the program, as its frame names it (`program`).
    rels: HashMap<String, FrameRel>,
    /// What is stored of those relations.
    was: HashMap<String, FrameStored>,
    /// The inputs bound before the run, and those bound as their first reader runs (`loaded`, by the caller).
    inputs: HashMap<String, Arc<dyn Source>>,
    pub(in crate::code_rule_dsl) loaded: Rc<RefCell<HashMap<String, Arc<dyn Source>>>>,
    digests: HashMap<String, u64>,
    puts: Vec<FramePut>,
    values: usize,
    gone: Vec<(FrameRel, i64)>,
    pub(in crate::code_rule_dsl) written_rows: usize,
    /// The strata the run evaluated, of all (`KeptStrata::counted`).
    pub(in crate::code_rule_dsl) strata: Option<(usize, usize)>,
}

impl<'s> Kept<'s> {
    pub(in crate::code_rule_dsl) fn new(store: &'s ContentStore, space: [u8; 32], inputs: &[(String, Arc<dyn Source>)]) -> Self {
        Kept {
            store,
            space,
            rels: HashMap::new(),
            was: HashMap::new(),
            inputs: inputs.iter().map(|(r, s)| (r.clone(), Arc::clone(s))).collect(),
            loaded: Default::default(),
            digests: HashMap::new(),
            puts: Vec::new(),
            values: 0,
            gone: Vec::new(),
            written_rows: 0,
            strata: None,
        }
    }

    fn flush(&mut self, used: Option<Vec<FrameRel>>) {
        let (puts, gone) = (std::mem::take(&mut self.puts), std::mem::take(&mut self.gone));
        self.values = 0;
        if !puts.is_empty() || !gone.is_empty() || used.is_some() {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "keep the strata's frames, queued".into());
            self.store.queue_frames(self.space, puts, gone, used);
        }
    }

    /// Writes what is left, and marks the program's relations used by this run.
    pub(in crate::code_rule_dsl) fn finish(mut self) {
        if self.rels.is_empty() {
            return;
        }
        let used: Vec<FrameRel> = self.rels.values().cloned().collect();
        self.flush(Some(used));
    }
}

impl KeptStrata for Kept<'_> {
    fn program(&mut self, identities: &HashMap<String, [u8; 32]>) {
        self.rels = identities.iter().map(|(name, ident)| (name.clone(), FrameRel { name: name.clone(), ident: *ident })).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the strata kept, from the store".into());
        let rels: Vec<FrameRel> = self.rels.values().cloned().collect();
        let stored = self.store.frames(&self.space, &rels, 0).unwrap_or_default();
        self.was = stored.into_iter().map(|(rel, s)| (rel.name, s)).collect();
    }

    fn input(&mut self, rel: &str) -> Option<u64> {
        if let Some(d) = self.digests.get(rel) {
            return Some(*d);
        }
        let source = self.inputs.get(rel).cloned().or_else(|| self.loaded.borrow().get(rel).cloned())?;
        let d = super::retain::digest(source.as_ref());
        self.digests.insert(rel.to_owned(), d);
        Some(d)
    }

    fn was(&self, rel: &str) -> Option<u64> {
        self.was.get(rel).map(|w| w.digest)
    }

    fn read_was(&self, rel: &str, read: &str) -> Option<u64> {
        self.was.get(rel)?.reads.iter().find(|(r, _)| r == read).map(|(_, d)| *d)
    }

    fn has_rows(&self, rel: &str) -> bool {
        self.was.contains_key(rel)
    }

    fn rows(&mut self, rel: &str) -> Option<(Vec<u8>, Vec<i64>, usize)> {
        let frame = self.rels.get(rel)?;
        self.store.frame_rows(&self.space, frame, 0).ok().flatten().map(|k| (k.types, k.rows, k.n))
    }

    fn counted(&mut self, evaluated: usize, of: usize) {
        self.strata = Some((evaluated, of));
    }

    fn ran(&mut self, rel: &str, got: Option<(u64, Vec<u8>, Vec<i64>, usize, Vec<(String, u64)>)>) {
        let Some(frame) = self.rels.get(rel).cloned() else { return };
        match got {
            Some((digest, types, rows, n, reads)) => {
                if self.was.get(rel).is_some_and(|w| w.digest == digest && w.reads == reads) {
                    return;
                }
                self.values += rows.len();
                self.written_rows += n;
                self.puts.push(FramePut { rel: frame, part: 0, digest, reads, rows: FrameRows { types, rows, n } });
                if self.values >= FLUSH_VALUES {
                    self.flush(None);
                }
            }
            None => {
                if self.was.contains_key(rel) {
                    self.gone.push((frame, 0));
                }
            }
        }
    }
}
