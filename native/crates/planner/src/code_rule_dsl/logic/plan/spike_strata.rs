//! SPIKE (throw-away): specialized executors for the strata of the reference library that stand for W's Rust.
//!
//! `RERAIL_SPIKE_STRATA=1`: a stratum whose relations are one of the sets below is run here, not by the generic
//! joins: its rows are computed by W's algorithm (a parent walk to the nearest scope or function, the scope chain
//! walked per name, as W's `ScopeTable` and `resolve_python_index` do) from the stratum's own inputs (its lower
//! strata's relations) and the chunk's tree, and retained as the stratum's relations, so the execution shape (one
//! engine run per stratum per chunk, relations materialized between them) stays as it is. Detected by relation name
//! (a spike's shortcut).
//!
//! The chunk's tree (`Tree`) is read once off the batch's projections (`Batch::tree_arrays`: what the `kind`,
//! `child`, `field` and `text` views give, with no view read) and held in the batch's own slot (`Batch::spike`), so it
//! lives and dies with the batch. Rows are flat (`Flat`: `k` values a row), in and out: no vector a row.
//!
//! `RERAIL_SPIKE_CHECK=1` as well: the stratum is computed both ways and the rows compared (`SPIKE CHECK` lines).

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::facts::vtab::syntax::Batch;

pub fn enabled() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_STRATA").is_some())
}

pub fn check() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_CHECK").is_some())
}

/// The strata run here, by their relations (sorted).
const STRATA: [&[&str]; 8] = [&["look", "walk"], &["wstart"], &["rf_in"], &["rf_fn_in"], &["nameref"], &["bind_only"], &["rf_site"], &["rf_selfish"]];

/// Whether the stratum of `rels` is run here.
pub fn handles(rels: &[&str]) -> bool {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    STRATA.contains(&sorted.as_slice())
}

/// No row: a root's parent, a node with no ancestor in a set, an id not of the batch.
const NONE: u32 = u32::MAX;
/// No value: a row with no field, no text.
const NIL: i64 = i64::MIN;

/// Rows held flat: `k` values a row.
#[derive(Default)]
pub struct Flat {
    pub k: usize,
    pub v: Vec<i64>,
}

impl Flat {
    fn new(k: usize) -> Self {
        Flat { k, v: Vec::new() }
    }

    pub fn rows(&self) -> std::slice::ChunksExact<'_, i64> {
        self.v.chunks_exact(self.k.max(1))
    }

    fn push(&mut self, row: &[i64]) {
        self.v.extend_from_slice(row);
    }

    /// Its rows as sorted tuples of their first `N` values, for binary search.
    fn sorted<const N: usize>(&self) -> Vec<[i64; N]> {
        let mut out: Vec<[i64; N]> = self.rows().map(|r| std::array::from_fn(|i| r[i])).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A batch's named tree as dense arrays by its node rows (W's tape cursor, flattened): made once for the batch, at
/// its first spiked stratum, held in its slot (`Batch::spike`).
pub struct Tree {
    ids: Vec<i64>,
    parent: Vec<u32>,
    /// Each row's place among its siblings (`child(p, c, idx)`).
    idx: Vec<u32>,
    kind: Vec<i64>,
    /// The field each row is in (`NIL` for none).
    field: Vec<i64>,
    /// Each identifier row's text atom (`NIL` for another row).
    text: Vec<i64>,
    depth: Vec<u32>,
    /// Each row's children, in order: `kids[child_at[r]..child_at[r + 1]]`.
    child_at: Vec<u32>,
    kids: Vec<u32>,
    atoms: HashMap<String, i64>,
    fields: HashMap<String, i64>,
    /// Each row's nearest strict ancestor in a set of kinds, by the set's name: computed once for the batch.
    nearest: Mutex<HashMap<&'static str, Arc<Vec<u32>>>>,
}

/// The tree of a batch's stratum, and its batch, which finds a node id's row (`Batch::row_of`).
pub struct Chunk<'a> {
    pub t: &'a Tree,
    pub b: &'a Batch,
    memo: Cell<u32>,
}

