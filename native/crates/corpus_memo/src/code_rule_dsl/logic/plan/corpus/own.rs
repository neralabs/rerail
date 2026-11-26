//! A chunk's own statistics: what it measures of its base relations as it binds them, kept in its memo entry's rows
//! (`memo::Entry::rows`, under `OWN_*`).

use rerail_blocks::maps::BlockHash;
use rerail_blocks::vector::Vector;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A memo entry's rows holding its file's tree statistics (`Own::put`).
pub const OWN_KINDS: &str = "own:kinds";
pub const OWN_EDGES: &str = "own:edges";
pub const OWN_FIELDS: &str = "own:fields";
pub const OWN_DOCS: &str = "own:docs";
pub const OWN_POSTINGS: &str = "own:postings:";

/// The base relations' statistics a chunk measures as it binds them, beyond each column's moments
/// (`syntax::statistics`), which T's `stats.py` made at ingestion: per kind atom its nodes and summed subtree sizes
/// (`stat_kind`), per (parent kind, child kind) its edges (`stat_edge`), the documents, and per (relation, column,
/// constant) of the program's constants its rows (`stat_val`'s `n`). Summed over the chunks for the cross-file part:
/// a document is in one chunk, so every count adds exactly.
/// Each count in a block table (decision 0628 §4), read in its keys' order where the order is written.
#[derive(Clone, Default)]
pub struct Own {
    /// Per kind: its nodes and their summed subtree sizes.
    pub kinds: BlockHash<i64, [i64; 2]>,
    /// Per (parent kind, child kind): its edges.
    pub edges: BlockHash<[i64; 2], i64>,
    /// Per (parent kind, child kind, field) its fielded edges (`field`'s rows): which kinds have a field, and which
    /// kinds a field's children are (Q44: `kind($n, k)` after `field($n, f, _)` passes nearly every row).
    pub fields: BlockHash<[i64; 3], i64>,
    pub docs: i64,
    /// Per (relation, column, value), the names numbered by `table::rel_id`: its rows.
    pub postings: BlockHash<[i64; 3], i64>,
}

/// `m`'s entries in its keys' order.
fn in_order<K: rerail_blocks::maps::Key + Ord, V: rerail_blocks::Pod + Copy>(m: &BlockHash<K, V>) -> impl Iterator<Item = (K, V)> + '_ {
    let mut keys: Vector<K> = m.keys_iter().collect();
    keys.sort_unstable();
    (0..keys.len()).map(move |i| (keys[i], *m.get(&keys[i]).expect("listed")))
}

/// Rows of `N` values, each led by its entry's key, written once and sorted in place: in the keys' order, as each key
/// is one entry's (`in_order`'s, with no list of the keys: 1.13 M on cold PostHog).
fn sorted_rows<const N: usize>(values: impl Iterator<Item = i64>) -> Vector<i64> {
    let mut rows: Vector<i64> = values.collect();
    rows.as_chunks_mut::<N>().0.sort_unstable();
    rows
}

/// A posting's key: its relation's and column's numbers, and the value.
fn posting_key(rel: &str, col: &str, v: i64) -> [i64; 3] {
    [i64::from(crate::code_rule_dsl::logic::plan::table::rel_id(rel)), i64::from(crate::code_rule_dsl::logic::plan::table::rel_id(col)), v]
}

/// A posting key's relation and column names.
fn posting_names(k: [i64; 3]) -> (Arc<str>, Arc<str>) {
    (crate::code_rule_dsl::logic::plan::table::rel_name(k[0] as u32), crate::code_rule_dsl::logic::plan::table::rel_name(k[1] as u32))
}

/// `m`'s count at `k` changed by `by`, the entry dropped where it reaches 0 (or below, `floor`).
fn count_by<K: rerail_blocks::maps::Key>(m: &mut BlockHash<K, i64>, k: K, by: i64, floor: bool) {
    let c = m.get(&k).copied().unwrap_or(0) + by;
    if c == 0 || floor && c < 0 {
        m.remove(&k);
    } else {
        m.insert(k, c);
    }
}

