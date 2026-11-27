//! A step's candidates (moved out of `run_join_in_each`, the spike on a test asked as the probe alone): where an
//! atom's rows are, and the rows a key finds there.

use std::sync::Arc;
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
    ///
    /// A probe read as found (`Cands::Stream`) gives each match to `f` as the source finds it, and keeps the matches in
    /// the step's list (`last`) for its next probe of the same key; where the source refuses the read, its ids.
    pub(super) fn each(self, cands: Cands<'_>, need: u64, last: &mut LastProbe, mut f: impl FnMut(u32, RowRef<'a>) -> bool) -> bool {
        if let Cands::Stream(s) = cands {
            // A lone match is kept as itself; the step's list (the probe before's, where nothing holds it now) from
            // a second match on, as `View::find_read` keeps them.
            let mut list = last.spare.take().filter(|l| Arc::strong_count(l) == 1);
            let (mut first, mut n, mut hit) = (None, 0usize, false);
            let read = s.view.read_each(&s, need, &mut last.cursor, &mut |id, vals| {
                n += 1;
                if n == 1 {
                    first = Some((id, vals));
                } else {
                    let rows = Arc::get_mut(list.get_or_insert_with(|| Arc::new(Vec::new()))).expect("the step's own list");
                    if n == 2 {
                        rows.clear();
                        rows.extend(first);
                    }
                    rows.push((id, vals));
                }
                if !hit {
                    hit = f(id, RowRef::Read(vals, id as usize));
                }
            });
            let answer = if read {
                match (n, first) {
                    (0, _) | (_, None) => { last.spare = list; Found::None }
                    (1, Some(one)) => { last.spare = list; Found::One(one) }
                    _ => Found::Many(list.expect("a list of several matches")),
                }
            } else {
                last.spare = list;
                let mut ids = last.ids();
                s.view.find_keyed_into(s.cols, &s.key[..s.len], s.lead, Arc::get_mut(&mut ids).expect("the step's own list"));
                hit = ids.iter().any(|&id| f(id, self.row_need(id, need)));
                if let Some((_, q)) = &mut last.memo { Probed::put(q, Probed::Ids(ids), &mut last.spare_ids); }
                return hit;
            };
            if let Some((_, q)) = &mut last.memo { Probed::put(q, Probed::Read(answer, need), &mut last.spare_ids); }
            return hit;
        }
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

/// `stream`: the caller reads the candidates through `Rows::each`, which reads a view's probe as the source finds its
/// rows (`Cands::Stream`); else a read probe's rows are listed first (`Cands::Read`).
/// A probe of `v` by `cols` (`key`) read into the step as found (`Cands::Stream`), by column `lead`: the memo is its
/// key, pending, until `Rows::each` reads it.
fn stream_probe<'a>(v: &'a View, cols: &'a [usize], lead: usize, key: &[i64], last: &mut LastProbe) -> Cands<'a> {
    let mut k = [0i64; STREAM_KEY];
    k[..key.len()].copy_from_slice(key);
    match &mut last.memo {
        Some((mk, q)) => {
            mk.clear();
            // copy: not block data (a probe key's values, kept to compare the next probe with)
            mk.extend_from_slice(key);
            Probed::put(q, Probed::Pending, &mut last.spare_ids);
        }
        None => last.memo = Some((key.iter().copied().collect(), Probed::Pending)),
    }
    Cands::Stream(Stream { view: v, cols, lead, key: k, len: key.len() })
}

/// The key columns a probe of `src` (by its key columns, all bound) matches exactly, which its rows need not be read
/// or checked for:
/// - a view's: the sorted index's column, a lookup on one key column, every column of a composite lookup. A view's
///   column may be a search on the tape (decision 0353 step 2: a node's parent). Where the probe is a membership test
///   (`membership`: the caller reads nothing else), a view's test by several columns (`View::tests_keys`) compares
///   every key there.
/// - an order's (a derived input's own, or a kept chain, `Csr::runs`): its leading columns that are key columns. Its
///   search compared each row it finds on them (`Index::find_at`), and the step read and compared them again: a
///   sealed chunk's decode a column a candidate (t56 step 7).
pub(super) fn exact_cols(j: &JoinRule, src: &Src, membership: bool) -> u64 {
    let Src::In(rel, ix) = src else { return 0 };
    let (rel, ix) = j.src_in(rel, ix);
    // A probe by value (`candidates`) gives the key's rows alone.
    if ix.by_value_kind() && ix.by_value(rel, 0, &mut 0).is_some() {
        return 1 << ix.cols[0];
    }
    if let Some(index) = ix.runs(rel) {
        return index.cols.iter().take_while(|c| ix.cols.contains(c)).fold(0, |m, &c| m | 1 << c);
    }
    if rel.view.is_none() {
        0
    } else if ix.composite {
        ix.cols.iter().fold(0, |m, &c| m | 1 << c)
    } else if ix.sorted || (ix.source && ix.cols.len() == 1) {
        1 << ix.cols[ix.lead]
    } else if ix.source && membership && rel.view.as_ref().is_some_and(|v| v.keys_tested(&ix.cols)) {
        ix.cols.iter().fold(0, |m, &c| m | 1 << c)
    } else {
        0
    }
}

/// A test of a derived input by one bound column (decision 0694 step 1): whether its rows hold `v` there, read in its
/// node-keyed index (one group found, `hint` the last, and one bit), not by probing an index and reading candidates.
/// `None` when the input has no such index (`Csr::node_bits_has`): the step probes as before.
pub(super) fn node_bits_test(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)], v: i64, hint: &mut u32) -> Option<bool> {
    let _kind = rerail_prof::work::doing(rerail_prof::work::DERIVED);
    let Src::In(rel, ix) = src else { return None };
    if keys.len() != 1 || !tests.is_empty() {
        return None;
    }
    let (rel, ix) = j.src_in(rel, ix);
    if ix.cols.len() != 1 || ix.cols[0] != keys[0].0 {
        return None;
    }
    ix.node_bits_has(rel, v, hint)
}

