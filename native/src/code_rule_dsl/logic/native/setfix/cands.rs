//! A step's candidates (moved out of `run_join_in_each`, the spike on a test asked as the probe alone): where an
//! atom's rows are, and the rows a key finds there.

use super::*;

/// Where an atom's candidate tuples are: an input relation or a component relation, by id.
#[derive(Clone, Copy)]
pub(super) enum Rows<'a> { In(&'a InRel), Comp(&'a Rel), Source(&'a View) }
impl<'a> Rows<'a> {
    pub(super) fn row(self, id: u32) -> RowRef<'a> {
        match self { Rows::In(r) => r.at(id as usize).read(), Rows::Comp(r) => RowRef::Rows(&r.data, id, r.k), Rows::Source(v) => RowRef::Source(v, id as usize).read() }
    }
    /// `row`, a view's columns in `need` alone (`RowRef::read_need`).
    pub(super) fn row_need(self, id: u32, need: u64) -> RowRef<'a> {
        match self { Rows::In(r) => r.at(id as usize).read_need(need), Rows::Comp(r) => RowRef::Rows(&r.data, id, r.k), Rows::Source(v) => RowRef::Source(v, id as usize).read_need(need) }
    }
    /// Calls `f` on each candidate and its row (`row_need`) until it returns true; returns whether one did. A
    /// scan of a whole view (a range of its rows) reads them in one walk (`Source::for_rows_in`): a view of the
    /// tape then finds each row's node, id and parent as it walks, where a row at a time is a select, a granule
    /// search and a parent search (decision 0353 step 2).
    pub(super) fn each(self, cands: Cands<'_>, need: u64, mut f: impl FnMut(u32, RowRef<'a>) -> bool) -> bool {
        if let (Rows::In(r), &Cands::Range(lo, hi)) = (self, &cands)
            && hi - lo >= WALK_SCAN
            && let Some(v) = &r.view
            && v.ids.is_none()
            && v.cols.len() <= READ
        {
            let (mut cols, mut at, mut m) = ([0usize; READ], [0usize; READ], 0);
            for c in 0..v.cols.len() {
                if need >> c & 1 != 0 {
                    cols[m] = v.cols[c];
                    at[m] = c;
                    m += 1;
                }
            }
            let mut hit = false;
            v.src.for_rows_in(lo as usize..hi as usize, &cols[..m], &mut |row, got| {
                let mut vals = [0i64; READ];
                for j in 0..m {
                    vals[at[j]] = got[j];
                }
                hit = f(row as u32, RowRef::Read(vals, row));
                !hit
            });
            return hit;
        }
        if let Cands::Read(rows) = cands {
            return rows.rows().iter().any(|&(id, vals)| f(id, RowRef::Read(vals, id as usize)));
        }
        cands.any(|id| f(id, self.row_need(id, need)))
    }
    /// A row deleted in place (`Rel::based`), which no step reads.
    #[inline]
    pub(super) fn dead(self, id: u32) -> bool {
        match self { Rows::In(r) => is_dead(&r.dead, id), Rows::Comp(r) => r.dead(id), Rows::Source(_) => false }
    }
}

