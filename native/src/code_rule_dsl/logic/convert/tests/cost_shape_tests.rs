//! The reference graph's cost shape (note t57): what the rules that replaced W's reference graph (`python/names.rerail`,
//! `python/references.rerail`, decision 0483) cost per name a file reads, against what W's Rust did for the same name.
//!
//! W's `ScopeTable` walks each load once up its scope chain, and its summary reads each site once. The rules' joins
//! should then do a bounded number of steps a name read, and the same amount for a file twice as long. This module
//! runs the rules over generated files (`module`, of `n` definitions) with the profile on and reads, per head
//! relation, the rows into every join step (its "work", as `profile::plan_work` prices a plan), the bytes each span
//! allocated, and the copies 0353 counts. Each test pins one cost to what W's shape needs.

use super::write_files;
use crate::code_rule_dsl::logic::profile;
use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One generated module of `n` units: a class with two methods, a function with locals, a comprehension, a lambda, a
/// nested function, imports and calls between them, as the corpus's modules have, in proportion.
pub(super) fn module(n: usize, seed: usize) -> String {
    let mut s = String::from("\"\"\"A module.\"\"\"\nimport os\nimport json as js\nfrom typing import Optional, List\nfrom collections import defaultdict\n\nLIMIT = 10\n\n");
    for i in 0..n {
        let k = i + seed;
        let _ = write!(
            s,
            r#"
class Model{k}(object):
    kind = "m{k}"
    size: int = LIMIT

    def __init__(self, name: str, value: Optional[int] = None) -> None:
        self.name = name
        self.value = value or LIMIT
        self.items: List[str] = []

    def run{k}(self, data, *args, **kwargs):
        total = 0
        for idx, item in enumerate(data):
            if item and idx < self.size:
                total += len(item) + self.value
            else:
                total = helper{k}(total, item)
        names = [str(x) for x in data if x]
        pairs = {{a: b for a, b in zip(names, data)}}
        def inner(y):
            return y + total + len(names)
        return inner(total), pairs, js.dumps(self.items)


def helper{k}(acc, item, flag=False):
    result = acc
    lookup = defaultdict(list)
    lookup[item].append(acc)
    try:
        result = int(os.path.join(str(item), "x"))
    except ValueError as err:
        print(err)
    key = lambda v: v * 2
    return max(result, key(acc)) if flag else sorted(lookup, key=len)


def use{k}():
    m = Model{k}("n", 1)
    out = m.run{k}([1, 2, 3])
    return helper{k}(out, None, flag=True), Model{k}.kind
"#
        );
    }
    s
}

/// What one run of the rules gave.
#[derive(Default, Debug)]
pub(super) struct Cost {
    /// By head relation (the plan's derived relation, generated prefixes cut): work, the rows its last steps made,
    /// and its plans.
    pub heads: BTreeMap<String, (u64, u64, usize)>,
    /// By plan text: rows in and out per step.
    pub plans: BTreeMap<String, Vec<(u64, u64)>>,
    /// By span path: bytes allocated while open, less the spans inside it.
    pub alloc: BTreeMap<String, (u64, u64)>,
    /// By copy site: bytes, calls.
    pub copies: BTreeMap<String, (u64, u64)>,
    /// Rows of each output relation.
    pub outputs: BTreeMap<String, usize>,
}

impl Cost {
    pub fn work(&self, head: &str) -> u64 {
        self.heads.get(head).map_or(0, |h| h.0)
    }
    pub fn made(&self, head: &str) -> u64 {
        self.heads.get(head).map_or(0, |h| h.1)
    }
    pub fn all_work(&self) -> u64 {
        self.heads.values().map(|h| h.0).sum()
    }
    pub fn rows(&self, out: &str) -> usize {
        self.outputs.get(out).copied().unwrap_or(0)
    }
    /// Bytes allocated in the spans whose last part starts with `leaf`.
    pub fn allocated(&self, leaf: &str) -> u64 {
        self.alloc.iter().filter(|(p, _)| p.rsplit('/').next().is_some_and(|l| l.starts_with(leaf))).map(|(_, a)| a.0).sum()
    }
    pub fn copied(&self, name_part: &str) -> u64 {
        self.copies.iter().filter(|(n, _)| n.contains(name_part)).map(|(_, c)| c.0).sum()
    }
    /// The plans of `head`, each with its steps' rows in:out, for a failure's message.
    pub fn shown(&self, head: &str) -> String {
        let mut s = String::new();
        for (plan, steps) in self.plans.iter().filter(|(p, _)| profile::kinds::head(p.split_once("] ").map_or(p.as_str(), |x| x.1)) == head) {
            let _ = writeln!(s, "  {} | {}", steps.iter().map(|(a, b)| format!("{a}:{b}")).collect::<Vec<_>>().join(" "), profile::short(plan, 400));
        }
        s
    }
    /// Every head by work, with its plans, for the diagnostic dump.
    pub fn report(&self, top: usize) -> String {
        let mut v: Vec<_> = self.heads.iter().collect();
        v.sort_by_key(|(_, h)| std::cmp::Reverse(h.0));
        let mut s = format!("work in all {}\n", self.all_work());
        for (head, (work, made, plans)) in v.into_iter().take(top) {
            let _ = writeln!(s, "{work:>10} {made:>9} {plans:>3}  {head}");
            s += &self.shown(head);
        }
        s
    }
}

