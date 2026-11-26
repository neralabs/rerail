//! The prefilter's rows as kept facts of the store (t33 steps 6 and 7): what `prefilter_sets`, `tape_kind_counts`,
//! `term_postings`, `term_coverage` and the store-wide counts (`kind_size`, `term_kind_freq`, `term_node_freq`,
//! `term_freq`) held, read in place by the set route and, through the virtual tables of the same names
//! (`super::vtab`), by the SQL route.
//!
//! - **A tape's rows**: one partition a tape (its `tape_id`), put whole when it is published, gone in the
//!   transaction that deletes the tape's row (its id may be given again). `pf_set(version, nodes, lang)`,
//!   `pf_kind(kind, n, size, at, len)`, `pf_term(term, kind, n, at, len)`, `pf_cover(kind, bad, residue, at, len)`;
//!   `at` and `len` name a slice of the tape's byte heap, `pf_bytes(at, word)`: a posting's bytes (the delta-varint
//!   or container segment its readers decode, `nera_postings`, `nera_segment`), each starting on a word.
//! - **The store-wide counts**: an aggregate of the tapes' rows, counted in runs of deltas (partitions below 0, the
//!   tapes' are above): each publication or collection writes one run, the counts its tapes add less those of the
//!   tapes it replaces or deletes, and a run merges with the runs before it while they are at most twice its size
//!   (a cold run of N files writes O(N log N) rows; a reader adds O(log N) runs). `pf_run(lo, hi, rows)`: the
//!   publications it holds; `pf_kind_sum(kind, n, size, covered, residue)`; `pf_term_sum(lang, term, kind, n, rows)`, `rows` the (tape, kind)
//!   rows that carry the term.
//!   Written in the publication's own transaction, under the write lock: no count is lost to another process.
//!
//! Terms are atoms, as the store numbers them (the `atoms` table's ids), typed `s`.
use super::super::kept_facts::{self, ChainRows, KeptPut, KeptSpace, sorted_put};
use super::super::{StoreError, StoreResult};
use rerail_blocks::kept::{FactKey, Kept, KeptRead};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub const SET: &str = "pf_set";
pub const KIND: &str = "pf_kind";
pub const TERM: &str = "pf_term";
pub const COVER: &str = "pf_cover";
pub const BYTES: &str = "pf_bytes";
pub const RUN: &str = "pf_run";
pub const KIND_SUM: &str = "pf_kind_sum";
pub const TERM_SUM: &str = "pf_term_sum";

/// The relations of a tape's partition.
const TAPE_RELS: [&str; 5] = [SET, KIND, TERM, COVER, BYTES];
/// The relations of a run's partition.
const RUN_RELS: [&str; 3] = [RUN, KIND_SUM, TERM_SUM];

/// The prefilter's space.
pub fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.prefilter/v1").into()
}

pub fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.prefilter/v1\0{rel}").as_bytes()).into()
}

fn entry<'a, R: KeptRead + ?Sized>(view: &'a R, rel: &str, part: i64) -> Option<Kept<'a>> {
    let ident = ident(rel);
    view.get(&FactKey { rel, ident: &ident, part }).filter(|e| *e.key.ident == ident)
}

/// One tape's rows, where they lie.
#[derive(Clone, Copy)]
pub struct Tape<'a> {
    pub version: i64,
    pub nodes: i64,
    pub lang: i64,
    kinds: Option<Kept<'a>>,
    terms: Option<Kept<'a>>,
    covers: Option<Kept<'a>>,
    bytes: Option<Kept<'a>>,
}

/// The rows of `tape`, if it has them.
pub fn tape<R: KeptRead + ?Sized>(view: &R, tape: i64) -> Option<Tape<'_>> {
    let set = entry(view, SET, tape)?;
    if set.rows.len() != 1 || set.rows.arity < 3 {
        return None;
    }
    Some(Tape {
        version: set.rows.get(0, 0),
        nodes: set.rows.get(0, 1),
        lang: set.rows.get(0, 2),
        kinds: entry(view, KIND, tape),
        terms: entry(view, TERM, tape),
        covers: entry(view, COVER, tape),
        bytes: entry(view, BYTES, tape),
    })
}

/// A kind's row of a tape: its count and descendants, and its preorders' bytes (`at`, `len`).
#[derive(Clone, Copy, Debug)]
pub struct KindRow {
    pub kind: i64,
    pub n: i64,
    pub size: i64,
    pub at: i64,
    pub len: i64,
}

