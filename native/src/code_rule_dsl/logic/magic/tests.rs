use super::*;
use crate::code_rule_dsl::logic::front::prune;
use crate::code_rule_dsl::logic::rr::parse;

fn root() -> std::path::PathBuf {
    rerail_store::source_tree::native().join("src/code_rule_dsl/logic")
}

fn strings(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| serde_json::to_string(s).unwrap()).collect::<Vec<_>>().join(", "))
}

/// The program as the goldens print it (T's repr, and `json.dumps` with sorted keys for gates and the report).
fn golden_text(p: &Program) -> String {
    let mut lines = Vec::new();
    for (r, cols) in &p.decls {
        if r.starts_with("magic_") || r.starts_with("done_") {
            lines.push(format!("decl {r}({})", cols.iter().map(|(c, t)| format!("{c}: {t}")).collect::<Vec<_>>().join(", ")));
        }
    }
    for r in &p.rules {
        lines.push(format!("rule {r}"));
    }
    let mut gates = p.gates.clone();
    gates.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, g) in gates {
        let src = g.src.as_ref().map_or_else(|| "null".to_owned(), |s| serde_json::to_string(s).unwrap());
        lines.push(format!(
            "gate {name} {{\"deps\": {}, \"level\": {}, \"quiet\": {}, \"rel\": {}, \"src\": {src}}}",
            strings(&g.deps),
            g.level,
            strings(&g.quiet),
            serde_json::to_string(&g.rel).unwrap()
        ));
    }
    if let Some(r) = &p.magic_report {
        lines.push(format!(
            "report {{\"adornments\": {}, \"extras kept\": {}, \"full\": {}, \"full for negation\": {}, \"gates\": {}, \"magic rules\": {}}}",
            strings(&r.adornments),
            r.extras_kept,
            strings(&r.full),
            strings(&r.full_for_negation),
            r.gates,
            r.magic_rules
        ));
    }
    let dropped: Vec<String> = p.demand_dropped.iter().map(|(r, why)| strings(&[r.clone(), why.clone()])).collect();
    lines.push(format!("dropped [{}]", dropped.join(", ")));
    lines.push(format!("extras {}", p.magic_extras.map_or_else(|| "null".to_owned(), |n| n.to_string())));
    lines.push(format!("demand {}", strings(&p.demand)));
    lines.join("\n") + "\n"
}

/// On T's five programs, `.demand` (magic sets), `.demand *` (adornment) and gated negation produce T's programs
/// rule for rule: the goldens are `rdl.py`'s output, printed by T's repr.
/// A demand answered through a coarser one keeps no literal that only the demanded atom tied to the key (decision
/// 0677). `w`'s `un(v, c)` comes after the test of c, so it asks for `un` with both columns bound, which `out`'s
/// `un(v, u)` answers through `un_bf`, keyed by v alone. The test of c (`kind`, `field`, `named`) reached v only by
/// the value t of `whole(t, q)`: default-called-once's `magic_unparen_bf(v)`, which the split refused as a relation of
/// one file joining two. Left out, the demand only widens; `whole` stays, and binds nothing of another file.
#[test]
fn a_coarser_demand_keeps_no_literal_only_the_demanded_atom_tied() {
    let src = ".decl kind(n: node, k: symbol)\n.input kind\n.decl child(p: node, c: node, i: number)\n.input child\n\
        .decl field(p: node, x: symbol, c: node)\n.input field\n.decl named(e: node, q: symbol)\n.input named\n\
        .decl arg(f: node, v: node)\n.input arg\n.decl ftype(f: node, t: symbol)\n.input ftype\n\
        .decl whole(t: symbol, q: symbol)\nwhole(\"Date\", \"now\").\n\
        .decl un(e: node, m: node)\n.pragma \"magic-transform\" \"un\"\n\
        un(e, e) :- kind(e, \"x\").\nun(e, m) :- kind(e, \"paren\"), child(e, i, 0), un(i, m).\n\
        .decl w(f: node, v: node)\n\
        w(f, v) :- ftype(f, t), arg(f, v), kind(c, \"call\"), field(c, \"function\", e), named(e, q), whole(t, q), un(v, c).\n\
        .decl out(f: node, u: node)\n.output out\nout(f, u) :- ftype(f, _), arg(f, v), un(v, u), !w(f, v).\n";
    for gate in [false, true] {
        let p = prune(parse(src).unwrap());
        let got = demand_bound_columns(p, &Options { gate, ..Options::default() }).unwrap_or_else(|e| panic!("gate {gate}: {e}"));
        let rules: Vec<String> = got.rules.iter().map(|r| r.to_string()).collect();
        let magic: Vec<&String> = rules.iter().filter(|r| r.starts_with("magic_un_")).collect();
        assert!(magic.iter().any(|r| r.contains("whole(")), "gate {gate}: the demand of `un` from `w`: {rules:#?}");
        for r in magic {
            assert!(!r.contains("named(") && !r.contains("field("), "gate {gate}: joins the call's file to v's by a value: {r}\n{rules:#?}");
        }
    }
}