/// The interface the tests read: every relation that W's graph has a counterpart of, and the names read.
const PROGRAM: &str = r#"
.language python
.scope include "**/*.py", "**/*.pyi"
#include "python/references.rerail"
.decl c_nameref(n: node)
c_nameref(n) :- nameref(n, _).
.decl c_look(n: node, s: node)
c_look(n, s) :- look(n, _, s).
.decl c_ref(t: node, k: symbol, a: symbol, x: symbol, q: symbol)
c_ref(t, k, a, x, q) :- rf_ref(t, k, a, x, q).
.decl c_edge(s: node, t: node)
c_edge(s, t) :- reference_edge(s, t, _).
.decl c_ident(n: node)
c_ident(n) :- kind(n, "identifier").
.decl c_node(n: node)
c_node(n) :- node(_, n, _, _, _).
.output c_nameref, c_look, c_ref, c_edge, c_ident, c_node
"#;

const OUTPUTS: [&str; 6] = ["c_nameref", "c_look", "c_ref", "c_edge", "c_ident", "c_node"];

/// The rules over `files` modules of `n` units each, in one chunk, profiled.
pub(super) fn run(files: usize, n: usize) -> Cost {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("cost-shape-{files}-{n}"));
    let out = dir.with_extension("out");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
    let sources: Vec<(String, String)> = (0..files).map(|f| (format!("pkg/m{f}.py"), module(n, f * 1000))).collect();
    write_files(&dir, sources.iter().map(|(p, s)| (p.as_str(), s.as_str())).chain([("pkg/__init__.py", "")]));
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let was = profile::on();
    profile::set(true);
    profile::reset();
    crate::code_rule_dsl::logic::plan::corpus::run(PROGRAM, &dir, None, &out, "program", Binding::Cpu, 1 << 20).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut cost = Cost::default();
    {
        let held = profile::merged();
        for ((path, plan), steps) in &held.native {
            let h = cost.heads.entry(profile::kinds::head(plan)).or_default();
            h.0 += profile::plan_work(steps);
            h.1 += steps.last().map_or(0, |s| s.1);
            h.2 += 1;
            let at = path.split('/').filter(|s| !s.starts_with("stratum ") && !s.starts_with("engine run")).collect::<Vec<_>>().join("/");
            let p = cost.plans.entry(format!("[{at}] {plan}")).or_default();
            if p.len() < steps.len() {
                p.resize(steps.len(), (0, 0));
            }
            for (a, b) in p.iter_mut().zip(steps) {
                a.0 += b.0;
                a.1 += b.1;
            }
        }
    }
    for (path, t) in profile::spans() {
        cost.alloc.insert(path, (t.own_alloc, t.alloc));
    }
    for (site, t) in rerail_prof::copies::sites() {
        let c = cost.copies.entry(site.name.to_owned()).or_default();
        c.0 += t.bytes;
        c.1 += t.calls;
    }
    profile::set(was);
    for rel in OUTPUTS {
        cost.outputs.insert(rel.to_owned(), crate::code_rule_dsl::logic::plan::output::shown(&out, rel).map_or(0, |r| r.len()));
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
    cost
}

