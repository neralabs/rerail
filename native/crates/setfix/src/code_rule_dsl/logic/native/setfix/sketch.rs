//! A column's statistics for the planner (decision 0112), in a fixed size whatever the rows: the one linear sketch
//! (`rerail_blocks::sketch::Sketch`, decision 0678 step 10) of its distinct values, the sum of its values' rows squared,
//! its join counters and, while they stay bounded, each value's exact count. Exact counts are updated as rows
//! arrive, never by a planning-time scan.
//!
//! What this adds to it: past its few values it keeps its column's keys in a blocked Bloom filter too (`bloom::Kept`,
//! decision 0488), set from the same hash as each row is counted, and made again larger from the column where its
//! keys outgrow it, with a sample of the keys. The screens test probes against it, and the planner counts the keys
//! two columns share by it. A row is taken out as it was put in (`remove`, decision 0678 step 7); its keys' filter
//! keeps a removed key: keys may hold more than the column, never fewer.

use crate::code_rule_dsl::logic::bloom::{self, Keys, Kept, Reading, Sample, Words};
use std::sync::Arc;

/// The values whose rows it counts exactly: `join_size::Counts`' few.
const SMALL: usize = crate::code_rule_dsl::logic::join_size::EXACT;

pub(super) struct Sketch {
    /// Its rows' sketch: its arrays in the run's room, this sketch their one writer; the planner's join sizes and the
    /// statistics' keys read its counts there.
    core: rerail_blocks::sketch::Sketch,
    /// Its keys' filter, once it holds more than `SMALL` values (while its counts hold them, they are its keys). This
    /// sketch is its one writer; the planner's statistics and the screens read it where it is.
    keys: Option<Arc<Kept>>,
    /// Whether it writes `keys`: a clone reads the filter it was cloned with until it first adds a key, then writes a
    /// copy of its own (a filter has one writer; the one it shares keeps all of the clone's keys until then).
    own_keys: bool,
    /// A sample of its distinct keys (`bloom::Sample`), offered each key its filter takes as new.
    sample: Sample,
}

impl Default for Sketch {
    fn default() -> Sketch {
        Sketch { core: rerail_blocks::sketch::Sketch::new_in(crate::code_rule_dsl::facts::rows::run_room()), keys: None, own_keys: true, sample: Sample::default() }
    }
}

/// A clone shares the filter until it adds to it (`own_keys`): a kept relation reopened to be maintained copies its
/// columns' filters only where its rows change them. Its core shares its counts the same way.
impl Clone for Sketch {
    fn clone(&self) -> Sketch {
        Sketch { core: self.core.clone(), keys: self.keys.clone(), own_keys: false, sample: self.sample.clone() }
    }
}

/// The keys' hash: the Bloom filters' too (`logic::bloom`), so a filter and a sketch see the same keys alike.
#[inline]
pub(super) fn mix(v: i64) -> u64 {
    crate::code_rule_dsl::logic::bloom::hash(v)
}

impl Sketch {
    pub(super) fn add(&mut self, v: i64) {
        let h = mix(v);
        let ended = self.core.add(v);
        if let Some(keys) = &mut self.keys {
            if !self.own_keys {
                *keys = Arc::new(Kept::clone(keys));
                self.own_keys = true;
            }
            if keys.add_hashed(h) {
                self.sample.offer(h);
            }
        }
        // Its `SMALL + 1`th value: its keys from here on in a filter, from the values it held and this one.
        if let Some(values) = ended {
            let keys = Kept::new(bloom::kept_words_for(4 * SMALL));
            for h in values.into_iter().map(mix) {
                keys.add_hashed(h);
                self.sample.offer(h);
            }
            self.keys = Some(Arc::new(keys));
            self.own_keys = true;
        }
    }

    /// A row of value `v` taken out, as `add` put it in (`Sketch::remove`). The row must be one it was given.
    pub(super) fn remove(&mut self, v: i64) {
        self.core.remove(v);
    }

    /// Distinct values.
    pub(super) fn ndv(&self) -> usize {
        self.core.ndv()
    }

    /// Estimated fraction of the probe column's distinct keys present in the build column.
    pub(super) fn overlap(&self, other: &Self) -> f64 {
        self.core.overlap(&other.core)
    }

    /// The sum over values of each value's rows squared.
    pub(super) fn sq(&self) -> u64 {
        self.core.sq()
    }

    /// Each value's rows, while they are exact: read from the sketch's arrays.
    pub(super) fn exact_counts(&self) -> Option<rerail_blocks::vector::Vector<[i64; 2]>> {
        self.core.exact_counts()
    }

