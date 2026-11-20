//! A negative membership test for a scan followed by a keyed probe (decision 0506): insurance against a join whose
//! probes mostly miss, cheap enough to be on wherever a walk has that shape.
//!
//! - The filter is the probe side's own, kept with its column as its rows were counted (`Sketch::keys`): a walk
//!   borrows it, and builds nothing. Only a probe side without one (a view, a relation with no sketches) has a
//!   filter built for its walk, from its current build range, within the old bounds.
//! - 64 probe keys sampled from the scan choose it (one hash each). A walk (each morsel) counts what it tests and
//!   backs off where a window of tests refuses little: the screen costs a hash a row only while it pays.
//! - Every hit is still probed exactly: a wrong choice costs time, never an answer.
//!
//! The filter is blocked (`logic::bloom`): a test is one hash, one load and one compare.
use super::{
    JoinRule, Key, Rows, Src, Step, Store,
    sketch::Sketch,
};
use crate::code_rule_dsl::logic::bloom::{self, Kept};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

const SAMPLES: usize = 64;
/// The fewest scan rows a walk is screened for (its 64 samples are a quarter of them at most).
const MIN_SCAN: usize = 256;
/// A walk's tests between its checks of what they refused, and the share under which it backs off.
const WINDOW: u64 = 1024;
const PAYS: u64 = 16;

pub(super) struct JoinFilter {
    /// The step screened (an atom after the scan), whether it is negated, and the slot its key is read from.
    pub(super) step: usize,
    pub(super) neg: bool,
    slot: usize,
    /// The scan's column the probe's key is bound from: a screen of a probe that is not negated selects the scan's
    /// rows by it (`Screening::select`).
    scan: usize,
    keys: Arc<Kept>,
    /// The allowance a filter built for its walk takes; none for a column's own.
    _reservation: Option<super::join_cache::Reservation>,
    /// The rows its walks tested and refused (`record`, as each walk or morsel ends).
    tested: AtomicU64,
    refused: AtomicU64,
    /// With `RERAIL_JOIN_SCREEN=trace` (a measurement aid): what it was chosen for, printed with its counts as it drops.
    trace: Option<String>,
}

impl Drop for JoinFilter {
    fn drop(&mut self) {
        if let Some(t) = &self.trace {
            eprintln!("join screen: {t}; {} rows tested, {} refused", self.tested.load(Relaxed), self.refused.load(Relaxed));
        }
    }
}

/// A walk's (or a morsel's) use of its screen: its counts, and how many probes it lets through untested while it
/// backs off. A window of `WINDOW` tests that refused under one in `PAYS` stops it for twice as many probes as the
/// last time (from `WINDOW` up to `WINDOW << 10`); a window that pays resets that.
#[derive(Default)]
pub(super) struct Screening {
    tested: u64,
    refused: u64,
    mark: u64,
    skip: u64,
    backoff: u64,
    /// The scan's rows were selected (`select`): every binding at the screened step passed, or went untested while
    /// the screen backed off.
    scanned: bool,
}

/// The keys a selection pass tests at once (`Bloom::may_hold_each`).
const BLOCK: usize = 64;

impl Screening {
    /// Whether `f` refuses the probe `slots` bind: then it has no rows. None once the scan's rows were selected.
    #[inline]
    pub(super) fn refuses(&mut self, f: &JoinFilter, slots: &[i64]) -> bool {
        if self.scanned {
            return false;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return false;
        }
        let refused = !f.keys.may_hold(slots[f.slot]);
        self.count(1, u64::from(refused));
        refused
    }