#[test]
fn t_programs_transform_like_t() {
    // T's goldens order nodes by their ids, as preorder ids did; by definition the front end compares them by
    // `node_order` (decision 0094 §2a), a rewrite of its own that T never made.
    crate::code_rule_dsl::logic::front::KEEP_NODE_COMPARISONS.set(true);
    let mut failures = Vec::new();
    let renamed = regex::Regex::new(r"(^|[^A-Za-z0-9])glob(_[bf]+\b|\b)").unwrap();
    for name in ["calls", "calls_rust", "workload", "workload_demand", "workload_perdoc"] {
        let src = std::fs::read_to_string(root().join(format!("plan/golden/programs/{name}.rerail"))).unwrap();
        let p = prune(parse(&src).unwrap());
        for mode in ["demand", "adorn", "gate"] {
            let options = Options { gate: mode == "gate", ..Options::default() };
            let got = if mode == "demand" { magic_sets(p.clone(), &options) } else { magic_adorn(p.clone(), &options) };
            let got = golden_text(&prune(got.unwrap_or_else(|e| panic!("{name} {mode}: {e}"))));
            let expected = std::fs::read_to_string(root().join(format!("magic/golden/{name}.{mode}.txt"))).unwrap();
            // `glob` is a `.rerail` built-in, so the program's relation of that name reads as `glob_` (the goldens are
            // copies, kept as written).
            let expected = renamed.replace_all(&expected, |c: &regex::Captures| format!("{}glob_{}", &c[1], &c[2])).into_owned();
            // A demand answered through a coarser one keeps no literal that only the demanded atom tied to its key
            // (decision 0677): T's `home_bbb`, answered through `home_ffb`, kept `rifrom(s, …)`, which reaches t by the
            // value x alone and joins s's document to t's.
            let expected = expected.replace(
                "rule magic_home_ffb($t) :- magic_ifrom_bbff($t, $x), rifrom($s, $x, $m, $y)\n",
                "rule magic_home_ffb($t) :- magic_ifrom_bbff($t, $x)\n",
            );
            if got != expected {
                let line = got.lines().zip(expected.lines()).position(|(a, b)| a != b);
                let at = line.map_or_else(
                    || format!("lengths {} vs {}", got.lines().count(), expected.lines().count()),
                    |i| format!("line {}:\n  got      {}\n  expected {}", i + 1, got.lines().nth(i).unwrap(), expected.lines().nth(i).unwrap()),
                );
                failures.push(format!("{name}.{mode}: {at}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A negated use whose demand comes from the negating relation's own recursion would close a negative cycle
/// (`reach` → `!blocked` → `magic_blocked` → `reach`): that relation is computed in full instead.
#[test]
fn a_negative_cycle_through_demand_drops_the_demand() {
    let p = parse(
        ".decl start(x: node)\n.decl e(x: node, y: node)\n.decl wall(x: node)\n.decl reach(x: node)\n.decl blocked(x: node)\n\
         .input start, e, wall\n.output reach\n.pragma \"magic-transform\" \"blocked\"\n\
         reach(x) :- start(x).\nreach(y) :- reach(x), e(x, y), !blocked(y).\nblocked(y) :- wall(y).\n",
    )
    .unwrap();
    assert!(strata(&p).is_ok());
    let got = magic_sets(p, &Options::default()).unwrap();
    assert_eq!(got.demand_dropped, [("blocked".to_owned(), "negative cycle through its demand".to_owned())]);
    assert!(got.demand.is_empty() && !got.decls.contains_key("magic_blocked"));
}

/// A magic rule keeps no literal that the literals it leaves out had tied to its key, and that joins another
/// document's nodes to the key's by a value alone (decision 0677). `pair(x, y)` ties x and y; kept as an extra it
/// would close a negative cycle (`!leaf` reads `un`), so it is left out, and `child(y, d, i)` then joined y's file to
/// c's by the index i: RUF034's `magic_unparen_bf`, which the split refused as a relation of one file joining two.
/// Left out with it, the demand only widens.
#[test]
fn a_demand_joins_no_other_documents_nodes_by_a_value() {
    let src = ".decl kind(n: node, k: symbol)\n.input kind\n.decl child(p: node, c: node, i: number)\n.input child\n\
        .decl ask(a: node, b: node)\n.input ask\n.decl un(e: node, m: node)\n.pragma \"magic-transform\" \"un\"\n\
        un(e, e) :- kind(e, \"x\").\nun(e, m) :- kind(e, \"paren\"), child(e, i, 0), un(i, m).\n\
        .decl leaf(x: node)\nleaf(x) :- kind(x, _), un(x, m), kind(m, \"leaf\").\n\
        .decl pair(x: node, y: node)\npair(x, y) :- ask(a, b), un(a, x), un(b, y).\n\
        pair(u, v) :- pair(x, y), !leaf(x), child(x, c, i), child(y, d, i), un(c, u), un(d, v).\n\
        .decl same(x: node, y: node)\n.output same\nsame(x, y) :- pair(x, y), kind(x, k), kind(y, k).\n";
    for gate in [false, true] {
        let p = prune(parse(src).unwrap());
        let got = demand_bound_columns(p, &Options { gate, ..Options::default() }).unwrap_or_else(|e| panic!("gate {gate}: {e}"));
        let rules: Vec<String> = got.rules.iter().map(|r| r.to_string()).collect();
        let demand: Vec<&String> = rules.iter().filter(|r| r.starts_with("magic_un_") && r.contains("child(")).collect();
        assert!(!demand.is_empty(), "gate {gate}: the demand of `un` from `pair`'s children: {rules:#?}");
        for r in demand {
            assert!(r.matches("child(").count() == 1 || r.contains("pair("), "gate {gate}: joins two documents' children by their index: {r}\n{rules:#?}");
        }
    }
}