impl<'a> Chunk<'a> {
    pub fn new(t: &'a Tree, b: &'a Batch) -> Self {
        Chunk { t, b, memo: Cell::new(crate::code_rule_dsl::facts::vtab::NO_MEMO) }
    }

    fn row_of(&self, n: i64) -> u32 {
        let mut memo = self.memo.get();
        let r = self.b.row_of(n, &mut memo).unwrap_or(NONE);
        self.memo.set(memo);
        r
    }

    /// The rows of `rel`'s rows' column `col`, as a mark by row.
    fn mark(&self, rows: &Flat, col: usize) -> Vec<bool> {
        let mut m = vec![false; self.t.ids.len()];
        for r in rows.rows() {
            let at = self.row_of(r[col]);
            if at != NONE {
                m[at as usize] = true;
            }
        }
        m
    }
}

impl Tree {
    /// The batch's tree, read off its projections.
    pub fn of(b: &Batch) -> Tree {
        let _span = super::super::profile::span(|| "spike: the batch's tree".into());
        // The identifier's kind atom in each dialect the batch has documents of: their texts are read with the tree.
        let mut tags: Vec<u8> = b.docs.iter().map(|p| p.tag()).collect();
        tags.sort_unstable();
        tags.dedup();
        let idents: Vec<i64> = tags
            .iter()
            .filter_map(|&tag| {
                let code = crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().kind_id("identifier");
                Some(i64::from(crate::code_rule_dsl::facts::core::kind_atom(tag, code)))
            })
            .collect();
        let a = b.tree_arrays(&idents);
        let n = a.ids.len();
        let mut child_at = vec![0u32; n + 1];
        for &p in &a.parent {
            if p != NONE {
                child_at[p as usize + 1] += 1;
            }
        }
        for r in 0..n {
            child_at[r + 1] += child_at[r];
        }
        let mut at = child_at.clone();
        let mut kids = vec![0u32; child_at[n] as usize];
        // Rows are each document's nodes in preorder: a parent's children placed in row order are in their order.
        for (c, &p) in a.parent.iter().enumerate() {
            if p != NONE {
                kids[at[p as usize] as usize] = c as u32;
                at[p as usize] += 1;
            }
        }
        let mut depth = vec![u32::MAX; n];
        let mut chain = Vec::new();
        for r in 0..n {
            let mut up = r as u32;
            while up != NONE && depth[up as usize] == u32::MAX {
                chain.push(up);
                up = a.parent[up as usize];
            }
            let base = if up == NONE { 0 } else { depth[up as usize] + 1 };
            for (d, c) in (base..).zip(chain.drain(..).rev()) {
                depth[c as usize] = d;
            }
        }
        // A field atom is `tag << 16 | code` (`syntax::field_atom`): named by its dialect's catalog.
        let field_name = |f: i64| {
            let (tag, code) = (u8::try_from(f >> 16).ok()?, (f & 0xffff) as u16);
            crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().field_name(code).map(str::to_owned)
        };
        let fields = by_name(a.field.iter().copied().filter(|&f| f != NIL), field_name);
        // A kind atom is `tag * 1024 + code` (`core::kind_atom`), its name's canonical code's, as the `kind` view
        // gives it: the same atom a rule's constant of the name is.
        let kind_name = |k: i64| {
            let k = u32::try_from(k).ok()?;
            let tag = crate::code_rule_dsl::facts::core::kind_tag(k);
            crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().kind_name(crate::code_rule_dsl::facts::core::kind_code(k)).map(str::to_owned)
        };
        let atoms = by_name(a.kind.iter().copied(), kind_name);
        Tree { ids: a.ids, parent: a.parent, idx: a.idx, kind: a.kind, field: a.field, text: a.text, depth, child_at, kids, atoms, fields, nearest: Mutex::default() }
    }

    fn atom(&self, kind: &str) -> i64 {
        self.atoms.get(kind).copied().unwrap_or(NIL)
    }