    /// A screened scan's candidates `ids` (decision 0506), the ones `f` passes, in order: their keys (`key`, the
    /// scan's column `JoinFilter::scan_col`) a block of 64 at a time, each block hashed in one loop and tested
    /// (`Kept::may_hold_each`). A selection pass, the device's shape, in place of a test a binding at the screened
    /// step. While the screen backs off, ids pass unread and untested.
    pub(super) fn select(&mut self, f: &JoinFilter, ids: impl Iterator<Item = u32>, key: impl Fn(u32) -> i64) -> Vec<u32> {
        self.scanned = true;
        let mut out = Vec::new();
        let (mut bid, mut bkey, mut n) = ([0u32; BLOCK], [0i64; BLOCK], 0);
        for id in ids {
            if self.skip > 0 {
                self.skip -= 1;
                out.push(id);
                continue;
            }
            bid[n] = id;
            bkey[n] = key(id);
            n += 1;
            if n == BLOCK {
                self.block(f, &bid, &bkey[..n], &mut out);
                n = 0;
            }
        }
        if n > 0 {
            self.block(f, &bid, &bkey[..n], &mut out);
        }
        out
    }

    fn block(&mut self, f: &JoinFilter, ids: &[u32; BLOCK], keys: &[i64], out: &mut Vec<u32>) {
        let held = f.keys.may_hold_each(keys);
        out.extend((0..keys.len()).filter(|&b| held >> b & 1 != 0).map(|b| ids[b]));
        self.count(keys.len() as u64, keys.len() as u64 - u64::from(held.count_ones()));
    }

    /// `tested` more tests, `refused` of them refused; at each window's end, whether to back off.
    fn count(&mut self, tested: u64, refused: u64) {
        let window = self.tested / WINDOW;
        self.tested += tested;
        self.refused += refused;
        if self.tested / WINDOW != window {
            if (self.refused - self.mark) * PAYS < self.tested - window * WINDOW {
                self.backoff = (self.backoff * 2).clamp(WINDOW, WINDOW << 10);
                self.skip = self.backoff;
            } else {
                self.backoff = 0;
            }
            self.mark = self.refused;
        }
    }
}

/// A step's column, read where it is: a relation's rows, or a view's (a tape's, read in place, decision 0353).
struct Column<'a> {
    rows: Held<'a>,
    column: usize,
    start: usize,
    len: usize,
    sketch: Option<&'a Sketch>,
    /// Its sketch, where its keys (`Sketch::filter`) hold every row a probe of it reads.
    keys: Option<&'a Sketch>,
}