/// The dump the tests' bounds were read from: `RERAIL_COST_FILES`, `RERAIL_COST_N` (defaults 4 and 8).
#[test]
#[ignore = "a diagnostic dump, not a check"]
fn cost_shape_dump() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (files, n) = (env("RERAIL_COST_FILES", 4), env("RERAIL_COST_N", 8));
    let c = run(files, n);
    println!("outputs {:?}", c.outputs);
    println!("{}", c.report(env("RERAIL_COST_TOP", 60)));
    let mut a: Vec<_> = c.alloc.iter().collect();
    a.sort_by_key(|(_, x)| std::cmp::Reverse(x.0));
    for (p, (own, _)) in a.into_iter().take(30) {
        println!("alloc {:>10} {}", own, profile::short(p, 200));
    }
    for (n, (b, calls)) in &c.copies {
        println!("copy {b:>10} {calls:>6} {n}");
    }
}

/// How each head grows: the rules over (4 files, n), (4·k files, n) and (4 files, n·k), each head's work as a multiple
/// of the names read's (`RERAIL_COST_K`, default 4). A head that grows faster than the names is an expansion.
#[test]
#[ignore = "a diagnostic dump, not a check"]
fn cost_shape_growth() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (n, k) = (env("RERAIL_COST_N", 4), env("RERAIL_COST_K", 4));
    let base = run(4, n);
    let wide = run(4 * k, n);
    let long = run(4, n * k);
    let names = |c: &Cost| c.rows("c_nameref").max(1) as f64;
    let (fw, fl) = (names(&wide) / names(&base), names(&long) / names(&base));
    println!("names read: base {} wide {} ({fw:.1}x) long {} ({fl:.1}x)", names(&base), names(&wide), names(&long));
    println!("work in all: base {} wide {} ({:.2} of names' growth) long {} ({:.2})", base.all_work(), wide.all_work(), wide.all_work() as f64 / base.all_work() as f64 / fw, long.all_work(), long.all_work() as f64 / base.all_work() as f64 / fl);
    let mut heads: Vec<&String> = wide.heads.keys().chain(long.heads.keys()).collect();
    heads.sort();
    heads.dedup();
    let mut rows: Vec<(f64, String)> = heads
        .into_iter()
        .map(|h| {
            let b = base.work(h).max(1) as f64;
            let (gw, gl) = (wide.work(h) as f64 / b / fw, long.work(h) as f64 / b / fl);
            (gw.max(gl) * (wide.work(h).max(long.work(h)) as f64), format!("{:>9} {:>9} {:>9}  wide {gw:>5.2} long {gl:>5.2}  {h}", base.work(h), wide.work(h), long.work(h)))
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, r) in rows.iter().take(env("RERAIL_COST_TOP", 50)) {
        println!("{r}");
    }
    if let Ok(h) = std::env::var("RERAIL_COST_PLANS") {
        for h in h.split(',') {
            println!("== {h} base\n{}== {h} wide\n{}== {h} long\n{}", base.shown(h), wide.shown(h), long.shown(h));
        }
    }
}

impl Cost {
    /// Own bytes allocated by span, keyed by the span's path with its stratum and counts cut, summed.
    pub fn alloc_by_leaf(&self) -> BTreeMap<String, u64> {
        let mut by: BTreeMap<String, u64> = BTreeMap::new();
        for (p, (own, _)) in &self.alloc {
            let parts: Vec<&str> = p.split('/').collect();
            let leaf = parts[parts.len().saturating_sub(2)..].join("/");
            let leaf: String = leaf.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
            *by.entry(leaf).or_default() += own;
        }
        by
    }
    pub fn all_alloc(&self) -> u64 {
        self.alloc.values().map(|a| a.0).sum()
    }
}

/// Allocation's growth: own bytes by span (its last two parts), at (4, n) and (4·k, n) files.
#[test]
#[ignore = "a diagnostic dump, not a check"]
fn cost_alloc_growth() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (n, k) = (env("RERAIL_COST_N", 8), env("RERAIL_COST_K", 4));
    let base = run(4, n);
    let wide = run(4 * k, n);
    let f = wide.rows("c_nameref") as f64 / base.rows("c_nameref").max(1) as f64;
    println!("names read {} -> {} ({f:.1}x); allocated {} -> {} MB; per name read {} -> {} bytes", base.rows("c_nameref"), wide.rows("c_nameref"), base.all_alloc() >> 20, wide.all_alloc() >> 20, base.all_alloc() / base.rows("c_nameref").max(1) as u64, wide.all_alloc() / wide.rows("c_nameref").max(1) as u64);
    let (b, w) = (base.alloc_by_leaf(), wide.alloc_by_leaf());
    let mut rows: Vec<(i64, String)> = w.iter().map(|(l, x)| { let y = b.get(l).copied().unwrap_or(0); (*x as i64 - y as i64, format!("{:>10} {:>11} {:>+11}  {}", y, x, *x as i64 - y as i64, profile::short(l, 170))) }).collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, r) in rows.iter().take(env("RERAIL_COST_TOP", 40)) {
        println!("{r}");
    }
    for (n, (b, calls)) in &wide.copies {
        println!("copy {b:>10} {calls:>6} {n}");
    }
}