pub(super) fn candidates<'a>(src: &'a Src, store: &'a Store, key: &[i64], keys: &[(usize, Key)], hints: &mut [u32; runs::HINTS], last: &mut LastProbe, need: u64, within: Within) -> (Rows<'a>, Cands<'a>) {
    match src {
        Src::In(rel, ix) => {
            if !keys.is_empty() { probed::note(rel.name.get().map_or("?input", String::as_str), ix.cols.iter().copied()); }
            if keys.is_empty() { scan_within(Rows::In(rel), 0, rel.len() as u32, within) }
            else if let Some(index) = ix.runs(rel) {
                // An order answering the key's columns, or leading with some of them (`chain_lead`: every key
                // is checked): the key's values in the order's column order.
                let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
                let mut buf = [0i64; MAX_KEY];
                let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
                let found = index.find_at(&rel.rows, rel.k, permuted(&index.cols[..m], at, &mut buf), None, hints);
                let found = match within {
                    Some((c, lo, hi)) if m == keys.len() && index.cols.get(m) == Some(&c) => {
                        if probed::on() { probed::note(&format!("{} range", rel.name.get().map_or("?input", String::as_str)), std::iter::once(c)); }
                        found.within(&rel.rows, rel.k, c, lo, hi)
                    }
                    _ => found,
                };
                (Rows::In(rel), Cands::Runs(found))
            }
            else if ix.keyed { let r = rel.keyed_rows(key[ix.lead]); (Rows::In(rel), Cands::Range(r.start, r.end)) }
            else if ix.composite && let Some(v) = &rel.view {
                let mut eq = [(0usize, 0i64); MAX_KEY];
                for (j, &c) in ix.cols.iter().enumerate() { eq[j] = (v.cols[c], key[j]); }
                // A step that reads no column of the row (every key exact, `need` 0) asks only whether it is
                // there: a source's test where it has one (`Source::holds`, `within` with both bound). The
                // candidate it gives is never read.
                // `RERAIL_WITHIN_TEST=off` (a measurement aid) probes such a step's rows as before: the
                // run's setting (decision 0442), not the process's.
                let test = || rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_WITHIN_TEST").map_or(true, |v| v != "off"));
                if need == 0 && test() && let Some(h) = v.src.holds(&eq[..ix.cols.len()]) {
                    return (Rows::Source(v), Cands::Range(0, u32::from(h)));
                }
                let rows = v.src.lookup_all(&eq[..ix.cols.len()]).unwrap_or_default();
                (Rows::Source(v), Cands::Owned(rows.into_iter().map(|r| r as u32).collect()))
            }
            else if ix.sorted && let Some(v) = &rel.view {
                // From where the last probe of this step was (`hints`, unused by a source otherwise): probes
                // in the column's order are a merge over it (decision 0456 K7).
                let (c, k) = (v.cols[ix.cols[ix.lead]], key[ix.lead]);
                (Rows::Source(v), match v.src.sorted_range_at(c, k, &mut hints[0]) { Some(r) => Cands::Range(r.start, r.end), None => Cands::Slice(v.src.sorted_rows(c, k).unwrap_or(&[])) })
            }
            else if ix.source && let Some(v) = &rel.view {
                let probed = match last {
                    Some((k, p)) if k.as_slice() == key && !matches!(&*p, Probed::Read(_, n) if *n != need) => p.clone(),
                    _ => {
                        // A step's rows are read as the probe finds them, with the columns it reads (none for
                        // a test): a source that reads in place answers without a list of ids; one that does
                        // not gives the ids.
                        let read = v.find_read(&ix.cols, key, ix.lead, need);
                        let p = match read {
                            Some(rows) => Probed::Read(rows, need),
                            None => Probed::Ids(v.find_keyed(&ix.cols, key, ix.lead)),
                        };
                        match last {
                            Some((k, q)) => {
                                k.clear();
                                // copy: not block data (a probe key's values, kept to compare the next probe with)
                                k.extend_from_slice(key);
                                *q = p.clone();
                            }
                            None => *last = Some((key.to_vec(), p.clone())),
                        }
                        p
                    }
                };
                match probed {
                    Probed::Ids(ids) => (Rows::Source(v), Cands::Owned(ids)),
                    Probed::Read(rows, _) => (Rows::Source(v), Cands::Read(rows)),
                }
            }
            else { (Rows::In(rel), Cands::Slice(ix.get(rel, key))) }
        }
        Src::FullMulti(r, cols) => {
            let rel = &store.rels[*r];
            probed::note(&rel.table, cols.iter().copied());
            (Rows::Comp(rel), rel.probe_at(cols, key, false, hints, within).0)
        }
        Src::Delta(r) | Src::Full(r) => {
            let rel = &store.rels[*r];
            let delta = matches!(src, Src::Delta(_));
            if keys.is_empty() {
                (Rows::Comp(rel), if delta { Cands::Range(rel.lo as u32, rel.hi as u32) } else { Cands::Range(0, rel.n as u32) })
            } else {
                if probed::on() { probed::note(&if delta { format!("{}+delta", rel.table) } else { rel.table.clone() }, keys.iter().map(|&(c, _)| c)); }
                let mut cols = [0usize; MAX_KEY];
                for (j, &(c, _)) in keys.iter().enumerate() { cols[j] = c; }
                (Rows::Comp(rel), rel.probe_at(&cols[..keys.len()], key, delta, hints, within).0)
            }
        }
}
}