/// A term's row of a tape: its kind, its count and its preorders' bytes.
#[derive(Clone, Copy, Debug)]
pub struct TermRow {
    pub term: i64,
    pub kind: i64,
    pub n: i64,
    pub at: i64,
    pub len: i64,
}

/// A kind's coverage row of a tape: its flags, unclean leaves, and the bytes of the nodes no term covers.
#[derive(Clone, Copy, Debug)]
pub struct CoverRow {
    pub kind: i64,
    pub bad: i64,
    pub residue: i64,
    pub at: i64,
    pub len: i64,
}

fn rows_of(e: Option<Kept<'_>>) -> usize {
    e.map_or(0, |e| e.rows.len())
}

impl<'a> Tape<'a> {
    pub fn kinds(&self) -> impl Iterator<Item = KindRow> + 'a {
        let e = self.kinds;
        (0..rows_of(e)).map(move |i| {
            let r = e.expect("rows").rows;
            KindRow { kind: r.get(i, 0), n: r.get(i, 1), size: r.get(i, 2), at: r.get(i, 3), len: r.get(i, 4) }
        })
    }

    /// The tape's row of kind `kind`.
    pub fn kind(&self, kind: i64) -> Option<KindRow> {
        let r = self.kinds?.rows;
        let i = r.probe(&[0], &[kind]).next()?;
        Some(KindRow { kind, n: r.get(i, 1), size: r.get(i, 2), at: r.get(i, 3), len: r.get(i, 4) })
    }

    pub fn terms(&self) -> impl Iterator<Item = TermRow> + 'a {
        let e = self.terms;
        (0..rows_of(e)).map(move |i| term_row(e.expect("rows"), i))
    }

    /// The tape's rows of term `term` (one a kind).
    pub fn term(&self, term: i64) -> impl Iterator<Item = TermRow> + 'a {
        let e = self.terms;
        let found: Vec<usize> = e.map(|e| e.rows.probe(&[0], &[term]).collect()).unwrap_or_default();
        found.into_iter().map(move |i| term_row(e.expect("rows"), i))
    }

    pub fn covers(&self) -> impl Iterator<Item = CoverRow> + 'a {
        let e = self.covers;
        (0..rows_of(e)).map(move |i| {
            let r = e.expect("rows").rows;
            CoverRow { kind: r.get(i, 0), bad: r.get(i, 1), residue: r.get(i, 2), at: r.get(i, 3), len: r.get(i, 4) }
        })
    }

    /// Whether some node of the tape is covered by no term (a coverage row with bytes).
    pub fn has_uncovered(&self) -> bool {
        self.covers().any(|c| c.len > 0)
    }

    /// The `len` bytes at word `at` of the tape's heap.
    pub fn bytes(&self, at: i64, len: i64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len.max(0) as usize);
        self.with_bytes(at, len, |b| out.extend_from_slice(b));
        out
    }

    /// `f` given the `len` bytes at word `at` of the tape's heap, a word at a time.
    pub fn with_bytes(&self, at: i64, len: i64, mut f: impl FnMut(&[u8])) {
        let Some(e) = self.bytes else { return };
        let (Ok(at), Ok(mut left)) = (usize::try_from(at), usize::try_from(len)) else { return };
        let r = e.rows;
        let mut i = at;
        while left > 0 && i < r.len() {
            let word = r.get(i, 1).to_le_bytes();
            let n = left.min(8);
            f(&word[..n]);
            left -= n;
            i += 1;
        }
    }
}

fn term_row(e: Kept<'_>, i: usize) -> TermRow {
    let r = e.rows;
    TermRow { term: r.get(i, 0), kind: r.get(i, 1), n: r.get(i, 2), at: r.get(i, 3), len: r.get(i, 4) }
}

/// Every tape with rows, by id.
pub fn tapes<R: KeptRead + ?Sized>(view: &R) -> impl Iterator<Item = i64> + '_ {
    view.of(SET).map(|e| e.key.part).filter(|&p| p > 0)
}

/// One run of the counts.
#[derive(Clone, Copy)]
struct Run<'a> {
    part: i64,
    lo: i64,
    hi: i64,
    rows: i64,
    kinds: Option<Kept<'a>>,
    terms: Option<Kept<'a>>,
}

/// The store-wide counts: the runs, oldest first.
pub struct Sums<'a> {
    runs: Vec<Run<'a>>,
}