impl Own {
    /// The program's integer constants by (input relation, column name) they are compared at, read from the
    /// program's version by its rules' handles. A kind, field or name is a branded atom (decision 0270), compared as
    /// its atom's value, as `Body::constant` reads it (Q36: unread, `field($l, "left", $t)` had no postings, and was
    /// priced at one row a line). A constant tested of a relation that copies an input's columns (`kind(n, k) :-
    /// node(_, n, k, _, _)`, Q37) is counted at the input's column it copies (`constants_tested_in`): the input is
    /// what has statistics, and `kind`'s tests had none since `kind` became a view.
    pub fn constants_of(v: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> BTreeMap<(String, String), BTreeSet<i64>> {
        let inputs: BTreeSet<&str> = v.inputs().map(|a| v.text(a)).collect();
        let mut out: BTreeMap<(String, String), BTreeSet<i64>> = BTreeMap::new();
        for (rel, tested) in &tested_constants(v).0 {
            if !inputs.contains(rel.as_str()) {
                continue;
            }
            for &(col, n) in tested {
                out.entry((rel.clone(), super::super::sql_col(rel, col))).or_default().insert(n);
            }
        }
        out
    }

    pub fn measure(sources: &[(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)], constants: &BTreeMap<(String, String), BTreeSet<i64>>) -> Self {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut own = Own::default();
        let of = |rel: &str| sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref());
        let column = |source: &dyn crate::code_rule_dsl::facts::vtab::Source, name: &str| source.columns().iter().position(|c| c == name);
        // Kinds by node id, then the tape's sizes and parents against them. Counted in hash maps (a chunk is a
        // bounded prefix: an ordered map per node was most of this measure's time) and ordered once.
        let mut kind_of = FxHashMap::default();
        let mut kinds: BlockHash<i64, [i64; 2]> = BlockHash::default();
        let mut edges: BlockHash<[i64; 2], i64> = BlockHash::default();
        if let Some(node) = of("node")
            && let (Some(n), Some(k), Some(d)) = (column(node, "n"), column(node, "kind"), column(node, "d"))
        {
            kind_of.reserve(node.len().min(STAT_SAMPLE_ROWS));
            let mut docs = FxHashSet::default();
            for r in 0..node.len().min(STAT_SAMPLE_ROWS) {
                let kind = node.value(r, k);
                kind_of.insert(node.value(r, n), kind);
                kinds.entry(kind).or_default()[0] += 1;
                docs.insert(node.value(r, d));
            }
            own.docs = docs.len() as i64;
        }
        if let Some(tape) = of("tape")
            && let (Some(n), Some(par), Some(size)) = (column(tape, "n"), column(tape, "par"), column(tape, "size"))
        {
            for r in 0..tape.len().min(STAT_SAMPLE_ROWS) {
                let Some(&kind) = kind_of.get(&tape.value(r, n)) else { continue };
                kinds.entry(kind).or_default()[1] += tape.value(r, size);
                if let Some(&parent) = kind_of.get(&tape.value(r, par)) {
                    *edges.entry([parent, kind]).or_default() += 1;
                }
            }
        }
        own.kinds = kinds;
        own.edges = edges;
        if let Some(node) = of("node") {
            let sampled = node.len().min(STAT_SAMPLE_ROWS);
            if sampled > 0 { own = own.scaled(node.len() as f64 / sampled as f64); }
        }
        // The fielded edges of the sampled nodes (`field`'s rows in node order, as the tape's above): each with its
        // parent's and child's kind, scaled as the edges are. A view's rows past the sampled nodes have no kind here.
        if let Some(field) = of("field")
            && let (Some(p), Some(f), Some(c)) = (column(field, "p"), column(field, "f"), column(field, "c"))
            && let Some(node) = of("node")
        {
            let sampled = node.len().min(STAT_SAMPLE_ROWS);
            let scale = if sampled > 0 { node.len() as f64 / sampled as f64 } else { 1.0 };
            let mut fields: BlockHash<[i64; 3], i64> = BlockHash::default();
            for r in 0..field.len().min(2 * STAT_SAMPLE_ROWS) {
                if let (Some(&pk), Some(&ck)) = (kind_of.get(&field.value(r, p)), kind_of.get(&field.value(r, c))) {
                    *fields.entry([pk, ck, field.value(r, f)]).or_default() += 1;
                }
            }
            fields.for_each_mut(|_, n| *n = (*n as f64 * scale).round().max(1.0) as i64);
            own.fields = fields;
        }
        if let Some(doc) = of("doc") { own.docs = doc.estimated_len() as i64; }
        own.postings = Self::postings(sources, constants, |_| true);
        own
    }

