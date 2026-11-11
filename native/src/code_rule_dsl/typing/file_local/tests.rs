use super::*;

/// Source spans are locations but are not tree nodes. Their explicit document column must
/// survive the local split, so locating their rows never adds `node(d, span, ...)`.
#[test]
fn source_relations_keep_their_document_location() {
    use crate::code_rule_dsl::logic::rr::parse;
    for (relation, columns, arguments) in [
        ("source", "d: number, sp: loc", "d, sp"),
        ("src_line", "d: number, l: number, sp: loc", "d, l, sp"),
        ("line_count", "d: number, k: number", "d, k"),
        ("docof", "sp: loc, d: number", "sp, d"),
    ] {
        let text = format!(".decl {relation}({columns})\n.input {relation}\n.decl selected(d: number)\nselected(d) :- {relation}({arguments}).\n.output selected\n");
        let program = parse(&text).unwrap();
        let (_, locations, _) = split_located(&program);
        assert_eq!(locations["selected"], [0], "{relation}: {locations:?}");
        let mut located = program;
        locate(&mut located).unwrap();
        assert!(!located.rules.iter().any(|rule| rule.body.iter().filter_map(|lit| lit.atom()).any(|atom| atom.rel == "node")), "{relation}: a span acquired a tree-node constraint");
    }
}

/// Two trees in one chunk (kinds 1 = module, 2 = function, 3 = other):
///
/// ```text
/// 0 module            5 module
/// ├ 1 function        └ 6 other
/// │ └ 2 other           └ 7 function
/// │   └ 3 function
/// └ 4 other
/// ```
fn chunk() -> (Vec<u32>, Vec<i64>) {
    (vec![NONE, 0, 1, 2, 0, NONE, 5, 6], vec![1, 2, 3, 2, 3, 1, 3, 2])
}

#[test]
fn nearest_is_each_nodes_closest_ancestor_in_the_set() {
    let (parent, kind) = chunk();
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2], vec![1, 2]];
    let got = nearest(&trees, &sets, Binding::Cpu).unwrap();
    assert_eq!(got[0], [NONE, 0, 0, 0, 0, NONE, 5, 5]);
    assert_eq!(got[1], [NONE, NONE, 1, 1, NONE, NONE, NONE, NONE]);
    assert_eq!(got[2], [NONE, 0, 1, 1, 0, NONE, 5, 5]);
}

/// The device's work queue solves the same system to the same values (where there is a device).
#[test]
fn the_device_finds_the_cpus_nearest() {
    // A deep tree and a wide one, over three sets.
    let mut parent = vec![NONE];
    let mut kind = vec![1];
    for i in 1..20_000_u32 {
        parent.push(if i % 7 == 0 { i / 2 } else { i - 1 });
        kind.push(i64::from(i % 5));
    }
    let trees = Trees { parent: &parent, kind: &kind };
    let sets = [vec![1], vec![2, 3], vec![4]];
    let cpu = nearest(&trees, &sets, Binding::Cpu).unwrap();
    let metal = nearest(&trees, &sets, Binding::Metal).unwrap();
    assert_eq!(cpu, metal);
    // Climbing parents gives the same answer.
    for (k, set) in sets.iter().enumerate() {
        for node in 0..parent.len() {
            let mut at = parent[node];
            while at != NONE && !set.contains(&kind[at as usize]) {
                at = parent[at as usize];
            }
            assert_eq!(cpu[k][node], at, "set {k} node {node}");
        }
    }
}