/// A kind's counts over every tape: its nodes, their descendants, the nodes terms cover, and its unclean leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindSum {
    pub n: i64,
    pub size: i64,
    pub covered: i64,
    pub residue: i64,
}

pub fn sums<R: KeptRead + ?Sized>(view: &R) -> Sums<'_> {
    let mut runs: Vec<Run<'_>> = view
        .of(RUN)
        .filter(|e| e.key.part < 0 && *e.key.ident == ident(RUN) && e.rows.len() == 1)
        .map(|e| Run {
            part: e.key.part,
            lo: e.rows.get(0, 0),
            hi: e.rows.get(0, 1),
            rows: e.rows.get(0, 2),
            kinds: entry(view, KIND_SUM, e.key.part),
            terms: entry(view, TERM_SUM, e.key.part),
        })
        .collect();
    runs.sort_by_key(|r| r.lo);
    Sums { runs }
}

impl<'a> Sums<'a> {
    /// The last publication the counts hold.
    pub fn head(&self) -> i64 {
        self.runs.iter().map(|r| r.hi).max().unwrap_or(0)
    }

    pub fn kind(&self, kind: i64) -> KindSum {
        let mut s = KindSum::default();
        for run in &self.runs {
            let Some(e) = run.kinds else { continue };
            for i in e.rows.probe(&[0], &[kind]) {
                s.n += e.rows.get(i, 1);
                s.size += e.rows.get(i, 2);
                s.covered += e.rows.get(i, 3);
                s.residue += e.rows.get(i, 4);
            }
        }
        s
    }

    /// Every kind's counts, in kind order (none with all 0).
    pub fn kinds(&self) -> Vec<(i64, KindSum)> {
        let runs: Vec<Kept<'a>> = self.runs.iter().filter_map(|r| r.kinds).collect();
        let mut out = Vec::new();
        merge::<5>(&runs, &[], 1, |r| out.push((r[0], KindSum { n: r[1], size: r[2], covered: r[3], residue: r[4] })));
        out
    }

    /// The nodes of kind `kind` whose text is term `term` of language `lang`, over every tape.
    pub fn term_nodes(&self, lang: i64, term: i64, kind: i64) -> i64 {
        self.runs.iter().filter_map(|r| r.terms).map(|e| e.rows.probe(&[0, 1, 2], &[lang, term, kind]).map(|i| e.rows.get(i, 3)).sum::<i64>()).sum()
    }

    /// Term `term` of `lang`'s nodes over every tape, by kind (the kinds with any).
    pub fn term_kinds(&self, lang: i64, term: i64) -> BTreeMap<i64, i64> {
        let mut by_kind: BTreeMap<i64, i64> = BTreeMap::new();
        for e in self.runs.iter().filter_map(|r| r.terms) {
            for i in e.rows.probe(&[0, 1], &[lang, term]) {
                *by_kind.entry(e.rows.get(i, 2)).or_default() += e.rows.get(i, 3);
            }
        }
        by_kind.retain(|_, n| *n != 0);
        by_kind
    }

    /// The (tape, kind) rows over every tape that carry term `term` of `lang` (`term_freq.n`).
    pub fn term_tape_rows(&self, lang: i64, term: i64) -> i64 {
        self.runs.iter().filter_map(|r| r.terms).map(|e| e.rows.probe(&[0, 1], &[lang, term]).map(|i| e.rows.get(i, 4)).sum::<i64>()).sum()
    }

    /// Whether some tape holds term `term` of `lang`.
    pub fn has_term(&self, lang: i64, term: i64) -> bool {
        !self.term_kinds(lang, term).is_empty()
    }

    /// The (lang, term, kind, nodes, tape rows) counts over every tape (of `lang`, or every language), in that order
    /// (`term_node_freq`, `term_freq`): the runs merged where they lie.
    pub fn term_rows(&self, lang: Option<i64>) -> Vec<[i64; 5]> {
        let runs: Vec<Kept<'a>> = self.runs.iter().filter_map(|r| r.terms).collect();
        let mut out = Vec::new();
        merge::<5>(&runs, &[], 3, |r| if lang.is_none_or(|l| r[0] == l) { out.push(*r) });
        out
    }

    /// The distinct terms of `lang` some tape holds, ascending.
    pub fn terms(&self, lang: i64) -> Vec<i64> {
        let mut terms: Vec<i64> = self.term_rows(Some(lang)).into_iter().map(|r| r[1]).collect();
        terms.dedup();
        terms
    }