    fn field_atom(&self, field: &str) -> i64 {
        self.fields.get(field).copied().unwrap_or(NIL)
    }

    fn set(&self, kinds: &[&str]) -> HashSet<i64> {
        kinds.iter().map(|k| self.atom(k)).collect()
    }

    fn rows(&self) -> std::ops::Range<u32> {
        0..self.ids.len() as u32
    }

    fn id(&self, r: u32) -> i64 {
        self.ids[r as usize]
    }

    fn kind(&self, r: u32) -> i64 {
        if r == NONE { NIL } else { self.kind[r as usize] }
    }

    fn kids(&self, r: u32) -> &[u32] {
        &self.kids[self.child_at[r as usize] as usize..self.child_at[r as usize + 1] as usize]
    }

    /// `field(_, f, r)`.
    fn in_field(&self, r: u32, field: i64) -> bool {
        self.field[r as usize] == field
    }

    /// `child(r, c, 0)`.
    fn first_child(&self, r: u32) -> Option<u32> {
        self.kids(r).iter().copied().find(|&c| self.idx[c as usize] == 0)
    }

    /// `r`'s child in `field`.
    fn field_child(&self, r: u32, field: i64) -> Option<u32> {
        self.kids(r).iter().copied().find(|&c| self.in_field(c, field))
    }

    /// Each row strictly below `r`.
    fn each_below(&self, r: u32, f: &mut impl FnMut(u32)) {
        let mut stack: Vec<u32> = self.kids(r).to_vec();
        while let Some(c) = stack.pop() {
            f(c);
            stack.extend_from_slice(self.kids(c));
        }
    }

    /// `within(n, a)`: `n` strictly below `a`.
    fn within(&self, n: u32, a: u32) -> bool {
        let (mut up, d) = (self.parent[n as usize], self.depth[a as usize]);
        while up != NONE && self.depth[up as usize] >= d {
            if up == a {
                return true;
            }
            up = self.parent[up as usize];
        }
        false
    }

    /// Each row's nearest strict ancestor of a kind in `kinds` (the set named `name`), computed once for the batch
    /// in one pass, parents before children (W's enclosing scope, kept as it walks).
    fn nearest(&self, name: &'static str, kinds: &[&str]) -> Arc<Vec<u32>> {
        if let Some(n) = self.nearest.lock().unwrap().get(name) {
            return Arc::clone(n);
        }
        let set = self.set(kinds);
        let n = self.ids.len();
        let mut near = vec![NONE; n];
        let mut done = vec![false; n];
        let mut chain = Vec::new();
        for r in 0..n as u32 {
            let mut at = r;
            while at != NONE && !done[at as usize] {
                chain.push(at);
                at = self.parent[at as usize];
            }
            for c in chain.drain(..).rev() {
                let p = self.parent[c as usize];
                near[c as usize] = if p == NONE { NONE } else if set.contains(&self.kind[p as usize]) { p } else { near[p as usize] };
                done[c as usize] = true;
            }
        }
        let near = Arc::new(near);
        self.nearest.lock().unwrap().insert(name, Arc::clone(&near));
        near
    }

    /// `n`'s kind and each ancestor's, with the field it is in (check mode's account of a row).
    pub fn describe(&self, c: &Chunk<'_>, n: i64) -> String {
        let name = |k: i64| self.atoms.iter().find(|&(_, &a)| a == k).map_or("?".to_owned(), |(s, _)| s.clone());
        let fname = |f: i64| self.fields.iter().find(|&(_, &a)| a == f).map_or("-".to_owned(), |(s, _)| s.clone());
        let mut out = Vec::new();
        let mut at = c.row_of(n);
        while at != NONE && out.len() < 5 {
            out.push(format!("{}[{}]", name(self.kind[at as usize]), fname(self.field[at as usize])));
            at = self.parent[at as usize];
        }
        out.join(" < ")
    }
}