/// A first scan's rows `lo..hi` of an input, those whose column `c` lies in the range: where they are in `c`'s
/// order, two searches; where the input is a view whose source reads a range of that column (`range_lookup`, the
/// node views' own column), the source's rows in it. No row outside it is walked; the comparisons stay as filters.
/// `rr_ann_node(n) :- kind(n, _), n >= a, n < b` read all of `kind` (4.46 M rows) for the annotations' 2,454.
pub(super) fn scan_within<'a>(rows: Rows<'a>, lo: u32, hi: u32, within: Within) -> (Rows<'a>, Cands<'a>) {
    let (Some((c, a, b)), Rows::In(r)) = (within, rows) else { return (rows, Cands::Range(lo, hi)) };
    if hi <= lo {
        return (rows, Cands::Range(lo, hi));
    }
    if r.sorted.within(&[c], r.k, lo, hi) == [(lo, hi)] {
        let at = |p: usize| rows.row(p as u32).at(c);
        let s = a.map_or(lo as usize, |(x, eq)| runs::split_at_pos(lo as usize, hi as usize, |p| if eq { at(p) < x } else { at(p) <= x }));
        let e = b.map_or(hi as usize, |(x, eq)| runs::split_at_pos(s, hi as usize, |p| if eq { at(p) <= x } else { at(p) < x }));
        return (rows, Cands::Range(s as u32, e.max(s) as u32));
    }
    if (lo, hi) == (0, r.len() as u32)
        && let Some(v) = &r.view
        && v.ids.is_none()
        && v.eqs.is_empty()
        && v.src.range_indexed(v.cols[c])
        && let Some(ids) = v.src.range_lookup(v.cols[c], a, b)
    {
        return (Rows::Source(v), Cands::Owned(ids.into_iter().map(|i| i as u32).collect()));
    }
    (rows, Cands::Range(lo, hi))
}

/// A probe's state kept from one row's test to the next: the key's buffer, the index runs' hints, the last probe.
#[derive(Default)]
pub(super) struct Asked {
    key: Vec<i64>,
    hints: [u32; runs::HINTS],
    last: LastProbe,
}

/// A test (an atom of an input that binds nothing) asked as the index's probe alone, `slots` the row's values: whether
/// a row matches its keys and its column pairs. The join's step for it (`enter`'s `Step::Atom`, a test) does the same
/// inside a walk, with the walk's frames around it (decision 0561, "a test asked directly").
pub(super) fn holds(step: &Step, store: &Store, slots: &[i64], asked: &mut Asked) -> bool {
    let Step::Atom { src, keys, tests, binds, .. } = step else { unreachable!("a test is an atom") };
    debug_assert!(binds.is_empty(), "a test binds nothing");
    let val = |k: Key| match k {
        Key::Slot(s) => slots[s],
        Key::Const(c) => c,
    };
    asked.key.clear();
    asked.key.extend(keys.iter().map(|&(_, k)| val(k)));
    // As the join's step: the key columns a view's probe matched exactly, which its rows need not be checked for.
    let exact: u64 = match src {
        _ if keys.is_empty() => 0,
        Src::In(rel, ix) if rel.view.is_some() && ix.runs(rel).is_none() => {
            if ix.composite {
                ix.cols.iter().fold(0, |m, &c| m | 1 << c)
            } else if ix.sorted || (ix.source && ix.cols.len() == 1) {
                1 << ix.cols[ix.lead]
            } else {
                0
            }
        }
        _ => 0,
    };
    let need: u64 = keys.iter().filter(|&&(c, _)| exact >> c & 1 == 0).fold(0u64, |m, &(c, _)| m | 1 << c) | tests.iter().fold(0u64, |m, &(c, c0)| m | 1 << c | 1 << c0);
    let (rows, cands) = candidates(src, store, &asked.key, keys, &mut asked.hints, &mut asked.last, need, None);
    rows.each(cands, need, |id, row| {
        !rows.dead(id) && keys.iter().all(|&(c, k)| exact >> c & 1 != 0 || row.at(c) == val(k)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
    })
}