impl Cost {
    /// By head: the work of its plans that ran in the cross-file session, and in all.
    pub fn cross_work(&self) -> BTreeMap<String, (u64, u64)> {
        let mut by: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for (p, steps) in &self.plans {
            let (at, plan) = p.split_once("] ").unwrap_or(("", p));
            let e = by.entry(profile::kinds::head(plan)).or_default();
            let w = profile::plan_work(steps);
            e.1 += w;
            if at.starts_with("[cross") {
                e.0 += w;
            }
        }
        by
    }
}

/// Which heads' joins run in the cross-file session, and how much of their work.
#[test]
#[ignore = "a diagnostic dump, not a check"]
fn cost_cross_dump() {
    let c = run(4, 8);
    let mut v: Vec<_> = c.cross_work().into_iter().filter(|(_, (x, _))| *x > 0).collect();
    v.sort_by_key(|(_, (x, _))| std::cmp::Reverse(*x));
    let all: u64 = v.iter().map(|(_, (x, _))| x).sum();
    println!("cross work {all} of {} in all", c.all_work());
    for (h, (x, a)) in v {
        println!("{x:>9} {a:>9}  {h}");
    }
}

// ---- The cases (note t57 §7). Each pins one cost of the rules' shape to what W's shape spends, and fails with the
// ---- plans or spans that spend it. They run 4 generated files of 4 units each (about 1,600 names read).

/// The run the cases read, and the names it reads.
fn case() -> (Cost, u64) {
    let c = run(4, 4);
    let names = c.rows("c_nameref") as u64;
    assert!(names > 1000, "the corpus reads names: {:?}", c.outputs);
    (c, names)
}

/// `head`'s join work is at most `per` units a row it makes (`per_name`: a name read): a lookup reads its input
/// once and tests its filters in the same step.
fn bounded(c: &Cost, head: &str, bound: u64, what: &str) {
    let (work, made) = (c.work(head), c.made(head));
    assert!(work <= bound, "{head}: {work} units of join work for {made} rows made, over {bound} ({what}). Its plans, rows in:out per step:\n{}", c.shown(head));
}

/// The steps whose literal is only a filter of bound variables (an id's range, a constant kind, a comparison): each
/// costs the join a unit a row though it binds nothing.
fn filter_steps(plan: &str) -> usize {
    let body = plan.split_once(": ").map_or(plan, |x| x.1);
    body.split(" , ").filter(|l| l.starts_with('$') && (l.contains(" < ") || l.contains(" > ") || l.contains(" != ")) && !l.contains('@')).count()
}

// -- the lookup (W: `ScopeTable::resolve_python_index`, one walk a load)

/// E3, E7. `wstart`: where a name's lookup begins is its nearest scope: one read a name. The rules make it in six
/// steps a name (the name, `!def_outside`, `nearest`, an id's range, `kind`, `scope`).
#[test]
fn the_lookups_start_is_one_read_a_name() {
    let (c, names) = case();
    bounded(&c, "wstart", 2 * names, "one read of the nearest scope a name read");
}

/// E3. `walk`: the lookup's first scope is one row a name, made from `wstart` with no further join.
#[test]
fn the_walk_is_one_step_a_name() {
    let (c, names) = case();
    bounded(&c, "walk", names + names / 4, "a row a name from wstart, its negations tested in the same step");
}

/// E4. `bind_only`, a union of ten rules read only negated (`!bind_only(i)`): demand made every rule probe every
/// identifier. W marks a binding target as it walks the statement that binds it.
#[test]
fn a_negated_union_is_not_probed_rule_by_rule_for_every_name() {
    let (c, _) = case();
    bounded(&c, "bind_only", 2 * c.made("bind_only") + c.rows("c_nameref") as u64 / 4, "its binding sites, made once from the statements that bind");
}