/// Whether the prepare of a filter by a view's constant is read (`access::Kind::ValueBits`): `RERAIL_VALUE_BITS=off`
/// reads the views' rows by node as before (a measurement aid).
pub(super) fn value_bits_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_VALUE_BITS").map_or(true, |v| v != "off"))
}

/// A view's test of a bound node against a constant (`kind(n, K)`): the source's columns of the bound node and of the
/// constant, and the constant, where the view is unfiltered. The source says whether it keeps bits for them
/// (`Source::value_bits`: the node column its own).
pub(super) fn value_bits_need(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)]) -> Option<(usize, usize, i64)> {
    let Src::In(rel, ix) = src else { return None };
    if keys.len() != 2 || !tests.is_empty() {
        return None;
    }
    let (rel, _) = j.src_in(rel, ix);
    let v = rel.view.as_ref()?;
    if v.ids.is_some() || !v.eqs.is_empty() {
        return None;
    }
    let (node, (col, k)) = match (keys[0], keys[1]) {
        ((a, Key::Slot(_)), (b, Key::Const(k))) | ((b, Key::Const(k)), (a, Key::Slot(_))) => (a, (b, k)),
        _ => return None,
    };
    Some((v.cols[node], v.cols[col], k))
}

/// The apply of a filter by a view's constant (prepare and apply): the bit of the step's bound node in the bits the
/// view's source built ahead for the constant (`Source::build_value_bits`), no row of the view read. `seen` is the
/// walk's for the step: the bits' chunk found at its first test (`Some(None)`: none, the row is read as before).
/// `None` where the step has no bits.
#[inline]
pub(super) fn value_bits_test(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)], slots: &[i64], seen: &mut Option<Option<crate::code_rule_dsl::facts::vtab::ValueBits>>, hint: &mut u32) -> Option<bool> {
    if seen.is_none() {
        let b = value_bits_on().then(|| value_bits_of(j, src, keys, tests)).flatten();
        if crate::code_rule_dsl::prof::on() && value_bits_need(j, src, keys, tests).is_some() {
            crate::code_rule_dsl::prof::count_named_args(format_args!("view bits: a walk's step {}", match &b { Some(b) if b.others.is_empty() => "found", Some(_) => "found, beside parts", None => "found none" }), 1);
        }
        *seen = Some(b);
    }
    let bits = seen.as_ref()?.as_ref()?;
    let n = keys.iter().find_map(|&(_, k)| match k { Key::Slot(s) => Some(slots[s]), Key::Const(_) => None })?;
    // A node of a document a part holds rows of: read as without the bits.
    if !bits.others.is_empty() && bits.others.binary_search(&((n >> 32) as u32)).is_ok() {
        return None;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    VALUE_BIT_TESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if crate::code_rule_dsl::prof::on() {
        crate::code_rule_dsl::prof::count_named_args(format_args!("view bits: tests"), 1);
    }
    // SAFETY: a chunk the source built and keeps in its block while it lives (`Source::value_bits`), which the walk's
    // join holds.
    Some(unsafe { rerail_blocks::relation::NodeIndex::in_chunk(bits.bits) }.has(n, hint))
}

/// The bits of `value_bits_need`'s constant, where the source has built them.
fn value_bits_of(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)]) -> Option<crate::code_rule_dsl::facts::vtab::ValueBits> {
    let (node, col, k) = value_bits_need(j, src, keys, tests)?;
    let Src::In(rel, ix) = src else { return None };
    let v = j.src_in(rel, ix).0.view.as_ref()?;
    if v.src.value_bits_gen() == 0 {
        return None;
    }
    v.src.value_bits(node, col, k)
}

