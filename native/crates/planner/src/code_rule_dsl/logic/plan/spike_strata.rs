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

/// Whether the stratum of `rels` is run here.
pub fn handles(rels: &[&str]) -> bool {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    STRATA.contains(&sorted.as_slice())
}

/// A chunk's named tree: each node's parent and kind, and the kinds' atoms by text.
pub struct Tree {
    parent: HashMap<i64, i64>,
    kind: HashMap<i64, i64>,
    atoms: HashMap<String, i64>,
    /// Each node's named children, in order.
    children: HashMap<i64, Vec<i64>>,
    /// Each fielded node's parent and field atom, and the field atoms by text.
    field_of: HashMap<i64, Vec<(i64, i64)>>,
    fields: HashMap<String, i64>,
}

impl Tree {
    fn atom(&self, kind: &str) -> i64 {
        self.atoms.get(kind).copied().unwrap_or(i64::MIN)
    }

    fn field_atom(&self, field: &str) -> i64 {
        self.fields.get(field).copied().unwrap_or(i64::MIN)
    }

    fn kids(&self, n: i64) -> &[i64] {
        self.children.get(&n).map_or(&[], Vec::as_slice)
    }

    /// `field(_, f, n)`.
    fn in_field(&self, n: i64, field: i64) -> bool {
        self.field_of.get(&n).is_some_and(|fs| fs.iter().any(|&(_, f)| f == field))
    }

    /// `n`'s kind and each ancestor's, with the fields it is in (check mode's account of a row).
    pub fn describe(&self, n: i64) -> String {
        let name = |k: i64| self.atoms.iter().find(|&(_, &a)| a == k).map_or("?".to_owned(), |(s, _)| s.clone());
        let fname = |f: i64| self.fields.iter().find(|&(_, &a)| a == f).map_or("?".to_owned(), |(s, _)| s.clone());
        let mut out = Vec::new();
        let mut at = Some(n);
        while let Some(a) = at.filter(|_| out.len() < 5) {
            let fs: Vec<String> = self.field_of.get(&a).into_iter().flatten().map(|&(_, f)| fname(f)).collect();
            out.push(format!("{}{fs:?}", name(self.kind(a))));
            at = self.parent.get(&a).copied();
        }
        out.join(" < ")
    }

    /// `n`'s child in `field` (`field(n, f, c)`).
    fn field_child(&self, n: i64, field: i64) -> Option<i64> {
        self.kids(n).iter().copied().find(|&c| self.in_field(c, field))
    }

    /// Each node strictly below `n`.
    fn each_below(&self, n: i64, f: &mut impl FnMut(i64)) {
        let mut stack: Vec<i64> = self.kids(n).to_vec();
        while let Some(c) = stack.pop() {
            f(c);
            stack.extend_from_slice(self.kids(c));
        }
    }

    fn set(&self, kinds: &[&str]) -> HashSet<i64> {
        kinds.iter().map(|k| self.atom(k)).collect()
    }

    fn kind(&self, n: i64) -> i64 {
        self.kind.get(&n).copied().unwrap_or(i64::MIN)
    }

    /// `nearest(n, s, set)`: the nearest strict ancestor of a kind in `set`.
    fn nearest(&self, n: i64, set: &HashSet<i64>) -> Option<i64> {
        let mut up = self.parent.get(&n).copied();
        while let Some(a) = up {
            if set.contains(&self.kind(a)) {
                return Some(a);
            }
            up = self.parent.get(&a).copied();
        }
        None
    }