/// T's programs split into file-local relations and the rest: what the cross-file rest reads.
#[test]
fn t_programs_split_into_file_local_and_cross_file() {
    use crate::code_rule_dsl::logic::rr::parse;
    for name in ["calls.rr", "calls_rust.rr", "workload.rr"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs").join(name);
        let prog = parse(&crate::code_rule_dsl::logic::settings::program_text(&path).unwrap()).unwrap();
        let local = split(&prog);
        let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
        let global: Vec<&&str> = derived.iter().filter(|r| !local.contains(**r)).collect();
        let mut reads_base = std::collections::BTreeSet::new();
        let mut reads_local = std::collections::BTreeSet::new();
        for rule in prog.rules.iter().filter(|r| !local.contains(r.head.rel.as_str())) {
            for l in &rule.body {
                if let Some(a) = l.atom() {
                    if prog.inputs.iter().any(|x| x == &a.rel) { reads_base.insert(a.rel.to_string()); }
                    if local.contains(a.rel.as_str()) { reads_local.insert(a.rel.to_string()); }
                }
            }
        }
        // Projections of the tree are file-local; what an import binds is not.
        assert!(local.contains("kind"), "{name}");
        assert!(!local.contains("calls") && !local.contains("res") || name == "workload.rr", "{name}");
        eprintln!("SPLIT {name}: {} local, {} cross-file {global:?}\n  cross-file reads base {reads_base:?}\n  and local {reads_local:?}", local.len(), global.len());
    }
}

/// Where the device's nearest time goes (ignored; `--ignored --nocapture`): one chunk-sized forest (about
/// 172K nodes, as a 1-in-8 data-layer chunk), solved by the work queue, by the CPU worklist, and by one pass in
/// preorder (a parent precedes its children, so each node reads its parent's answer once).
#[test]
#[ignore]
fn nearest_costs_by_solver() {
    // Depth capped as real syntax trees are (`RERAIL_NEAREST_DEPTH`, default 40), and uncapped.
    for cap in [std::env::var("RERAIL_NEAREST_DEPTH").ok().and_then(|d| d.parse().ok()).unwrap_or(40usize), usize::MAX] {
        nearest_costs(cap);
    }
}