    /// The program's constants' postings in the relations `of_rel` keeps: rows per (relation, column, value).
    pub fn postings(
        sources: &[(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)],
        constants: &BTreeMap<(String, String), BTreeSet<i64>>,
        of_rel: impl Fn(&str) -> bool,
    ) -> BlockHash<[i64; 3], i64> {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut out = BlockHash::default();
        for ((rel, col), values) in constants {
            if !of_rel(rel) {
                continue;
            }
            let Some(source) = sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref()) else { continue };
            let Some(c) = source.columns().iter().position(|n| n == col) else { continue };
            let asked: Vec<_> = values.iter().map(|&v| (c,v)).collect();
            if let Some(counts) = source.value_counts(&asked) {
                for ((_, v), n) in asked.into_iter().zip(counts) {
                    out.insert(posting_key(rel, col, v), n);
                }
                continue;
            }
            let wanted: FxHashSet<i64> = values.iter().copied().collect();
            let mut counts: FxHashMap<i64, i64> = FxHashMap::default();
            if source.made_on_lookup() { continue; }
            let rows = source.len();
            let sampled = rows.min(STAT_SAMPLE_ROWS);
            for r in 0..sampled {
                let at = (r as u128 * rows as u128 / sampled as u128) as usize;
                let v = source.value(at, c);
                if wanted.contains(&v) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            for (v, n) in counts {
                out.insert(posting_key(rel, col, v), (n as f64 * rows as f64 / sampled.max(1) as f64).round() as i64);
            }
        }
        out
    }

    /// These counts for a chunk `r` times this one's size: every count is of nodes, edges, documents or rows, so
    /// each scales by `r`.
    pub fn scaled(&self, r: f64) -> Own {
        let s = |n: i64| (n as f64 * r).round() as i64;
        let mut own = self.clone();
        own.kinds.for_each_mut(|_, v| *v = [s(v[0]), s(v[1])]);
        own.edges.for_each_mut(|_, n| *n = s(*n));
        own.fields.for_each_mut(|_, n| *n = s(*n));
        own.docs = s(self.docs).max(1);
        own.postings.for_each_mut(|_, n| *n = s(*n));
        own
    }


    pub fn add(&mut self, other: &Own) {
        for (k, [n, size]) in other.kinds.iter() {
            let e = self.kinds.entry(k).or_default();
            e[0] += n;
            e[1] += size;
        }
        for (e, n) in other.edges.iter() {
            *self.edges.entry(e).or_default() += n;
        }
        for (e, n) in other.fields.iter() {
            *self.fields.entry(e).or_default() += n;
        }
        self.docs += other.docs;
        for (key, n) in other.postings.iter() {
            *self.postings.entry(key).or_default() += n;
        }
    }

    /// `other` taken away (a file's share counted out): counts that reach 0 are dropped, as a sum without it holds.
    pub fn sub(&mut self, other: &Own) {
        for (k, [n, size]) in other.kinds.iter() {
            let [a, b] = self.kinds.get(&k).copied().unwrap_or([0, 0]);
            if [a - n, b - size] == [0, 0] {
                self.kinds.remove(&k);
            } else {
                self.kinds.insert(k, [a - n, b - size]);
            }
        }
        for (e, n) in other.edges.iter() {
            count_by(&mut self.edges, e, -n, false);
        }
        for (e, n) in other.fields.iter() {
            count_by(&mut self.fields, e, -n, true);
        }
        self.docs -= other.docs;
        for (key, n) in other.postings.iter() {
            count_by(&mut self.postings, key, -n, false);
        }
    }