    /// `within(n, a)`: `n` strictly below `a`.
    fn within(&self, n: i64, a: i64) -> bool {
        let mut up = self.parent.get(&n).copied();
        while let Some(p) = up {
            if p == a {
                return true;
            }
            up = self.parent.get(&p).copied();
        }
        false
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
pub fn tree(child: &Arc<dyn Source>, kind: Option<&Arc<dyn Source>>, node: Option<&Arc<dyn Source>>, field: Option<&Arc<dyn Source>>) -> Option<Arc<Tree>> {
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
    cell.get_or_init(|| make_tree(child, kind, node, field)).clone()
}

fn make_tree(child: &Arc<dyn Source>, kind: Option<&Arc<dyn Source>>, node: Option<&Arc<dyn Source>>, field: Option<&Arc<dyn Source>>) -> Option<Arc<Tree>> {
    let _span = super::super::profile::span(|| "spike: the chunk's tree".into());
    let mut parent = HashMap::default();
    let mut ordered: HashMap<i64, Vec<(i64, i64)>> = HashMap::default();
    child.for_rows(&[0, 1, 2], &mut |_, r| {
        parent.insert(r[1], r[0]);
        ordered.entry(r[0]).or_default().push((r[2], r[1]));
    });
    let children = ordered.into_iter().map(|(p, mut cs)| {
        cs.sort_unstable();
        (p, cs.into_iter().map(|(_, c)| c).collect())
    }).collect();
    let mut field_of = HashMap::default();
    if let Some(f) = field {
        f.for_rows(&[0, 1, 2], &mut |_, r| {
            field_of.entry(r[2]).or_insert_with(Vec::new).push((r[0], r[1]));
        });
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
    let fields = by_name(field_of.values().flatten().map(|&(_, f)| f), field_name, field_own);
    let mut kinds = HashMap::default();
    match (kind, node) {
        (Some(k), _) => k.for_rows(&[0, 1], &mut |_, r| {
            kinds.insert(r[0], r[1]);
        }),
        (None, Some(n)) => n.for_rows(&[1, 2], &mut |_, r| {
            kinds.insert(r[0], r[1]);
        }),
        (None, None) => return None,
    }
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
    let atoms = by_name(kinds.values().copied(), kind_name, kind_own);
    if check() {
        let sample: Vec<(&String, &i64)> = atoms.iter().take(6).collect();
        eprintln!("SPIKE TREE parents {} kinds {} (from {}) atoms {} {sample:?} fields {} child rows {} kind rows {:?}", parent.len(), kinds.len(), if kind.is_some() { "kind" } else { "node" }, atoms.len(), fields.len(), child.len(), kind.map(|k| (k.len(), k.columns())));
    }
    let t = Arc::new(Tree { parent, kind: kinds, atoms, children, field_of, fields });
    Some(t)
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

const SCOPE: [&str; 4] = ["module", "function_definition", "class_definition", "lambda"];
const COMP: [&str; 4] = ["list_comprehension", "set_comprehension", "dictionary_comprehension", "generator_expression"];

fn set1(rows: &[Vector<i64>]) -> HashSet<i64> {
    rows.iter().map(|r| r[0]).collect()
}

fn multi(rows: &[Vector<i64>], k: usize, v: usize) -> HashMap<i64, Vec<i64>> {
    let mut m: HashMap<i64, Vec<i64>> = HashMap::default();
    for r in rows {
        m.entry(r[k]).or_default().push(r[v]);
    }
    m
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

/// `nameref`: identifiers that are not an attribute's or a definition's name, a keyword pattern's key, nor a dotted
/// pattern's tail; with their text.
fn nameref(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let mut not: HashSet<i64> = set1(&inp.read("kwpat_key"));
    not.extend(set1(&inp.read("pat_tail")));
    let (attribute, name) = (t.field_atom("attribute"), t.field_atom("name"));
    not.extend(t.field_of.keys().copied().filter(|&c| t.in_field(c, attribute) || t.in_field(c, name)));
    let mut ids: Vec<i64> = t.kind.iter().filter(|&(n, &k)| k == id && !not.contains(n)).map(|(&n, _)| n).collect();
    ids.sort_unstable();
    inp.lookup("text", 0, &ids).into_iter().map(|r| rerail_blocks::vector![r[0], r[1]]).collect()
}

/// `wstart`: where a name's lookup starts (the five rules of `names.rerail`).
fn wstart(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let lscope: HashSet<i64> = SCOPE.iter().chain(COMP.iter()).map(|k| t.atom(k)).collect();
    let (scope, comp) = (t.set(&SCOPE), t.set(&COMP));
    let nameref = inp.read("nameref");
    let def_outside = inp.read("def_outside");
    let in_first_iter = inp.read("in_first_iter");
    let outside: HashSet<i64> = set1(&def_outside);
    let first: HashSet<(i64, i64)> = in_first_iter.iter().map(|r| (r[0], r[1])).collect();
    let host = multi(&inp.read("host_node"), 0, 1);
    let ann = multi(&inp.read("ann_start"), 0, 1);
    let mut out: HashSet<(i64, i64)> = HashSet::default();
    let names: HashSet<i64> = set1(&nameref);
    for &n in &names {
        if let Some(s) = t.nearest(n, &lscope) {
            let k = t.kind(s);
            if scope.contains(&k) && !outside.contains(&n) {
                out.insert((n, s));
            }
            if comp.contains(&k) && !first.contains(&(n, s)) {
                out.insert((n, s));
            }
        }
        for h in host.get(&n).into_iter().flatten() {
            for &s in ann.get(h).into_iter().flatten() {
                out.insert((n, s));
            }
        }
    }
    for r in def_outside.iter().chain(in_first_iter.iter()) {
        if let Some(s) = t.nearest(r[1], &lscope) {
            out.insert((r[0], s));
        }
    }
    out.into_iter().map(|(n, s)| rerail_blocks::vector![n, s]).collect()
}

/// `walk` and `look`: each name's scope chain walked from its start until a scope binds it (W's resolve).
fn look_walk(t: &Tree, inp: &dyn Inputs) -> Vec<(String, Vec<Vector<i64>>)> {
    let lscope: HashSet<i64> = SCOPE.iter().chain(COMP.iter()).map(|k| t.atom(k)).collect();
    let lfscope: HashSet<i64> = ["module", "function_definition", "lambda"].iter().chain(COMP.iter()).map(|k| t.atom(k)).collect();
    // `scope_up(s, p) :- kind(s, k), lscope(k), nearest(s, p, lfscope)` (folded into its reader: no relation of its own).
    let scope_up = |s: i64| if lscope.contains(&t.kind(s)) { t.nearest(s, &lfscope) } else { None };
    let nameref = multi(&inp.read("nameref"), 0, 1);
    let tp_ref: HashSet<(i64, i64)> = inp.read("tp_ref").iter().map(|r| (r[0], r[1])).collect();
    let bound: HashSet<(i64, i64)> = inp.read("bound_in").iter().map(|r| (r[0], r[1])).collect();
    let gbefore: HashSet<(i64, i64, i64)> = inp.read("gbefore").iter().map(|r| (r[0], r[1], r[2])).collect();
    let late: HashSet<(i64, i64, i64)> = inp.read("cls_late").iter().map(|r| (r[0], r[1], r[2])).collect();
    let mut walk: HashSet<(i64, i64, i64)> = HashSet::default();
    let mut look: Vec<Vector<i64>> = Vec::new();
    let mut work: Vec<(i64, i64, i64)> = Vec::new();
    for r in inp.read("wstart") {
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

/// The function whose body holds `n`, through the functions of `fn_in` where `n` is in a definition's head.
/// `rf_body(g, b) :- kind(g, "function_definition"), field(g, "body", b)` is the tree's (no relation of its own).
fn enclosing(t: &Tree, n: i64, fdef: &HashSet<i64>, fn_in: &HashMap<i64, Vec<i64>>, out: &mut Vec<i64>) {
    let Some(g) = t.nearest(n, fdef) else { return };
    if let Some(b) = t.field_child(g, t.field_atom("body")) {
        if t.within(n, b) {
            out.push(g);
        } else {
            out.extend(fn_in.get(&g).into_iter().flatten().copied());
        }
    }
}

/// `rf_in`: each site's enclosing function.
fn rf_in(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let fdef = t.set(&["function_definition"]);
    let fn_in = multi(&inp.read("rf_fn_in"), 0, 1);
    let mut out = HashSet::default();
    let mut found = Vec::new();
    for n in set1(&inp.read("rf_site")) {
        found.clear();
        enclosing(t, n, &fdef, &fn_in, &mut found);
        out.extend(found.iter().map(|&g| (n, g)));
    }
    out.into_iter().map(|(n, g)| rerail_blocks::vector![n, g]).collect()
}

/// `rf_fn_in`: each function's enclosing function, outermost first (so a nested one finds its parent's).
fn rf_fn_in(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let fdef = t.set(&["function_definition"]);
    let mut fns: Vec<(usize, i64)> = inp.read("rf_def").iter().map(|r| r[0]).filter(|&g| fdef.contains(&t.kind(g))).collect::<HashSet<_>>().into_iter().map(|g| {
        let mut depth = 0;
        let mut up = t.parent.get(&g).copied();
        while let Some(p) = up {
            depth += 1;
            up = t.parent.get(&p).copied();
        }
        (depth, g)
    }).collect();
    fns.sort_unstable();
    let mut fn_in: HashMap<i64, Vec<i64>> = HashMap::default();
    let mut found = Vec::new();
    for (_, g) in fns {
        found.clear();
        enclosing(t, g, &fdef, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        if !found.is_empty() {
            fn_in.insert(g, found.clone());
        }
    }
    fn_in.into_iter().flat_map(|(g, hs)| hs.into_iter().map(move |h| rerail_blocks::vector![g, h])).collect()
}

/// `bind_only`: names a statement binds without reading them (the eight rules of `names.rerail`).
fn bind_only(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let is_id = |n: i64| t.kind(n) == id;
    let mut out: HashSet<i64> = HashSet::default();
    let aug: HashSet<i64> = set1(&inp.read("aug_target"));
    out.extend(set1(&inp.read("tgt")).into_iter().filter(|&n| is_id(n) && !aug.contains(&n)));
    out.extend(inp.read("tp_site").iter().map(|r| r[1]));
    let alias_read: HashSet<i64> = set1(&inp.read("tp_alias_read"));
    let (assignment, alias, typed) = (t.atom("assignment"), t.atom("type_alias_statement"), t.atom("typed_parameter"));
    let (left, parameters) = (t.field_atom("left"), t.field_atom("parameters"));
    let imports = t.set(&["import_statement", "import_from_statement", "future_import_statement"]);
    let globals = t.set(&["global_statement", "nonlocal_statement"]);
    for (&c, &(p, f)) in t.field_of.iter().flat_map(|(c, fs)| fs.iter().map(move |pf| (c, pf))) {
        if f == left && t.kind(p) == assignment && is_id(c) {
            out.insert(c);
        }
        if f == left && t.kind(p) == alias {
            t.each_below(c, &mut |i| {
                if is_id(i) && !alias_read.contains(&i) {
                    out.insert(i);
                }
            });
        }
        if f == parameters {
            for &q in t.kids(c) {
                if is_id(q) {
                    out.insert(q);
                }
                if let Some(&first) = t.kids(q).first() {
                    if is_id(first) {
                        out.insert(first);
                    }
                    if t.kind(q) == typed
                        && let Some(&id0) = t.kids(first).first()
                        && is_id(id0)
                    {
                        out.insert(id0);
                    }
                }
            }
        }
    }
    for (&n, &k) in &t.kind {
        if imports.contains(&k) {
            t.each_below(n, &mut |i| {
                if is_id(i) {
                    out.insert(i);
                }
            });
        }
        if globals.contains(&k) {
            out.extend(t.kids(n).iter().copied().filter(|&c| is_id(c)));
        }
    }
    out.into_iter().map(|n| rerail_blocks::vector![n]).collect()
}

/// `rf_site`: calls, attributes of a name, and read names, where none is a call's function.
fn rf_site(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let (id, call, attribute) = (t.atom("identifier"), t.atom("call"), t.atom("attribute"));
    let (function, object) = (t.field_atom("function"), t.field_atom("object"));
    let in_field = |n: i64, f: i64| t.in_field(n, f);
    // `rf_read(i) :- nameref(i, _), !bind_only(i), !rf_deleted(i), !guest_of(i, _)` (no relation of its own).
    let mut unread: HashSet<i64> = set1(&inp.read("bind_only"));
    unread.extend(set1(&inp.read("rf_deleted")));
    unread.extend(set1(&inp.read("guest_of")));
    let read: HashSet<i64> = set1(&inp.read("nameref")).into_iter().filter(|n| !unread.contains(n)).collect();
    let objects: HashSet<i64> = set1(&inp.read("rf_object"));
    let mut out = Vec::new();
    for (&n, &k) in &t.kind {
        let site = if k == call {
            true
        } else if k == attribute {
            !in_field(n, function) && t.field_child(n, object).is_some_and(|o| t.kind(o) == id)
        } else if k == id {
            !in_field(n, function) && read.contains(&n) && !objects.contains(&n)
        } else {
            false
        };
        if site {
            out.push(rerail_blocks::vector![n]);
        }
    }
    out
}

/// `rf_selfish`: the names `self` and `cls`.
fn rf_selfish(t: &Tree, inp: &dyn Inputs) -> Vec<Vector<i64>> {
    let id = t.atom("identifier");
    let mut ids: Vec<i64> = t.kind.iter().filter(|&(_, &k)| k == id).map(|(&n, _)| n).collect();
    ids.sort_unstable();
    let texts = inp.lookup("text", 0, &ids);
    let atoms: Vec<i64> = texts.iter().map(|r| r[1]).collect::<HashSet<_>>().into_iter().collect();
    let strings = inp.texts(&atoms);
    let selfish: HashSet<i64> = atoms.iter().zip(&strings).filter(|(_, s)| *s == "self" || *s == "cls").map(|(a, _)| *a).collect();
    let found: HashSet<i64> = texts.iter().filter(|r| selfish.contains(&r[1])).map(|r| r[0]).collect();
    found.into_iter().map(|n| rerail_blocks::vector![n]).collect()
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
