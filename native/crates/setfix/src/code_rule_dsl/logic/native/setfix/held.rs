//! A relation held in memory for the runs that read it (`columns_source`): its rows one relation alone in a block, and
//! every probe of it its own index there (decision 0325 step 5), shared with the device, not a host table. It replaced
//! columns held as vectors, each sorted on its first probe into a host index (`Stored`, `Sorted`): 120–140 MB of
//! sorted ids at a 300-file run's peak, every column of a measured source sorted and kept.
//!
//! - A column's index is the relation's own, its buckets sized to the column's distinct values: about 4 bytes a row
//!   and 4 a value (a sorted index was 4 a row and 12 a value). A probe reads its bucket and checks the value.
//! - Statistics are measured by a sort of one column at a time, dropped when measured, not kept as an index.
//! - Columns no rule reads are not held, and read as 0.
//! - Past the relation's directory (`ADDED_INDEXES`), a column is probed by a sorted index on the host
//!   (`Sorted`), the one form left there.
use super::Sorted;
use crate::code_rule_dsl::facts::rows::{InBlock, Life, Rows};
use crate::code_rule_dsl::facts::vtab::Source;
use std::sync::OnceLock;

pub(super) struct Held {
    columns: Vec<String>,
    /// Each source column's place in the relation; `None`: not held, read as 0.
    slot: Vec<Option<u16>>,
    rows: Rows,
    ib: Option<InBlock>,
    n: usize,
    statistics: OnceLock<Vec<(String, i64, i64, i64, i64)>>,
    distinct: Vec<OnceLock<usize>>,
    /// Each column's own index's number, once made; `None` past the directory.
    index: Vec<OnceLock<Option<usize>>>,
    /// The host's sorted index of a column past the directory.
    sorted: Vec<OnceLock<Sorted>>,
    /// Each source column's least and greatest value, taken as its rows are written (`Source::col_bounds`): a column
    /// not held reads 0; `[MAX, MIN]` with no rows.
    bounds: Vec<[i64; 2]>,
}

impl Held {
    pub(super) fn new(columns: Vec<String>, values: Vec<Option<Vec<i64>>>, statistics: Option<Vec<(String, i64, i64, i64, i64)>>) -> Held {
        let n = values.iter().flatten().map(Vec::len).max().unwrap_or(0);
        let mut slot = Vec::with_capacity(values.len());
        let mut held: Vec<Vec<i64>> = Vec::new();
        for v in values {
            match v {
                Some(v) => {
                    slot.push(Some(held.len() as u16));
                    held.push(v);
                }
                None => slot.push(None),
            }
        }
        let k = held.len();
        let mut range = vec![[i64::MAX, i64::MIN]; k];
        let mut rows = if n > 0 && k > 0 { Rows::with_index_room(k, Life::Kept, n, rerail_blocks::relation::ADDED_INDEXES) } else { Rows::living(k, Life::Kept) };
        if k > 0 {
            let mut row = vec![0i64; k];
            for r in 0..n {
                for (c, col) in held.iter().enumerate() {
                    row[c] = col.get(r).copied().unwrap_or(0);
                    range[c] = [range[c][0].min(row[c]), range[c][1].max(row[c])];
                }
                rows.extend_from_slice(&row);
            }
        }
        drop(held);
        let empty = [i64::MAX, i64::MIN];
        let bounds = slot.iter().map(|s| match s { Some(s) => range[usize::from(*s)], None if n > 0 => [0, 0], None => empty }).collect();
        let rows = rows.freeze(n);
        let ib = rows.in_block();
        let k_all = slot.len();
        let kept = OnceLock::new();
        if let Some(st) = statistics {
            let _ = kept.set(st);
        }
        Held { columns, slot, rows, ib, n, statistics: kept, distinct: (0..k_all).map(|_| OnceLock::new()).collect(), index: (0..k_all).map(|_| OnceLock::new()).collect(), sorted: (0..k_all).map(|_| OnceLock::new()).collect(), bounds }
    }

    fn at(&self, r: usize, s: u16) -> i64 {
        self.rows.get(r as u32, s as usize)
    }

    /// Column `c` read whole (its slot's values), for a measure or a host index.
    fn column(&self, s: u16) -> Vec<i64> {
        (0..self.n).map(|r| self.at(r, s)).collect()
    }

