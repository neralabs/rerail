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
        let v = parse_version(src).unwrap_or_else(|e| panic!("{e}"));
        let mut p = Pipeline::new("listed demand", v);
        p.program(&mut Prune::default());
        let mut demand = ProgramDemand { gate, ..ProgramDemand::default() };
        p.program(&mut demand);
        let v = p.finish().unwrap_or_else(|e| panic!("{e}")).0;
        assert!(matches!(demand.outcome, Some(Ok(_))), "gate {gate}: {:?}", demand.outcome);
        let rules: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        let demand: Vec<&String> = rules.iter().filter(|r| r.starts_with("magic_un_") && r.contains("child(")).collect();
        assert!(!demand.is_empty(), "gate {gate}: the demand of `un` from `pair`'s children: {rules:#?}");
        for r in demand {
            assert!(r.matches("child(").count() == 1 || r.contains("pair("), "gate {gate}: joins two documents' children by their index: {r}\n{rules:#?}");
        }
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
            // Demand is a set, stored sorted (program_facts); T printed it in the order its pragmas stated it.
            let sorted_demand = |l: &str| match l.strip_prefix("demand [").and_then(|d| d.strip_suffix(']')) {
                Some(items) => {
                    let mut items: Vec<&str> = items.split(", ").collect();
                    items.sort();
                    format!("demand [{}]", items.join(", "))
                }
                None => l.to_owned(),
            };
            let expected: String = expected
                .lines()
                .filter(|l| l.strip_prefix("decl done_").and_then(|d| d.split_once('(')).is_none_or(|(rel, _)| named(&format!("done_{rel}"))))
                .map(|l| format!("{}\n", sorted_demand(l)))
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

/// The demand of `src` (`.pragma "magic-transform"`, as a corpus run's front makes it): the program's rules, the
/// relations split with their copies, and the report's reasons.
fn demanded(src: &str) -> (Vec<String>, Vec<(String, String)>, Vec<String>) {
    let mut p = Pipeline::new("demand", parse_version(src).unwrap());
    let mut own = ProgramDemand::default();
    p.program(&mut own);
    let v = p.finish().unwrap().0;
    let Some(Ok(report)) = own.outcome else { panic!("{:?}", own.outcome) };
    // The program keeps the reasons with its report.
    assert_eq!(ProgramFields::magic_report(&*v).map(|r| r.reasons), Some(report.reasons.clone()));
    (v.rules().map(|r| v.show_rule(r)).collect(), own.splits, report.reasons)
}