/// Each name of `values`' atoms, to the first atom of it.
fn by_name(values: impl Iterator<Item = i64>, name: impl Fn(i64) -> Option<String>) -> HashMap<String, i64> {
    let mut distinct: Vec<i64> = values.collect::<HashSet<i64>>().into_iter().collect();
    distinct.sort_unstable();
    let mut out = HashMap::default();
    for a in distinct {
        if let Some(n) = name(a) {
            out.entry(n).or_insert(a);
        }
    }
    out
}

/// Every row of `source`, flat.
pub fn rows(source: &dyn Source) -> Flat {
    let k = source.columns().len();
    let all: Vec<usize> = (0..k).collect();
    let mut out = Flat { k, v: Vec::with_capacity(source.len() * k) };
    source.for_rows(&all, &mut |_, row| out.v.extend_from_slice(row));
    out
}

/// A stratum's inputs, read by name: its rows, or none where the relation is not held (no rows).
pub trait Inputs {
    fn read(&self, rel: &str) -> Flat;
    /// The texts of symbol atoms.
    fn texts(&self, atoms: &[i64]) -> Vec<String>;
}

const SCOPE: [&str; 4] = ["module", "function_definition", "class_definition", "lambda"];
const COMP: [&str; 4] = ["list_comprehension", "set_comprehension", "dictionary_comprehension", "generator_expression"];

/// The values paired with `k` in sorted pairs.
fn paired(pairs: &[[i64; 2]], k: i64) -> impl Iterator<Item = i64> + '_ {
    let from = pairs.partition_point(|p| p[0] < k);
    pairs[from..].iter().take_while(move |p| p[0] == k).map(|p| p[1])
}

/// `nameref`: identifiers that are not an attribute's or a definition's name, a keyword pattern's key, nor a dotted
/// pattern's tail; with their text.
fn nameref(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let (kw, tail) = (c.mark(&inp.read("kwpat_key"), 0), c.mark(&inp.read("pat_tail"), 0));
    let (attribute, name) = (t.field_atom("attribute"), t.field_atom("name"));
    let mut out = Flat::new(2);
    for r in t.rows() {
        let u = r as usize;
        if t.kind[u] == id && t.text[u] != NIL && !kw[u] && !tail[u] && !t.in_field(r, attribute) && !t.in_field(r, name) {
            out.push(&[t.id(r), t.text[u]]);
        }
    }
    out
}

/// `wstart`: where a name's lookup starts (the five rules of `names.rerail`).
fn wstart(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let lscope_kinds: Vec<&str> = SCOPE.iter().chain(COMP.iter()).copied().collect();
    let near = t.nearest("lscope", &lscope_kinds);
    let (scope, comp) = (t.set(&SCOPE), t.set(&COMP));
    let def_outside = inp.read("def_outside");
    let in_first_iter = inp.read("in_first_iter");
    let outside = c.mark(&def_outside, 0);
    let first: Vec<[i64; 2]> = in_first_iter.sorted();
    let host: Vec<[i64; 2]> = inp.read("host_node").sorted();
    let ann: Vec<[i64; 2]> = inp.read("ann_start").sorted();
    let nameref = inp.read("nameref");
    let mut out = Flat::new(2);
    for r in nameref.rows() {
        let n = r[0];
        let at = c.row_of(n);
        if at != NONE && near[at as usize] != NONE {
            let s = near[at as usize];
            let k = t.kind(s);
            if scope.contains(&k) && !outside[at as usize] {
                out.push(&[n, t.id(s)]);
            }
            if comp.contains(&k) && first.binary_search(&[n, t.id(s)]).is_err() {
                out.push(&[n, t.id(s)]);
            }
        }
        for h in paired(&host, n) {
            for s in paired(&ann, h) {
                out.push(&[n, s]);
            }
        }
    }
    for r in def_outside.rows().chain(in_first_iter.rows()) {
        let f = c.row_of(r[1]);
        if f != NONE && near[f as usize] != NONE {
            out.push(&[r[0], t.id(near[f as usize])]);
        }
    }
    out
}

