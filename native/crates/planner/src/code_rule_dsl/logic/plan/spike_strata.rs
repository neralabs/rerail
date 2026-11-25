//! SPIKE (throw-away): specialized executors for the strata of the reference library that stand for W's Rust.
//!
//! `RERAIL_SPIKE_STRATA=1`: a stratum whose relations are one of the sets below is run here, not by the generic
//! joins: its rows are computed by W's algorithm (a parent walk to the nearest scope or function, the scope chain
//! walked per name with hash sets, as W's `ScopeTable` and `resolve_python_index` do) from the stratum's own inputs
//! (its lower strata's relations, and the chunk's `child` and `kind`), and retained as the stratum's relations, so
//! the execution shape (one engine run per stratum per chunk, relations materialized between them) stays as it is.
//! Detected by relation name (a spike's shortcut).
//!
//! `RERAIL_SPIKE_CHECK=1` as well: the stratum is computed both ways and the rows compared (`SPIKE CHECK` lines).

use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use rerail_blocks::vector::Vector;

use crate::code_rule_dsl::facts::vtab::Source;

pub fn enabled() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_STRATA").is_some())
}

pub fn check() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_CHECK").is_some())
}

/// The strata run here, by their relations (sorted).
const STRATA: [&[&str]; 8] = [&["look", "walk"], &["wstart"], &["rf_in"], &["rf_fn_in"], &["nameref"], &["bind_only"], &["rf_site"], &["rf_selfish"]];

/// Relations of the spiked strata no stratum outside them reads in this program (`walk`: `look` alone, in its own
/// stratum; `miss` and `annotated_on_chain` are not run): kept for the chunk, never retained.
pub fn kept_only(rel: &str) -> bool {
    rel == "walk" && std::env::var_os("RERAIL_SPIKE_RETAIN_ALL").is_none()
}

/// Whether the stratum of `rels` is run here.
pub fn handles(rels: &[&str]) -> bool {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    STRATA.contains(&sorted.as_slice())
}

/// No row: a root's parent, a node with no ancestor in a set.
const NONE: u32 = u32::MAX;

/// A chunk's named tree as dense arrays by row (W's tape cursor, flattened): made once for the chunk, shared by its
/// strata; and the spiked strata's own rows, kept for the chunk's later spiked strata (the derived-data cache).
pub struct Tree {
    /// Each row's node id, ascending, and each id's row.
    ids: Vec<i64>,
    row: HashMap<i64, u32>,
    parent: Vec<u32>,
    kind: Vec<i64>,
    depth: Vec<u32>,
    /// Each row's children, in order: `kids[child_at[r]..child_at[r + 1]]`.
    child_at: Vec<u32>,
    kids: Vec<u32>,
    /// The fields each row is in (at most two; `i64::MIN` for none).
    field: Vec<[i64; 2]>,
    atoms: HashMap<String, i64>,
    fields: HashMap<String, i64>,
    /// Each identifier row's text atom (`i64::MIN` for another row), read once for the chunk.
    text: Vec<i64>,
    /// Each row's nearest strict ancestor in a set of kinds, by the set's name: computed once for the chunk.
    nearest: Mutex<HashMap<&'static str, Arc<Vec<u32>>>>,
    /// The spiked strata's rows, by relation, for the chunk's later spiked strata.
    made: Mutex<HashMap<String, Arc<Vec<Vector<i64>>>>>,
}

impl Tree {
    fn atom(&self, kind: &str) -> i64 {
        self.atoms.get(kind).copied().unwrap_or(i64::MIN)
    }

    fn field_atom(&self, field: &str) -> i64 {
        self.fields.get(field).copied().unwrap_or(i64::MIN)
    }

    fn set(&self, kinds: &[&str]) -> HashSet<i64> {
        kinds.iter().map(|k| self.atom(k)).collect()
    }

    fn rows(&self) -> std::ops::Range<u32> {
        0..self.ids.len() as u32
    }

    fn row_of(&self, n: i64) -> u32 {
        self.row.get(&n).copied().unwrap_or(NONE)
    }

