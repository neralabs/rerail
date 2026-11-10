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
//! Kept in the store, per program (the cross-file program's key, the engine build, and the program as the plan's
//! whole-program choices left it): each input's digest; each derived relation's digest and rows. A clean relation's
//! rows are read back only when a stratum that runs reads it, or the run's end keeps it (an output): on the pack,
//! 68 k rows of 695 relations, against 3.0 M rows derived. A run writes the relations whose rows changed.
//!
//! The inputs' digests are written last: a run that stops after writing some relations leaves no input digest, so
//! the next run finds nothing clean, rather than rows of other inputs.

use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::plan::KeptStrata;
use crate::content_store::{ContentStore, KeptPut, KeptRows};
use sha2::{Digest as _, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Rows (values) buffered before a write.
const FLUSH_VALUES: usize = 1 << 22;

pub(in crate::code_rule_dsl) struct Kept<'s> {
    store: &'s ContentStore,
    /// The cross-file program's key (`retain::key`); with the program run, this run's key (`prog`).
    base: [u8; 32],
    prog: Option<[u8; 32]>,
    /// What the last run of `prog` kept: each relation's digest, and whether its rows are kept.
    was: HashMap<String, (u64, bool)>,
    /// The inputs bound before the run, and those bound as their first reader runs (`loaded`, by the caller).
    inputs: HashMap<String, Arc<dyn Source>>,
    pub(in crate::code_rule_dsl) loaded: Rc<RefCell<HashMap<String, Arc<dyn Source>>>>,
    digests: HashMap<String, u64>,
    puts: Vec<KeptPut>,
    values: usize,
    gone: Vec<String>,
    /// Whether this run has written (the last run's input digests are dropped first).
    wrote: bool,
    pub(in crate::code_rule_dsl) written_rows: usize,
    /// The strata the run evaluated, of all (`KeptStrata::counted`).
    pub(in crate::code_rule_dsl) strata: Option<(usize, usize)>,
}

impl<'s> Kept<'s> {
    pub(in crate::code_rule_dsl) fn new(store: &'s ContentStore, base: [u8; 32], inputs: &[(String, Arc<dyn Source>)]) -> Self {
        Kept {
            store,
            base,
            prog: None,
            was: HashMap::new(),
            inputs: inputs.iter().map(|(r, s)| (r.clone(), Arc::clone(s))).collect(),
            loaded: Default::default(),
            digests: HashMap::new(),
            puts: Vec::new(),
            values: 0,
            gone: Vec::new(),
            wrote: false,
            written_rows: 0,
            strata: None,
        }
    }

    fn flush(&mut self) {
        let Some(prog) = self.prog else { return };
        if !self.wrote {
            // The last run's input digests go first: until this run's are written, nothing is clean.
            self.gone.extend(self.was.iter().filter(|(_, (_, rows))| !rows).map(|(r, _)| r.clone()));
            self.wrote = true;
        }
        let (puts, gone) = (std::mem::take(&mut self.puts), std::mem::take(&mut self.gone));
        self.values = 0;
        if !puts.is_empty() || !gone.is_empty() {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "keep the strata's rows".into());
            let _ = self.store.keep_strata(&prog, &puts, &gone);
        }
    }

    /// Writes what is left, and every input's digest.
    pub(in crate::code_rule_dsl) fn finish(mut self) {
        if self.prog.is_none() {
            return;
        }
        // Every input this run bound, digested whether or not a stratum asked.
        let loaded: Vec<(String, Arc<dyn Source>)> = self.loaded.borrow().iter().map(|(r, s)| (r.clone(), Arc::clone(s))).collect();
        for (rel, source) in self.inputs.clone().into_iter().chain(loaded) {
            if !self.digests.contains_key(&rel) {
                self.digests.insert(rel, super::retain::digest(source.as_ref()));
            }
        }
        self.flush();
        let digests = std::mem::take(&mut self.digests);
        self.puts = digests.into_iter().map(|(rel, digest)| KeptPut { rel, digest, rows: None }).collect();
        self.flush();
    }
}

impl KeptStrata for Kept<'_> {
    fn program(&mut self, digest: &[u8; 32]) {
        let mut hash = Sha256::new();
        hash.update(b"rerail.kept-strata/v1\0");
        hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
        hash.update(self.base);
        hash.update(digest);
        let prog: [u8; 32] = hash.finalize().into();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the strata kept, from the store".into());
        self.was = self.store.kept_strata(&prog).unwrap_or_default();
        self.prog = Some(prog);
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
        self.was.get(rel).map(|w| w.0)
    }

    fn has_rows(&self, rel: &str) -> bool {
        self.was.get(rel).is_some_and(|w| w.1)
    }

    fn rows(&mut self, rel: &str) -> Option<(Vec<u8>, Vec<i64>, usize)> {
        let prog = self.prog?;
        self.store.kept_strata_rows(&prog, rel).ok().flatten().map(|k| (k.types, k.rows, k.n))
    }

    fn counted(&mut self, evaluated: usize, of: usize) {
        self.strata = Some((evaluated, of));
    }

    fn ran(&mut self, rel: &str, got: Option<(u64, Vec<u8>, Vec<i64>, usize)>) {
        match got {
            Some((digest, types, rows, n)) => {
                if self.was.get(rel) == Some(&(digest, true)) {
                    return;
                }
                self.values += rows.len();
                self.written_rows += n;
                self.puts.push(KeptPut { rel: rel.to_owned(), digest, rows: Some(KeptRows { types, rows, n }) });
                if self.values >= FLUSH_VALUES {
                    self.flush();
                }
            }
            None => {
                if self.was.contains_key(rel) {
                    self.gone.push(rel.to_owned());
                }
            }
        }
    }
}