/// A string annotation's shape (decision 0431 §8): the parse request reads `unparen` (through `tsub`), and a reader
/// of the request's answer (`parsed`, through `ref_site`) asks `unparen` too. One demand for both would join the
/// request's component with `unparen` and `tsub` (`rr_embed_req -> tsub -> unparen -> magic_unparen_bf -> ref_site ->
/// parsed -> rr_embed_req`), which the program keeps apart: the answer's reader reads a copy, with a demand of its
/// own, and `unparen`'s demand reads nothing the request answers.
const ANSWER_READS_DEMAND: &str = "
    .decl kind(n: int, k: symbol)
    .decl inner(e: int, i: int)
    .decl ann(a: int, e: int)
    .decl parsed(at: int, g: symbol, r: int)
    .input kind, inner, ann, parsed
    .decl unparen(e: int, m: int)
    .pragma \"magic-transform\" \"unparen\"
    unparen(e, e) :- kind(e, k), k != \"paren\".
    unparen(e, m) :- kind(e, \"paren\"), inner(e, i), unparen(i, m).
    .decl tsub(a: int, m: int)
    tsub(a, m) :- ann(a, e), unparen(e, m).
    .decl rr_embed_req(at: int, g: symbol)
    rr_embed_req(m, \"python\") :- tsub(_, m).
    .decl ref_site(r: int)
    ref_site(r) :- parsed(_, _, r).
    .decl typevar(r: int, m: int)
    typevar(r, m) :- ref_site(r), unparen(r, m).
    .output typevar, rr_embed_req
";

#[test]
fn a_demand_that_reads_a_requests_answer_is_split_from_what_the_request_reads() {
    let (rules, splits, reasons) = demanded(ANSWER_READS_DEMAND);
    assert_eq!(splits, [("unparen".to_owned(), "unparen__rr_embed_req".to_owned())]);
    // The reason names the cycle, each step with its rule, and the rule moved to the copy.
    let [why] = reasons.as_slice() else { panic!("{reasons:#?}") };
    assert!(why.starts_with("unparen's demand magic_unparen_bf joins rr_embed_req's component: rr_embed_req -> tsub -> unparen -> magic_unparen_bf -> ref_site -> parsed -> rr_embed_req"), "{why}");
    assert!(why.contains("magic_unparen_bf -> ref_site: magic_unparen_bf($r) :- ref_site($r).") && why.contains("parsed -> rr_embed_req: parsed is rr_embed_req's answer"), "{why}");
    assert!(why.ends_with("the rules that read the copy (1):\n    typevar($r, $m) :- ref_site($r), unparen($r, $m)"), "{why}");
    let with = |head: &str| rules.iter().filter(|r| r.starts_with(&format!("{head}("))).cloned().collect::<Vec<_>>();
    // `unparen`'s demand comes from `tsub` alone; the copy's from the answer's reader alone.
    assert!(!with("magic_unparen_bf").is_empty() && with("magic_unparen_bf").iter().all(|r| !r.contains("ref_site(")), "{rules:#?}");
    assert!(!with("magic_unparen__rr_embed_req_bf").is_empty() && with("magic_unparen__rr_embed_req_bf").iter().all(|r| !r.contains("ann(")), "{rules:#?}");
    assert!(with("tsub").iter().all(|r| r.contains("unparen(") && !r.contains("unparen__")), "{rules:#?}");
    assert!(with("typevar").iter().all(|r| r.contains("unparen__rr_embed_req(")), "{rules:#?}");
    // The copy recurses into itself, under its own demand.
    assert!(with("unparen__rr_embed_req").iter().any(|r| r.contains("magic_unparen__rr_embed_req_bf(") && r.matches("unparen__rr_embed_req(").count() == 2), "{rules:#?}");
}

/// Without the request nothing joins: one demand, no copy.
#[test]
fn a_shared_demand_with_no_request_is_not_split() {
    let src = ANSWER_READS_DEMAND.replace("rr_embed_req(m, \"python\") :- tsub(_, m).", "").replace(".output typevar, rr_embed_req", ".output typevar, tsub");
    let (rules, splits, _) = demanded(&src);
    assert!(splits.is_empty(), "{splits:?}");
    assert!(rules.iter().all(|r| !r.contains("__rr_embed_req")), "{rules:#?}");
}

/// A split relation's copy asks the demanded relations below it under its own demand, which reads the answer in
/// turn: the next round splits those (`hop`) for the copy.
#[test]
fn a_split_copy_splits_the_demand_below_it_in_the_next_round() {
    let src = ANSWER_READS_DEMAND
        .replace("\"unparen\"", "\"unparen, hop\"")
        .replace("unparen(e, m) :- kind(e, \"paren\"), inner(e, i), unparen(i, m).", ".decl hop(e: int, i: int)\n    hop(e, i) :- inner(e, i), kind(i, _).\n    unparen(e, m) :- kind(e, \"paren\"), hop(e, i), unparen(i, m).");
    let (rules, splits, _) = demanded(&src);
    assert_eq!(splits, [("unparen".to_owned(), "unparen__rr_embed_req".to_owned()), ("hop".to_owned(), "hop__rr_embed_req".to_owned())], "{rules:#?}");
    let with = |head: &str| rules.iter().filter(|r| r.starts_with(&format!("{head}("))).cloned().collect::<Vec<_>>();
    assert!(with("magic_hop_bf").iter().all(|r| !r.contains("__rr_embed_req")), "{rules:#?}");
    assert!(with("unparen__rr_embed_req").iter().filter(|r| r.contains("hop")).all(|r| r.contains("hop__rr_embed_req(")), "{rules:#?}");
}