/// Tests: the view tests answered by a bit (`value_bits_test`).
#[cfg(any(test, feature = "test-hooks"))]
pub(super) static VALUE_BIT_TESTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tests: the view tests a bit has answered in this process.
#[cfg(any(test, feature = "test-hooks"))]
pub fn value_bit_tests_for_tests() -> u64 {
    VALUE_BIT_TESTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A membership test of a derived input by several bound columns (decision 0706 E2): whether the filter the level's
/// build ahead made rejects `key` (the step's keys' values): then no row holds it, and the step reads none.
pub(super) fn filter_rejects(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)], key: &[i64]) -> bool {
    let Src::In(rel, ix) = src else { return false };
    if keys.len() < 2 || !tests.is_empty() {
        return false;
    }
    let (rel, ix) = j.src_in(rel, ix);
    ix.cols.len() == keys.len() && ix.cols.iter().zip(keys).all(|(&c, &(k, _))| c == k) && ix.filter_rejects(rel, key)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn candidates<'a>(j: &'a JoinRule, src: &'a Src, store: &'a Store, key: &[i64], keys: &[(usize, Key)], hints: &mut [u32; runs::HINTS], last: &mut LastProbe, need: u64, within: Within, stream: bool) -> (Rows<'a>, Cands<'a>) {
    let _kind = rerail_prof::work::doing(rerail_prof::work::PROBE);
    match src {
        Src::In(rel, ix) => {
            let (rel, ix) = j.src_in(rel, ix);
            let settings = last.settings();
            if settings.noted && !keys.is_empty() { probed::note(rel.name.get().map_or("?input", String::as_str), ix.cols.iter().copied()); }
            if keys.is_empty() { scan_within(Rows::In(rel), 0, rel.len() as u32, within) }
            // A one-column key's rows by its value-keyed index (the shape kernel's keyed read): found by position,
            // each probe walking on from where the last ended, not by searching the runs' tagged order.
            else if keys.len() == 1 && ix.by_value_kind()
                && let Some(ids) = ix.by_value(rel, key[0], &mut hints[0])
            {
                (Rows::In(rel), Cands::Slice(ids))
            }
            else if let Some(index) = ix.runs(rel) {
                // An order answering the key's columns, or leading with some of them (`chain_lead`: every key
                // is checked): the key's values in the order's column order.
                let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
                let mut buf = [0i64; MAX_KEY];
                let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
                let found = index.find_at(&rel.rows, rel.k, permuted(&index.cols[..m], at, &mut buf), None, hints);
                let found = match within {
                    Some((c, lo, hi)) if m == keys.len() && index.cols.get(m) == Some(&c) => {
                        if settings.noted { probed::note(&format!("{} range", rel.name.get().map_or("?input", String::as_str)), std::iter::once(c)); }
                        found.within(&rel.rows, rel.k, c, lo, hi)
                    }
                    _ => found,
                };
                (Rows::In(rel), Cands::Runs(found))
            }
            else if ix.keyed { let r = rel.keyed_rows(key[ix.lead]); (Rows::In(rel), Cands::Range(r.start, r.end)) }
            else if ix.composite && let Some(v) = &rel.view && stream && key.len() <= STREAM_KEY && v.streams(&ix.cols) && need != 0 {
                // A composite probe the view reads as found (a field by its parent and field): streamed by its first
                // column, the others handed to the source as a hint and checked by the step.
                // Led by the column the source looks up most cheaply (a field's parent).
                let lead = (0..ix.cols.len()).min_by(|&a, &b| v.src.lookup_cost(v.cols[ix.cols[a]]).total_cmp(&v.src.lookup_cost(v.cols[ix.cols[b]]))).unwrap_or(0);
                (Rows::Source(v), stream_probe(v, &ix.cols, lead, key, last))
            }
            else if ix.composite && let Some(v) = &rel.view {
                let mut eq = [(0usize, 0i64); MAX_KEY];
                for (j, &c) in ix.cols.iter().enumerate() { eq[j] = (v.cols[c], key[j]); }
                // A step that reads no column of the row (every key exact, `need` 0) asks only whether it is
                // there: a source's test where it has one (`Source::holds`, `within` with both bound). The
                // candidate it gives is never read.
                // `RERAIL_WITHIN_TEST=off` (a measurement aid) probes such a step's rows as before: the
                // run's setting (decision 0442), not the process's.
                let test = || settings.within_test;
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
                // A membership step's test (`Step::Atom`'s `exact`: every key column, nothing read): whether a row has
                // the key, compared in the source's read; else its rows by the lead, each key checked here.
                // Only a join's step or a test asks so (`stream`): a fold's or a string join's need of 0 reads its rows.
                if stream && need == 0 && v.keys_tested(&ix.cols) {
                    let found = v.tests_keys(&ix.cols, key, ix.lead, &mut last.cursor).unwrap_or_else(|| {
                        let mut hit = false;
                        v.each_keyed(&ix.cols, key, ix.lead, &mut |r| hit = hit || ix.cols.iter().zip(key).all(|(&c, &k)| v.src.value(r as usize, v.cols[c]) == k));
                        hit
                    });
                    return (Rows::Source(v), Cands::Range(0, u32::from(found)));
                }
                let probed = match &mut last.memo {
                    Some((k, p)) if k.as_slice() == key && !matches!(&*p, Probed::Pending) && !matches!(&*p, Probed::Read(_, n) if *n != need) => p.clone(),
                    _ => {
                        // A step's rows are read as the probe finds them, with the columns it reads (none for
                        // a test): a source that reads in place answers without a list of ids; one that does
                        // not gives the ids.
                        // The memo's list, which the probe before's candidates no longer hold, is this probe's.
                        // (Its answer alone is taken: its key's buffer is kept for the next.)
                        if let Some((_, q)) = &mut last.memo
                            && matches!(&*q, Probed::Read(Found::Many(_), _))
                            && let Probed::Read(Found::Many(rows), _) = std::mem::replace(q, Probed::Pending)
                        {
                            last.spare = Some(rows);
                        }
                        // Read into the step as found: the memo is its key, pending, until `Rows::each` reads it.
                        if stream && key.len() <= STREAM_KEY && v.streams(&ix.cols) {
                            return (Rows::Source(v), stream_probe(v, &ix.cols, ix.lead, key, last));
                        }
                        let read = v.find_read(&ix.cols, key, ix.lead, need, &mut last.cursor, &mut last.spare);
                        let p = match read {
                            Some(rows) => Probed::Read(rows, need),
                            None => {
                                let mut ids = last.ids();
                                v.find_keyed_into(&ix.cols, key, ix.lead, Arc::get_mut(&mut ids).expect("the step's own list"));
                                Probed::Ids(ids)
                            }
                        };
                        match &mut last.memo {
                            Some((k, q)) => {
                                k.clear();
                                // copy: not block data (a probe key's values, kept to compare the next probe with)
                                k.extend_from_slice(key);
                                Probed::put(q, p.clone(), &mut last.spare_ids);
                            }
                            None => last.memo = Some((key.iter().copied().collect(), p.clone())),
                        }
                        p
                    }
                };
                match probed {
                    Probed::Ids(ids) => (Rows::Source(v), Cands::Shared(ids)),
                    Probed::Read(rows, _) => (Rows::Source(v), Cands::Read(rows)),
                    Probed::Pending => unreachable!("a pending probe is no memo's answer"),
                }
            }
            else { (Rows::In(rel), Cands::Slice(ix.get(rel, key))) }
        }
        Src::FullMulti(r, cols) => {
            let rel = &store.rels[*r];
            if last.settings().noted { probed::note(&rel.table, cols.iter().copied()); }
            (Rows::Comp(rel), rel.probe_at(cols, key, false, hints, within).0)
        }
        Src::Delta(r) | Src::Full(r) => {
            let rel = &store.rels[*r];
            let delta = matches!(src, Src::Delta(_));
            if keys.is_empty() {
                (Rows::Comp(rel), if delta { Cands::Range(rel.lo as u32, rel.hi as u32) } else { Cands::Range(0, rel.n as u32) })
            } else {
                if last.settings().noted { probed::note(&if delta { format!("{}+delta", rel.table) } else { rel.table.clone() }, keys.iter().map(|&(c, _)| c)); }
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
        let s = a.map_or(lo as usize, |(x, eq)| rerail_blocks::order::split_at_pos(lo as usize, hi as usize, |p| if eq { at(p) < x } else { at(p) <= x }));
        let e = b.map_or(hi as usize, |(x, eq)| rerail_blocks::order::split_at_pos(s, hi as usize, |p| if eq { at(p) <= x } else { at(p) < x }));
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
    key: rerail_blocks::vector::Vector<i64>,
    /// The test's view bits by its constant, found at its first test (`value_bits_test`).
    bits: Option<Option<crate::code_rule_dsl::facts::vtab::ValueBits>>,
    hints: [u32; runs::HINTS],
    last: LastProbe,
    /// A chunk's rows read at once (`ask_view_chunk`): their columns, one after another.
    many: rerail_blocks::vector::Vector<i64>,
}

/// A test (an atom of an input that binds nothing) asked as the index's probe alone, `slots` the row's values: whether
/// a row matches its keys and its column pairs. The join's step for it (`enter`'s `Step::Atom`, a test) does the same
/// inside a walk, with the walk's frames around it (decision 0561, "a test asked directly").
pub(super) fn holds(j: &JoinRule, step: &Step, store: &Store, slots: &[i64], asked: &mut Asked) -> bool {
    let _kind = rerail_prof::work::doing(rerail_prof::work::TEST);
    let Step::Atom { src, keys, tests, binds, .. } = step else { unreachable!("a test is an atom") };
    debug_assert!(binds.is_empty(), "a test binds nothing");
    let val = |k: Key| match k {
        Key::Slot(s) => slots[s],
        Key::Const(c) => c,
    };
    if let Some(found) = value_bits_test(j, src, keys, tests, slots, &mut asked.bits, &mut asked.hints[0]) {
        return found;
    }
    asked.key.clear();
    asked.key.extend(keys.iter().map(|&(_, k)| val(k)));
    if let Some(found) = node_bits_test(j, src, keys, tests, asked.key.first().copied().unwrap_or(0), &mut asked.hints[0]) {
        return found;
    }
    if filter_rejects(j, src, keys, tests, &asked.key) {
        return false;
    }
    // As the join's step: the key columns the probe matched exactly, which its rows need not be checked for.
    let exact: u64 = if keys.is_empty() { 0 } else { exact_cols(j, src, tests.is_empty()) };
    let need: u64 = keys.iter().filter(|&&(c, _)| exact >> c & 1 == 0).fold(0u64, |m, &(c, _)| m | 1 << c) | tests.iter().fold(0u64, |m, &(c, c0)| m | 1 << c | 1 << c0);
    // A whole key of a derived input's index: its sorted hash's candidates (t50 step 5), each checked against its row.
    if let Src::In(rel, ix) = src
        && !keys.is_empty()
        && sorted_hash_on()
    {
        let (rel, ix) = j.src_in(rel, ix);
        if rel.view.is_none() && keys.len() == ix.cols.len() && ix.runs_kind() {
            let h = super::sorted_hash::hash_key(ix.cols.iter().map(|&c| keys.iter().find(|&&(kc, _)| kc == c).map_or(i64::MIN, |&(_, k)| val(k))));
            return ix.hashed(rel).candidates(h).iter().any(|&id| {
                let row = rel.at(id as usize);
                !is_dead(&rel.dead, id) && keys.iter().all(|&(c, k)| row.at(c) == val(k)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
            });
        }
    }
    let (rows, cands) = candidates(j, src, store, &asked.key, keys, &mut asked.hints, &mut asked.last, need, None, true);
    rows.each(cands, need, &mut asked.last, |id, row| {
        !rows.dead(id) && keys.iter().all(|&(c, k)| exact >> c & 1 != 0 || row.at(c) == val(k)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
    })
}

/// A lookup's one row (`setfix::lookups`: a view by a key with at most one row, a node's own facts by its id), read
/// in place (`View::read_one`): whether it holds the keys and `tests`, its values of view columns `need` in `out`.
/// `None` where the source has no such read now (it probes).
pub(super) fn read_one(j: &JoinRule, src: &Src, keys: &[(usize, Key)], tests: &[(usize, usize)], key: &[i64], need: u64, cursor: &mut crate::code_rule_dsl::facts::vtab::ProbeCursor, out: &mut [i64; super::READ]) -> Option<bool> {
    let Src::In(rel, ix) = src else { return None };
    if keys.len() > MAX_KEY {
        return None;
    }
    let (rel, ix) = j.src_in(rel, ix);
    let Some(v) = rel.view.as_ref() else { return read_one_in(rel, ix, keys, tests, key, out) };
    let _kind = rerail_prof::work::doing(rerail_prof::work::VIEW);
    let mut cols = [0usize; MAX_KEY];
    for (o, &(c, _)) in cols.iter_mut().zip(keys) {
        *o = c;
    }
    v.read_one(&cols[..keys.len()], key, tests, need, cursor, out)
}

/// `read_one` of a derived input whose key has at most one live row (`Csr::at_most_one`): its row found by the index
/// a probe reads (by value, a one-column key; else the sorted order), no candidates handed to a walk. Its columns in
/// `out`; whether it is there and holds the keys and tests. `None` where neither index answers it.
pub(super) fn read_one_in(rel: &InRel, ix: &Csr, keys: &[(usize, Key)], tests: &[(usize, usize)], key: &[i64], out: &mut [i64; super::READ]) -> Option<bool> {
    let _kind = rerail_prof::work::doing(rerail_prof::work::DERIVED);
    if rel.k > super::READ || !ix.at_most_one(rel) {
        return None;
    }
    let mut hints = [0u32; runs::HINTS];
    if keys.len() == 1 && ix.by_value_kind() && let Some(found) = ix.by_value(rel, key[0], &mut hints[0]) {
        return Some(found.iter().any(|&id| one_row(rel, id, keys, tests, key, out)));
    }
    // Its index in the relation's block: the key's bucket, each row checked (`one_row`).
    if ix.block.is_some() && keys.len() == ix.cols.len() {
        return Some(ix.get(rel, key).iter().any(|&id| one_row(rel, id, keys, tests, key, out)));
    }
    let index = ix.runs(rel)?;
    let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
    if m == 0 {
        return None;
    }
    let mut buf = [0i64; MAX_KEY];
    let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
    let found = index.find_at(&rel.rows, rel.k, permuted(&index.cols[..m], at, &mut buf), None, &mut hints);
    Some(found.ids().any(|id| one_row(rel, id, keys, tests, key, out)))
}

/// Whether row `id` of `rel` is live and holds the step's keys (`key`, in `keys`' order) and tests: its columns then
/// in `out`.
fn one_row(rel: &InRel, id: u32, keys: &[(usize, Key)], tests: &[(usize, usize)], key: &[i64], out: &mut [i64; super::READ]) -> bool {
    if is_dead(&rel.dead, id) {
        return false;
    }
    let row = rel.at(id as usize);
    if !keys.iter().zip(key).all(|(&(c, _), &k)| row.at(c) == k) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) {
        return false;
    }
    for (c, o) in out.iter_mut().enumerate().take(rel.k) {
        *o = row.at(c);
    }
    true
}

/// A fused test or lookup (`rowcode::Test`, step `step`) asked of every row of a chunk still `alive` at once, where
/// its one key column is read by value: a loop over the key's column, not a probe a row. Its forms:
/// - a test of a derived input (binds nothing): the input's node-keyed bit of each value (`Csr::node_bits_has`);
/// - a lookup of a derived input: each value's row by its value-keyed index (`Csr::by_value`, at most one live row);
/// - a lookup of a view by the column its source reads one row by (`Source::row_column`, `read_one_at`).
///
/// A row that passes gets 1 in column `verdict` and a lookup's values in `looks`' columns; one that fails is cleared
/// in `alive`. Returns the rows it found no row for and those it found one for; `None` (nothing written) where the
/// step has no such read: the caller asks row by row.
#[allow(clippy::too_many_arguments)]
pub(super) fn ask_chunk(j: &JoinRule, step: &Step, test: &rowcode::Test, columns: &mut [Vector<i64>], params: &[i64], alive: &mut [u64], n: usize, verdict: usize, looks: &[(usize, usize)], asked: &mut Asked) -> Option<(u64, u64)> {
    let Step::Atom { src: Src::In(rel, ix), keys, tests, binds, neg } = step else { return None };
    if let Some(v) = &j.src_in(rel, ix).0.view {
        return ask_view_chunk(v, keys, tests, *neg, test, columns, params, alive, n, verdict, looks, asked);
    }
    let one_key = match keys.as_slice() {
        [(col, Key::Slot(s))] if tests.is_empty() => Some((col, s)),
        _ => None,
    };
    let (rel0, ix0) = j.src_in(rel, ix);
    let _kind = rerail_prof::work::doing(rerail_prof::work::DERIVED);
    let Some((col, s)) = one_key else { return ask_derived_chunk(rel0, ix0, keys, tests, *neg, !binds.is_empty(), test, columns, params, alive, n, verdict, looks) };
    // The key's values: a column of the chunk (a test fused so reads the row, `rowcode::fused`).
    let kc = test.slots.iter().find_map(|&(x, b)| match b { rowcode::Bind::Column(k) if x == *s => Some(k), _ => None })?;
    let (rel, ix) = j.src_in(rel, ix);
    let lookup = !binds.is_empty();
    let hint = &mut asked.hints[0];
    let (mut none, mut one) = (0u64, 0u64);
    // Each alive row's verdict from `found(v)`, its look columns from `got` (input column `c`'s value).
    let mut each = |columns: &mut [Vector<i64>], alive: &mut [u64], found: &mut dyn FnMut(i64, &mut [i64; super::READ]) -> bool| {
        let mut got = [0i64; super::READ];
        for r in 0..n {
            if alive[r / 64] >> (r % 64) & 1 == 0 {
                continue;
            }
            let hit = found(columns[kc][r], &mut got);
            if hit { one += 1 } else { none += 1 }
            if hit != *neg {
                columns[verdict][r] = 1;
                for &(k, c) in looks {
                    columns[k][r] = got[c];
                }
            } else {
                alive[r / 64] &= !(1u64 << (r % 64));
            }
        }
    };
    let hashed = |columns: &mut [Vector<i64>], alive: &mut [u64]| ask_derived_chunk(rel, ix, keys, tests, *neg, lookup, test, columns, params, alive, n, verdict, looks);
    match &rel.view {
        None if ix.cols.len() != 1 || ix.cols[0] != *col => return hashed(columns, alive),
        None if !lookup => {
            if ix.node_bits_has(rel, 0, &mut 0).is_none() {
                return hashed(columns, alive);
            }
            run_counts().chunk_asks[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            each(columns, alive, &mut |v, _| ix.node_bits_has(rel, v, hint).unwrap_or(false));
        }
        None => {
            if rel.k > super::READ || !ix.by_value_kind() || ix.by_value(rel, 0, &mut 0).is_none() {
                return hashed(columns, alive);
            }
            run_counts().chunk_asks[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            each(columns, alive, &mut |v, got| {
                let Some(&id) = ix.by_value(rel, v, hint).and_then(|ids| ids.first()) else { return false };
                let row = rel.at(id as usize);
                for (c, o) in got.iter_mut().enumerate().take(rel.k) {
                    *o = row.at(c);
                }
                true
            });
        }
        Some(_) => return None,
    }
    Some((none, one))
}

/// A chunk's alive words cleared past its `n` rows (its last word's high bits): a pass over their bits reads rows
/// `0..n` alone, as a loop to `n` does.
#[inline]
fn chunk_rows_only(alive: &mut [u64], n: usize) {
    if n % 64 != 0 && let Some(w) = alive.get_mut(n / 64) {
        *w &= (1u64 << (n % 64)) - 1;
    }
}

/// `ask_chunk` of a derived input by its whole key (every column of an index `ix`, from the chunk's columns, its
/// parameters or constants): its keys hashed, a pass over each word of the chunk; their candidates found at once, by
/// the input's sorted hash (`SortedHash::candidates_word`, the index a walk's test reads) or by its block's hashed
/// index (`Relation::group`); each candidate checked against its row, a lookup's values written from it.
#[allow(clippy::too_many_arguments)]
fn ask_derived_chunk(rel: &InRel, ix: &Csr, keys: &[(usize, Key)], tests: &[(usize, usize)], neg: bool, lookup: bool, test: &rowcode::Test, columns: &mut [Vector<i64>], params: &[i64], alive: &mut [u64], n: usize, verdict: usize, looks: &[(usize, usize)]) -> Option<(u64, u64)> {
    let _kind = rerail_prof::work::doing(rerail_prof::work::DERIVED);
    if rel.view.is_some() || keys.is_empty() || keys.len() != ix.cols.len() || keys.len() > MAX_KEY || n > rowcode::CHUNK || (lookup && rel.k > super::READ) {
        return None;
    }
    let sorted = ix.runs_kind() && sorted_hash_on();
    let block = ix.block.as_ref().filter(|_| !sorted);
    if !sorted && block.is_none() {
        return None;
    }
    // Each key's value in the chunk, in the index's column order: one of its columns, else one value for the chunk.
    let mut by = [(false, 0i64); MAX_KEY];
    for (o, &c) in by.iter_mut().zip(ix.cols.iter()) {
        let &(_, k) = keys.iter().find(|&&(kc, _)| kc == c)?;
        *o = match k {
            Key::Const(v) => (false, v),
            Key::Slot(s) => match test.slots.iter().find(|&&(x, _)| x == s)?.1 {
                rowcode::Bind::Column(k) => (true, k as i64),
                rowcode::Bind::Param(p) => (false, params[p]),
            },
        };
    }
    let by = &by[..keys.len()];
    let value = |columns: &[Vector<i64>], q: usize, r: usize| if by[q].0 { columns[by[q].1 as usize][r] } else { by[q].1 };
    let hash = if sorted { Some(ix.hashed(rel)) } else { None };
    chunk_rows_only(alive, n);
    run_counts().chunk_asks[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut none, mut one) = (0u64, 0u64);
    let (mut hs, mut found) = ([0u64; 64], [(0u32, 0u32); 64]);
    for w in 0..n.div_ceil(64) {
        let (base, live) = (w * 64, alive[w]);
        let mut m = live;
        while m != 0 {
            let r = m.trailing_zeros() as usize;
            m &= m - 1;
            hs[r] = if sorted {
                super::sorted_hash::hash_key((0..by.len()).map(|q| value(columns, q, base + r)))
            } else {
                (0..by.len()).fold(rerail_blocks::SEED, |h, q| rerail_blocks::mix(h, value(columns, q, base + r)))
            };
        }
        if let Some(h) = hash {
            h.candidates_word(&hs, live, &mut found);
        }
        let (mut pass, mut m) = (0u64, live);
        while m != 0 {
            let b = m.trailing_zeros() as usize;
            m &= m - 1;
            let r = base + b;
            let cands: &[u32] = match (hash, block) {
                (Some(h), _) => h.ids(found[b]),
                (None, Some((ib, i))) => ib.relation().group(*i, hs[b]),
                (None, None) => &[],
            };
            let hit = cands.iter().copied().find(|&id| {
                if is_dead(&rel.dead, id) {
                    return false;
                }
                let row = rel.at(id as usize);
                ix.cols.iter().enumerate().all(|(q, &c)| row.at(c) == value(columns, q, r)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
            });
            if hit.is_some() { one += 1 } else { none += 1 }
            if hit.is_some() != neg {
                pass |= 1 << b;
                columns[verdict][r] = 1;
                if let Some(id) = hit && lookup {
                    let row = rel.at(id as usize);
                    for &(k, c) in looks {
                        columns[k][r] = row.at(c);
                    }
                }
            }
        }
        alive[w] = pass;
    }
    Some((none, one))
}

/// `ask_chunk` of a view: its rows of the chunk's values read by one call (`Source::read_many_at`), by a key column
/// the source reads one row by (`row_column`, `keys_rows`) that the chunk holds; the other keys (the chunk's, its
/// parameters, constants: `kind(n, #k)`) and the step's equal columns compared over the columns read, in a loop each.
#[allow(clippy::too_many_arguments)]
fn ask_view_chunk(v: &super::View, keys: &[(usize, Key)], tests: &[(usize, usize)], neg: bool, test: &rowcode::Test, columns: &mut [Vector<i64>], params: &[i64], alive: &mut [u64], n: usize, verdict: usize, looks: &[(usize, usize)], asked: &mut Asked) -> Option<(u64, u64)> {
    let _kind = rerail_prof::work::doing(rerail_prof::work::VIEW);
    if !v.eqs.is_empty() || v.ids.is_some() || v.cols.len() > super::READ || keys.len() > MAX_KEY || n > rowcode::CHUNK {
        return None;
    }
    // A key's value in the chunk: one of its columns, else one value for the chunk (a parameter, a constant).
    #[derive(Clone, Copy)]
    enum Val { Col(usize), One(i64) }
    let key_val = |k: Key| match k {
        Key::Const(c) => Some(Val::One(c)),
        Key::Slot(s) => test.slots.iter().find(|&&(x, _)| x == s).map(|&(_, b)| match b {
            rowcode::Bind::Column(c) => Val::Col(c),
            rowcode::Bind::Param(p) => Val::One(params[p]),
        }),
    };
    let own = v.src.row_column();
    let lead = keys.iter().position(|&(c, k)| matches!(key_val(k), Some(Val::Col(_))) && (own == Some(v.cols[c]) || v.src.keys_rows(v.cols[c])))?;
    let (lc, lk) = keys[lead];
    let Some(Val::Col(kc)) = key_val(lk) else { return None };
    // The view columns read: the other keys', the tests', the looks'.
    let (mut src, mut at, mut m) = ([0usize; super::READ], [0usize; super::READ], 0usize);
    let mut want = |c: usize| match at[..m].iter().position(|&x| x == c) {
        Some(p) => p,
        None => {
            (src[m], at[m]) = (v.cols[c], c);
            m += 1;
            m - 1
        }
    };
    let mut others = [(0usize, Val::One(0)); MAX_KEY];
    let mut no = 0;
    for (x, &(c, k)) in keys.iter().enumerate() {
        if x != lead {
            others[no] = (want(c), key_val(k)?);
            no += 1;
        }
    }
    let mut eqs = [(0usize, 0usize); MAX_KEY];
    if tests.len() > MAX_KEY {
        return None;
    }
    for (e, &(c, c0)) in eqs.iter_mut().zip(tests) {
        *e = (want(c), want(c0));
    }
    let mut lk_at = [(0usize, 0usize); super::READ];
    for (o, &(k, c)) in lk_at.iter_mut().zip(looks) {
        *o = (k, if c == lc { usize::MAX } else { want(c) });
    }
    let (src, m) = (&src[..m], m);
    let words = n.div_ceil(64);
    chunk_rows_only(alive, n);
    let mut hit = [0u64; rowcode::CHUNK / 64];
    hit[..words].copy_from_slice(&alive[..words]);
    let buf = &mut asked.many;
    if buf.len() < m * n {
        buf.resize(m * n, 0);
    }
    v.src.read_many_at(v.cols[lc], &columns[kc][..n], src, &mut buf[..m * n], &mut hit[..words], &mut asked.last.cursor)?;
    run_counts().chunk_asks[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // The other keys and equal columns, a pass each over the rows read.
    for &(p, k) in &others[..no] {
        let got = &buf[p * n..p * n + n];
        for (w, word) in hit[..words].iter_mut().enumerate() {
            let mut mm = *word;
            while mm != 0 {
                let r = w * 64 + mm.trailing_zeros() as usize;
                mm &= mm - 1;
                let want = match k { Val::Col(c) => columns[c][r], Val::One(x) => x };
                if got[r] != want {
                    *word &= !(1u64 << (r % 64));
                }
            }
        }
    }
    for &(a, b) in &eqs[..tests.len()] {
        for (w, word) in hit[..words].iter_mut().enumerate() {
            let mut mm = *word;
            while mm != 0 {
                let r = w * 64 + mm.trailing_zeros() as usize;
                mm &= mm - 1;
                if buf[a * n + r] != buf[b * n + r] {
                    *word &= !(1u64 << (r % 64));
                }
            }
        }
    }
    let (mut none, mut one) = (0u64, 0u64);
    for w in 0..words {
        let (was, found) = (alive[w], hit[w] & alive[w]);
        one += u64::from(found.count_ones());
        none += u64::from((was & !found).count_ones());
        let pass = if neg { was & !found } else { found };
        alive[w] = pass;
        let mut mm = pass;
        while mm != 0 {
            let r = w * 64 + mm.trailing_zeros() as usize;
            mm &= mm - 1;
            columns[verdict][r] = 1;
            if !neg {
                for &(k, p) in &lk_at[..looks.len()] {
                    columns[k][r] = if p == usize::MAX { columns[kc][r] } else { buf[p * n + r] };
                }
            }
        }
    }
    Some((none, one))
}

/// Why `ask_chunk` has no read of a step (profiled, `setfix::SHAPES` 27..34): its names.
pub const CHUNK_REFUSALS: [&str; 7] = ["other", "view_ids", "derived_part_key", "view_no_read", "view_no_lead", "view_eqs", "derived_no_index"];

/// The reason (`CHUNK_REFUSALS`) a step's chunk ask fell back to row by row, by `ask_view_chunk`'s and
/// `ask_derived_chunk`'s conditions in turn.
pub(super) fn chunk_refusal(j: &JoinRule, step: &Step, test: &rowcode::Test) -> usize {
    let Step::Atom { src: Src::In(rel, ix), keys, .. } = step else { return 0 };
    let (rel, ix) = j.src_in(rel, ix);
    let in_chunk = |k: Key| matches!(k, Key::Slot(s) if test.slots.iter().any(|&(x, b)| x == s && matches!(b, rowcode::Bind::Column(_))));
    match &rel.view {
        Some(v) if !v.eqs.is_empty() => 5,
        Some(v) if v.ids.is_some() => 1,
        Some(v) => {
            let own = v.src.row_column();
            if keys.iter().any(|&(c, k)| in_chunk(k) && (own == Some(v.cols[c]) || v.src.keys_rows(v.cols[c]))) { 3 } else { 4 }
        }
        None if keys.is_empty() || keys.len() != ix.cols.len() => 2,
        None => 6,
    }
}

/// A lookup asked in a fused chunk (`rowcode::Test::reads`), `slots` the row's values: whether its one row is there
/// and holds its keys and tests, its values of the columns it binds in `out` (by column). Read in place
/// (`read_one`); where the source cannot read it so now, probed, its first matching row's.
pub(super) fn look(j: &JoinRule, step: &Step, store: &Store, slots: &[i64], asked: &mut Asked, out: &mut [i64]) -> bool {
    let _kind = rerail_prof::work::doing(rerail_prof::work::TEST);
    let Step::Atom { src, keys, tests, binds, .. } = step else { unreachable!("a lookup is an atom") };
    // A lookup that binds nothing is a test: a bit, where the view's bits by its constant are built.
    if binds.is_empty() && let Some(found) = value_bits_test(j, src, keys, tests, slots, &mut asked.bits, &mut asked.hints[0]) {
        return found;
    }
    let val = |k: Key| match k {
        Key::Slot(s) => slots[s],
        Key::Const(c) => c,
    };
    asked.key.clear();
    asked.key.extend(keys.iter().map(|&(_, k)| val(k)));
    let need = binds.iter().fold(0u64, |m, &(c, _)| m | 1 << c);
    let mut row = [0i64; super::READ];
    if let Some(found) = read_one(j, src, keys, tests, &asked.key, need, &mut asked.last.cursor, &mut row) {
        if found {
            for &(c, _) in binds {
                out[c] = row[c];
            }
        }
        return found;
    }
    let need = need | keys.iter().fold(0u64, |m, &(c, _)| m | 1 << c) | tests.iter().fold(0u64, |m, &(c, c0)| m | 1 << c | 1 << c0);
    let (rows, cands) = candidates(j, src, store, &asked.key, keys, &mut asked.hints, &mut asked.last, need, None, true);
    let mut found = false;
    rows.each(cands, need, &mut asked.last, |id, r| {
        if rows.dead(id) || !keys.iter().all(|&(c, k)| r.at(c) == val(k)) || !tests.iter().all(|&(c, c0)| r.at(c) == r.at(c0)) {
            return false;
        }
        for &(c, _) in binds {
            out[c] = r.at(c);
        }
        found = true;
        true
    });
    found
}

/// Whether a test of one node column reads its node-keyed index: `RERAIL_NODE_BITS=off` (a measurement aid, the
/// run's setting) probes as before.
pub(super) fn node_bits_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_NODE_BITS").map_or(true, |v| v != "off"))
}

/// Whether a one-column probe of a derived input reads its value-keyed rows index (`Csr::by_value`):
/// `RERAIL_BY_VALUE=off` (a measurement aid, the run's setting) probes as before.
pub(super) fn by_value_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_BY_VALUE").map_or(true, |v| v != "off"))
}

/// Whether a test of a derived input's whole key asks its sorted hash: `RERAIL_SORTED_HASH=off` (a measurement aid,
/// the run's setting) asks its index runs, as before.
fn sorted_hash_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SORTED_HASH").map_or(true, |v| v != "off"))
}
