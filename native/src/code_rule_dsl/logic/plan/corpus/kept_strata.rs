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
use crate::content_store::{AdmittedFrame, ContentStore, FramePut, FrameRel, FrameRows, FrameStored};
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
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut h = Sha256::new();
    h.update(b"rerail.space/v1\0cross\0");
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
    /// Encoded rows admitted before scheduling. Reading a kept relation never calls SQLite.
    admitted: HashMap<String, Arc<AdmittedFrame>>,
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
            admitted: HashMap::new(),
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
        self.was = self.store.frames(&self.space, &rels, 0).unwrap_or_default().into_iter().map(|(r, s)| (r.name, s)).collect();
    }

    fn admit(&mut self, relations: &[String]) {
        let rels: Vec<_> = relations.iter().filter_map(|r| self.rels.get(r).cloned()).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "admit demanded retained facts".into());
        let stored = self.store.admit_frames(&self.space, &rels, 0).unwrap_or_default();
        // Metadata and payload are separate batches. A concurrent publication must never let
        // the scheduler use old metadata with new rows. A miss or change conservatively derives again.
        if rels.iter().any(|r| stored.get(r).map(|f| &f.stored) != self.was.get(&r.name)) {
            self.was.clear();
            self.admitted.clear();
            crate::code_rule_dsl::logic::profile::decision("fact admission", || "frames changed during admission; derive again".into());
            return;
        }
        crate::code_rule_dsl::logic::profile::decision("fact admission", || format!(
            "{} retained relations, {} demanded payloads, {} admitted, {} encoded bytes; consumers read memory",
            self.was.len(), rels.len(), stored.len(), stored.values().map(|f| f.encoded_bytes()).sum::<usize>()));
        self.admitted = stored.into_iter().map(|(rel, f)| (rel.name, f)).collect();
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
        self.admitted.get(rel)?.rows().ok().map(|k| (k.types, k.rows, k.n))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::{lower, native, plan, rr};

    #[test]
    fn an_unchanged_native_plan_reuses_facts_and_admits_only_consumed_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("facts.sqlite");
        let text = ".decl e(a:number,b:number)\ne(1,2).\ne(2,3).\ne(3,4).\n\
            .decl r(a:number,b:number)\nr(a,b) :- e(a,b).\nr(a,c) :- r(a,b),e(b,c).\n\
            .decl total(n:number)\ntotal(n) :- n=count : {r(_,_)}.\n.output total\n";
        let run = || {
            let store = ContentStore::open(&path).unwrap();
            let p = rr::parse(text).unwrap();
            let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
            let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| plan::Error::Plan(e.to_string()))).unwrap();
            let db = rusqlite::Connection::open(&path).unwrap();
            native::register(&db).unwrap();
            native::register_symbols(&db).unwrap();
            let mut kept = Kept::new(&store, [42; 32], &[]);
            let outcome = plan::run_atoms_native_hooked(&lowered, &db, "main", plan::Stats::empty(&lowered.program),
                Default::default(), "program", plan::Hooks { strata: Some(&mut kept), ..Default::default() }).unwrap();
            let counts = kept.strata.unwrap();
            let admitted: Vec<_> = kept.admitted.keys().cloned().collect();
            kept.finish();
            (counts, admitted, outcome.outputs)
        };
        let first = run();
        assert!(first.0.0 > 0, "cold native evaluation must do work");
        let warm = run();
        assert_eq!(warm.2, first.2);
        assert_eq!(warm.0.0, 0, "an unchanged native transformation must not execute");
        assert!(!warm.1.contains(&"r".to_owned()), "the retained closure has no running consumer and must not be hydrated: {:?}", warm.1);
        assert!(warm.1.contains(&"total".to_owned()), "the final output is demanded");
    }

    #[test]
    fn publication_between_metadata_and_payload_admission_invalidates_the_cut() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
        let rel = FrameRel { name: "r".into(), ident: [1; 32] };
        let put = |v| FramePut { rel: rel.clone(), part: 0, digest: v as u64, reads: vec![],
            rows: FrameRows { types: vec![0], rows: vec![v], n: 1 } };
        store.keep_frames(&[42; 32], &[put(1)], &[], None).unwrap();
        let mut kept = Kept::new(&store, [42; 32], &[]);
        kept.program(&HashMap::from([("r".into(), [1; 32])]));
        assert!(kept.has_rows("r"));
        store.keep_frames(&[42; 32], &[put(2)], &[], None).unwrap();
        kept.admit(&["r".into()]);
        assert!(!kept.has_rows("r"), "old metadata must never certify replacement rows");
    }
}