/// E3. `rf_in`: a site's function is its nearest `def` when the site is in its body: one read, one test.
#[test]
fn a_sites_function_is_one_read() {
    let (c, _) = case();
    bounded(&c, "rf_in", 2 * c.made("rf_in"), "two units a row made");
}

/// E1. `rf_qn`: a definition's qualified name is its scope's and its name. The recursive rule binds the scope and
/// lists every node under it (`rr_nearest_rf_dc($f, $s)` with `$s` bound) to keep the few that are definitions.
#[test]
fn a_qualified_name_does_not_list_a_scopes_descendants() {
    let (c, _) = case();
    let made = c.made("rf_qn").max(1);
    let widest = c.plans.iter().filter(|(p, _)| p.contains("rf_qn($f, $q)")).flat_map(|(_, s)| s.iter().map(|x| x.1)).max().unwrap_or(0);
    assert!(widest <= 2 * made, "rf_qn: a step made {widest} rows for {made} qualified names. Its plans:\n{}", c.shown("rf_qn"));
}

/// E2. `f < n` over nodes is a read of the two nodes' order on the tape, not two joins with `node_order`.
#[test]
fn a_comparison_of_nodes_is_no_join() {
    let (c, _) = case();
    let joined: Vec<&String> = c.plans.keys().filter(|p| p.contains("node_order(")).collect();
    let units: u64 = c.plans.iter().filter(|(p, _)| p.contains("node_order(")).map(|(_, s)| profile::plan_work(s)).sum();
    assert!(joined.is_empty(), "{} plans join node_order to compare nodes ({units} units of work):\n{}", joined.len(), joined.iter().map(|p| profile::short(p, 300)).collect::<Vec<_>>().join("\n"));
}

/// E2. `cls_early` reads the class's bindings before a name, not every name of the class's body against each.
#[test]
fn a_class_bodys_early_bindings_do_not_cross_its_names() {
    let (c, _) = case();
    bounded(&c, "cls_early", 2 * c.made("cls_early") + c.made("cls_late") * 2 + 64, "the class's bindings and names, each once");
}

/// E10. `cls_late` reads a class body's names once, not through six negations a name.
#[test]
fn a_class_bodys_late_bindings_are_read_once() {
    let (c, _) = case();
    bounded(&c, "cls_late", 4 * c.made("cls_late") + 64, "a name of the class body, tested once");
}

// -- the summary (W: `calls::summarize`, a pass over the table)

/// E3. `param`: a function's parameters are its parameter list's children, read once each.
#[test]
fn a_functions_parameters_are_read_once() {
    let (c, _) = case();
    bounded(&c, "param", 3 * c.made("param"), "three units a parameter");
}

/// E6. `rf_builtin`: the builtin names are a set of 150; a name's text is tested against it in the step that reads
/// the text, not after joining every read name's text.
#[test]
fn a_builtin_is_tested_where_its_text_is_read() {
    let (c, _) = case();
    bounded(&c, "rf_builtin", c.made("rf_named") / 4 + 4 * c.made("rf_builtin"), "a test in the text's step");
}

/// E6. `rf_fcond`: a site's nearest condition is read and its kind tested in one step.
#[test]
fn a_sites_condition_is_one_read() {
    let (c, _) = case();
    bounded(&c, "rf_fcond", 2 * c.made("rf_fcond") + c.made("rf_fn_at") / 2, "one read a site");
}

/// E6. `rf_crossing`: a read in a nested function crosses its scope; the nearest function is read once a read.
#[test]
fn a_crossing_is_one_read() {
    let (c, _) = case();
    bounded(&c, "rf_crossing", 2 * c.made("rf_crossing") + c.made("rf_fn_read") / 2, "one read a function read");
}

/// E9. `rf_same_qn`: two definitions of one qualified name in a document: a group of the names, not every pair
/// joined through two `docof`s and two `node_order`s to find none.
#[test]
fn same_qualified_names_are_a_group_not_pairs() {
    let (c, _) = case();
    bounded(&c, "rf_same_qn", c.made("rf_qn") + 2 * c.made("rf_same_qn"), "one read of each qualified name");
}

/// E3. A plain scan of `node`: one unit a row. The lowering's id-range bounds (`$n < 6917529027641081856`) are
/// steps every row passes.
#[test]
fn a_scan_is_one_unit_a_row() {
    let (c, _) = case();
    bounded(&c, "c_node", c.made("c_node") + 1, "one unit a node");
}

