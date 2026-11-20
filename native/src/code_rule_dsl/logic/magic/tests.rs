use crate::code_rule_dsl::logic::plan::corpus::preparation::magic::{KeyDemand, ProgramDemand};
use crate::code_rule_dsl::logic::plan::corpus::preparation::prune::Prune;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
use crate::code_rule_dsl::logic::rr::parse_version;
use std::sync::Arc;

fn root() -> std::path::PathBuf {
    rerail_store::source_tree::native().join("src/code_rule_dsl/logic")
}

fn strings(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| serde_json::to_string(s).unwrap()).collect::<Vec<_>>().join(", "))
}

/// A nullary atom in a demanded body is an existence test: it is ready with nothing bound, and adorning the program
/// keeps it in the body (it read the first argument of an atom with none; ported from cost-classifier-metal-rf's
/// test of the tree magic, which is gone here).
#[test]
fn a_nullary_guard_is_ready_in_a_demanded_body() {
    let src = ".decl rows(x: number)\n.input rows\n.decl ready()\n.input ready\n.decl answer(x: number)\n.output answer\nanswer(x) :- rows(x), ready().\n";
    for mode in ["adorn", "demand"] {
        let v = transformed(src, mode).unwrap_or_else(|e| panic!("{mode}: {e}"));
        let rules: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        // A nullary relation is lowered to one unit column (`ready(1)`), shown so.
        assert!(rules.iter().any(|r| r.starts_with("answer(") && (r.contains("ready()") || r.contains("ready(1)"))), "{mode}: {rules:?}");
    }
}

/// T's `rdl.py sql` front over `src`: prune, demand (`mode`: `demand` is `.demand`'s magic sets, `adorn` the
/// adornment of every derived relation, `gate` that with gated negations), prune.
fn transformed(src: &str, mode: &str) -> Result<Arc<Version>, String> {
    let v = parse_version(src).map_err(|e| e.to_string())?;
    let mut p = Pipeline::new("T's front", v);
    p.program(&mut Prune::default());
    let mut keys = KeyDemand::default();
    let mut adorned = ProgramDemand { every: true, gate: mode == "gate", ..ProgramDemand::default() };
    if mode == "demand" {
        p.program(&mut keys);
    } else {
        p.program(&mut adorned);
    }
    p.program(&mut Prune::default());
    let v = p.finish()?.0;
    if let Some(Err(e)) = adorned.outcome {
        return Err(e);
    }
    Ok(v)
}

/// The program as the goldens print it (T's repr, and `json.dumps` with sorted keys for gates and the report).
fn golden_text(p: &Version) -> String {
    let mut lines = Vec::new();
    let mut decls: Vec<(String, String)> = p
        .decls()
        .iter()
        .map(|d| (p.text(d.name()).to_owned(), d.cols().map(|(c, t)| format!("{}: {}", p.text(c), p.text(t))).collect::<Vec<_>>().join(", ")))
        .filter(|(r, _)| r.starts_with("magic_") || r.starts_with("done_"))
        .collect();
    decls.sort();
    for (r, cols) in decls {
        lines.push(format!("decl {r}({cols})"));
    }
    for r in p.rules() {
        lines.push(format!("rule {}", p.show_rule(r)));
    }
    let mut gates = ProgramFields::gates(p);
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
    if let Some(r) = ProgramFields::magic_report(p) {
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
    let dropped: Vec<String> = ProgramFields::demand_dropped(p).iter().map(|(r, why)| strings(&[r.clone(), why.clone()])).collect();
    lines.push(format!("dropped [{}]", dropped.join(", ")));
    lines.push(format!("extras {}", ProgramFields::magic_extras(p).map_or_else(|| "null".to_owned(), |n| n.to_string())));
    lines.push(format!("demand {}", strings(&ProgramFields::demand(p))));
    lines.join("\n") + "\n"
}

/// On T's five programs, `.demand` (magic sets), `.demand *` (adornment) and gated negation produce T's programs
/// rule for rule: the goldens are `rdl.py`'s output, printed by T's repr.
#[test]
fn t_programs_transform_like_t() {
    // T's goldens order nodes by their ids, as preorder ids did; by definition the front end compares them by
    // `node_order` (decision 0094 §2a), a rewrite of its own that T never made.
    crate::code_rule_dsl::logic::front::KEEP_NODE_COMPARISONS.set(true);
    let mut failures = Vec::new();
    let renamed = regex::Regex::new(r"(^|[^A-Za-z0-9])glob(_[bf]+\b|\b)").unwrap();
    for name in ["calls", "calls_rust", "workload", "workload_demand", "workload_perdoc"] {
        let src = std::fs::read_to_string(root().join(format!("plan/golden/programs/{name}.rerail"))).unwrap();
        for mode in ["demand", "adorn", "gate"] {
            let got = golden_text(&transformed(&src, mode).unwrap_or_else(|e| panic!("{name} {mode}: {e}")));
            let expected = std::fs::read_to_string(root().join(format!("magic/golden/{name}.{mode}.txt"))).unwrap();
            // `glob` is a `.rerail` built-in, so the program's relation of that name reads as `glob_` (the goldens are
            // copies, kept as written).
            let expected = renamed.replace_all(&expected, |c: &regex::Captures| format!("{}glob_{}", &c[1], &c[2])).into_owned();
            // T declares the gates of every trial its greedy search made, a rejected one's too (`done_islocal` in
            // `calls_rust.gate`); the version declares the gates of the program it made. A declaration no rule or
            // gate names is not compared, for a gate.
            let named = |rel: &str| expected.lines().filter(|l| !l.starts_with("decl ")).any(|l| l.contains(&format!("{rel}(")) || l.contains(&format!("\"{rel}\"")));
            let expected: String = expected
                .lines()
                .filter(|l| l.strip_prefix("decl done_").and_then(|d| d.split_once('(')).is_none_or(|(rel, _)| named(&format!("done_{rel}"))))
                .map(|l| format!("{l}\n"))
                .collect();
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
    let src = ".decl start(x: node)\n.decl e(x: node, y: node)\n.decl wall(x: node)\n.decl reach(x: node)\n.decl blocked(x: node)\n\
         .input start, e, wall\n.output reach\n.pragma \"magic-transform\" \"blocked\"\n\
         reach(x) :- start(x).\nreach(y) :- reach(x), e(x, y), !blocked(y).\nblocked(y) :- wall(y).\n";
    let p = parse_version(src).unwrap();
    assert!(crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::Strata>(&p).is_ok());
    let mut keys = KeyDemand::default();
    let mut pipeline = Pipeline::new("demand", p);
    pipeline.program(&mut keys);
    let got = pipeline.finish().unwrap().0;
    assert_eq!(ProgramFields::demand_dropped(&*got), [("blocked".to_owned(), "negative cycle through its demand".to_owned())]);
    assert!(ProgramFields::demand(&*got).is_empty() && !got.declares("magic_blocked"));
}
