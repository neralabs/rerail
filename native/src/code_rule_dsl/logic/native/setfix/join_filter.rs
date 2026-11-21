//! A negative membership test for a scan followed by a keyed probe. One walk builds the filter
//! from its current build range and lends it to all morsels. An exact probe checks every positive.
//! The filter is blocked (`logic::bloom`): a test is one hash, one load and one compare.
use super::{
    JoinRule, Key, Rows, Src, Step, Store,
    sketch::Sketch,
};
use crate::code_rule_dsl::logic::bloom::{self, Filter};

const SAMPLES: usize = 64;

pub(super) struct JoinFilter {
    /// The step screened (an atom after the scan), whether it is negated, and the slot its key is read from.
    pub(super) step: usize,
    pub(super) neg: bool,
    slot: usize,
    filter: Filter,
    _reservation: super::join_cache::Reservation,
    /// With `RERAIL_JOIN_SCREEN=trace` (a measurement aid): the rows tested and refused, printed as it drops.
    trace: Option<Trace>,
}

struct Trace {
    what: String,
    tested: std::sync::atomic::AtomicU64,
    refused: std::sync::atomic::AtomicU64,
}

impl Drop for JoinFilter {
    fn drop(&mut self) {
        if let Some(t) = &self.trace {
            use std::sync::atomic::Ordering::Relaxed;
            eprintln!("join screen: {}; {} rows tested, {} refused", t.what, t.tested.load(Relaxed), t.refused.load(Relaxed));
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
}

enum Held<'a> {
    Rows(&'a Rows, usize),
    View(&'a super::InRel),
}

impl<'a> Column<'a> {
    fn of(src: &'a Src, store: &'a Store, column: usize) -> Option<Self> {
        let (rows, start, end, sketch) = match src {
            Src::In(r, _) => match &r.view {
                // A whole view (no ids of its own): its rows are its source's, each read on the tape.
                Some(v) if v.ids.is_none() => (Held::View(r), 0, r.len(), None),
                Some(_) => return None,
                None => (Held::Rows(&r.rows, r.k), 0, r.n, r.sketches.get(column)),
            },
            Src::Full(n) | Src::FullMulti(n, _) | Src::Delta(n) => {
                let r = &store.rels[*n];
                if matches!(src, Src::Delta(_)) {
                    (Held::Rows(&r.data, r.k), r.lo, r.hi, None)
                } else {
                    (Held::Rows(&r.data, r.k), 0, r.n, r.sketches.get(column))
                }
            }
        };
        Some(Self { rows, column, start, len: end - start, sketch })
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
        if outer.len < 4096 {
            return Err("under 4,096 scan rows");
        }
        if inner.len == 0 || inner.len > outer.len / 4 {
            return Err("the probe side is empty or past a quarter of the scan");
        }
        // The filter at its largest gives 16 bits a key to 8,192 keys.
        match inner.sketch {
            Some(s) if s.ndv() > 8192 => return Err("the probe side has over 8,192 distinct keys"),
            None if inner.len > 32_768 => return Err("the probe side, unsketched, is past 32,768 rows"),
            _ => {}
        }
        let overlap = outer.sketch.zip(inner.sketch).map(|(a, b)| a.overlap(b));
        if overlap.is_some_and(|p| p > 0.8) {
            return Err("the sides share most keys");
        }
        let words = bloom::words_for(inner.sketch.map_or(inner.len, |s| s.ndv().min(inner.len)));
        let (screen, rejects) = Self::sized(
            slot,
            words,
            inner.len,
            |i| inner.at(i),
            outer.len,
            |i| outer.at(i),
        )
        .ok_or("no allowance left")?;
        use crate::code_rule_dsl::logic::plan::joins;
        let use_filter = joins::use_bloom(joins::BloomProbeInput { probe_rows: outer.len,
            build_rows: inner.len, distinct_overlap: overlap, sampled: SAMPLES, rejected: rejects });
        let what = || format!(
            "{} probe rows, {} build rows, HLL overlap {overlap:?}, {rejects}/{SAMPLES} sampled probes rejected, use filter {use_filter}",
            outer.len, inner.len);
        crate::code_rule_dsl::logic::profile::decision("join Bloom filter", what);
        let traced = trace.then(|| Trace { what: what(), tested: Default::default(), refused: Default::default() });
        if !use_filter && let Some(t) = traced {
            eprintln!("join screen: {}", t.what);
            return Ok(None);
        }
        let mut screen = screen;
        screen.trace = traced;
        screen.step = step;
        screen.neg = *neg;
        Ok(use_filter.then_some(screen))
    }

    fn build(
        slot: usize,
        inner: usize,
        key: impl Fn(usize) -> i64,
        outer: usize,
        probe: impl Fn(usize) -> i64,
    ) -> Option<(Self, usize)> {
        Self::sized(slot, bloom::words_for(inner), inner, key, outer, probe)
    }

    /// `build` with `words` words: sized by the build side's distinct keys where its sketch has them.
    fn sized(
        slot: usize,
        words: usize,
        inner: usize,
        key: impl Fn(usize) -> i64,
        outer: usize,
        probe: impl Fn(usize) -> i64,
    ) -> Option<(Self, usize)> {
        let reservation = super::join_cache::Reservation::take(words * 8)?;
        let mut filter = Filter::new(words);
        for i in 0..inner {
            filter.add(key(i));
        }
        let screen = Self { step: 1, neg: false, slot, filter, _reservation: reservation, trace: None };
        // Row-weighted probes expose a shared hot key even when distinct-key overlap is small.
        let rejects = (0..SAMPLES)
            .filter(|&i| screen.absent(probe((i as u64 * outer as u64 / SAMPLES as u64) as usize)))
            .count();
        Some((screen, rejects))
    }

    #[inline]
    fn absent(&self, key: i64) -> bool {
        !self.filter.may_hold(key)
    }

    #[inline]
    pub(super) fn rejects(&self, slots: &[i64]) -> bool {
        let refused = self.absent(slots[self.slot]);
        if let Some(t) = &self.trace {
            use std::sync::atomic::Ordering::Relaxed;
            t.tested.fetch_add(1, Relaxed);
            t.refused.fetch_add(u64::from(refused), Relaxed);
        }
        refused
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
        assert!(first.absent(i64::MAX));
        let (next, _) = JoinFilter::build(0, 2, |i| [key, i64::MAX][i], 8192, |_| key).unwrap();
        assert!(!next.absent(i64::MAX));
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
}