    /// The terms of `lang` the runs past publication `upto` count (added, or counted down), ascending: the terms a
    /// key's matches brought up to date from `upto` test.
    pub fn terms_since(&self, lang: i64, upto: i64) -> Vec<i64> {
        let mut terms: Vec<i64> = Vec::new();
        for e in self.runs.iter().filter(|r| r.hi > upto).filter_map(|r| r.terms) {
            terms.extend(e.rows.probe(&[0], &[lang]).map(|i| e.rows.get(i, 1)));
        }
        terms.sort_unstable();
        terms.dedup();
        terms
    }
}

/// A tape's rows to publish: its set, and its kinds', terms' and coverage rows, each with its bytes.
pub struct TapeRows {
    pub version: i64,
    pub nodes: i64,
    pub lang: i64,
    /// (kind, n, size, preorders), by kind.
    pub kinds: Vec<(i64, i64, i64, Vec<u8>)>,
    /// (term, kind, n, preorders), by term then kind; terms as the store numbers them.
    pub terms: Vec<(i64, i64, i64, Vec<u8>)>,
    /// (kind, bad, residue, missing), by kind.
    pub covers: Vec<(i64, i64, i64, Vec<u8>)>,
}

/// Counts a change adds to the runs: by kind, and by (lang, term, kind).
#[derive(Default)]
pub struct Delta {
    pub kinds: BTreeMap<i64, KindSum>,
    /// (nodes, tape rows) by (lang, term, kind).
    pub terms: BTreeMap<(i64, i64, i64), [i64; 2]>,
}

impl Delta {
    fn add_kind(&mut self, kind: i64, s: KindSum, sign: i64) {
        let d = self.kinds.entry(kind).or_default();
        d.n += sign * s.n;
        d.size += sign * s.size;
        d.covered += sign * s.covered;
        d.residue += sign * s.residue;
    }

    /// The residue of `kind` it adds.
    pub fn residue(&self, kind: i64) -> i64 {
        self.kinds.get(&kind).map_or(0, |s| s.residue)
    }

    /// `tape`'s counts, kept, added with `sign`.
    fn add_kept(&mut self, tape: &Tape<'_>, sign: i64) {
        let mut covered: BTreeMap<i64, i64> = BTreeMap::new();
        for t in tape.terms() {
            let d = self.terms.entry((tape.lang, t.term, t.kind)).or_default();
            d[0] += sign * t.n;
            d[1] += sign;
            *covered.entry(t.kind).or_default() += t.n;
        }
        let mut residue: BTreeMap<i64, i64> = BTreeMap::new();
        for c in tape.covers().filter(|c| c.residue > 0) {
            *residue.entry(c.kind).or_default() += c.residue;
        }
        let mut kinds: BTreeMap<i64, KindSum> = BTreeMap::new();
        for k in tape.kinds() {
            let s = kinds.entry(k.kind).or_default();
            s.n += k.n;
            s.size += k.size;
        }
        for (kind, n) in covered {
            kinds.entry(kind).or_default().covered += n;
        }
        for (kind, n) in residue {
            kinds.entry(kind).or_default().residue += n;
        }
        for (kind, s) in kinds {
            self.add_kind(kind, s, sign);
        }
    }

    /// `rows`' counts, added.
    fn add_rows(&mut self, rows: &TapeRows) {
        let mut kinds: BTreeMap<i64, KindSum> = BTreeMap::new();
        for (kind, n, size, _) in &rows.kinds {
            let s = kinds.entry(*kind).or_default();
            s.n += n;
            s.size += size;
        }
        for (term, kind, n, _) in &rows.terms {
            let d = self.terms.entry((rows.lang, *term, *kind)).or_default();
            d[0] += n;
            d[1] += 1;
            kinds.entry(*kind).or_default().covered += n;
        }
        for (kind, _, residue, _) in rows.covers.iter().filter(|c| c.2 > 0) {
            kinds.entry(*kind).or_default().residue += residue;
        }
        for (kind, s) in kinds {
            self.add_kind(kind, s, 1);
        }
    }
}