    /// Into a memo entry's rows (decision 0052): a memoised file hands on its share of the tree statistics as an
    /// evaluated one does.
    pub fn put(&self, rows: &mut BTreeMap<String, (usize, Vector<i64>)>) {
        rows.insert(OWN_KINDS.into(), (3, sorted_rows::<3>(self.kinds.iter().flat_map(|(k, [n, size])| [k, n, size]))));
        rows.insert(OWN_EDGES.into(), (3, sorted_rows::<3>(self.edges.iter().flat_map(|([p, k], n)| [p, k, n]))));
        rows.insert(OWN_FIELDS.into(), (4, sorted_rows::<4>(self.fields.iter().flat_map(|([p, k, f], n)| [p, k, f, n]))));
        rows.insert(OWN_DOCS.into(), (1, rerail_blocks::vector![self.docs]));
        // By relation and column name, then value: the order the entry's rows were written in by name.
        let mut posted: Vec<(Arc<str>, Arc<str>, i64, i64)> = self.postings.iter().map(|(k, n)| {
            let (rel, col) = posting_names(k);
            (rel, col, k[2], n)
        }).collect();
        posted.sort_unstable();
        // Sorted by relation and column: each (relation, column)'s rows a run, its name made once (a name a posting
        // was 0.89 M on cold PostHog).
        let mut at = 0;
        while at < posted.len() {
            let (rel, col) = (&posted[at].0, &posted[at].1);
            let end = at + posted[at..].iter().take_while(|p| &p.0 == rel && &p.1 == col).count();
            let held = &mut rows.entry(format!("{OWN_POSTINGS}{rel}:{col}")).or_insert_with(|| (2, rerail_blocks::vector::Vector::new())).1;
            held.extend(posted[at..end].iter().flat_map(|&(_, _, v, n)| [v, n]));
            at = end;
        }
    }

    /// Add a frame directly to the run's totals. No per-file maps are reconstructed.
    pub fn add_rows(&mut self, rows: &BTreeMap<String, (usize, Vector<i64>)>) {
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        for r in flat(OWN_KINDS).as_chunks::<3>().0 {
            let k = self.kinds.entry(r[0]).or_default(); k[0] += r[1]; k[1] += r[2];
        }
        for r in flat(OWN_EDGES).as_chunks::<3>().0 { *self.edges.entry([r[0], r[1]]).or_default() += r[2]; }
        for r in flat(OWN_FIELDS).as_chunks::<4>().0 { *self.fields.entry([r[0], r[1], r[2]]).or_default() += r[3]; }
        self.docs += flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            for r in values.as_chunks::<2>().0 { *self.postings.entry(posting_key(rel, col, r[0])).or_default() += r[1]; }
        }
    }

    /// From a memo entry's rows (`put`).
    pub fn taken(rows: &BTreeMap<String, (usize, Vector<i64>)>) -> Own {
        let mut own = Own::default();
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        own.kinds = flat(OWN_KINDS).chunks(3).map(|r| (r[0], [r[1], r[2]])).collect();
        own.edges = flat(OWN_EDGES).chunks(3).map(|r| ([r[0], r[1]], r[2])).collect();
        own.fields = flat(OWN_FIELDS).as_chunks::<4>().0.iter().map(|r| ([r[0], r[1], r[2]], r[3])).collect();
        own.docs = flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            for r in values.chunks(2) {
                own.postings.insert(posting_key(rel, col, r[0]), r[1]);
            }
        }
        own
    }

    pub fn apply(&self, stats: &mut super::super::Stats) {
        stats.tree(in_order(&self.kinds).map(|(k, [n, size])| (k, n, size)), in_order(&self.edges).map(|([p, c], n)| (p, c, n)), self.docs);
        stats.fielded(in_order(&self.fields).map(|([p, c, f], n)| (p.to_string(), c.to_string(), f.to_string(), n as f64)));
        let mut by: BTreeMap<Arc<str>, Vec<(String, i64, i64)>> = BTreeMap::new();
        for (k, n) in in_order(&self.postings) {
            let (rel, col) = posting_names(k);
            by.entry(rel).or_default().push((col.to_string(), k[2], n));
        }
        for (rel, vals) in by {
            stats.measured(&rel, [], vals);
        }
    }
}

// A statistics sample has its own row budget, independent of execution chunks.
#[doc(hidden)]
pub const STAT_SAMPLE_ROWS: usize = 8192;

/// Per relation, the integer constants its atoms test, by column (`constant_counts`'s), read from version `v` by its
/// rules' handles: one pass over the rules, not one per relation. A relation that copies another's columns (a rule
/// of one positive atom) passes its tests on to the columns it copies (`tested_through`).
pub fn constants_tested_in(v: &crate::code_rule_dsl::logic::program_image::Version) -> HashMap<&str, BTreeSet<(usize, i64)>> {
    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
    let mut by: HashMap<&str, BTreeSet<(usize, i64)>> = HashMap::new();
    let mut projections: HashMap<(&str, usize), Vec<(&str, usize)>> = HashMap::new();
    for r in v.rules() {
        for a in r.reads() {
            for (c, t) in a.args().enumerate() {
                if let Some(n) = match t { TermRef::Int(n) => Some(n), TermRef::Branded(b) => Some(b.value()), _ => None } {
                    by.entry(v.rel_name(a.rel())).or_default().insert((c, n));
                }
            }
        }
        let mut body = r.body();
        let (Some(only), None) = (body.next(), body.next()) else { continue };
        let Lit::Atom(a) = only.view() else { continue };
        if a.neg() { continue; }
        let head = r.head();
        for (hc, term) in head.args().enumerate() {
            let TermRef::Var(s) = term else { continue };
            for (bc, arg) in a.args().enumerate() {
                if arg.var() == Some(s) {
                    projections.entry((v.rel_name(head.rel()), hc)).or_default().push((v.rel_name(a.rel()), bc));
                }
            }
        }
    }
    tested_through(by, &projections)
}