fn nearest_costs(cap: usize) {
    let n = 172_000usize;
    // Preorder forest: 80 documents; a node's parent is the open node on top of a stack that grows and shrinks.
    let mut parent = Vec::with_capacity(n);
    let mut kind = Vec::with_capacity(n);
    let mut stack: Vec<u32> = Vec::new();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rand = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..n {
        if i % (n / 80) == 0 {
            stack.clear();
        }
        // Close some open nodes (keeps depth around 20-40), then this node's parent is the top.
        while stack.len() > 1 && (rand() % 3 == 0 || stack.len() >= cap) {
            stack.pop();
        }
        parent.push(stack.last().copied().unwrap_or(NONE));
        kind.push((rand() % 300) as i64);
        stack.push(i as u32);
    }
    let depth = (0..n).map(|mut v| { let mut d = 0; while parent[v] != NONE { v = parent[v] as usize; d += 1; } d }).max().unwrap();
    let sets = vec![vec![7]];
    let trees = Trees { parent: &parent, kind: &kind };
    let sys = system(&trees, &sets).unwrap();
    let t = std::time::Instant::now();
    let cpu = solve_cpu(&sys);
    let cpu_worklist = t.elapsed();
    // One pass in preorder.
    let t = std::time::Instant::now();
    let mut one = vec![NONE; n];
    for v in 0..n {
        let p = parent[v];
        if p != NONE {
            one[v] = if sets[0].contains(&kind[p as usize]) { p } else { one[p as usize] };
        }
    }
    let one_pass = t.elapsed();
    std::hint::black_box(&one);
    println!("NEAREST {n} nodes, max depth {depth}, {} copies; CPU worklist {:.2} ms, one preorder pass {:.3} ms", sys.copies.len(), cpu_worklist.as_secs_f64() * 1e3, one_pass.as_secs_f64() * 1e3);
    #[cfg(target_os = "macos")]
    use crate::code_rule_dsl::transducer::packed::metal_nearest;
    #[cfg(target_os = "macos")]
    for how in [metal_nearest::Buffers::Fresh, metal_nearest::Buffers::Blocks, metal_nearest::Buffers::Fresh, metal_nearest::Buffers::Blocks] {
        let target: Vec<u32> = parent.clone();
        let done: Vec<u8> = parent.iter().map(|&p| u8::from(p == NONE || sets[0].contains(&kind[p as usize]))).collect();
        for round in 0..4 {
            // `RERAIL_NEAREST_IDLE_MS`: the GPU left idle before each call, as between a run's chunks.
            if let Some(ms) = std::env::var("RERAIL_NEAREST_IDLE_MS").ok().and_then(|v| v.parse().ok()) {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            let t = std::time::Instant::now();
            let solved = metal_nearest::solve_with(n, &target, &done, how).unwrap();
            let p: Vec<String> = solved.phases.iter().map(|s| format!("{:.3}", s * 1e3)).collect();
            println!(
                "JUMP {how:?} round {round}: host {:.2} ms, GPU {:.3} ms, {} steps at most; phases ms [made, queue, encode, submit, schedule, GPU, wake] = [{}]; {} KB made",
                t.elapsed().as_secs_f64() * 1e3,
                solved.gpu * 1e3,
                solved.rounds,
                p.join(", "),
                solved.made >> 10
            );
            assert_eq!(solved.targets, one);
        }
    }
    #[cfg(target_os = "macos")]
    for round in 0..3 {
        use crate::code_rule_dsl::transducer::packed::metal_lattice;
        let device = metal_lattice::System { seeds: &sys.seeds, copies: &sys.copies, bins: &[] };
        let t = std::time::Instant::now();
        let solved = metal_lattice::solve(&device).unwrap();
        println!(
            "METAL round {round}: host {:.2} ms, GPU {:.2} ms, wait {:.2} ms, {} launches, {} visits, {} retries",
            t.elapsed().as_secs_f64() * 1e3,
            solved.gpu * 1e3,
            solved.wall * 1e3,
            solved.launches,
            solved.visits,
            solved.retries
        );
        assert_eq!(solved.values, cpu);
    }
}

/// A cross-file rule's file-local part is made per chunk: the rule reads one exported join, not its base relations.
#[test]
fn a_cross_file_rules_local_join_is_pushed_into_the_chunks() {
    use crate::code_rule_dsl::logic::rr::parse;
    let text = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl op(n: node, s: symbol)
.input doc, node, field, text, op
// Any file's names: no location column, so cross-file.
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl finding(n: node, s: symbol)
.output finding
finding(n, s) :- node(_, n, "call", _, _), field(n, "function", f), text(f, s), !op(n, "await"), s != "print", !defined(s).
"#;
    let mut prog = parse(text).unwrap();
    assert!(!split(&prog).contains("finding"), "finding negates a cross-file relation");
    let made = push_local_joins(&mut prog);
    assert_eq!(made.len(), 1, "{made:?}");
    let local = split(&prog);
    assert!(local.contains(&made[0]), "the made join is file-local");
    let finding: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == "finding").collect();
    assert_eq!(finding.len(), 1);
    // What is left of the rule: the join and the cross-file negation.
    let reads: Vec<&str> = finding[0].body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str()).collect();
    assert_eq!(reads, [made[0].as_str(), "defined"]);
    let split = programs(&prog);
    assert!(split.exports.contains(&made[0]));
    assert!(!split.base.iter().any(|b| b == "field" || b == "text" || b == "op"), "{:?}", split.base);
    // The join takes the comparison and the negated base read along.
    let join = prog.rules.iter().find(|r| r.head.rel == made[0]).unwrap();
    assert_eq!(join.body.len(), 5, "{join:?}");
    assert_eq!(join.head.args.len(), 2);
    // Its rows are attributable to their file: the node column is a location.
    let (_, locations, _) = split_located(&prog);
    assert_eq!(locations[&made[0]], [0], "{locations:?}");
}