/// `look` (and `walk`, which only `look` reads, in this stratum: not made): each name's scope chain walked from its
/// starts until a scope binds it (W's resolve). Per name, the scopes walked are a few: kept in a list for the name.
fn look_walk(c: &Chunk<'_>, inp: &dyn Inputs) -> Vec<(String, Flat)> {
    let t = c.t;
    let lscope = t.set(&SCOPE.iter().chain(COMP.iter()).copied().collect::<Vec<_>>());
    let lf_kinds: Vec<&str> = ["module", "function_definition", "lambda"].iter().chain(COMP.iter()).copied().collect();
    let lf = t.nearest("lfscope", &lf_kinds);
    // `scope_up(s, p) :- kind(s, k), lscope(k), nearest(s, p, lfscope)` (folded into its reader: no relation of its own).
    let scope_up = |s: i64| {
        let r = c.row_of(s);
        (r != NONE && lscope.contains(&t.kind(r)) && lf[r as usize] != NONE).then(|| t.id(lf[r as usize]))
    };
    let nameref: Vec<[i64; 2]> = inp.read("nameref").sorted();
    let tp_ref: Vec<[i64; 2]> = inp.read("tp_ref").sorted();
    // `bound_in(s, x)`, `gbefore(n, x, s)`, `cls_late(n, x, s)`: sorted, searched.
    let bound: Vec<[i64; 2]> = inp.read("bound_in").sorted();
    let gbefore: Vec<[i64; 3]> = inp.read("gbefore").sorted();
    let late: Vec<[i64; 3]> = inp.read("cls_late").sorted();
    let mut starts: Vec<[i64; 2]> = inp.read("wstart").sorted();
    starts.dedup();
    let mut look = Flat::new(3);
    let mut walked: Vec<(i64, i64)> = Vec::new();
    let mut work: Vec<(i64, i64)> = Vec::new();
    let mut i = 0;
    while i < starts.len() {
        let n = starts[i][0];
        let j = i + starts[i..].partition_point(|s| s[0] == n);
        walked.clear();
        for x in paired(&nameref, n) {
            if tp_ref.binary_search(&[n, x]).is_ok() {
                continue;
            }
            for s in &starts[i..j] {
                if !walked.contains(&(x, s[1])) {
                    walked.push((x, s[1]));
                    work.push((x, s[1]));
                }
            }
        }
        while let Some((x, s)) = work.pop() {
            let b = bound.binary_search(&[s, x]).is_ok();
            let l = late.binary_search(&[n, x, s]).is_ok();
            let g = gbefore.binary_search(&[n, x, s]).is_ok();
            if (b && !l) || (!b && g) {
                look.push(&[n, x, s]);
            }
            if ((!b && !g) || l)
                && let Some(p) = scope_up(s)
                && !walked.contains(&(x, p))
            {
                walked.push((x, p));
                work.push((x, p));
            }
        }
        i = j;
    }
    vec![("look".into(), look)]
}

/// The function whose body holds row `n`, through the functions of `fn_in` where `n` is in a definition's head.
/// `rf_body(g, b) :- kind(g, "function_definition"), field(g, "body", b)` is the tree's (no relation of its own).
fn enclosing(t: &Tree, n: u32, near: &[u32], body: i64, fn_in: &HashMap<u32, Vec<u32>>, out: &mut Vec<u32>) {
    let g = near[n as usize];
    if g == NONE {
        return;
    }
    if let Some(b) = t.field_child(g, body) {
        if t.within(n, b) {
            out.push(g);
        } else {
            out.extend(fn_in.get(&g).into_iter().flatten().copied());
        }
    }
}