/// E3. No plan carries a step that only bounds an id's range or compares bound values: those are tests in the
/// step that binds them.
#[test]
fn filters_are_no_steps_of_their_own() {
    let (c, names) = case();
    let units: u64 = c.plans.iter().map(|(p, s)| {
        let body: Vec<&str> = p.split_once(": ").map_or(p.as_str(), |x| x.1).split(" , ").collect();
        body.iter().zip(s).filter(|(l, _)| filter_steps(l) == 1).map(|(_, (a, _))| a).sum::<u64>()
    }).sum();
    let plans = c.plans.keys().filter(|p| filter_steps(p) > 0).count();
    assert!(units == 0, "{plans} plans spend {units} units ({:.1} a name read) on steps that only filter bound variables", units as f64 / names as f64);
}

// -- the parts (W: one pass a file, then the link)

/// E5. A relation the per-file part can make is made there (decision 0652): none of `references.rerail`'s per-file
/// relations (above its "across files" line) joins in the cross-file session, which holds every file's rows at once.
#[test]
fn per_file_relations_join_in_the_per_file_part() {
    let (c, _) = case();
    let lib = rerail_store::source_tree::native().join("../docs/rule-syntax/lib/python");
    let refs = std::fs::read_to_string(lib.join("references.rerail")).unwrap();
    let names = std::fs::read_to_string(lib.join("names.rerail")).unwrap();
    let (before, _) = refs.split_once("// ==== across files").expect("references.rerail's across-files line");
    let local: std::collections::HashSet<String> = before.lines().chain(names.lines()).filter_map(|l| l.strip_prefix(".decl ")).filter_map(|l| l.split_once('(')).map(|(h, _)| h.trim().to_owned()).collect();
    let crossed: Vec<(String, u64)> = c.cross_work().into_iter().filter(|(h, (x, _))| *x > 0 && local.contains(h)).map(|(h, (x, _))| (h, x)).collect();
    let all: u64 = crossed.iter().map(|x| x.1).sum();
    assert!(crossed.is_empty(), "{} per-file relations join in the cross-file part, {all} units of work: {crossed:?}", crossed.len());
}

/// E5. The cross-file part is the link: a twentieth of the work at most (W's link was 8% of its graph's CPU).
#[test]
fn the_cross_file_part_is_the_link() {
    let (c, _) = case();
    let cross: u64 = c.cross_work().values().map(|x| x.0).sum();
    assert!(cross * 20 <= c.all_work(), "the cross-file part does {cross} of {} units of join work ({:.0}%)", c.all_work(), 100.0 * cross as f64 / c.all_work() as f64);
}

/// E7. The whole reference graph's joins: W's run joined 17 units a name read, its graph in Rust besides. The rules
/// join about 140; each name read flows through a dozen per-node relations, each a materialized relation.
#[test]
fn the_reference_graphs_joins_are_bounded_a_name_read() {
    let (c, names) = case();
    let per = c.all_work() as f64 / names as f64;
    assert!(per <= 40.0, "{per:.0} units of join work a name read ({} in all, {names} names). The heads by work:\n{}", c.all_work(), c.report(25));
}

// -- memory

/// Counting a base relation that is a view of the tape (`node`, `text`, `docof`, `node_order`) allocates nothing: its
/// counts are the tape's (decision 0656's auxiliary index). `all` allocated 7.2 GB in these counts.
#[test]
fn counting_a_base_relation_allocates_nothing() {
    let (c, names) = case();
    let spans: Vec<(&String, u64)> = c.alloc.iter().filter(|(p, _)| { let l = p.rsplit('/').next().unwrap_or(p); l.starts_with("count ") && p.contains("the base relations counted") }).map(|(p, a)| (p, a.0)).collect();
    let all: u64 = spans.iter().map(|x| x.1).sum();
    assert!(all == 0, "counting the base relations allocated {all} bytes ({} a name read): {:?}", all / names, spans.iter().map(|(p, a)| format!("{a} {}", p.rsplit('/').next().unwrap())).collect::<Vec<_>>());
}

