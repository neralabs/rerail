//! A negative membership test for a scan followed by a keyed probe. One walk builds the filter
//! from its current build range and lends it to all morsels. An exact probe checks every positive.
use super::{
    JoinRule, Key, Rows, Src, Step, Store,
    sketch::{Sketch, mix},
};

const WORDS: usize = 256;
const SAMPLES: usize = 64;

pub(super) struct JoinFilter {
    slot: usize,
    bits: Box<[u64; WORDS]>,
    _reservation: super::join_cache::Reservation,
}

fn positions(key: i64) -> impl Iterator<Item = usize> {
    let hash = mix(key);
    let stride = (hash >> 32) | 1;
    (0..7).map(move |i: u64| hash.wrapping_add(i.wrapping_mul(stride)) as usize & (WORDS * 64 - 1))
}

struct Column<'a> {
    rows: &'a Rows,
    width: usize,
    column: usize,
    start: usize,
    len: usize,
    sketch: Option<&'a Sketch>,
}

impl<'a> Column<'a> {
    fn of(src: &'a Src, store: &'a Store, column: usize) -> Option<Self> {
        let (rows, width, start, end, sketch) = match src {
            Src::In(r, _) if r.view.is_none() => (&r.rows, r.k, 0, r.n, r.sketches.get(column)),
            Src::In(..) => return None,
            Src::Full(n) | Src::FullMulti(n, _) | Src::Delta(n) => {
                let r = &store.rels[*n];
                if matches!(src, Src::Delta(_)) {
                    (&r.data, r.k, r.lo, r.hi, None)
                } else {
                    (&r.data, r.k, 0, r.n, r.sketches.get(column))
                }
            }
        };
        Some(Self {
            rows,
            width,
            column,
            start,
            len: end - start,
            sketch,
        })
    }

    fn at(&self, i: usize) -> i64 {
        super::RowRef::Rows(self.rows, (self.start + i) as u32, self.width).at(self.column)
    }
}

impl JoinFilter {
    pub(super) fn choose(j: &JoinRule, store: &Store) -> Option<Self> {
        let [
            Step::Atom {
                src: left,
                neg: false,
                keys,
                binds,
                ..
            },
            Step::Atom {
                src: right,
                neg: false,
                keys: probe,
                ..
            },
            ..,
        ] = j.steps.as_slice()
        else {
            return None;
        };
        let [(right_col, Key::Slot(slot))] = probe.as_slice() else {
            return None;
        };
        if !keys.is_empty() || j.fused.first().is_some_and(Option::is_some) {
            return None;
        }
        let &(left_col, _) = binds.iter().find(|(_, s)| s == slot)?;
        let outer = Column::of(left, store, left_col)?;
        let inner = Column::of(right, store, *right_col)?;
        if outer.len < 4096 || inner.len == 0 || inner.len > (outer.len / 4).min(32_768) {
            return None;
        }
        if inner.sketch.is_some_and(|s| s.ndv() > 4096) { return None; }
        let overlap = outer.sketch.zip(inner.sketch).map(|(a, b)| a.overlap(b));
        if overlap.is_some_and(|p| p > 0.8) {
            return None;
        }
        let (screen, rejects) = Self::build(
            *slot,
            inner.len,
            |i| inner.at(i),
            outer.len,
            |i| outer.at(i),
        )?;
        use crate::code_rule_dsl::logic::plan::joins;
        let use_filter = joins::use_bloom(joins::BloomProbeInput { probe_rows: outer.len,
            build_rows: inner.len, distinct_overlap: overlap, sampled: SAMPLES, rejected: rejects });
        crate::code_rule_dsl::logic::profile::decision("join Bloom filter", || format!(
            "{} probe rows, {} build rows, HLL overlap {overlap:?}, {rejects}/{SAMPLES} sampled probes rejected, use filter {use_filter}",
            outer.len, inner.len));
        use_filter.then_some(screen)
    }

    fn build(
        slot: usize,
        inner: usize,
        key: impl Fn(usize) -> i64,
        outer: usize,
        probe: impl Fn(usize) -> i64,
    ) -> Option<(Self, usize)> {
        let reservation = super::join_cache::Reservation::take(WORDS * 8)?;
        let mut screen = Self {
            slot,
            bits: Box::new([0; WORDS]),
            _reservation: reservation,
        };
        for i in 0..inner {
            for bit in positions(key(i)) {
                screen.bits[bit / 64] |= 1 << (bit % 64);
            }
        }
        // Row-weighted probes expose a shared hot key even when distinct-key overlap is small.
        let rejects = (0..SAMPLES)
            .filter(|&i| screen.absent(probe((i as u64 * outer as u64 / SAMPLES as u64) as usize)))
            .count();
        Some((screen, rejects))
    }

    fn absent(&self, key: i64) -> bool {
        positions(key).any(|bit| self.bits[bit / 64] & (1 << (bit % 64)) == 0)
    }

    pub(super) fn rejects(&self, slots: &[i64]) -> bool {
        self.absent(slots[self.slot])
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