/// Position comparisons must retain the semantic candidate filters. Document equality alone
/// does not make their Cartesian product a bounded per-file producer.
#[test]
fn cross_file_filters_keep_independent_node_positions_out_of_local_products() {
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl visit_index(n: node, i: number)
.input node, field, text, docof, visit_index
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl chosen(n: node)
chosen(n) :- node(_, n, "call", _, _), field(n, "function", f), text(f, s), !defined(s).
.decl triangle(a: node, b: node, c: node)
triangle(a, b, c) :- chosen(a), chosen(b), chosen(c),
    docof(a, d), docof(b, d), docof(c, d), visit_index(a, ai), visit_index(b, bi), visit_index(c, ci), ai < bi, bi < ci.
.output triangle
"#;
    let mut program = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    assert!(!split(&program).contains("chosen"));
    let original = program.rules.iter().find(|rule| rule.head.rel == "triangle").unwrap().clone();
    push_local_joins(&mut program);
    let actual = program.rules.iter().find(|rule| rule.head.rel == "triangle").unwrap();
    assert_eq!(actual, &original, "do not materialize all document-local node triples before chosen() filters them");
}

/// `v = v0` carries a location: a disjunction's branch (`(kind(v0, "as_pattern"), child(v0, v, 0) ; v = v0)`, as
/// lowering leaves it) keeps the head's node column a location, so its readers stay file-local. And a relation made
/// from constants alone (`side(x) :- table("k", x)`) is a constant to the split: reading it joins no document, and
/// both parts derive it.
#[test]
fn equalities_and_constant_relations_keep_rules_file_local() {
    use crate::code_rule_dsl::logic::rr::parse;
    let program = parse(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl table(k: symbol, x: symbol)
table("k", "a"). table("k", "b").
.decl side(x: symbol)
side(x) :- table("k", x).
.decl root(s: node, v: node)
root(s, v) :- node(_, s, "with", _, _), child(s, v0, 0), (node(_, v0, "as", _, _), child(v0, v, 0) ; !node(_, v0, "as", _, _), v = v0).
.decl under(s: node, n: node)
under(s, n) :- root(s, r), child(r, n, _).
.decl named(n: node)
named(n) :- node(_, n, "identifier", _, _), text(n, x), side(x).
.output under, named
"#).unwrap();
    let (local, locations, _) = split_located(&program);
    assert_eq!(locations["root"], [0, 1], "{locations:?}");
    assert!(local.contains("under") && local.contains("named"), "{local:?}");
    assert!(constant_relations(&program).contains("side"));
    let parts = programs(&program);
    assert!(parts.local.rules.iter().any(|r| r.head.rel == "side"), "the chunks derive side for named");
    assert!(cross_causes(&program).get("under").is_none(), "{:?}", cross_causes(&program));
}

#[test]
fn the_cross_part_takes_node_attributes_as_a_semi_join_only_when_each_lookup_is_of_a_held_node() {
    use crate::code_rule_dsl::logic::rr::parse;
    let source = |extra: &str| format!(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl line(n: loc, l: number)
.input node, child, text, line
.decl names(x: symbol)
names(x) :- node(_, n, "string", _, _), text(n, x).
.decl ident(n: node, x: symbol)
ident(n, x) :- node(_, n, "identifier", _, _), text(n, x).
.decl pick(n: node)
pick(n) :- ident(n, x), names(x).
.decl hit(n: node, l: number)
hit(n, l) :- pick(n), line(n, l).
{extra}
.output hit
"#);
    // As the corpus run splits: after the file-local joins are pushed into the chunks.
    let split = |text: &str| {
        let mut program = parse(text).unwrap();
        push_local_joins(&mut program);
        programs(&program)
    };
    let parts = split(&source(""));
    assert!(parts.base.iter().any(|b| b == "line"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.cross, &parts.base).expect("every lookup is of a node an export holds");
    assert!(held.contains("line"), "{held:?}");
    // `child(p, c, _)` reaches nodes no export holds: their lines would be missing. So would a cross-file rule's
    // own enumeration of nodes (`node(_, n, "identifier", _, _), text(n, x), names(x)`).
    let parts = split(&source(".decl kid(c: node, l: number)\nkid(c, l) :- pick(p), child(p, c, _), line(c, l).\n.output kid"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    assert!(!semijoin_bases(&parts.cross, &parts.base).unwrap().contains("line"));
    let parts = programs(&parse(&source(".decl own(n: node)\nown(n) :- node(_, n, \"identifier\", _, _), text(n, x), names(x).\n.decl seen(n: node, l: number)\nseen(n, l) :- own(n), line(n, l).\n.output seen")).unwrap());
    let held = semijoin_bases(&parts.cross, &parts.base).unwrap();
    assert!(!held.contains("line") && !held.contains("node") && !held.contains("text"), "{held:?}");
    // Per relation: a walk of `child` that looks up no attribute leaves `line` a semi-join for the other rules.
    let parts = split(&source(".decl kid(c: node, i: number)\nkid(c, i) :- pick(p), child(p, c, i).\n.decl kidline(p: node, l: number)\nkidline(p, l) :- kid(c, _), pick(p), child(p, c, _), line(p, l).\n.output kid, kidline"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.cross, &parts.base).unwrap();
    assert!(held.contains("line"), "{held:?}");
    // A tainted output's findings are decoded through `node`: kept whole.
    assert!(!held.contains("node"), "{held:?}");
    // `tape(n, par, ..)`'s parent is a node though a number: a walk up it reaches nodes no export holds.
    let parts = split(&source(".decl tape(n: node, par: number, size: number, depth: number)\n.input tape\n.decl up(n: node, p: number)\nup(n, p) :- hit(n, _), names(_), tape(n, p, _, _).\nup(n, q) :- up(n, p), names(_), tape(p, q, _, _).\n.output up"));
    if parts.base.iter().any(|b| b == "tape") {
        assert!(!semijoin_bases(&parts.cross, &parts.base).unwrap().contains("tape"));
    }
}

/// `boot-python.rr`'s `deco_word`: a disjunction's branch `(e0 = d ; …)` leaves `e0 = d` between two per-file pieces.
/// The equality names one value, so the pieces are one join, pushed into the chunks whole; apart, the split pushed each
/// and left their join to the cross-file part (decision 0093 §4, O140).
#[test]
fn an_equality_between_two_pieces_joins_them_in_the_chunks() {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use crate::code_rule_dsl::logic::rr::parse;
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), field(n, "name", c), text(c, s).
.decl finding(f: node, x: symbol)
.output finding
finding(f, x) :- node(_, f, "decorated_definition", _, _), field(f, "decorator", d), e = d, field(e, "name", c), text(c, x), !defined(x).
"#;
    let mut prog = parse(text).unwrap();
    let made = push_local_joins(&mut prog);
    let local = split(&prog);
    assert!(made.iter().all(|m| local.contains(m)), "{made:?}");
    let finding: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == "finding").collect();
    assert_eq!(finding.len(), 1);
    let body = &finding[0].body;
    let equal = body.iter().any(|l| matches!(l, Literal::Cmp { l: Term::Var(_), op: "=", r: Term::Var(_) }));
    assert!(!equal, "the equality stays in the cross-file rule: {}", finding[0]);
    let pushed: Vec<&str> = body.iter().filter_map(|l| l.atom()).filter(|a| made.iter().any(|x| x == &a.rel)).map(|a| a.rel.as_str()).collect();
    assert_eq!(pushed.len(), 1, "one pushed join, not one per piece: {}", finding[0]);
}

/// Decision 0099, the factorized store: an export that joins two parts at a variable its head projects away
/// (`outer`'s pushed join `inside(f, p), bound(p, x)`) is handed on as its parts, each reduced by the other's keys,
/// and the cross-file reader joins them by the key where it read the product. One whose second part is a node's
/// attribute (`text(p, s)`: one row a key) is left whole: its product is no larger than its other part.
#[test]
#[ignore = "t31 A1 keeps outer's join in the cross-file part (uses, a cross-file read of the same rule, filters it), so this program has no export for decision 0099 to factorize; open: a case 0099 still factorizes"]
fn an_export_joined_at_a_projected_key_is_handed_on_as_its_parts() {
    use crate::code_rule_dsl::logic::front::Literal;
    use crate::code_rule_dsl::logic::rr::parse;
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
// Any file's names: no location column, so cross-file.
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- node(_, p, "block", _, _), child(p, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- node(_, p, "block", _, _), child(p, c, _), node(_, c, "assignment", _, _), text(c, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- uses(f, x), encl(f, p), bound(p, x).
.decl named(f: node, s: symbol)
.output named
named(f, s) :- uses(f, _), child(p, f, _), text(p, s).
"#;
    let mut prog = parse(text).unwrap();
    let pushed = push_local_joins(&mut prog);
    assert_eq!(pushed.len(), 2, "{pushed:?}");
    let lines = factorize_exports(&mut prog);
    assert_eq!(lines.len(), 1, "only outer's join is factorized: {lines:?}");
    let product = pushed.iter().find(|p| p.ends_with("_outer")).unwrap();
    assert!(lines[0].starts_with(&format!("{product}: handed on as parts")), "{lines:?}");
    assert!(!prog.rules.iter().any(|r| r.head.rel == *product), "the product's rule is gone");
    // The reader reads the parts, joined by the key, where it read the product.
    let outer: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == "outer").collect();
    assert_eq!(outer.len(), 1);
    let parts: Vec<&str> = outer[0].body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str()).filter(|r| r.starts_with("fz")).collect();
    assert_eq!(parts.len(), 2, "{}", outer[0]);
    let keys: Vec<_> = outer[0].body.iter().filter_map(|l| l.atom()).filter(|a| a.rel.starts_with("fz")).map(|a| a.args.last().cloned().unwrap()).collect();
    assert_eq!(keys[0], keys[1], "the parts meet at the key: {}", outer[0]);
    // Each part is reduced by the other's keys, and both are file-local exports.
    for part in &parts {
        let rule = prog.rules.iter().find(|r| r.head.rel == *part).unwrap();
        assert!(rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.starts_with("fzk"))), "{rule}");
    }
    let split = programs(&prog);
    for part in &parts {
        assert!(split.exports.iter().any(|e| e == part), "{part} is exported: {:?}", split.exports);
    }
    assert!(!split.exports.iter().any(|e| e == product));
    // The other join (`named`'s) is handed on whole.
    let named = pushed.iter().find(|p| p.ends_with("_named")).unwrap();
    assert!(split.exports.iter().any(|e| e == named), "{:?}", split.exports);
}

/// A composition that a cross-file read of the same rule filters stays in the cross-file part (t31 A1): pushed, the
/// chunks would export every (function, name bound around it) pair, not the few the filter asks about.
#[test]
fn a_composition_filtered_by_a_cross_file_read_is_not_pushed() {
    use crate::code_rule_dsl::logic::rr::parse;
    let text = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input doc, node, field, text
// Any file's names: no location column, so cross-file; and the pairs asked about, cross-file through it.
.decl named(x: symbol)
named(x) :- node(_, n, "call", _, _), field(n, "function", c), text(c, x).
.decl wanted(f: node, x: symbol)
wanted(f, x) :- named(x), node(_, f, "function_definition", _, _).
.decl bound_in(p: node, x: symbol)
bound_in(p, x) :- field(p, "name", c), text(c, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- wanted(f, x), field(p, "body", f), bound_in(p, x).
"#;
    let mut prog = parse(text).unwrap();
    let made = push_local_joins(&mut prog);
    assert!(!made.iter().any(|m| m.ends_with("_outer")), "{made:?}");
    let outer: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == "outer").collect();
    let reads: Vec<&str> = outer[0].body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str()).collect();
    assert_eq!(reads, ["wanted", "field", "bound_in"]);
}