    /// Column `c`'s count of distinct values: from the statistics where known, else counted by a sort of its values,
    /// which is dropped.
    fn distinct(&self, c: usize) -> usize {
        *self.distinct[c].get_or_init(|| {
            let known = self.statistics.get().and_then(|st| st.iter().find(|m| m.0 == self.columns[c]).map(|m| m.2 as usize));
            known.unwrap_or_else(|| match self.slot[c] {
                Some(s) => {
                    let mut v = self.column(s);
                    v.sort_unstable();
                    v.dedup();
                    v.len()
                }
                None => 1,
            }).max(1)
        })
    }

    /// Column `c`'s own index, made on its first probe; `None` past the directory.
    fn col_index(&self, c: usize, s: u16) -> Option<usize> {
        *self.index[c].get_or_init(|| {
            let i = self.ib.as_ref()?.own_index_for(&[s], self.distinct(c));
            if i.is_none() {
                crate::code_rule_dsl::logic::profile::decision("held rows: a column's index on the host", || format!("{} of {} rows: past its relation's index directory", self.columns[c], self.n));
            }
            i
        })
    }

    /// The distinct keys over source columns `cols` at most: the product of their distinct values, at most a key a
    /// row. An index's buckets are sized so, about one key a bucket.
    fn keys(&self, cols: impl Iterator<Item = usize>) -> usize {
        cols.fold(1usize, |p, c| p.saturating_mul(self.distinct(c))).min(self.n).max(1)
    }

    /// The rows whose column `c` (held, at `s`) is `v`, appended to `out`.
    fn rows_with(&self, c: usize, s: u16, v: i64, out: &mut Vec<usize>) {
        match (self.col_index(c, s), &self.ib) {
            (Some(i), Some(ib)) => {
                let h = rerail_blocks::mix(rerail_blocks::SEED, v);
                out.extend(ib.relation().group(i, h).iter().map(|&r| r as usize).filter(|&r| self.at(r, s) == v));
            }
            _ => {
                let sorted = self.sorted[c].get_or_init(|| Sorted::of(&self.column(s)));
                if let Ok(k) = sorted.keys.binary_search(&v) {
                    out.extend(sorted.ids[sorted.starts[k] as usize..sorted.starts[k + 1] as usize].iter().map(|&r| r as usize));
                }
            }
        }
    }

    /// `syntax::measured`'s statistics, a column at a time: its values with their rows' documents (`id >> 32` of the
    /// first column) sorted, counted, and dropped. A column not held is stated as a key.
    fn measured(&self) -> Vec<(String, i64, i64, i64, i64)> {
        let n = self.n as i64;
        let doc = |r: usize| self.slot[0].map_or(0, |s| self.at(r, s) >> 32);
        (0..self.columns.len())
            .map(|c| {
                let name = self.columns[c].clone();
                let Some(s) = self.slot[c].filter(|_| n > 0) else { return (name, n, n, n, n) };
                let mut pairs: Vec<(i64, i64)> = (0..self.n).map(|r| (self.at(r, s), doc(r))).collect();
                pairs.sort_unstable();
                let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
                let mut at = 0;
                while at < pairs.len() {
                    let v = pairs[at].0;
                    let end = at + pairs[at..].partition_point(|p| p.0 == v);
                    let l = (end - at) as i64;
                    ndv += 1;
                    sq += l * l;
                    let mut d = at;
                    while d < end {
                        let de = d + pairs[d..end].partition_point(|p| p.1 == pairs[d].1);
                        sqd += ((de - d) * (de - d)) as i64;
                        d = de;
                    }
                    at = end;
                }
                let _ = self.distinct[c].set(ndv.max(1) as usize);
                (name, n, ndv, sq, sqd)
            })
            .collect()
    }
}