    fn id(&self, r: u32) -> i64 {
        self.ids[r as usize]
    }

    fn kind(&self, r: u32) -> i64 {
        if r == NONE { i64::MIN } else { self.kind[r as usize] }
    }

    fn kids(&self, r: u32) -> &[u32] {
        &self.kids[self.child_at[r as usize] as usize..self.child_at[r as usize + 1] as usize]
    }

    /// `field(_, f, r)`.
    fn in_field(&self, r: u32, field: i64) -> bool {
        self.field[r as usize].contains(&field)
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

    /// Each row's nearest strict ancestor of a kind in `kinds` (the set named `name`), computed once for the chunk
    /// in one pass, parents before children (W's enclosing scope, kept as it walks).
    fn nearest(&self, name: &'static str, kinds: &[&str]) -> Arc<Vec<u32>> {
        if let Some(n) = self.nearest.lock().unwrap().get(name) {
            return Arc::clone(n);
        }
        let set: HashSet<i64> = kinds.iter().map(|k| self.atom(k)).collect();
        let mut order: Vec<u32> = self.rows().collect();
        order.sort_unstable_by_key(|&r| self.depth[r as usize]);
        let mut near = vec![NONE; self.ids.len()];
        for r in order {
            let p = self.parent[r as usize];
            if p != NONE {
                near[r as usize] = if set.contains(&self.kind[p as usize]) { p } else { near[p as usize] };
            }
        }
        let near = Arc::new(near);
        self.nearest.lock().unwrap().insert(name, Arc::clone(&near));
        near
    }

    /// `n`'s kind and each ancestor's, with the fields it is in (check mode's account of a row).
    pub fn describe(&self, n: i64) -> String {
        let name = |k: i64| self.atoms.iter().find(|&(_, &a)| a == k).map_or("?".to_owned(), |(s, _)| s.clone());
        let fname = |f: i64| self.fields.iter().find(|&(_, &a)| a == f).map_or("?".to_owned(), |(s, _)| s.clone());
        let mut out = Vec::new();
        let mut at = self.row_of(n);
        while at != NONE && out.len() < 5 {
            let fs: Vec<String> = self.field[at as usize].iter().filter(|&&f| f != i64::MIN).map(|&f| fname(f)).collect();
            out.push(format!("{}{fs:?}", name(self.kind[at as usize])));
            at = self.parent[at as usize];
        }
        out.join(" < ")
    }

    /// Keeps a spiked stratum's rows for the chunk's later spiked strata.
    pub fn keep(&self, rel: &str, rows: Arc<Vec<Vector<i64>>>) {
        self.made.lock().unwrap().insert(rel.to_owned(), rows);
    }
}

/// The trees of the chunks whose strata run now, by their `child` source (held, so its address names that chunk
/// alone), each made once by whichever stratum asks first.
type Trees = Vec<(Arc<dyn Source>, Arc<OnceLock<Option<Arc<Tree>>>>)>;
static TREES: Mutex<Trees> = Mutex::new(Vec::new());

/// Every row of `source`.
pub fn rows(source: &dyn Source) -> Vec<Vector<i64>> {
    let width = source.columns().len();
    let all: Vec<usize> = (0..width).collect();
    let mut out = Vec::with_capacity(source.len());
    source.for_rows(&all, &mut |_, row| out.push(Vector::from(row)));
    out
}

/// The chunk's tree, made from its `child` and `kind` (or `node`) sources.
pub fn tree(child: &Arc<dyn Source>, kind: Option<&Arc<dyn Source>>, node: Option<&Arc<dyn Source>>, field: Option<&Arc<dyn Source>>, text: Option<&Arc<dyn Source>>) -> Option<Arc<Tree>> {
    let cell = {
        let mut trees = TREES.lock().unwrap();
        match trees.iter().find(|(c, _)| Arc::ptr_eq(c, child)) {
            Some((_, cell)) => Arc::clone(cell),
            None => {
                if trees.len() >= 8 {
                    trees.remove(0);
                }
                let cell = Arc::new(OnceLock::new());
                trees.push((Arc::clone(child), Arc::clone(&cell)));
                cell
            }
        }
    };
    cell.get_or_init(|| make_tree(child, kind, node, field, text)).clone()
}

fn make_tree(child: &Arc<dyn Source>, kind: Option<&Arc<dyn Source>>, node: Option<&Arc<dyn Source>>, field: Option<&Arc<dyn Source>>, text: Option<&Arc<dyn Source>>) -> Option<Arc<Tree>> {
    let _span = super::super::profile::span(|| "spike: the chunk's tree".into());
    let mut pairs: Vec<(i64, i64)> = Vec::new();
    match (kind, node) {
        (Some(k), _) => k.for_rows(&[0, 1], &mut |_, r| pairs.push((r[0], r[1]))),
        (None, Some(n)) => n.for_rows(&[1, 2], &mut |_, r| pairs.push((r[0], r[1]))),
        (None, None) => return None,
    }
    pairs.sort_unstable();
    pairs.dedup_by_key(|p| p.0);
    let ids: Vec<i64> = pairs.iter().map(|p| p.0).collect();
    let kinds: Vec<i64> = pairs.iter().map(|p| p.1).collect();
    let row: HashMap<i64, u32> = ids.iter().enumerate().map(|(r, &n)| (n, r as u32)).collect();
    let at = |n: i64| row.get(&n).copied().unwrap_or(NONE);
    let n = ids.len();
    let mut parent = vec![NONE; n];
    let mut edges: Vec<(u32, i64, u32)> = Vec::with_capacity(n);
    child.for_rows(&[0, 1, 2], &mut |_, r| {
        let (p, c) = (at(r[0]), at(r[1]));
        if p != NONE && c != NONE {
            parent[c as usize] = p;
            edges.push((p, r[2], c));
        }
    });
    edges.sort_unstable();
    let mut child_at = vec![0u32; n + 1];
    for &(p, _, _) in &edges {
        child_at[p as usize + 1] += 1;
    }
    for r in 0..n {
        child_at[r + 1] += child_at[r];
    }
    let kids: Vec<u32> = edges.iter().map(|e| e.2).collect();
    let mut fieldv = vec![[i64::MIN; 2]; n];
    if let Some(f) = field {
        f.for_rows(&[1, 2], &mut |_, r| {
            let c = at(r[1]);
            if c != NONE {
                let slot = &mut fieldv[c as usize];
                if slot[0] == i64::MIN { slot[0] = r[0] } else if slot[1] == i64::MIN { slot[1] = r[0] }
            }
        });
    }
    let mut depth = vec![u32::MAX; n];
    for r in 0..n {
        let mut chain = Vec::new();
        let mut at = r as u32;
        while at != NONE && depth[at as usize] == u32::MAX {
            chain.push(at);
            at = parent[at as usize];
        }
        let base = if at == NONE { 0 } else { depth[at as usize] + 1 };
        for (d, &c) in (base..).zip(chain.iter().rev()) {
            depth[c as usize] = d;
        }
    }
    // A field atom is `tag << 16 | code` (`syntax::field_atom`): named by its dialect's catalog, one atom a name.
    let field_name = |a: i64| {
        let (tag, code) = (u8::try_from(a >> 16).ok()?, (a & 0xffff) as u16);
        crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().field_name(code).map(str::to_owned)
    };
    // A rule's field constant is the catalog's id of its name (`field_id`).
    let field_own = |a: i64, name: &str| {
        let tag = u8::try_from(a >> 16).ok()?;
        let code = crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().field_id(name)?;
        Some(i64::from(tag) << 16 | i64::from(code))
    };
    let fields = by_name(fieldv.iter().flatten().copied().filter(|&f| f != i64::MIN), field_name, field_own);
    // A kind atom is `tag * 1024 + code` (`core::kind_atom`): named by its dialect's catalog, one atom a name.
    let kind_name = |a: i64| {
        let a = u32::try_from(a).ok()?;
        let tag = crate::code_rule_dsl::facts::core::kind_tag(a);
        crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().kind_name(crate::code_rule_dsl::facts::core::kind_code(a)).map(str::to_owned)
    };
    // A rule's kind constant is the catalog's id of its name (`kind_id`): a node of an alias's code of the same name
    // is not of it, as the joins read it.
    let kind_own = |a: i64, name: &str| {
        let tag = crate::code_rule_dsl::facts::core::kind_tag(u32::try_from(a).ok()?);
        let code = crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().kind_id(name);
        Some(i64::from(crate::code_rule_dsl::facts::core::kind_atom(tag, code)))
    };
    let atoms = by_name(kinds.iter().copied(), kind_name, kind_own);
    // The identifiers' texts, probed once for the chunk (`text(n, x)` of a name).
    let mut textv = vec![i64::MIN; n];
    if let (Some(t), Some(&id)) = (text, atoms.get("identifier")) {
        for r in (0..n).filter(|&r| kinds[r] == id) {
            t.lookup_read(0, ids[r], &[1], &mut |_, v| textv[r] = v[0]);
        }
    }
    Some(Arc::new(Tree { ids, row, parent, kind: kinds, depth, child_at, kids, field: fieldv, atoms, fields, text: textv, nearest: Mutex::default(), made: Mutex::default() }))
}

/// Each name of `values`' atoms (`name`), to the atom a rule's constant of it is (`own`).
fn by_name(values: impl Iterator<Item = i64>, name: impl Fn(i64) -> Option<String>, own: impl Fn(i64, &str) -> Option<i64>) -> HashMap<String, i64> {
    let distinct: HashSet<i64> = values.collect();
    let mut out = HashMap::default();
    for a in distinct {
        if let Some(n) = name(a)
            && !out.contains_key(&n)
            && let Some(o) = own(a, &n)
        {
            out.insert(n, o);
        }
    }
    out
}

/// A stratum's inputs, read by name: its rows, or none where the relation is not held (no rows).
pub trait Inputs {
    fn read(&self, rel: &str) -> Vec<Vector<i64>>;
    /// The rows of `rel` whose `column` is one of `values`.
    fn lookup(&self, rel: &str, column: usize, values: &[i64]) -> Vec<Vector<i64>>;
    /// The texts of symbol atoms.
    fn texts(&self, atoms: &[i64]) -> Vec<String>;
}

/// A stratum's input: a spiked stratum's rows kept for the chunk where there are, else the relation's.
fn input(t: &Tree, inp: &dyn Inputs, rel: &str) -> Arc<Vec<Vector<i64>>> {
    if let Some(rows) = t.made.lock().unwrap().get(rel) {
        return Arc::clone(rows);
    }
    Arc::new(inp.read(rel))
}

/// The rows of `rel` (an input, `input`) whose first column is a node, as a row mark.
fn mark(t: &Tree, inp: &dyn Inputs, rel: &str, col: usize) -> Vec<bool> {
    let mut m = vec![false; t.ids.len()];
    for r in input(t, inp, rel).iter() {
        let at = t.row_of(r[col]);
        if at != NONE {
            m[at as usize] = true;
        }
    }
    m
}

const SCOPE: [&str; 4] = ["module", "function_definition", "class_definition", "lambda"];
const COMP: [&str; 4] = ["list_comprehension", "set_comprehension", "dictionary_comprehension", "generator_expression"];

fn multi(rows: &[Vector<i64>], k: usize, v: usize) -> HashMap<i64, Vec<i64>> {
    let mut m: HashMap<i64, Vec<i64>> = HashMap::default();
    for r in rows {
        m.entry(r[k]).or_default().push(r[v]);
    }
    m
}

/// `nameref`: identifiers that are not an attribute's or a definition's name, a keyword pattern's key, nor a dotted
/// pattern's tail; with their text.
fn nameref(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let (kw, tail) = (mark(t, inp, "kwpat_key", 0), mark(t, inp, "pat_tail", 0));
    let (attribute, name) = (t.field_atom("attribute"), t.field_atom("name"));
    t.rows()
        .filter(|&r| t.kind[r as usize] == id && t.text[r as usize] != i64::MIN && !kw[r as usize] && !tail[r as usize] && !t.in_field(r, attribute) && !t.in_field(r, name))
        .map(|r| rerail_blocks::vector![t.id(r), t.text[r as usize]])
        .collect()
}

/// `wstart`: where a name's lookup starts (the five rules of `names.rerail`).
fn wstart(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let lscope_kinds: Vec<&str> = SCOPE.iter().chain(COMP.iter()).copied().collect();
    let near = t.nearest("lscope", &lscope_kinds);
    let (scope, comp) = (t.set(&SCOPE), t.set(&COMP));
    let outside = mark(t, inp, "def_outside", 0);
    let in_first_iter = input(t, inp, "in_first_iter");
    let def_outside = input(t, inp, "def_outside");
    let first: HashSet<(i64, i64)> = in_first_iter.iter().map(|r| (r[0], r[1])).collect();
    let host = multi(&input(t, inp, "host_node"), 0, 1);
    let ann = multi(&input(t, inp, "ann_start"), 0, 1);
    let mut out: HashSet<(i64, i64)> = HashSet::default();
    let mut names: Vec<i64> = input(t, inp, "nameref").iter().map(|r| r[0]).collect();
    names.sort_unstable();
    names.dedup();
    for n in names {
        let r = t.row_of(n);
        if r != NONE && near[r as usize] != NONE {
            let s = near[r as usize];
            let k = t.kind(s);
            if scope.contains(&k) && !outside[r as usize] {
                out.insert((n, t.id(s)));
            }
            if comp.contains(&k) && !first.contains(&(n, t.id(s))) {
                out.insert((n, t.id(s)));
            }
        }
        for h in host.get(&n).into_iter().flatten() {
            for &s in ann.get(h).into_iter().flatten() {
                out.insert((n, s));
            }
        }
    }
    for r in def_outside.iter().chain(in_first_iter.iter()) {
        let f = t.row_of(r[1]);
        if f != NONE && near[f as usize] != NONE {
            out.insert((r[0], t.id(near[f as usize])));
        }
    }
    out.into_iter().map(|(n, s)| rerail_blocks::vector![n, s]).collect()
}

/// `walk` and `look`: each name's scope chain walked from its start until a scope binds it (W's resolve).
fn look_walk(t: &Tree, inp: &dyn Inputs) -> Vec<(String, Vec<Vector<i64>>)> {
    let lscope = t.set(&SCOPE.iter().chain(COMP.iter()).copied().collect::<Vec<_>>());
    let lf_kinds: Vec<&str> = ["module", "function_definition", "lambda"].iter().chain(COMP.iter()).copied().collect();
    let lf = t.nearest("lfscope", &lf_kinds);
    // `scope_up(s, p) :- kind(s, k), lscope(k), nearest(s, p, lfscope)` (folded into its reader: no relation of its own).
    let scope_up = |s: i64| {
        let r = t.row_of(s);
        (r != NONE && lscope.contains(&t.kind(r)) && lf[r as usize] != NONE).then(|| t.id(lf[r as usize]))
    };
    let nameref = multi(&input(t, inp, "nameref"), 0, 1);
    let tp_ref: HashSet<(i64, i64)> = input(t, inp, "tp_ref").iter().map(|r| (r[0], r[1])).collect();
    let bound: HashSet<(i64, i64)> = input(t, inp, "bound_in").iter().map(|r| (r[0], r[1])).collect();
    let gbefore: HashSet<(i64, i64, i64)> = input(t, inp, "gbefore").iter().map(|r| (r[0], r[1], r[2])).collect();
    let late: HashSet<(i64, i64, i64)> = input(t, inp, "cls_late").iter().map(|r| (r[0], r[1], r[2])).collect();
    let mut walk: HashSet<(i64, i64, i64)> = HashSet::default();
    let mut look: Vec<Vector<i64>> = Vec::new();
    let mut work: Vec<(i64, i64, i64)> = Vec::new();
    for r in input(t, inp, "wstart").iter() {
        let (n, s) = (r[0], r[1]);
        for &x in nameref.get(&n).into_iter().flatten() {
            if !tp_ref.contains(&(n, x)) && walk.insert((n, x, s)) {
                work.push((n, x, s));
            }
        }
    }
    while let Some((n, x, s)) = work.pop() {
        let b = bound.contains(&(s, x));
        let l = late.contains(&(n, x, s));
        let g = gbefore.contains(&(n, x, s));
        if (b && !l) || (!b && g) {
            look.push(rerail_blocks::vector![n, x, s]);
        }
        if ((!b && !g) || l)
            && let Some(p) = scope_up(s)
            && walk.insert((n, x, p))
        {
            work.push((n, x, p));
        }
    }
    vec![("look".into(), look), ("walk".into(), walk.into_iter().map(|(n, x, s)| rerail_blocks::vector![n, x, s]).collect())]
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
fn fn_in_rows(t: &Tree, inp: &dyn Inputs, near: &[u32], body: i64) -> HashMap<u32, Vec<u32>> {
    let fdef = t.atom("function_definition");
    let mut fns: Vec<u32> = input(t, inp, "rf_def").iter().map(|r| t.row_of(r[0])).filter(|&g| g != NONE && t.kind(g) == fdef).collect();
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
fn rf_fn_in(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let near = t.nearest("fdef", &["function_definition"]);
    let fn_in = fn_in_rows(t, inp, &near, t.field_atom("body"));
    fn_in.into_iter().flat_map(|(g, hs)| hs.into_iter().map(move |h| rerail_blocks::vector![t.id(g), t.id(h)])).collect()
}

/// `rf_in`: each site's enclosing function.
fn rf_in(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let near = t.nearest("fdef", &["function_definition"]);
    let body = t.field_atom("body");
    let mut fn_in: HashMap<u32, Vec<u32>> = HashMap::default();
    for r in input(t, inp, "rf_fn_in").iter() {
        let (g, h) = (t.row_of(r[0]), t.row_of(r[1]));
        if g != NONE && h != NONE {
            fn_in.entry(g).or_default().push(h);
        }
    }
    let sites = mark(t, inp, "rf_site", 0);
    let mut out = Vec::new();
    let mut found = Vec::new();
    for n in t.rows().filter(|&r| sites[r as usize]) {
        found.clear();
        enclosing(t, n, &near, body, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        out.extend(found.iter().map(|&g| rerail_blocks::vector![t.id(n), t.id(g)]));
    }
    out
}

/// `bind_only`: names a statement binds without reading them (the eight rules of `names.rerail`).
fn bind_only(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let is_id = |r: u32| t.kind(r) == id;
    let mut out = vec![false; t.ids.len()];
    let aug = mark(t, inp, "aug_target", 0);
    for r in input(t, inp, "tgt").iter() {
        let at = t.row_of(r[0]);
        if at != NONE && is_id(at) && !aug[at as usize] {
            out[at as usize] = true;
        }
    }
    for r in input(t, inp, "tp_site").iter() {
        let at = t.row_of(r[1]);
        if at != NONE {
            out[at as usize] = true;
        }
    }
    let alias_read = mark(t, inp, "tp_alias_read", 0);
    let (assignment, alias, typed) = (t.atom("assignment"), t.atom("type_alias_statement"), t.atom("typed_parameter"));
    let (left, parameters) = (t.field_atom("left"), t.field_atom("parameters"));
    let imports = t.set(&["import_statement", "import_from_statement", "future_import_statement"]);
    let globals = t.set(&["global_statement", "nonlocal_statement"]);
    for c in t.rows() {
        let p = t.parent[c as usize];
        if p != NONE && t.in_field(c, left) {
            if t.kind(p) == assignment && is_id(c) {
                out[c as usize] = true;
            }
            if t.kind(p) == alias {
                t.each_below(c, &mut |i| {
                    if is_id(i) && !alias_read[i as usize] {
                        out[i as usize] = true;
                    }
                });
            }
        }
        if p != NONE && t.in_field(c, parameters) {
            for &q in t.kids(c) {
                if is_id(q) {
                    out[q as usize] = true;
                }
                if let Some(&first) = t.kids(q).first() {
                    if is_id(first) {
                        out[first as usize] = true;
                    }
                    if t.kind(q) == typed
                        && let Some(&id0) = t.kids(first).first()
                        && is_id(id0)
                    {
                        out[id0 as usize] = true;
                    }
                }
            }
        }
        let k = t.kind(c);
        if imports.contains(&k) {
            t.each_below(c, &mut |i| {
                if is_id(i) {
                    out[i as usize] = true;
                }
            });
        }
        if globals.contains(&k) {
            for &g in t.kids(c) {
                if is_id(g) {
                    out[g as usize] = true;
                }
            }
        }
    }
    t.rows().filter(|&r| out[r as usize]).map(|r| rerail_blocks::vector![t.id(r)]).collect()
}

/// `rf_site`: calls, attributes of a name, and read names, where none is a call's function.
fn rf_site(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let (id, call, attribute) = (t.atom("identifier"), t.atom("call"), t.atom("attribute"));
    let (function, object) = (t.field_atom("function"), t.field_atom("object"));
    // `rf_read(i) :- nameref(i, _), !bind_only(i), !rf_deleted(i), !guest_of(i, _)` (no relation of its own).
    let (names, bound, deleted, guest) = (mark(t, inp, "nameref", 0), mark(t, inp, "bind_only", 0), mark(t, inp, "rf_deleted", 0), mark(t, inp, "guest_of", 0));
    let objects = mark(t, inp, "rf_object", 0);
    t.rows()
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
        .map(|n| rerail_blocks::vector![t.id(n)])
        .collect()
}

/// `rf_selfish`: the names `self` and `cls`.
fn rf_selfish(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let atoms: Vec<i64> = t.rows().filter(|&r| t.kind(r) == id && t.text[r as usize] != i64::MIN).map(|r| t.text[r as usize]).collect::<HashSet<_>>().into_iter().collect();
    let strings = inp.texts(&atoms);
    let selfish: HashSet<i64> = atoms.iter().zip(&strings).filter(|(_, s)| *s == "self" || *s == "cls").map(|(a, _)| *a).collect();
    t.rows().filter(|&r| t.kind(r) == id && selfish.contains(&t.text[r as usize])).map(|r| rerail_blocks::vector![t.id(r)]).collect()
}

/// The stratum's rows by relation, by W's algorithm.
pub fn run(rels: &[&str], t: &Tree, inp: &dyn Inputs) -> Vec<(String, Vec<Vector<i64>>)> {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    match sorted.as_slice() {
        ["look", "walk"] => look_walk(t, inp),
        ["wstart"] => vec![("wstart".into(), wstart(t, inp))],
        ["rf_in"] => vec![("rf_in".into(), rf_in(t, inp))],
        ["rf_fn_in"] => vec![("rf_fn_in".into(), rf_fn_in(t, inp))],
        ["nameref"] => vec![("nameref".into(), nameref(t, inp))],
        ["bind_only"] => vec![("bind_only".into(), bind_only(t, inp))],
        ["rf_site"] => vec![("rf_site".into(), rf_site(t, inp))],
        ["rf_selfish"] => vec![("rf_selfish".into(), rf_selfish(t, inp))],
        _ => Vec::new(),
    }
}

/// `SPIKE CHECK` lines: the spike's rows against the generic joins'.
pub fn compare(rel: &str, spike: &[Vector<i64>], generic: &[Vector<i64>], tree: Option<&Tree>) {
    let a: HashSet<&[i64]> = spike.iter().map(|r| &r[..]).collect();
    let b: HashSet<&[i64]> = generic.iter().map(|r| &r[..]).collect();
    let only_spike: Vec<_> = a.difference(&b).take(3).collect();
    let only_generic: Vec<_> = b.difference(&a).take(3).collect();
    let (ns, ng) = (a.difference(&b).count(), b.difference(&a).count());
    if let Some(t) = tree {
        for r in only_spike.iter().chain(&only_generic) {
            eprintln!("SPIKE WHY {rel} {r:?}: {}", t.describe(r[0]));
        }
    }
    eprintln!("SPIKE CHECK {rel}: spike {} generic {} only-spike {ns} {only_spike:?} only-generic {ng} {only_generic:?}", a.len(), b.len());
}