fn puts_of_tape(tape: i64, rows: &TapeRows) -> StoreResult<Vec<KeptPut>> {
    // The heap: every posting's bytes, each from a word.
    let mut heap: Vec<i64> = Vec::new();
    let mut place = |bytes: &[u8]| -> (i64, i64) {
        let at = heap.len() as i64;
        for chunk in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..chunk.len()].copy_from_slice(chunk);
            heap.push(i64::from_le_bytes(w));
        }
        (at, bytes.len() as i64)
    };
    let kinds: Vec<[i64; 5]> = rows.kinds.iter().map(|(k, n, s, b)| { let (at, len) = place(b); [*k, *n, *s, at, len] }).collect();
    let terms: Vec<[i64; 5]> = rows.terms.iter().map(|(t, k, n, b)| { let (at, len) = place(b); [*t, *k, *n, at, len] }).collect();
    let covers: Vec<[i64; 5]> = rows.covers.iter().map(|(k, bad, r, b)| { let (at, len) = place(b); [*k, *bad, *r, at, len] }).collect();
    let put = |rel: &str, types: &[u8], n: usize, fill: &mut dyn FnMut(&mut dyn FnMut(i64))| {
        sorted_put(rel, ident(rel), tape, types, n, |push| {
            fill(push);
            Ok::<(), StoreError>(())
        })
    };
    Ok(vec![
        put(SET, b"iii", 1, &mut |push| [rows.version, rows.nodes, rows.lang].into_iter().for_each(&mut *push))?,
        put(KIND, b"iiiii", kinds.len(), &mut |push| kinds.iter().flatten().for_each(|&v| push(v)))?,
        put(TERM, b"siiii", terms.len(), &mut |push| terms.iter().flatten().for_each(|&v| push(v)))?,
        put(COVER, b"iiiii", covers.len(), &mut |push| covers.iter().flatten().for_each(|&v| push(v)))?,
        put(BYTES, b"ii", heap.len(), &mut |push| heap.iter().enumerate().for_each(|(i, &w)| { push(i as i64); push(w) }))?,
    ])
}