    /// Its filter made again at twice its keys where they outgrew it (`Kept::outgrown`), from `all`: every value it
    /// was given (a dead row's too: a filter may hold more keys than its column, never fewer). The filters double,
    /// so a column's values are read again a few times at most, and only by a column of many distinct values.
    pub(super) fn grow<I: Iterator<Item = i64>>(&mut self, all: impl FnOnce() -> I) {
        let Some(keys) = &self.keys else { return };
        if !keys.outgrown() {
            return;
        }
        let made = Kept::new(bloom::kept_words_for(2 * self.ndv().max(Words::keys(&**keys) as usize)));
        for v in all() {
            made.add_hashed(mix(v));
        }
        self.keys = Some(Arc::new(made));
        self.own_keys = true;
    }

    /// Its keys, as the statistics read them: its few values, else its filter (held where it is) as read now.
    pub(super) fn keys(&self) -> Keys {
        match (self.core.counts().few_len(), &self.keys) {
            (Some(_), _) => Keys::Few(Arc::clone(self.core.counts())),
            (None, Some(k)) => Keys::Filter(Arc::new(Reading::new(Arc::clone(k), self.sample.hashes()))),
            (None, None) => unreachable!("a sketch past its few values keeps their filter"),
        }
    }

    /// Its counts for the planner's join sizes (`logic::join_size`): its counters, and its few values' counts.
    pub(super) fn counts(&self) -> crate::code_rule_dsl::logic::join_size::JoinCounts {
        crate::code_rule_dsl::logic::join_size::JoinCounts::new(Arc::clone(self.core.counts()), self.core.rows(), self.sq())
    }

    /// Its keys' filter, where it keeps one (past its few values); else its few values are its keys.
    pub(crate) fn filter(&self) -> Option<&Arc<Kept>> {
        self.keys.as_ref()
    }