/// `constants_tested_in`'s, held by the version (`manager::analysis`): the run asks it of the prepared program three
/// times (its owned statistics' constants, the values tested, the exports' counts), each a walk of every rule.
#[derive(Debug, Default, PartialEq)]
pub struct TestedConstants(pub HashMap<String, BTreeSet<(usize, i64)>>);

impl crate::code_rule_dsl::logic::program_image::manager::Analysis for TestedConstants {
    const NAME: &'static str = "constants tested";
    const READS: crate::code_rule_dsl::logic::program_image::manager::Reads = crate::code_rule_dsl::logic::program_image::manager::Reads::RULES;
    fn compute(cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>) -> Result<TestedConstants, String> {
        Ok(TestedConstants(constants_tested_in(cx.v()).into_iter().map(|(r, c)| (r.to_owned(), c)).collect()))
    }
}

/// `v`'s constants tested (`constants_tested_in`), held by it.
pub fn tested_constants(v: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Arc<TestedConstants> {
    crate::code_rule_dsl::logic::program_image::manager::analysis::<TestedConstants>(v)
        .unwrap_or_else(|_| Arc::new(TestedConstants(constants_tested_in(v).into_iter().map(|(r, c)| (r.to_owned(), c)).collect())))
}

/// The constants tested `by` relation, each also tested of the column a single-atom projection copies it from
/// (`projections`: a head column to the body columns it copies): a consumer may test a projection, while
/// statistics are collected on its source. A worklist also handles alias chains and cycles.
pub fn tested_through<'a>(mut by: HashMap<&'a str, BTreeSet<(usize, i64)>>, projections: &HashMap<(&'a str, usize), Vec<(&'a str, usize)>>) -> HashMap<&'a str, BTreeSet<(usize, i64)>> {
    let mut queue: Vec<(&str, usize, i64)> = by.iter().flat_map(|(r, cols)| cols.iter().map(move |(c, v)| (*r, *c, *v))).collect();
    while let Some((rel, col, value)) = queue.pop() {
        for &(source, at) in projections.get(&(rel, col)).into_iter().flatten() {
            if by.entry(source).or_default().insert((at, value)) { queue.push((source, at, value)); }
        }
    }
    by
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_tree_statistics_accumulate_once_without_per_file_maps() {
        let mut first = Own::default();
        first.kinds.insert(1,[10,20]); first.edges.insert([1,2],5); first.fields.insert([1,2,3],4); first.docs=1;
        first.postings.insert(posting_key("kind","k",1),10);
        let mut rows=BTreeMap::new(); first.put(&mut rows);
        let mut totals=Own::default(); totals.add_rows(&rows); totals.add_rows(&rows);
        assert_eq!(totals.kinds.get(&1),Some(&[20,40])); assert_eq!(totals.edges.get(&[1,2]),Some(&10));
        assert_eq!(totals.docs,2); assert_eq!(totals.postings.values().sum::<i64>(),20);
        assert_eq!(totals.fields.get(&[1,2,3]),Some(&8), "fielded edges sum as the edges do");
        assert_eq!(in_order(&Own::taken(&rows).fields).collect::<Vec<_>>(), in_order(&first.fields).collect::<Vec<_>>());
        totals.sub(&first); totals.sub(&first); assert!(totals.fields.is_empty());
        let projected=BTreeMap::from([(OWN_DOCS.into(),(1,rerail_blocks::vector![1]))]);
        let mut narrow=Own::default(); narrow.add_rows(&projected);
        assert_eq!(narrow.docs,1); assert!(narrow.kinds.is_empty()); assert!(narrow.edges.is_empty());
    }
}