fn gone<'a>(rels: &'a [&'a str], part: i64) -> impl Iterator<Item = KeptPut> + 'a {
    rels.iter().map(move |rel| KeptPut { rel: (*rel).to_owned(), ident: ident(rel), part, digest: 0, reads: Vec::new(), types: Vec::new(), what: kept_facts::KeptWhat::Gone })
}

/// What a publication or a collection changes: each tape's new rows, or `None` for a tape that goes. `old` is the
/// space as the transaction finds it; `prior` gives, for a tape about to be counted, the delta so far (for the
/// residue cap, which reads the store-wide residue). Returns the puts and the delta they count.
pub struct Change {
    pub puts: Vec<KeptPut>,
    pub delta: Delta,
}

impl Change {
    pub fn new() -> Self {
        Change { puts: Vec::new(), delta: Delta::default() }
    }

    /// Tape `tape`'s rows as `old` keeps them counted out, and its partition gone (a deleted tape).
    pub fn remove<R: KeptRead + ?Sized>(&mut self, old: Option<&R>, tape: i64) {
        if let Some(t) = old.and_then(|v| self::tape(v, tape)) {
            self.delta.add_kept(&t, -1);
            self.puts.extend(gone(&TAPE_RELS, tape));
        }
    }

    /// Tape `tape`'s old rows counted out (`remove`, but its partition is put again), for `put`.
    pub fn replace<R: KeptRead + ?Sized>(&mut self, old: Option<&R>, tape: i64) {
        if let Some(t) = old.and_then(|v| self::tape(v, tape)) {
            self.delta.add_kept(&t, -1);
        }
    }

    /// Tape `tape`'s new rows, counted in and put whole.
    pub fn put(&mut self, tape: i64, rows: &TapeRows) -> StoreResult<()> {
        self.delta.add_rows(rows);
        self.puts.extend(puts_of_tape(tape, rows)?);
        Ok(())
    }

    /// The puts, with the run of the delta: written into transaction `t` of the store at `store`, `old` the space as
    /// `t` finds it. The chain's rows once written.
    pub fn write(mut self, store: &Path, t: &rusqlite::Connection, old: Option<&KeptSpace>) -> StoreResult<Option<ChainRows>> {
        let view = old.map(KeptSpace::view);
        let sums = view.as_ref().map(|v| sums(v));
        let head = sums.as_ref().map_or(0, Sums::head);
        let runs: Vec<Run<'_>> = sums.as_ref().map(|s| s.runs.clone()).unwrap_or_default();
        let new_kinds: Vec<[i64; 5]> = self.delta.kinds.iter().filter(|(_, s)| **s != KindSum::default()).map(|(k, s)| [*k, s.n, s.size, s.covered, s.residue]).collect();
        let new_terms: Vec<[i64; 5]> = self.delta.terms.iter().filter(|(_, d)| **d != [0, 0]).map(|((l, t, k), [n, rows])| [*l, *t, *k, *n, *rows]).collect();
        if !(new_kinds.is_empty() && new_terms.is_empty()) {
            // The runs it merges with: the newest while each is at most twice what is merged so far.
            let mut size = (new_kinds.len() + new_terms.len()) as i64;
            let mut merged: Vec<Run<'_>> = Vec::new();
            for run in runs.iter().rev() {
                if run.rows > 2 * size {
                    break;
                }
                size += run.rows;
                merged.push(*run);
            }
            let lo = merged.last().map_or(head + 1, |r| r.lo);
            let part = merged.last().map_or(-(head + 1), |r| r.part);
            for run in &merged {
                if run.part != part {
                    self.puts.extend(gone(&RUN_RELS, run.part));
                }
            }
            let kind_runs: Vec<Kept<'_>> = merged.iter().filter_map(|r| r.kinds).collect();
            let term_runs: Vec<Kept<'_>> = merged.iter().filter_map(|r| r.terms).collect();
            // Counted, then filled: the merged rows are written into the put's block as they are merged.
            let (mut n_kinds, mut n_terms) = (0usize, 0usize);
            merge::<5>(&kind_runs, &new_kinds, 1, |_| n_kinds += 1);
            merge::<5>(&term_runs, &new_terms, 3, |_| n_terms += 1);
            let rows = (n_kinds + n_terms) as i64;
            let put = |rel: &str, types: &[u8], n: usize, fill: &mut dyn FnMut(&mut dyn FnMut(i64))| {
                sorted_put(rel, ident(rel), part, types, n, |push| {
                    fill(push);
                    Ok::<(), StoreError>(())
                })
            };
            self.puts.push(put(RUN, b"iii", 1, &mut |push| [lo, head + 1, rows].into_iter().for_each(&mut *push))?);
            self.puts.push(put(KIND_SUM, b"iiiii", n_kinds, &mut |push| merge::<5>(&kind_runs, &new_kinds, 1, |row| row.iter().for_each(|&v| push(v))))?);
            self.puts.push(put(TERM_SUM, b"isiii", n_terms, &mut |push| merge::<5>(&term_runs, &new_terms, 3, |row| row.iter().for_each(|&v| push(v))))?);
        }
        drop(sums);
        drop(view);
        if self.puts.is_empty() {
            return Ok(None);
        }
        // Its terms are the store's ids already: nothing to map.
        kept_facts::write_space(store, t, &space(), self.puts, None)
    }
}

impl Default for Change {
    fn default() -> Self {
        Self::new()
    }
}

/// `runs`' rows of `W` columns merged with `new`, each sorted by its rows (canonical order), `new`'s keys distinct:
/// the rows of one key (the first `key` columns) summed, a row whose sums are all 0 dropped, given to `emit` in key
/// order. A k-way merge of the runs where they lie: nothing is staged.
fn merge<const W: usize>(runs: &[Kept<'_>], new: &[[i64; W]], key: usize, mut emit: impl FnMut(&[i64; W])) {
    let mut at = vec![0usize; runs.len()];
    let mut at_new = 0usize;
    let head = |r: usize, i: usize| -> [i64; W] { std::array::from_fn(|c| runs[r].rows.get(i, c)) };
    loop {
        // The least key among the heads.
        let mut least: Option<[i64; W]> = None;
        for (r, &i) in at.iter().enumerate() {
            if i < runs[r].rows.len() {
                let row = head(r, i);
                if least.is_none_or(|l| row[..key] < l[..key]) {
                    least = Some(row);
                }
            }
        }
        if let Some(row) = new.get(at_new) {
            if least.is_none_or(|l| row[..key] < l[..key]) {
                least = Some(*row);
            }
        }
        let Some(least) = least else { return };
        let mut sum = [0i64; W];
        sum[..key].copy_from_slice(&least[..key]);
        for (r, i) in at.iter_mut().enumerate() {
            while *i < runs[r].rows.len() {
                let row = head(r, *i);
                if row[..key] != least[..key] {
                    break;
                }
                (key..W).for_each(|c| sum[c] += row[c]);
                *i += 1;
            }
        }
        while let Some(row) = new.get(at_new).filter(|row| row[..key] == least[..key]) {
            (key..W).for_each(|c| sum[c] += row[c]);
            at_new += 1;
        }
        if sum[key..].iter().any(|&v| v != 0) {
            emit(&sum);
        }
    }
}
