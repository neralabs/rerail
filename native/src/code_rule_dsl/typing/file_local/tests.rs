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

/// A relation whose rows name no document (`aware()`, as data-layer's `ma_branch_aware`), handed on to a cross-file
/// reader (`found`, as `MA033` over `ma_owner(_)`), is moved to the cross-file part with its file-local readers
/// (decision 0290), so `locate` leaves every row the chunks hand on located.
#[test]
fn a_relation_naming_no_document_moves_to_the_cross_file_part() {
    use crate::code_rule_dsl::logic::rr::parse;
    let text = ".decl doc(d: number, p: symbol, x: number)\n.input doc\n.decl owner(d: number)\nowner(d) :- doc(d, \"a.py\", _).\n\
                .decl aware()\naware() :- owner(d), d > 0.\n.decl found(k: number)\nfound(1) :- owner(_), !aware().\n.output found\n";
    let mut program = parse(text).unwrap();
    assert!(split(&program).contains("aware"), "the case: aware is file-local before the move");
    let located = locate(&mut program).expect("every row the chunks hand on located");
    assert_eq!(moved(&program), ["aware"]);
    let parts = split(&program);
    assert!(!parts.contains("aware") && !parts.contains("found") && parts.contains("owner"), "{parts:?}, located {located:?}");
    assert_eq!(cross_causes(&program)["aware"], "its rows name no document (moved by `locate`)");
    // Located again, nothing more moves.
    locate(&mut program).unwrap();
    assert_eq!(moved(&program), ["aware"]);
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
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] // the Metal rounds check against it
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
    let split = programs(&prog).unwrap();
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
    let parts = programs(&program).unwrap();
    let lv = &parts.versions.local;
    assert!(lv.rules().any(|r| lv.rel_name(r.head().rel()) == "side"), "the chunks derive side for named");
    assert!(!cross_causes(&program).contains_key("under"), "{:?}", cross_causes(&program));
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
        programs(&program).unwrap()
    };
    let parts = split(&source(""));
    assert!(parts.base.iter().any(|b| b == "line"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.versions.cross, &parts.base).expect("every lookup is of a node an export holds");
    assert!(held.contains("line"), "{held:?}");
    // `child(p, c, _)` reaches nodes no export holds: their lines would be missing. So would a cross-file rule's
    // own enumeration of nodes (`node(_, n, "identifier", _, _), text(n, x), names(x)`).
    let parts = split(&source(".decl kid(c: node, l: number)\nkid(c, l) :- pick(p), child(p, c, _), line(c, l).\n.output kid"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    assert!(!semijoin_bases(&parts.versions.cross, &parts.base).unwrap().contains("line"));
    let parts = programs(&parse(&source(".decl own(n: node)\nown(n) :- node(_, n, \"identifier\", _, _), text(n, x), names(x).\n.decl seen(n: node, l: number)\nseen(n, l) :- own(n), line(n, l).\n.output seen")).unwrap()).unwrap();
    let held = semijoin_bases(&parts.versions.cross, &parts.base).unwrap();
    assert!(!held.contains("line") && !held.contains("node") && !held.contains("text"), "{held:?}");
    // Per relation: a walk of `child` that looks up no attribute leaves `line` a semi-join for the other rules.
    let parts = split(&source(".decl kid(c: node, i: number)\nkid(c, i) :- pick(p), child(p, c, i).\n.decl kidline(p: node, l: number)\nkidline(p, l) :- kid(c, _), pick(p), child(p, c, _), line(p, l).\n.output kid, kidline"));
    assert!(parts.base.iter().any(|b| b == "child"), "{:?}", parts.base);
    let held = semijoin_bases(&parts.versions.cross, &parts.base).unwrap();
    assert!(held.contains("line"), "{held:?}");
    // A tainted output's findings are decoded through `node`: kept whole.
    assert!(!held.contains("node"), "{held:?}");
    // `tape(n, par, ..)`'s parent is a node though a number: a walk up it reaches nodes no export holds.
    let parts = split(&source(".decl tape(n: node, par: number, size: number, depth: number)\n.input tape\n.decl up(n: node, p: number)\nup(n, p) :- hit(n, _), names(_), tape(n, p, _, _).\nup(n, q) :- up(n, p), names(_), tape(p, q, _, _).\n.output up"));
    if parts.base.iter().any(|b| b == "tape") {
        assert!(!semijoin_bases(&parts.versions.cross, &parts.base).unwrap().contains("tape"));
    }
}

/// A climb up the tree (`inside`, `has`, a nearest over a derived set: lower.rs `climb`) reads the tape's parent
/// column, which `need_tape` declares over numbers. A node's parent is in its document, so the climb is file-local:
/// cross-file, every climb of a pack joined the corpus on one worker (O-cold-seed-join).
#[test]
fn a_climb_over_the_tapes_parent_column_is_file_local() {
    use crate::code_rule_dsl::logic::rr::parse;
    let text = ".decl kind(n: node, k: number)\n.input kind\n.decl tape(n: int, par: int, size: int, depth: int)\n.input tape\n\
        .decl stop(n: node)\nstop(n) :- kind(n, 7).\n\
        .decl up(n: node, a: int)\nup(n, p) :- kind(n, 3), tape(n, p, _, _), p >= 0.\nup(n, q) :- up(n, p), !stop(p), tape(p, q, _, _), q >= 0.\n\
        .decl under(n: node)\nunder(n) :- up(n, a), stop(a).\n.output under\n";
    let program = parse(text).unwrap();
    let (local, locations, _) = split_located(&program);
    assert!(local.contains("up") && local.contains("under"), "{local:?}");
    assert!(locations["up"].contains(&0), "{locations:?}");
}

/// `boot-python.rerail`'s `deco_word`: a disjunction's branch `(e0 = d ; …)` leaves `e0 = d` between two per-file pieces.
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
    let split = programs(&prog).unwrap();
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

/// The split read from the version (`FileLocal`) is maintained across an edit of a few rules: computed again only
/// for the relations the edit changed and those reading them, not split whole, and the same as a whole split (a debug
/// build checks each carried analysis against a fresh one). Dropping the rule of `q` that joins two documents makes
/// `q`, and `reads_q` with it, file-local.
#[test]
fn the_split_of_a_version_is_maintained_across_an_edit() {
    use crate::code_rule_dsl::logic::program_image::manager::{Cx, Pipeline, ProgramPass, ProgramW};
    let mut text = String::from(
        ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n\
         .decl q(n: node)\nq(n) :- node(_, n, \"a\", _, _).\nq(n) :- node(_, n, \"a\", _, _), node(_, m, \"b\", _, _), m != n.\n\
         .decl reads_q(n: node)\nreads_q(n) :- q(n).\n.output reads_q\n",
    );
    // Relations the edit does not reach: the cone stays under half of them.
    for k in 0..6 {
        text.push_str(&format!(".decl other{k}(n: node)\nother{k}(n) :- node(_, n, \"k{k}\", _, _).\n.output other{k}\n"));
    }
    let program = crate::code_rule_dsl::logic::rr::parse(&text).unwrap();
    let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&program).unwrap().finish().unwrap();
    let mut p = Pipeline::new("test", v.clone());
    let before = p.get::<FileLocal>().unwrap();
    assert!(!before.local.contains("q") && !before.local.contains("reads_q") && before.local.contains("other0"), "{:?}", before.local);
    struct DropJoin;
    impl ProgramPass for DropJoin {
        const NAME: &'static str = "drop the join";
        fn run(&mut self, _: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
            w.keep(|r| r.body().count() < 3)
        }
    }
    p.program(&mut DropJoin);
    let after = p.get::<FileLocal>().unwrap();
    assert!(after.local.contains("q") && after.local.contains("reads_q"), "{:?}", after.local);
    assert_eq!(p.store().computed("file-local split"), 0, "maintained from the input's split, not computed again");
    let (out, _) = p.finish().unwrap();
    let whole = crate::code_rule_dsl::logic::program_image::manager::analysis::<FileLocal>(&crate::code_rule_dsl::logic::program_image::Builder::of_program(&out.program(&Default::default()).unwrap()).unwrap().finish().unwrap().0).unwrap();
    assert_eq!(*after, *whole, "the maintained split is a whole split's");
}

/// Rows located on the version (`Locate`) as the tree's `locate` locates them: each export with no location column
/// derived as `<r>_at(d, …)`, its document a column a rule binds (`named2`), one it leaves unnamed (`named`), or one
/// through a node it binds (`named3`, by `node`; not a lone input atom, which the cross-file part derives itself), and
/// read back by the cross-file part. The same program.
#[test]
fn rows_are_located_on_a_version_as_on_the_tree() {
    use crate::code_rule_dsl::logic::program_image::{Builder, manager::Pipeline};
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n.input node, text\n\
                .decl named(s: symbol)\nnamed(s) :- node(_, n, \"function\", _, _), text(n, s).\n\
                .decl named2(s: symbol)\nnamed2(s) :- node(d, n, \"class\", _, _), text(n, s), d > 0.\n\
                .decl named3(s: symbol)\nnamed3(s) :- text(n, s), s != \"x\".\n\
                .decl all(s: symbol)\nall(s) :- named(s), named2(s), named3(s).\n.output all\n";
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let mut pass = Locate::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    let want = locate(&mut tree);
    assert_eq!(want, Ok(vec!["named".to_owned(), "named2".to_owned(), "named3".to_owned()]));
    assert_eq!(pass.result(), want);
    let (tv, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    assert_eq!((out.size(), out.digest()), (tv.size(), tv.digest()), "{}\n---\n{}", out.show(), tv.show());
}

/// A relation whose rows name no document is moved to the cross-file part (`CROSS_FILE`, decision 0290) and the
/// program located again, on the version the preparation runs (`preparation::run`: `Locate` rounds and
/// `MoveCrossFile`) as on the tree (`locate`): the same relations located. With one `Locate` round, as the
/// preparation once ran it, `aware` was left and reported blocked, and a debug build's check that the version's rows
/// are located as the tree's failed (`corpus.rs`).
#[test]
fn a_relation_naming_no_document_moves_cross_file_on_the_version_as_on_the_tree() {
    use crate::code_rule_dsl::logic::program_image::Builder;
    // `a_relation_naming_no_document_moves_to_the_cross_file_part`'s program: `aware` binds no document.
    let text = ".decl doc(d: number, p: symbol, x: number)\n.input doc\n.decl owner(d: number)\nowner(d) :- doc(d, \"a.py\", _).\n\
                .decl aware()\naware() :- owner(d), d > 0.\n.decl found(k: number)\nfound(1) :- owner(_), !aware().\n.output found\n";
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let (_, found) = crate::code_rule_dsl::logic::plan::corpus::preparation::run(v, false).unwrap();
    let want = locate(&mut tree);
    assert_eq!(moved(&tree), ["aware"], "the case: the tree moves aware");
    assert!(want.is_ok(), "{want:?}");
    assert_eq!(found.located, want);
}

/// A located rule keeps its branded atoms (decision 0270, a lowered program's atoms): `named`'s `node(_, n,
/// "function", _, _)`, its kind branded as lowering brands it, is the atom whose unnamed document column the located
/// rule names, its other terms copied (`passes::t_of`): the kind stays the same branded atom. The same program as the
/// tree's `locate`.
#[test]
fn a_located_rule_keeps_its_branded_atoms() {
    use crate::code_rule_dsl::logic::front::{Branded, Class, Literal, Term};
    use crate::code_rule_dsl::logic::program_image::{Builder, manager::Pipeline};
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n.input node, text\n\
                .decl named(s: symbol)\nnamed(s) :- node(_, n, \"function\", _, _), text(n, s).\n\
                .decl all(s: symbol)\nall(s) :- named(s).\n.output all\n";
    let kind = Branded { class: Class::Kind, atom: 7 };
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    for r in &mut tree.rules {
        for l in &mut r.body {
            if let Literal::Atom(a) = l {
                a.args.iter_mut().filter(|t| matches!(t, Term::Str(_))).for_each(|t| *t = Term::Branded(kind));
            }
        }
    }
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    assert!(!v.show().contains("\"function\""), "branded: {}", v.show());
    let mut pass = Locate::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    let want = locate(&mut tree);
    assert_eq!(want, Ok(vec!["named".to_owned()]));
    assert_eq!(pass.result(), want);
    let keeps = |l: &Literal| matches!(l, Literal::Atom(a) if a.rel.as_str() == "node" && a.args.contains(&Term::Branded(kind)));
    assert!(tree.rules.iter().any(|r| r.head.rel.as_str() == "named_at" && r.body.iter().any(keeps)), "the tree's located rule keeps it");
    let (tv, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    assert_eq!((out.size(), out.digest()), (tv.size(), tv.digest()), "{}\n---\n{}", out.show(), tv.show());
}

/// An export read by a cross-file rule with a demand is cut on a version as the tree cuts it: the same lines, the
/// same program (`xd_fl_later` through `dm_demand_0` and the key `xk_text_0`).
#[test]
fn exports_are_cut_on_a_version_as_on_the_tree() {
    use crate::code_rule_dsl::logic::program_image::{Builder, manager::Pipeline};
    let text = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.decl text(n: node, s: symbol)\n\
                .decl within(c: node, n: node)\n.input node, text, within\n\
                .decl fl_later(n: node, c: node)\nfl_later(n, c) :- within(c, n), node(_, c, \"call\", _, _).\n\
                .decl named(n: node, s: symbol)\nnamed(n, s) :- text(n, s).\n\
                .decl demand(n: node, m: node)\ndemand(n, m) :- named(n, s), named(m, s), node(_, n, \"function\", _, _), n != m.\n\
                .decl later(n: node, c: node)\nlater(n, c) :- demand(n, _), fl_later(n, c), text(c, _).\n\
                later(n, c) :- demand(n, _), fl_later(n, c), node(_, c, \"call\", 1, _).\n.output later\n";
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let mut pass = DemandExports::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    let want = demand_exports(&mut tree);
    assert!(want[0].starts_with("1 exports cut"), "{want:?}");
    assert_eq!(pass.lines, want);
    let (tv, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let shown = out.show();
    for made in ["xd_fl_later(", "dm_demand_0(", "xk_text_0("] {
        assert!(shown.contains(made), "{made} in\n{shown}");
    }
    assert_eq!((out.size(), out.digest()), (tv.size(), tv.digest()), "{}\n---\n{}", out.show(), tv.show());
}

/// An export joining two parts at a key its head projects away is handed on as its parts on a version as the tree
/// hands it on: the same lines, the same program (the parts, their keys, the readers' copies and fresh names).
#[test]
fn exports_are_factorized_on_a_version_as_on_the_tree() {
    use crate::code_rule_dsl::logic::program_image::{Builder, manager::Pipeline};
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- node(_, p, "block", _, _), child(p, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- node(_, p, "block", _, _), child(p, c, _), node(_, c, "assignment", _, _), text(c, x).
.decl fl_outer(f: node, x: symbol)
fl_outer(f, x) :- encl(f, p), bound(p, x).
.decl outer(f: node, x: symbol)
.output outer
outer(f, x) :- uses(f, x), fl_outer(f, x).
.decl outer2(f: node)
.output outer2
outer2(f) :- uses(f, _), fl_outer(f, y), y != "self".
"#;
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let mut pass = FactorizeExports::new(true);
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    let want = factorize_exports(&mut tree);
    assert_eq!(want.len(), 1, "{want:?}");
    assert!(want[0].starts_with("fl_outer: handed on as parts"), "{want:?}");
    assert_eq!(pass.lines, want);
    let (tv, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    assert_eq!((out.size(), out.digest()), (tv.size(), tv.digest()), "{}\n---\n{}", out.show(), tv.show());
    assert!(!out.rules().any(|r| out.rel_name(r.head().rel()) == "fl_outer"), "the product is gone:\n{}", out.show());
    // Each part and its key are named from the part's content: `fz<h>_` and `fzk<h>_`, one hash.
    let shown = out.show();
    let parts: std::collections::BTreeSet<&str> = shown.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| w.starts_with("fz") && w.ends_with("_fl_outer")).collect();
    assert_eq!(parts.len(), 4, "{parts:?}");
    for p in parts.iter().filter(|p| !p.starts_with("fzk")) {
        assert!(p["fz".len()..p.len() - "_fl_outer".len()].bytes().all(|b| b.is_ascii_digit()), "{p}");
        assert!(parts.contains(format!("fzk{}", &p["fz".len()..]).as_str()), "{parts:?}");
    }
}

/// The file-local joins of cross-file rules are pushed on a version as the tree pushes them: the same helpers
/// (`fl<h>_<head>`, named from their content), the same program.
#[test]
fn local_joins_are_pushed_on_a_version_as_on_the_tree() {
    use crate::code_rule_dsl::logic::program_image::{Builder, manager::Pipeline};
    let text = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl line(n: node, l: number)
.input node, child, text, docof, line
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
named(f, s) :- uses(f, _), child(p, f, _), text(p, s), !bound(p, s).
.decl at(f: node, l: number)
.output at
at(f, l) :- uses(f, _), docof(f, d), line(f, l), d > 0.
"#;
    let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
    let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    let mut pass = PushLocalJoins::default();
    let mut p = Pipeline::new("test", v);
    p.program(&mut pass);
    let (out, _) = p.finish().unwrap();
    let want = push_local_joins(&mut tree);
    assert!(!want.is_empty(), "{want:?}");
    assert_eq!(pass.made, want);
    let (tv, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
    assert_eq!((out.size(), out.digest()), (tv.size(), tv.digest()), "{}\n---\n{}", out.show(), tv.show());
}

/// A helper is named from its content: an edit of one rule renames its own helper, and no other rule's (a counter
/// renumbered every helper after the edited rule's).
#[test]
fn an_edit_renames_only_its_own_rules_helpers() {
    let program = |c: i64| {
        format!(
            r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl defined(s: symbol)
defined(s) :- node(_, n, "function_definition", _, _), text(n, s).
.decl uses(f: node, x: symbol)
uses(f, x) :- node(_, f, "identifier", _, _), text(f, x), defined(x).
.decl first(f: node, s: symbol)
.output first
first(f, s) :- uses(f, _), child(p, f, {c}), text(p, s).
.decl named(f: node, s: symbol)
.output named
named(f, s) :- uses(f, _), child(p, f, _), node(_, p, "call", _, _), text(p, s).
"#
        )
    };
    let made = |c: i64| push_local_joins(&mut crate::code_rule_dsl::logic::rr::parse(&program(c)).unwrap());
    let (before, after) = (made(1), made(2));
    let of = |m: &[String], head: &str| m.iter().find(|r| r.ends_with(&format!("_{head}"))).cloned().unwrap_or_else(|| panic!("{head} in {m:?}"));
    assert_ne!(of(&before, "first"), of(&after, "first"), "the edited rule's helper is renamed");
    assert_eq!(of(&before, "named"), of(&after, "named"), "another rule's is not");
}

/// The declarations and inputs the invariants' tests share (decisions 0649, 0652, 0677): nodes with their documents,
/// kinds and lines.
const INVARIANT_INPUTS: &str = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n\
                                .decl line(n: node, l: number)\n.input line\n";

/// The split of `text` (after `INVARIANT_INPUTS`), as a run makes it (`programs_of`, with `locate`'s moves).
fn invariant_split(text: &str) -> Programs {
    let mut program = crate::code_rule_dsl::logic::rr::parse(&format!("{INVARIANT_INPUTS}{text}")).unwrap();
    locate(&mut program).unwrap();
    let (v, _) = crate::code_rule_dsl::logic::program_image::Builder::of_program(&program).unwrap().finish().unwrap();
    programs_of(v).unwrap()
}

/// `.decl r(…) cross_file` is read, kept by the program, and printed back (decision 0652): a printed program that lost
/// it would split differently, read again. A relation `locate` moved prints without it: the move is not source.
#[test]
fn a_cross_file_declaration_is_read_and_printed_back() {
    use crate::code_rule_dsl::logic::rr::{parse, print};
    let text = format!("{INVARIANT_INPUTS}.decl kinds(k: symbol) cross_file\nkinds(k) :- node(d, _, k, _, _), node(e, _, k, _, _), d != e.\n.output kinds\n");
    let program = parse(&text).unwrap();
    assert!(program.qualified("kinds", CROSS_FILE) && !program.qualified("kinds", MOVED_CROSS_FILE));
    let printed = print(&program).unwrap();
    assert!(printed.contains(".decl kinds(k: symbol) cross_file"), "{printed}");
    assert!(parse(&printed).unwrap().qualified("kinds", CROSS_FILE));
    let mut moved_program = parse(&format!(
        "{INVARIANT_INPUTS}.decl owner(d: number)\nowner(d) :- node(d, _, \"a\", _, _).\n.decl aware()\naware() :- owner(d), d > 0.\n\
         .decl found(k: number)\nfound(1) :- owner(_), !aware().\n.output found\n"
    ))
    .unwrap();
    locate(&mut moved_program).unwrap();
    assert_eq!(moved(&moved_program), ["aware"]);
    assert!(!print(&moved_program).unwrap().contains("cross_file"), "a move is not printed as a declaration");
}

/// A rule joining two documents' rows makes an undeclared crossing (decision 0652), refused under strict; declared
/// `cross_file`, it is meant, and a relation reading it follows it and is no crossing of its own.
#[test]
fn an_undeclared_join_of_two_documents_is_refused() {
    let join = "kinds(k) :- node(d, _, k, _, _), node(e, _, k, _, _), d != e.\n.decl found(k: symbol)\nfound(k) :- kinds(k), k != \"x\".\n.output found\n";
    let undeclared = invariant_split(&format!(".decl kinds(k: symbol)\n{join}"));
    assert_eq!(undeclared.crossings.keys().collect::<Vec<_>>(), ["kinds"], "{:?}", undeclared.crossings);
    let refused = undeclared_crossings(&undeclared).unwrap();
    assert_eq!(refused.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>(), ["kinds"]);
    let err = crossings_checked(&undeclared, true).unwrap_err();
    assert!(err.contains("kinds") && err.contains("0652"), "{err}");
    assert!(crossings_checked(&undeclared, false).is_ok(), "reported, not refused, outside strict");
    let declared = invariant_split(&format!(".decl kinds(k: symbol) cross_file\n{join}"));
    assert!(declared.crossings.is_empty(), "{:?}", declared.crossings);
    assert!(!declared.file_local.contains("kinds") && !declared.file_local.contains("found"));
    crossings_checked(&declared, true).unwrap();
}

/// One document's rows read plainly by a cross-file rule are a union of each file's, not a join (decision 0652).
#[test]
fn a_plain_read_of_one_document_is_no_crossing() {
    let split = invariant_split(
        ".decl kinds(k: symbol) cross_file\nkinds(k) :- node(d, _, k, _, _), node(e, _, k, _, _), d != e.\n\
         .decl named(n: node, k: symbol)\nnamed(n, k) :- node(_, n, k, _, _).\n\
         .decl found(n: node)\nfound(n) :- named(n, k), kinds(k).\n.output found\n",
    );
    assert!(undeclared_crossings(&split).unwrap().is_empty(), "{:?}", split.crossings);
}

/// A relation of the cross-file part that holds a node is named (decision 0677), with where the node entered; keyed by
/// a symbol, with the file's own rules relating its nodes to it, it is not. An output holding one is a finding's place.
#[test]
fn a_node_in_the_cross_file_part_is_named() {
    let by_node = invariant_split(
        ".decl pairs(a: node, b: node) cross_file\npairs(a, b) :- node(d, a, k, _, _), node(e, b, k, _, _), d != e.\n\
         .decl found(n: node)\nfound(n) :- pairs(n, _).\n.output found\n",
    );
    let named: Vec<String> = by_node.nodes.iter().map(|c| c.named()).collect();
    assert_eq!(named, ["pairs"], "{:?}", by_node.nodes);
    assert!(by_node.nodes[0].why.contains("pairs.a"), "{}", by_node.nodes[0].why);
    assert!(node_crossings::nodes_checked(&by_node.nodes, true).is_err());
    let by_symbol = invariant_split(
        ".decl kinds(k: symbol) cross_file\nkinds(k) :- node(d, _, k, _, _), node(e, _, k, _, _), d != e.\n\
         .decl found(n: node)\nfound(n) :- node(_, n, k, _, _), kinds(k).\n.output found\n",
    );
    assert!(by_symbol.nodes.is_empty(), "{:?}", by_symbol.nodes);
}

/// A value computed from a node is a node's (`@cat`, `+`), but a node's document, text or digest is not (decision
/// 0677's `VALUES_OF_A_NODE`): a column filled from one holds a node by inference, to a fixpoint over the rules.
#[test]
fn node_columns_are_inferred_from_what_fills_them() {
    use crate::code_rule_dsl::logic::program_image::Builder;
    let program = crate::code_rule_dsl::logic::rr::parse(&format!(
        "{INVARIANT_INPUTS}.decl keyed(x: number)\nkeyed(x) :- node(_, n, _, _, _), x = n + 1.\n\
         .decl again(x: number)\nagain(x) :- keyed(x).\n.decl docs(d: number)\ndocs(d) :- node(d, _, _, _, _).\n\
         .output again, docs\n"
    ))
    .unwrap();
    let (v, _) = Builder::of_program(&program).unwrap().finish().unwrap();
    let placed = crate::code_rule_dsl::logic::plan::body::Placed::whole(v.clone());
    let f = placed.fields().unwrap();
    let nodes = node_crossings::NodeColumns::of(&v, &f);
    assert!(nodes.holds("node", 1) && nodes.holds("keyed", 0) && nodes.holds("again", 0));
    assert!(!nodes.holds("docs", 0) && !nodes.holds("node", 0));
}

/// The cross-file part's reads of a position (decision 0649) are found, but for an output's (a finding's place).
#[test]
fn a_position_read_in_the_cross_file_part_is_found() {
    let split = invariant_split(
        ".decl kinds(k: symbol, l: number) cross_file\nkinds(k, l) :- node(d, n, k, _, _), node(e, _, k, _, _), d != e, line(n, l).\n\
         .decl found(n: node)\nfound(n) :- node(_, n, k, _, _), kinds(k, _), line(n, _).\n.output found\n",
    );
    assert_eq!(split.positions, [("kinds".to_owned(), "line".to_owned())], "{:?}", split.positions);
    assert!(positions_checked(&split.positions, true).is_err());
}