impl Source for Held {
    fn columns(&self) -> Vec<String> { self.columns.clone() }
    fn col_bounds(&self, column: usize) -> Option<[i64; 2]> { self.bounds.get(column).copied() }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        Some(self.statistics.get_or_init(|| self.measured()).clone())
    }
    fn len(&self) -> usize { self.n }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let one = self.n.div_ceil(self.distinct(c).max(1));
        if batch { one * 8 } else { one }
    }
    fn composite(&self) -> bool { true }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        // A column not held reads as 0: its test holds of every row or of none.
        if equal.iter().any(|&(c, v)| self.slot[c].is_none() && v != 0) {
            return Some(Vec::new());
        }
        let held: Vec<(u16, i64)> = equal.iter().filter_map(|&(c, v)| self.slot[c].map(|s| (s, v))).collect();
        if held.is_empty() {
            return Some((0..self.n).collect());
        }
        let key: Vec<u16> = held.iter().take(rerail_blocks::relation::MAX_KEY).map(|h| h.0).collect();
        let keys = self.keys(equal.iter().filter(|e| self.slot[e.0].is_some()).take(key.len()).map(|e| e.0));
        let mut out = Vec::new();
        if let Some(ib) = &self.ib
            && let Some(i) = ib.own_index_for(&key, keys)
        {
            let h = held.iter().take(key.len()).fold(rerail_blocks::SEED, |h, &(_, v)| rerail_blocks::mix(h, v));
            out.extend(ib.relation().group(i, h).iter().map(|&r| r as usize));
        } else {
            // Past the directory: the most selective column's rows.
            let (c, _) = equal.iter().filter(|(c, _)| self.slot[*c].is_some()).max_by_key(|(c, _)| self.distinct(*c)).copied().expect("a held column");
            self.rows_with(c, self.slot[c].expect("held"), equal.iter().find(|e| e.0 == c).expect("its value").1, &mut out);
        }
        out.retain(|&r| held.iter().all(|&(s, v)| self.at(r, s) == v));
        out.sort_unstable();
        Some(out)
    }
    fn indexed(&self, _: usize) -> bool { true }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        let Some(s) = self.slot[c] else {
            return Some(if values.contains(&0) { (0..self.n).collect() } else { Vec::new() });
        };
        let mut out = Vec::new();
        for &v in values {
            self.rows_with(c, s, v, &mut out);
        }
        out.sort_unstable();
        Some(out)
    }
    fn value(&self, row: usize, c: usize) -> i64 { self.slot[c].map_or(0, |s| self.at(row, s)) }
    /// Its rows as one relation where every column is held, at its own place.
    fn in_block(&self) -> Option<InBlock> {
        self.slot.iter().enumerate().all(|(c, s)| *s == Some(c as u16)).then(|| self.ib.clone()).flatten()
    }
    fn own_index(&self, cols: &[usize]) -> Option<(InBlock, usize)> {
        let ib = self.ib.as_ref()?;
        let slots: Vec<u16> = cols.iter().map(|&c| self.slot[c]).collect::<Option<_>>()?;
        let keys = self.keys(cols.iter().copied());
        let i = ib.own_index_for(&slots, keys)?;
        Some((ib.clone(), i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lookup a scan's rows, whatever the column, key or held form: one column at a time past the relation's
    /// index directory (12 columns, 8 indexes: the rest are the host's sorted ones), keys of two and three columns,
    /// and a column not held, read as 0.
    #[test]
    fn held_rows_look_up_what_a_scan_finds() {
        let n = 3_000i64;
        let cols = 12;
        let values: Vec<Option<Vec<i64>>> = (0..cols)
            .map(|c| if c == 5 { None } else { Some((0..n).map(|r| (r * (c as i64 * 7 + 3) + c as i64) % (c as i64 * 13 + 2) - 4).collect()) })
            .collect();
        let names: Vec<String> = (0..cols).map(|c| format!("c{c}")).collect();
        let held = Held::new(names, values.clone(), None);
        let at = |r: usize, c: usize| values[c].as_ref().map_or(0, |v| v[r]);
        let scan = |eq: &[(usize, i64)]| (0..n as usize).filter(|&r| eq.iter().all(|&(c, v)| at(r, c) == v)).collect::<Vec<_>>();
        for c in 0..cols {
            for v in [-4, -3, 0, 1, 5, 9, 40] {
                assert_eq!(held.lookup(c, &[v]), Some(scan(&[(c, v)])), "column {c} = {v}");
                assert_eq!(held.value(7, c), at(7, c));
            }
            assert_eq!(held.lookup(c, &[0, 1]).map(|r| r.len()), Some(scan(&[(c, 0)]).len() + scan(&[(c, 1)]).len()));
        }
        for eq in [vec![(0, 1), (1, 3)], vec![(2, 0), (3, 1), (4, 2)], vec![(1, 1), (5, 0)], vec![(1, 1), (5, 3)], vec![(10, 2), (11, 7)]] {
            assert_eq!(held.lookup_all(&eq), Some(scan(&eq)), "{eq:?}");
        }
        let st = held.statistics().expect("measured");
        assert_eq!(st.len(), cols);
        assert_eq!(st[5], ("c5".to_owned(), n, n, n, n), "a column not held is stated as a key");
    }
}