enum Held<'a> {
    Rows(&'a Rows, usize),
    View(&'a super::InRel),
}

impl<'a> Column<'a> {
    fn of(src: &'a Src, store: &'a Store, column: usize) -> Option<Self> {
        let (rows, start, end, sketch, keys) = match src {
            Src::In(r, _) => match &r.view {
                // A whole view (no ids of its own): its rows are its source's, each read on the tape.
                Some(v) if v.ids.is_none() => (Held::View(r), 0, r.len(), None, None),
                Some(_) => return None,
                // A held relation's sketches, where it has them, count all its rows.
                None => {
                    let sketch = r.sketches.get(column).filter(|_| r.sketches.len() == r.k);
                    (Held::Rows(&r.rows, r.k), 0, r.n, sketch, sketch)
                }
            },
            Src::Full(n) | Src::FullMulti(n, _) | Src::Delta(n) => {
                let r = &store.rels[*n];
                // Its sketches count rows `..indexed`: all a probe of it reads once they are all indexed, and every
                // row of its delta (a wave's rows are indexed as its delta is set).
                let keys = r.sketches.get(column).filter(|_| r.indexed == r.n || matches!(src, Src::Delta(_)));
                if matches!(src, Src::Delta(_)) {
                    (Held::Rows(&r.data, r.k), r.lo, r.hi, None, keys)
                } else {
                    (Held::Rows(&r.data, r.k), 0, r.n, r.sketches.get(column), keys)
                }
            }
        };
        let keys = keys.filter(|_| bloom::kept_keys_read());
        Some(Self { rows, column, start, len: end - start, sketch, keys })
    }

    fn at(&self, i: usize) -> i64 {
        match self.rows {
            Held::Rows(rows, width) => super::RowRef::Rows(rows, (self.start + i) as u32, width).at(self.column),
            Held::View(rel) => rel.at(self.start + i).at(self.column),
        }
    }
}

impl JoinFilter {
    /// The screen for `j`'s walk, where one pays. `RERAIL_JOIN_SCREEN` (measurement aids): `off`, no screen, every
    /// probe made; `trace`, each walk's choice on stderr, why where none is made, and a screen's rows as it drops.
    pub(super) fn choose(j: &JoinRule, store: &Store) -> Option<Self> {
        let mode = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_JOIN_SCREEN").unwrap_or_default());
        if mode == "off" {
            return None;
        }
        let made = Self::consider(j, store, mode == "trace");
        if mode == "trace" && let Err(why) = &made {
            eprintln!("join screen refused: {why}");
        }
        made.ok().flatten()
    }

    fn consider(j: &JoinRule, store: &Store, trace: bool) -> Result<Option<Self>, &'static str> {
        // A scan first, then the first atom after it (the steps between are tests and derivations: they narrow the
        // rows, never rebind the scan's slots), probed by a column the scan binds. A miss on that one column is a
        // miss on the probe's whole key. A negated probe that misses holds, without its probe.
        let Some(Step::Atom { src: left, neg: false, keys, binds, .. }) = j.steps.first() else {
            return Err("the first step is not a scan");
        };
        if !keys.is_empty() {
            return Err("the first step is a probe, not a scan");
        }
        let Some((step, Step::Atom { src: right, neg, keys: probe, .. })) =
            j.steps.iter().enumerate().skip(1).find(|(_, s)| matches!(s, Step::Atom { .. }))
        else {
            return Err("no atom after the scan");
        };
        let Some((right_col, slot, left_col)) = probe.iter().find_map(|&(c, k)| match k {
            Key::Slot(s) => binds.iter().find(|&&(_, b)| b == s).map(|&(l, _)| (c, s, l)),
            _ => None,
        }) else {
            return Err("the probe is by no column the scan binds");
        };
        let outer = Column::of(left, store, left_col).ok_or("the scan is of a view with ids")?;
        let inner = Column::of(right, store, right_col).ok_or("the probe is of a view with ids")?;
        if outer.len < MIN_SCAN {
            return Err("under 256 scan rows");
        }
        // The probe side's own filter, else one built for this walk. A full one refuses little, and its samples say so.
        let (filter, reservation, kept) = match inner.keys.map(|sk| (sk, sk.filter())) {
            Some((_, Some(f))) => (Arc::clone(f), None, true),
            Some((sk, None)) => {
                let f = Kept::new(bloom::MIN_WORDS);
                for &(v, _) in sk.exact_counts().expect("a sketch without a filter has its few values").iter() {
                    f.add_hashed(bloom::hash(v));
                }
                (Arc::new(f), None, true)
            }
            None => {
                if outer.len < 4096 {
                    return Err("under 4,096 scan rows, and no filter kept");
                }
                if inner.len == 0 || inner.len > outer.len / 4 {
                    return Err("the probe side is empty or past a quarter of the scan, and no filter kept");
                }
                // The filter at its largest gives 16 bits a key to 8,192 keys.
                match inner.sketch {
                    Some(s) if s.ndv() > 8192 => return Err("the probe side has over 8,192 distinct keys"),
                    None if inner.len > 32_768 => return Err("the probe side, unsketched, is past 32,768 rows"),
                    _ => {}
                }
                let words = bloom::words_for(inner.sketch.map_or(inner.len, |s| s.ndv().min(inner.len)));
                let reservation = super::join_cache::Reservation::take(words * 8).ok_or("no allowance left")?;
                (Arc::new(Self::built(words, inner.len, |i| inner.at(i))), Some(reservation), false)
            }
        };
        let rejects = Self::sampled(&filter, outer.len, |i| outer.at(i));
        let overlap = outer.sketch.zip(inner.sketch).map(|(a, b)| a.overlap(b));
        use crate::code_rule_dsl::logic::plan::joins;
        let use_filter = joins::use_bloom(joins::BloomProbeInput { probe_rows: outer.len,
            build_rows: inner.len, distinct_overlap: overlap, sampled: SAMPLES, rejected: rejects, kept });
        let what = || format!(
            "{} probe rows, {} build rows ({}), HLL overlap {overlap:?}, {rejects}/{SAMPLES} sampled probes rejected, use filter {use_filter}",
            outer.len, inner.len, if kept { "its filter kept" } else { "a filter built" });
        crate::code_rule_dsl::logic::profile::decision("join Bloom filter", || what());
        if !use_filter {
            if trace {
                eprintln!("join screen: {}", what());
            }
            return Ok(None);
        }
        Ok(Some(Self { step, neg: *neg, slot, scan: left_col, keys: filter, _reservation: reservation, tested: AtomicU64::new(0), refused: AtomicU64::new(0),
                       trace: trace.then(what) }))
    }

    /// A filter of `words` words of the build side's `inner` keys.
    fn built(words: usize, inner: usize, key: impl Fn(usize) -> i64) -> Kept {
        let filter = Kept::new(words);
        for i in 0..inner {
            filter.add_hashed(bloom::hash(key(i)));
        }
        filter
    }

    /// Of `SAMPLES` scan rows spread over its `outer` rows, those whose key `filter` refuses. Row-weighted, they
    /// show a shared hot key even where the shared distinct keys are few.
    fn sampled(filter: &Kept, outer: usize, probe: impl Fn(usize) -> i64) -> usize {
        (0..SAMPLES).filter(|&i| !filter.may_hold(probe((i as u64 * outer as u64 / SAMPLES as u64) as usize))).count()
    }

    /// A walk's (a morsel's) counts, added as it ends.
    /// The scan's column a selection pass reads its keys by (`Screening::select`): a screen of a probe that is not
    /// negated, whose refused rows have no bindings past it. A negated probe's are tested a binding at a time: a
    /// refused key holds it.
    pub(super) fn scan_col(&self) -> Option<usize> {
        (!self.neg).then_some(self.scan)
    }

    pub(super) fn record(&self, s: &Screening) {
        if s.tested > 0 {
            self.tested.fetch_add(s.tested, Relaxed);
            self.refused.fetch_add(s.refused, Relaxed);
        }
    }

    #[cfg(test)]
    fn rejects(&self, slots: &[i64]) -> bool {
        !self.keys.may_hold(slots[self.slot])
    }

    #[cfg(test)]
    fn build(slot: usize, inner: usize, key: impl Fn(usize) -> i64, outer: usize, probe: impl Fn(usize) -> i64) -> Option<(Self, usize)> {
        let filter = Self::built(bloom::words_for(inner), inner, key);
        let rejects = Self::sampled(&filter, outer, probe);
        Some((Self { step: 1, neg: false, slot, scan: 0, keys: Arc::new(filter), _reservation: None, tested: AtomicU64::new(0), refused: AtomicU64::new(0), trace: None }, rejects))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_walk_probes_each_bloom_positive_exactly() {
        use super::super::{InRel, Interner, parse_join, run_join_collect, symbols};
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&path));
        let db = rusqlite::Connection::open(&path).unwrap();
        let symbols = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&symbols);
        let store = Store {
            rels: Vec::new(),
            syms: symbols,
        };
        let mut j = parse_join(&db, "pos c0 v0\npos c1 v0\nhead v0", &mut syms).unwrap();
        let outer = InRel::kept(vec![b'i'], (0..16_384).collect(), 16_384);
        let inner = InRel::kept(vec![b'i'], (0..1659).collect(), 1659);
        for (step, (rel, cols)) in j
            .steps
            .iter_mut()
            .zip([(outer, &[][..]), (inner, &[0][..])])
        {
            let Step::Atom { src, .. } = step else {
                panic!("expected an atom")
            };
            *src = Src::In(rel.clone(), rel.index(cols));
        }
        assert!(JoinFilter::choose(&j, &store).is_some());
        let mut answers = Vec::new();
        run_join_collect(&j, &store, &mut syms, &[], &mut answers).unwrap();
        answers.sort_unstable();
        assert_eq!(answers, (0..1659).collect::<Vec<_>>());
    }

    /// `spec`'s walk over a scan of 0..16,384 (`c0`) and a probe side of 0..1,659 (`c1`): the screen it chooses
    /// (its step and whether negated) and the head's answers, sorted.
    fn walk(spec: &str) -> (Option<(usize, bool)>, Vec<i64>) {
        use super::super::{InRel, Interner, parse_join, run_join_collect, symbols};
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&path));
        let db = rusqlite::Connection::open(&path).unwrap();
        let symbols = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&symbols);
        let store = Store { rels: Vec::new(), syms: symbols };
        let mut j = parse_join(&db, spec, &mut syms).unwrap();
        let mut rels = [(InRel::kept(vec![b'i'], (0..16_384).collect(), 16_384), &[][..]), (InRel::kept(vec![b'i'], (0..1659).collect(), 1659), &[0][..])].into_iter();
        for step in &mut j.steps {
            if let Step::Atom { src, .. } = step {
                let (rel, cols) = rels.next().unwrap();
                *src = Src::In(rel.clone(), rel.index(cols));
            }
        }
        let chosen = JoinFilter::choose(&j, &store).map(|f| (f.step, f.neg));
        let mut answers = Vec::new();
        run_join_collect(&j, &store, &mut syms, &[], &mut answers).unwrap();
        answers.sort_unstable();
        (chosen, answers)
    }

    /// A test between the scan and the probe: the probe, step 2, is screened, every match kept.
    #[test]
    fn a_probe_after_a_test_is_screened() {
        let (chosen, answers) = walk("pos c0 v0\nlt v0 i:100000\npos c1 v0\nhead v0");
        assert_eq!(chosen, Some((2, false)));
        assert_eq!(answers, (0..1659).collect::<Vec<_>>());
    }

    /// A negated probe (an anti-join): a key the filter refuses holds without its probe, a key it passes is probed.
    #[test]
    fn a_negated_probe_that_the_filter_refuses_holds() {
        let (chosen, answers) = walk("pos c0 v0\nneg c1 v0\nhead v0");
        assert_eq!(chosen, Some((1, true)));
        assert_eq!(answers, (1659..16_384).collect::<Vec<_>>());
    }

    #[test]
    fn sparse_overlap_skips_misses_and_verifies_false_positives() {
        let (filter, rejects) =
            JoinFilter::build(1, 1659, |i| i as i64 - 800, 8192, |i| i as i64).unwrap();
        assert!(rejects >= SAMPLES / 2);
        for key in -800..859 {
            assert!(!filter.rejects(&[0, key]));
        }
        // The positive screen admits collisions. The real join still checks the key.
        let admitted = (2000..20_000)
            .filter(|&key| !filter.rejects(&[0, key]))
            .count();
        assert!((1..1800).contains(&admitted), "{admitted}");
    }

    #[test]
    fn shared_hot_key_declines_the_filter_and_new_snapshot_sees_added_keys() {
        let key = i64::MIN;
        let (first, rejects) = JoinFilter::build(0, 32, |_| key, 8192, |_| key).unwrap();
        assert_eq!(rejects, 0);
        assert!(first.rejects(&[i64::MAX]));
        let (next, _) = JoinFilter::build(0, 2, |i| [key, i64::MAX][i], 8192, |_| key).unwrap();
        assert!(!next.rejects(&[i64::MAX]));
    }

    #[test]
    fn hll_union_separates_shared_and_disjoint_domains() {
        let mut a = Sketch::default();
        let mut b = Sketch::default();
        for i in 0..8192 {
            a.add(i);
            b.add(-1 - i);
        }
        assert!(a.overlap(&a) > 0.99);
        assert!(a.overlap(&b) < 0.25);
    }

    /// A probe side with its keys kept (its sketches) is screened by its own filter past the bounds a built one
    /// keeps: 40,000 rows, more than a quarter of the scan's 16,384, of which 1,659 match.
    #[test]
    fn a_probe_side_with_kept_keys_is_screened_by_its_own_filter() {
        use super::super::{InRel, Interner, parse_join, run_join_collect, symbols};
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::code_rule_dsl::fixtures::open(&path));
        let db = rusqlite::Connection::open(&path).unwrap();
        let symbols = symbols(&db).unwrap();
        let mut syms = Interner::for_loading(&symbols);
        let store = Store { rels: Vec::new(), syms: symbols };
        let mut j = parse_join(&db, "pos c0 v0\npos c1 v0\nhead v0", &mut syms).unwrap();
        let keys: Vec<i64> = (0..1659).chain(100_000..138_341).collect();
        let mut inner = InRel::kept(vec![b'i'], keys.clone(), keys.len());
        let mut sk = Sketch::default();
        for &v in &keys {
            sk.add(v);
        }
        sk.grow(|| keys.iter().copied());
        Arc::get_mut(&mut inner).unwrap().sketches = vec![sk];
        let outer = InRel::kept(vec![b'i'], (0..16_384).collect(), 16_384);
        for (step, (rel, cols)) in j.steps.iter_mut().zip([(outer, &[][..]), (inner, &[0][..])]) {
            let Step::Atom { src, .. } = step else { panic!("expected an atom") };
            *src = Src::In(rel.clone(), rel.index(cols));
        }
        let screen = JoinFilter::choose(&j, &store).expect("a kept filter screens");
        assert!(screen._reservation.is_none(), "nothing built");
        drop(screen);
        let mut answers = Vec::new();
        run_join_collect(&j, &store, &mut syms, &[], &mut answers).unwrap();
        answers.sort_unstable();
        assert_eq!(answers, (0..1659).collect::<Vec<_>>());
    }

    /// A walk whose probes the filter passes backs off: windows that refuse nothing stop its tests for longer each
    /// time, and a window that refuses most resets it.
    #[test]
    fn a_screen_that_refuses_little_backs_off() {
        let (f, _) = JoinFilter::build(0, 4096, |i| i as i64, 4096, |i| i as i64).unwrap();
        let mut s = Screening::default();
        let probes = 1 << 20;
        let refused = (0..probes).filter(|&i| s.refuses(&f, &[i % 4096])).count();
        assert_eq!(refused, 0);
        assert!(s.tested < probes as u64 / 8, "{} of {probes} tested", s.tested);
        let mut s = Screening::default();
        let refused = (0..probes).filter(|&i| s.refuses(&f, &[1_000_000 + i])).count() as u64;
        assert!(s.tested == probes as u64 && refused > s.tested * 9 / 10, "{refused} of {}", s.tested);
    }

    /// A screened scan's selection pass (`Screening::select`) keeps, in order, exactly the rows whose keys a test a
    /// key at a time passes, a block of 64 at a time and over a last part block; it backs off where it refuses little,
    /// passing rows untested; and once it ran, the screened step tests no binding again.
    #[test]
    fn a_scans_selection_pass_keeps_the_rows_the_screen_passes() {
        let (f, _) = JoinFilter::build(0, 3000, |i| 2 * i as i64, 4096, |i| i as i64).unwrap();
        let key = |id: u32| i64::from(id) * 3;
        let mut s = Screening::default();
        let got = s.select(&f, 0..10_001u32, key);
        let want: Vec<u32> = (0..10_001u32).filter(|&id| !f.rejects(&[key(id)])).collect();
        assert_eq!(got, want);
        assert!(got.len() < 6_000 && got.len() > 1_000, "{} rows kept", got.len());
        assert_eq!(s.tested, 10_001);
        assert!(!s.refuses(&f, &[1]), "no binding tested after the scan's selection");
        // Every key held: after the first windows it passes rows untested.
        let mut s = Screening::default();
        let got = s.select(&f, 0..1_000_000u32, |id| 2 * i64::from(id % 3000));
        assert_eq!(got.len(), 1_000_000);
        assert!(s.tested < 1_000_000 / 8, "{} tested", s.tested);
    }
}