/// `rf_fn_in` by row: each function's enclosing functions, outermost first (so a nested one finds its parent's).
fn fn_in_rows(c: &Chunk<'_>, inp: &dyn Inputs, near: &[u32], body: i64) -> HashMap<u32, Vec<u32>> {
    let t = c.t;
    let fdef = t.atom("function_definition");
    let mut fns: Vec<u32> = inp.read("rf_def").rows().map(|r| c.row_of(r[0])).filter(|&g| g != NONE && t.kind(g) == fdef).collect();
    fns.sort_unstable_by_key(|&g| (t.depth[g as usize], g));
    fns.dedup();
    let mut fn_in: HashMap<u32, Vec<u32>> = HashMap::default();
    let mut found = Vec::new();
    for g in fns {
        found.clear();
        enclosing(t, g, near, body, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        if !found.is_empty() {
            fn_in.insert(g, found.clone());
        }
    }
    fn_in
}

/// `rf_fn_in`: each function's enclosing function.
fn rf_fn_in(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let near = t.nearest("fdef", &["function_definition"]);
    let fn_in = fn_in_rows(c, inp, &near, t.field_atom("body"));
    let mut out = Flat::new(2);
    for (g, hs) in fn_in {
        for h in hs {
            out.push(&[t.id(g), t.id(h)]);
        }
    }
    out
}

/// `rf_in`: each site's enclosing function.
fn rf_in(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let near = t.nearest("fdef", &["function_definition"]);
    let body = t.field_atom("body");
    let mut fn_in: HashMap<u32, Vec<u32>> = HashMap::default();
    for r in inp.read("rf_fn_in").rows() {
        let (g, h) = (c.row_of(r[0]), c.row_of(r[1]));
        if g != NONE && h != NONE {
            fn_in.entry(g).or_default().push(h);
        }
    }
    let sites = c.mark(&inp.read("rf_site"), 0);
    let mut out = Flat::new(2);
    let mut found = Vec::new();
    for n in t.rows().filter(|&r| sites[r as usize]) {
        found.clear();
        enclosing(t, n, &near, body, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        for &g in &found {
            out.push(&[t.id(n), t.id(g)]);
        }
    }
    out
}

/// `bind_only`: names a statement binds without reading them (the eight rules of `names.rerail`).
fn bind_only(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let is_id = |r: u32| t.kind(r) == id;
    let mut out = vec![false; t.ids.len()];
    let aug = c.mark(&inp.read("aug_target"), 0);
    for r in inp.read("tgt").rows() {
        let at = c.row_of(r[0]);
        if at != NONE && is_id(at) && !aug[at as usize] {
            out[at as usize] = true;
        }
    }
    for r in inp.read("tp_site").rows() {
        let at = c.row_of(r[1]);
        if at != NONE {
            out[at as usize] = true;
        }
    }
    let alias_read = c.mark(&inp.read("tp_alias_read"), 0);
    let (assignment, alias, typed) = (t.atom("assignment"), t.atom("type_alias_statement"), t.atom("typed_parameter"));
    let (left, parameters) = (t.field_atom("left"), t.field_atom("parameters"));
    let imports = t.set(&["import_statement", "import_from_statement", "future_import_statement"]);
    let globals = t.set(&["global_statement", "nonlocal_statement"]);
    for r in t.rows() {
        let p = t.parent[r as usize];
        if p != NONE && t.in_field(r, left) {
            if t.kind(p) == assignment && is_id(r) {
                out[r as usize] = true;
            }
            if t.kind(p) == alias {
                t.each_below(r, &mut |i| {
                    if is_id(i) && !alias_read[i as usize] {
                        out[i as usize] = true;
                    }
                });
            }
        }
        if p != NONE && t.in_field(r, parameters) {
            for &q in t.kids(r) {
                if is_id(q) {
                    out[q as usize] = true;
                }
                if let Some(first) = t.first_child(q) {
                    if is_id(first) {
                        out[first as usize] = true;
                    }
                    if t.kind(q) == typed
                        && let Some(id0) = t.first_child(first)
                        && is_id(id0)
                    {
                        out[id0 as usize] = true;
                    }
                }
            }
        }
        let k = t.kind(r);
        if imports.contains(&k) {
            t.each_below(r, &mut |i| {
                if is_id(i) {
                    out[i as usize] = true;
                }
            });
        }
        if globals.contains(&k) {
            for &g in t.kids(r) {
                if is_id(g) {
                    out[g as usize] = true;
                }
            }
        }
    }
    Flat { k: 1, v: t.rows().filter(|&r| out[r as usize]).map(|r| t.id(r)).collect() }
}

/// `rf_site`: calls, attributes of a name, and read names, where none is a call's function.
fn rf_site(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let (id, call, attribute) = (t.atom("identifier"), t.atom("call"), t.atom("attribute"));
    let (function, object) = (t.field_atom("function"), t.field_atom("object"));
    // `rf_read(i) :- nameref(i, _), !bind_only(i), !rf_deleted(i), !guest_of(i, _)` (no relation of its own).
    let names = c.mark(&inp.read("nameref"), 0);
    let bound = c.mark(&inp.read("bind_only"), 0);
    // `rf_deleted(n) :- del_unbinds(_, n)` (one rule: folded into its readers, not stored).
    let deleted = c.mark(&inp.read("del_unbinds"), 1);
    let guest = c.mark(&inp.read("guest_of"), 0);
    let objects = c.mark(&inp.read("rf_object"), 0);
    let v = t
        .rows()
        .filter(|&n| {
            let (k, u) = (t.kind(n), n as usize);
            if k == call {
                true
            } else if k == attribute {
                !t.in_field(n, function) && t.field_child(n, object).is_some_and(|o| t.kind(o) == id)
            } else if k == id {
                !t.in_field(n, function) && names[u] && !bound[u] && !deleted[u] && !guest[u] && !objects[u]
            } else {
                false
            }
        })
        .map(|n| t.id(n))
        .collect();
    Flat { k: 1, v }
}

/// `rf_selfish`: the names `self` and `cls`.
fn rf_selfish(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let atoms: Vec<i64> = t.rows().filter(|&r| t.kind(r) == id && t.text[r as usize] != NIL).map(|r| t.text[r as usize]).collect::<HashSet<_>>().into_iter().collect();
    let strings = inp.texts(&atoms);
    let selfish: HashSet<i64> = atoms.iter().zip(&strings).filter(|(_, s)| *s == "self" || *s == "cls").map(|(a, _)| *a).collect();
    Flat { k: 1, v: t.rows().filter(|&r| t.kind(r) == id && selfish.contains(&t.text[r as usize])).map(|r| t.id(r)).collect() }
}

/// The stratum's rows by relation, by W's algorithm.
pub fn run(rels: &[&str], c: &Chunk<'_>, inp: &dyn Inputs) -> Vec<(String, Flat)> {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    let one = |rel: &str, rows: Flat| vec![(rel.to_owned(), rows)];
    match sorted.as_slice() {
        ["look", "walk"] => look_walk(c, inp),
        ["wstart"] => one("wstart", wstart(c, inp)),
        ["rf_in"] => one("rf_in", rf_in(c, inp)),
        ["rf_fn_in"] => one("rf_fn_in", rf_fn_in(c, inp)),
        ["nameref"] => one("nameref", nameref(c, inp)),
        ["bind_only"] => one("bind_only", bind_only(c, inp)),
        ["rf_site"] => one("rf_site", rf_site(c, inp)),
        ["rf_selfish"] => one("rf_selfish", rf_selfish(c, inp)),
        _ => Vec::new(),
    }
}

/// `SPIKE CHECK` lines: the spike's rows against the generic joins'.
pub fn compare(rel: &str, spike: &Flat, generic: &Flat, chunk: Option<&Chunk<'_>>) {
    let a: HashSet<&[i64]> = spike.rows().collect();
    let b: HashSet<&[i64]> = generic.rows().collect();
    let only_spike: Vec<_> = a.difference(&b).take(3).collect();
    let only_generic: Vec<_> = b.difference(&a).take(3).collect();
    let (ns, ng) = (a.difference(&b).count(), b.difference(&a).count());
    if let Some(c) = chunk {
        for r in only_spike.iter().chain(&only_generic) {
            eprintln!("SPIKE WHY {rel} {r:?}: {}", c.t.describe(c, r[0]));
        }
    }
    eprintln!("SPIKE CHECK {rel}: spike {} generic {} only-spike {ns} {only_spike:?} only-generic {ng} {only_generic:?}", a.len(), b.len());
}
