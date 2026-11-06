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
        let (_, locations) = split_located(&program);
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
        let prog = parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        let local = split(&prog);
        let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
        let global: Vec<&&str> = derived.iter().filter(|r| !local.contains(**r)).collect();
        let mut reads_base = std::collections::BTreeSet::new();
        let mut reads_local = std::collections::BTreeSet::new();
        for rule in prog.rules.iter().filter(|r| !local.contains(&r.head.rel)) {
            for l in &rule.body {
                if let Some(a) = l.atom() {
                    if prog.inputs.contains(&a.rel) { reads_base.insert(a.rel.clone()); }
                    if local.contains(&a.rel) { reads_local.insert(a.rel.clone()); }
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
    let (_, locations) = split_located(&prog);
    assert_eq!(locations[&made[0]], [0], "{locations:?}");
}
