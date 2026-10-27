use super::*;
use crate::code_rule_dsl::logic::front::{parse, prune};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic")
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
#[test]
fn t_programs_transform_like_t() {
    let mut failures = Vec::new();
    for name in ["calls", "calls_rust", "workload", "workload_demand", "workload_perdoc"] {
        let src = std::fs::read_to_string(root().join(format!("front/rdl/{name}.rdl"))).unwrap();
        let p = prune(parse(&src).unwrap());
        for mode in ["demand", "adorn", "gate"] {
            let options = Options { gate: mode == "gate", ..Options::default() };
            let got = if mode == "demand" { magic_sets(p.clone(), &options) } else { magic_adorn(p.clone(), &options) };
            let got = golden_text(&prune(got.unwrap_or_else(|e| panic!("{name} {mode}: {e}"))));
            let expected = std::fs::read_to_string(root().join(format!("magic/golden/{name}.{mode}.txt"))).unwrap();
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
         .input start, e, wall\n.output reach\n.demand blocked\n\
         reach($x) :- start($x).\nreach($y) :- reach($x), e($x, $y), !blocked($y).\nblocked($y) :- wall($y).\n",
    )
    .unwrap();
    assert!(strata(&p).is_ok());
    let got = magic_sets(p, &Options::default()).unwrap();
    assert_eq!(got.demand_dropped, [("blocked".to_owned(), "negative cycle through its demand".to_owned())]);
    assert!(got.demand.is_empty() && !got.decls.contains_key("magic_blocked"));
}