    pub(super) fn bytes(&self) -> usize {
        self.keys.as_ref().map_or(0, |k| k.bytes()) + self.sample.hashes().len() * 8 + self.core.bytes()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Decision 0478 step 6c: a sketch's arrays (registers, counters, exact counts in each form) are block arrays.
    #[test]
    fn a_sketchs_arrays_are_in_blocks() {
        let mut s = Sketch::default();
        assert!(s.core.in_blocks());
        for i in 0..100 { s.add(i); }
        assert!(s.core.is_exact() && s.core.in_blocks(), "a page of counts");
        let mut t = Sketch::default();
        for i in 0..100 { t.add(i * 9_999); }
        assert!(t.core.is_exact() && t.core.in_blocks(), "a label set's slots");
        assert_eq!(t.ndv(), 100);
    }

    /// Decision 0353: the planner's join counts and few keys are the sketch's own counts, read where they lie: a row
    /// the sketch adds after they were taken is in them (nothing was copied). A clone reads them until it adds, then
    /// writes counts of its own, and the first sketch's stay as they were.
    #[test]
    fn the_planners_counts_are_the_sketchs_own() {
        use crate::code_rule_dsl::logic::join_size::JoinCounts;
        let mut s = Sketch::default();
        for v in 0..10 {
            s.add(v);
        }
        let (counts, keys) = (s.counts(), s.keys());
        let Keys::Few(few) = &keys else { panic!("few values") };
        assert!(Arc::ptr_eq(few, s.core.counts()), "the keys are the sketch's counts");
        let ten = JoinCounts::of([10]);
        assert_eq!(counts.join(&ten).rows, 0.0);
        s.add(10);
        assert_eq!((few.few_len(), counts.join(&ten).rows), (Some(11), 1.0), "read where they lie, after the add");
        let mut c = s.clone();
        assert!(Arc::ptr_eq(c.core.counts(), s.core.counts()), "a clone reads them");
        c.add(10);
        assert!(!Arc::ptr_eq(c.core.counts(), s.core.counts()), "a clone writes its own once it adds");
        assert_eq!((counts.join(&ten).rows, c.counts().join(&ten).rows), (1.0, 2.0));
        // Past its few values, the counters, still the sketch's own.
        for v in 11..1_000 {
            s.add(v);
        }
        assert!(few.few_len().is_none() && Arc::ptr_eq(few, s.core.counts()));
        // Past them, a clone reads them where they lie until it adds.
        let mut d = s.clone();
        assert!(Arc::ptr_eq(d.core.counts(), s.core.counts()), "a clone reads them");
        d.add(5);
        assert!(!Arc::ptr_eq(d.core.counts(), s.core.counts()), "a clone writes its own once it adds");
    }

    /// A clone taken while the sketch keeps its few values still has keys after the sketch passes them: the sketch,
    /// adding while they are shared, takes counts of its own, and ends those few values (`Counts::end_few`), not the
    /// clone's.
    #[test]
    fn a_clone_of_few_values_keeps_its_keys_when_the_sketch_passes_them() {
        let mut s = Sketch::default();
        for v in 0..10 {
            s.add(v * 1_000_003);
        }
        let c = s.clone();
        for v in 10..(2 * SMALL as i64) {
            s.add(v * 1_000_003);
        }
        assert!(s.core.counts().few_len().is_none() && matches!(s.keys(), Keys::Filter(_)));
        let Keys::Few(few) = c.keys() else { panic!("the clone's ten values are its keys") };
        assert_eq!((few.few_len(), c.ndv()), (Some(10), 10));
    }

    /// A clone's counts are the column's as it was cloned: the first sketch's later rows, past its few values here,
    /// never reach them, and the clone still has its few values as its keys (the 9,010-file run's panic: "a sketch
    /// past its few values keeps their filter", from a clone whose shared counts the first sketch had ended).
    #[test]
    fn a_clone_keeps_its_counts_when_the_first_sketch_adds() {
        let mut s = Sketch::default();
        for v in 0..10 {
            s.add(v);
        }
        let c = s.clone();
        for v in 10..1_000 {
            s.add(v);
        }
        assert!(!Arc::ptr_eq(c.core.counts(), s.core.counts()));
        assert_eq!((c.core.counts().few_len(), c.ndv(), c.core.rows()), (Some(10), 10, 10));
        assert!(matches!(c.keys(), Keys::Few(_)), "its few values are its keys");
        assert!(s.core.counts().few_len().is_none() && matches!(s.keys(), Keys::Filter(_)));
        let mut c = c;
        c.add(3);
        assert_eq!(c.core.counts().few_len(), Some(10), "the clone, alone now, writes the counts in place");
    }

    #[test]
    fn sketches_estimate_distinct_values_and_squares() {
        let mut few = Sketch::default();
        for i in 0..1000 {
            few.add(i % 10);
        }
        assert_eq!((few.ndv(), few.sq()), (10, 10 * 100 * 100));
        let mut many = Sketch::default();
        for i in 0..200_000i64 {
            many.add(i % 50_000);
        }
        let ndv = many.ndv() as f64;
        assert!((ndv / 50_000.0 - 1.0).abs() < 0.2, "{ndv}");
        // 50,000 values of 4 rows: 800,000.
        let sq = many.sq() as f64;
        assert!((sq / 800_000.0 - 1.0).abs() < 0.35, "{sq}");
        let mut skewed = Sketch::default();
        for i in 0..100_000i64 {
            skewed.add(if i % 2 == 0 { 7 } else { i });
        }
        // One value of 50,000 rows dominates: about 2.5e9.
        let sq = skewed.sq() as f64;
        assert!((sq / 2.5e9 - 1.0).abs() < 0.2, "{sq}");
    }

    /// Past its few values a sketch keeps its keys' filter: every key added is held as it grows from 256 to
    /// 100,000 keys, and a clone writes a copy of its own once it adds.
    #[test]
    fn a_sketch_keeps_a_filter_of_its_keys_as_they_grow() {
        let mut sk = Sketch::default();
        assert!(matches!(sk.keys(), Keys::Few(_)));
        let values = |n: i64| (0..n).map(|i| i * 7_919);
        for v in values(256) {
            sk.add(v);
        }
        let first = Arc::clone(sk.filter().expect("past its few values"));
        for v in values(100_000).skip(256) {
            sk.add(v);
            if v % 1_024 == 0 {
                sk.grow(|| values(100_000).take_while(|&x| x <= v));
            }
        }
        sk.grow(|| values(100_000));
        let Keys::Filter(reading) = sk.keys() else { panic!("a filter") };
        let grown = Arc::clone(&reading.filter);
        assert!(grown.len() > first.len() && values(100_000).all(|v| grown.may_hold(v)), "{} words", grown.len());
        assert_eq!(reading.sample.len(), bloom::SAMPLE);
        let distinct = sk.keys().distinct();
        assert!((distinct / 100_000.0 - 1.0).abs() < 0.1, "{distinct}");
        let absent = (1..).map(|i| -i).find(|&v| !grown.may_hold(v)).unwrap();
        let mut copy = sk.clone();
        copy.add(absent);
        assert!(copy.keys().may_hold(absent) && !sk.keys().may_hold(absent), "a clone's key is in its own filter only");
    }

    /// Decision 0678 step 7: a row removed leaves the sketch as if it was never added, in each of its forms (few
    /// values, a page of counts, a label set, and past them the rank counts behind its registers), so its distinct
    /// values, squares and join counts are the live rows'. Each removal is O(1).
    #[test]
    fn a_removed_row_leaves_the_sketch_as_if_never_added() {
        let same = |a: &Sketch, b: &Sketch, what: &str| {
            assert_eq!((a.core.rows(), a.ndv(), a.sq()), (b.core.rows(), b.ndv(), b.sq()), "{what}: rows, distinct values, squares");
            if !a.core.is_exact() || a.core.counts().few_len().is_some() {
                assert_eq!(a.core.registers(), b.core.registers(), "{what}: registers");
            }
            assert_eq!(a.core.counts().counters().collect::<Vec<_>>(), b.core.counts().counters().collect::<Vec<_>>(), "{what}: join counters");
            let exact = |s: &Sketch| s.exact_counts().map(|v| { let mut v: Vec<[i64; 2]> = v.to_vec(); v.sort(); v });
            assert_eq!(exact(a), exact(b), "{what}: exact counts");
        };
        // (all rows, rows removed, what is left), for each form.
        let few: Vec<i64> = (0..30).map(|i| i % 10).collect();
        let page: Vec<i64> = (0..600).map(|i| i % 200).collect();
        let labels: Vec<i64> = (0..600).map(|i| (i % 200) * 293).collect();
        let many: Vec<i64> = (0..60_000i64).map(|i| (i % 20_000) * 7_919 + (1 << 40)).collect();
        for (what, all) in [("few values", few), ("a page", page), ("labels", labels), ("many", many)] {
            // Every third row gone, and one value's every row.
            let gone = |i: usize, v: i64| i.is_multiple_of(3) || v == all[1];
            let mut s = Sketch::default();
            for &v in &all {
                s.add(v);
            }
            for (_, &v) in all.iter().enumerate().filter(|&(i, &v)| gone(i, v)) {
                s.remove(v);
            }
            let mut fresh = Sketch::default();
            for (_, &v) in all.iter().enumerate().filter(|&(i, &v)| !gone(i, v)) {
                fresh.add(v);
            }
            if what == "many" {
                // Its few values ended with other values than the fresh one's: the registers and counters are the
                // live rows' all the same.
                assert!(!s.core.is_exact());
                assert_eq!((s.core.rows(), s.core.registers(), s.core.counts().counters().collect::<Vec<_>>()), (fresh.core.rows(), fresh.core.registers(), fresh.core.counts().counters().collect::<Vec<_>>()));
                assert_eq!((s.ndv(), s.sq()), (fresh.ndv(), fresh.sq()));
            } else {
                same(&s, &fresh, what);
            }
            // A value removed whole and added again counts again.
            s.add(all[1]);
            fresh.add(all[1]);
            assert_eq!((s.ndv(), s.sq()), (fresh.ndv(), fresh.sq()), "{what}: a value back");
        }
    }

    // Bounded histograms (were the integration test `bounded_histograms`, which included this file alone: it reads
    // the engine's filters now).
    #[test]
    fn compact_integer_domains_keep_exact_counts_past_64_distinct_values() {
        let mut s=Sketch::default();
        let mut sq=0;
        for value in 0..200 { for _ in 0..value+1 { s.add(value); } sq+=(value as u64+1).pow(2); }
        assert_eq!(s.ndv(),200,"bounded labels must retain exact cardinality");
        assert_eq!(s.sq(),sq,"bounded labels must retain exact posting counts");
        assert_eq!(s.exact_counts().unwrap().iter().map(|&[v,n]|(v,n)).collect::<std::collections::HashMap<_,_>>()[&137],138);
        assert!(s.bytes()<20_000,"histogram memory must be bounded");
        s.add(1_i64<<40);
        assert!(s.exact_counts().is_none(),"an escaped domain must fall back, not claim a complete histogram");
    }
    #[test]
    fn sparse_label_domains_also_keep_exact_counts_with_bounded_memory() {
        let mut s=Sketch::default();
        for i in 0..200 {for _ in 0..3 {s.add(i*293);}}
        assert_eq!(s.exact_counts().unwrap().len(),200);
        assert_eq!(s.sq(),1800);
        assert!(s.bytes()<65536);
        for i in 200..2000 {s.add((i*293)%60000);}
        assert!(s.exact_counts().is_none(),"large domains must return to bounded sketches");
    }
    #[test]
    fn label_ids_do_not_lose_histograms_when_the_atom_arena_grows() {
        let mut s=Sketch::default();
        for i in 0..200 {s.add(600_000+i*293);}
        assert_eq!(s.exact_counts().unwrap().len(),200);
    }
}
