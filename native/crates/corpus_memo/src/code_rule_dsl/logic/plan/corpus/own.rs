//! A chunk's own statistics: what it measures of its base relations as it binds them, kept in its memo entry's rows
//! (`memo::Entry::rows`, under `OWN_*`).

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
#[derive(Clone, Default)]
pub struct Own {
    pub kinds: BTreeMap<i64, (i64, i64)>,
    pub edges: BTreeMap<(i64, i64), i64>,
    /// Per (parent kind, child kind, field) its fielded edges (`field`'s rows): which kinds have a field, and which
    /// kinds a field's children are (Q44: `kind($n, k)` after `field($n, f, _)` passes nearly every row).
    pub fields: BTreeMap<(i64, i64, i64), i64>,
    pub docs: i64,
    pub postings: BTreeMap<(String, String, i64), i64>,
}

impl Own {
    /// The program's integer constants by (input relation, column name) they are compared at, read from the
    /// program's version by its rules' handles. A kind, field or name is a branded atom (decision 0270), compared as
    /// its atom's value, as `Body::constant` reads it (Q36: unread, `field($l, "left", $t)` had no postings, and was
    /// priced at one row a line). A constant tested of a relation that copies an input's columns (`kind(n, k) :-
    /// node(_, n, k, _, _)`, Q37) is counted at the input's column it copies (`constants_tested_in`): the input is
    /// what has statistics, and `kind`'s tests had none since `kind` became a view.
    pub fn constants_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> BTreeMap<(String, String), BTreeSet<i64>> {
        let inputs: BTreeSet<&str> = v.inputs().map(|a| v.text(a)).collect();
        let mut out: BTreeMap<(String, String), BTreeSet<i64>> = BTreeMap::new();
        for (rel, tested) in constants_tested_in(v) {
            if !inputs.contains(rel) {
                continue;
            }
            for (col, n) in tested {
                out.entry((rel.to_owned(), super::super::sql_col(rel, col))).or_default().insert(n);
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
        let mut kinds: FxHashMap<i64, (i64, i64)> = FxHashMap::default();
        let mut edges: FxHashMap<(i64, i64), i64> = FxHashMap::default();
        if let Some(node) = of("node")
            && let (Some(n), Some(k), Some(d)) = (column(node, "n"), column(node, "kind"), column(node, "d"))
        {
            kind_of.reserve(node.len().min(STAT_SAMPLE_ROWS));
            let mut docs = FxHashSet::default();
            for r in 0..node.len().min(STAT_SAMPLE_ROWS) {
                let kind = node.value(r, k);
                kind_of.insert(node.value(r, n), kind);
                kinds.entry(kind).or_default().0 += 1;
                docs.insert(node.value(r, d));
            }
            own.docs = docs.len() as i64;
        }
        if let Some(tape) = of("tape")
            && let (Some(n), Some(par), Some(size)) = (column(tape, "n"), column(tape, "par"), column(tape, "size"))
        {
            for r in 0..tape.len().min(STAT_SAMPLE_ROWS) {
                let Some(&kind) = kind_of.get(&tape.value(r, n)) else { continue };
                kinds.entry(kind).or_default().1 += tape.value(r, size);
                if let Some(&parent) = kind_of.get(&tape.value(r, par)) {
                    *edges.entry((parent, kind)).or_default() += 1;
                }
            }
        }
        own.kinds = kinds.into_iter().collect();
        own.edges = edges.into_iter().collect();
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
            let mut fields: FxHashMap<(i64, i64, i64), i64> = FxHashMap::default();
            for r in 0..field.len().min(2 * STAT_SAMPLE_ROWS) {
                if let (Some(&pk), Some(&ck)) = (kind_of.get(&field.value(r, p)), kind_of.get(&field.value(r, c))) {
                    *fields.entry((pk, ck, field.value(r, f))).or_default() += 1;
                }
            }
            own.fields = fields.into_iter().map(|(k, n)| (k, (n as f64 * scale).round().max(1.0) as i64)).collect();
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
    ) -> BTreeMap<(String, String, i64), i64> {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut out = BTreeMap::new();
        for ((rel, col), values) in constants {
            if !of_rel(rel) {
                continue;
            }
            let Some(source) = sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref()) else { continue };
            let Some(c) = source.columns().iter().position(|n| n == col) else { continue };
            let asked: Vec<_> = values.iter().map(|&v| (c,v)).collect();
            if let Some(counts) = source.value_counts(&asked) {
                out.extend(asked.into_iter().zip(counts).map(|((_,v),n)| ((rel.clone(),col.clone(),v),n)));
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
            out.extend(counts.into_iter().map(|(v, n)| ((rel.clone(), col.clone(), v), (n as f64 * rows as f64 / sampled.max(1) as f64).round() as i64)));
        }
        out
    }

    /// These counts for a chunk `r` times this one's size: every count is of nodes, edges, documents or rows, so
    /// each scales by `r`.
    pub fn scaled(&self, r: f64) -> Own {
        let s = |n: i64| (n as f64 * r).round() as i64;
        Own {
            kinds: self.kinds.iter().map(|(k, (n, size))| (*k, (s(*n), s(*size)))).collect(),
            edges: self.edges.iter().map(|(e, n)| (*e, s(*n))).collect(),
            fields: self.fields.iter().map(|(e, n)| (*e, s(*n))).collect(),
            docs: s(self.docs).max(1),
            postings: self.postings.iter().map(|(k, n)| (k.clone(), s(*n))).collect(),
        }
    }


    pub fn add(&mut self, other: &Own) {
        for (k, (n, size)) in &other.kinds {
            let e = self.kinds.entry(*k).or_default();
            e.0 += n;
            e.1 += size;
        }
        for (e, n) in &other.edges {
            *self.edges.entry(*e).or_default() += n;
        }
        for (e, n) in &other.fields {
            *self.fields.entry(*e).or_default() += n;
        }
        self.docs += other.docs;
        for (key, n) in &other.postings {
            *self.postings.entry(key.clone()).or_default() += n;
        }
    }

    /// `other` taken away (a file's share counted out): counts that reach 0 are dropped, as a sum without it holds.
    pub fn sub(&mut self, other: &Own) {
        for (k, (n, size)) in &other.kinds {
            let e = self.kinds.entry(*k).or_default();
            e.0 -= n;
            e.1 -= size;
            if *e == (0, 0) {
                self.kinds.remove(k);
            }
        }
        for (e, n) in &other.edges {
            let c = self.edges.entry(*e).or_default();
            *c -= n;
            if *c == 0 {
                self.edges.remove(e);
            }
        }
        for (e, n) in &other.fields {
            let c = self.fields.entry(*e).or_default();
            *c -= n;
            if *c <= 0 {
                self.fields.remove(e);
            }
        }
        self.docs -= other.docs;
        for (key, n) in &other.postings {
            let c = self.postings.entry(key.clone()).or_default();
            *c -= n;
            if *c == 0 {
                self.postings.remove(key);
            }
        }
    }

    /// Into a memo entry's rows (decision 0052): a memoised file hands on its share of the tree statistics as an
    /// evaluated one does.
    pub fn put(&self, rows: &mut BTreeMap<String, (usize, Vec<i64>)>) {
        rows.insert(OWN_KINDS.into(), (3, self.kinds.iter().flat_map(|(k, (n, size))| [*k, *n, *size]).collect()));
        rows.insert(OWN_EDGES.into(), (3, self.edges.iter().flat_map(|((p, k), n)| [*p, *k, *n]).collect()));
        rows.insert(OWN_FIELDS.into(), (4, self.fields.iter().flat_map(|((p, k, f), n)| [*p, *k, *f, *n]).collect()));
        rows.insert(OWN_DOCS.into(), (1, vec![self.docs]));
        for ((rel, col, v), n) in &self.postings {
            rows.entry(format!("{OWN_POSTINGS}{rel}:{col}")).or_insert_with(|| (2, Vec::new())).1.extend([*v, *n]);
        }
    }

    /// Add a frame directly to the run's totals. No per-file maps are reconstructed.
    pub fn add_rows(&mut self, rows: &BTreeMap<String, (usize, Vec<i64>)>) {
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        for r in flat(OWN_KINDS).as_chunks::<3>().0 {
            let k = self.kinds.entry(r[0]).or_default(); k.0 += r[1]; k.1 += r[2];
        }
        for r in flat(OWN_EDGES).as_chunks::<3>().0 { *self.edges.entry((r[0], r[1])).or_default() += r[2]; }
        for r in flat(OWN_FIELDS).as_chunks::<4>().0 { *self.fields.entry((r[0], r[1], r[2])).or_default() += r[3]; }
        self.docs += flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            for r in values.as_chunks::<2>().0 { *self.postings.entry((rel.into(), col.into(), r[0])).or_default() += r[1]; }
        }
    }

    /// From a memo entry's rows (`put`).
    pub fn taken(rows: &BTreeMap<String, (usize, Vec<i64>)>) -> Own {
        let mut own = Own::default();
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        own.kinds = flat(OWN_KINDS).chunks(3).map(|r| (r[0], (r[1], r[2]))).collect();
        own.edges = flat(OWN_EDGES).chunks(3).map(|r| ((r[0], r[1]), r[2])).collect();
        own.fields = flat(OWN_FIELDS).as_chunks::<4>().0.iter().map(|r| ((r[0], r[1], r[2]), r[3])).collect();
        own.docs = flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            own.postings.extend(values.chunks(2).map(|r| ((rel.to_owned(), col.to_owned(), r[0]), r[1])));
        }
        own
    }

    pub fn apply(&self, stats: &mut super::super::Stats) {
        stats.tree(&self.kinds, &self.edges, self.docs);
        stats.fielded(self.fields.iter().map(|(&(p, c, f), &n)| (p.to_string(), c.to_string(), f.to_string(), n as f64)));
        let mut by: BTreeMap<&str, Vec<(String, i64, i64)>> = BTreeMap::new();
        for ((rel, col, v), n) in &self.postings {
            by.entry(rel.as_str()).or_default().push((col.clone(), *v, *n));
        }
        for (rel, vals) in by {
            stats.measured(rel, [], vals);
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