/// The statistics of a view (its rows at the held nodes, the batch's counts) are read off the tape, not counted into
/// the heap.
#[test]
fn a_views_statistics_allocate_nothing() {
    let (c, names) = case();
    let leaves = ["'s rows at the held nodes, counted", "statistics off the corpus batch", "a base relation counted whole"];
    let spans: Vec<(&String, u64)> = c.alloc.iter().filter(|(p, _)| leaves.iter().any(|l| p.rsplit('/').next().is_some_and(|x| x.contains(l)))).map(|(p, a)| (p, a.0)).collect();
    let all: u64 = spans.iter().map(|x| x.1).sum();
    assert!(all == 0, "a view's statistics allocated {all} bytes ({} a name read): {:?}", all / names, spans.iter().map(|(p, a)| format!("{a} {}", p.rsplit('/').next().unwrap())).collect::<Vec<_>>());
}

/// A leaf's string is a read of the tape (0656 step 4: `text` is a view), not a document row made a string.
#[test]
fn string_annotations_are_no_rows() {
    let (c, names) = case();
    let a = c.allocated("produced rows: string annotations");
    assert!(a == 0, "the string annotations' rows allocated {a} bytes, {} a name read", a / names);
}

/// 0353 step 3: core's per-node rows are views of the tape; nothing copies them.
#[test]
fn per_node_rows_are_not_copied() {
    let (c, _) = case();
    let b = c.copied("per-node rows");
    assert!(b == 0, "{b} bytes of per-node rows copied ({:?})", c.copies.iter().find(|(n, _)| n.contains("per-node rows")));
}

/// 0353 step 6: a head's rows are written where its relation keeps them, not made in `Fresh` and copied.
#[test]
fn head_rows_are_not_copied_from_fresh() {
    let (c, names) = case();
    let b = c.copied("(push_fresh)");
    assert!(b == 0, "{b} bytes of head rows copied from Fresh into their relations ({} a name read)", b / names);
}

/// 0353 step 6: workers' head rows are not staged and merged into one Fresh.
#[test]
fn head_rows_are_not_merged_across_workers() {
    let (c, _) = case();
    let b = c.copied("Fresh::absorb");
    assert!(b == 0, "{b} bytes of head rows staged and merged across workers");
}

/// 0353 step 4a: a parser's tape is parsed into its room, not copied there.
#[test]
fn a_tape_is_not_copied_into_its_room() {
    let (c, _) = case();
    let b = c.copied("place_stored");
    assert!(b == 0, "{b} bytes of tapes copied into their rooms");
}

/// The joins allocate in blocks, not the heap a row: `all`'s strata allocated gigabytes in their joins (2.2 KB a name
/// read here).
#[test]
fn the_joins_allocate_little_a_name_read() {
    let (c, names) = case();
    let a = c.allocated("engine run (joins)");
    assert!(a / names <= 256, "the joins allocated {a} bytes, {} a name read", a / names);
}

/// Closing a stratum's engine frees; it does not allocate a row's worth a name.
#[test]
fn closing_an_engine_allocates_little() {
    let (c, names) = case();
    let a = c.allocated("engine close");
    assert!(a / names <= 64, "closing the engines allocated {a} bytes, {} a name read", a / names);
}

/// The live statistics of a run's relations are counts kept as the rows are made, not maps built per stratum.
#[test]
fn live_statistics_allocate_little() {
    let (c, names) = case();
    let a = c.allocated("engine live statistics");
    assert!(a / names <= 32, "the live statistics allocated {a} bytes, {} a name read", a / names);
}

/// The matcher wave's rows go to their relations' blocks: `all`'s waves allocated 6.2 GB.
#[test]
fn the_matcher_wave_allocates_little() {
    let (c, names) = case();
    let a = c.allocated("ast matcher wave run");
    assert!(a / names <= 128, "the AST matcher waves allocated {a} bytes, {} a name read", a / names);
}

/// Blocks freed in the run go back to the system: at its end none wait on the free lists (`all`: 829 MB did at the
/// peak).
#[test]
fn freed_blocks_are_given_back() {
    let (_c, _) = case();
    let (held, free) = rerail_accel::blocks::host_held();
    assert!(free <= 4 << 20, "{} MB of blocks free on their lists after the run ({} MB held)", free >> 20, held >> 20);
}

/// A run bound to the CPU leases no device memory (`all`'s CPU run held 190 MB of the Metal pool).
#[test]
fn a_cpu_run_leases_no_device_memory() {
    let (_c, _) = case();
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        let (live, idle) = rerail_accel::gpu::shared::held_bytes();
        assert!(live + idle == 0, "a CPU run holds {} MB of device leases and {} MB idle", live >> 20, idle >> 20);
    }
}
