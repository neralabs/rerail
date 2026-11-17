//! 06-first-slice S1's gate: every Python pack clause the lowering accepts matches, on the pack's
//! fixtures, at exactly the nodes W's matcher does (`GrammarMatcher::match_root` at each node of
//! the same tape). Prints the coverage: clauses lowered, refused by operator, mismatched.

use super::compile_matcher::{Entry, Label, LoweredMatcher, domain};
use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{GrammarMatcher, MatchTree, Stop, match_tree, node_at};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use ast_grep_core::AstGrep as Root;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python")
}

fn files(directory: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, extension, found);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }
}

fn python_tape(source: &str) -> TapeDoc {
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    TapeDoc::restore(source, grammar, &image, &|| false).expect("restored tape")
}

/// The Python pack's AST clauses: (policy/clause, matcher).
fn clauses() -> Vec<(String, crate::code_rule_dsl::CompiledCodeRuleDsl)> {
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    documents
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let compiled =
                CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned()).ok()?;
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some((name, compiled))
        })
        .collect()
}

/// The fixtures a gate reads: every eighth by default, all with `RERAIL_S1_ALL=1`; of those, n
/// from the m-th with `RERAIL_S1_FILES=n` and `RERAIL_S1_FROM=m`, for a targeted check or a
/// sample in pieces.
/// The pack's fixtures (every eighth, or all with `RERAIL_S1_ALL`), or the `.py` files under
/// `RERAIL_S1_DIR` (06 S7's real files).
fn fixtures() -> Vec<TapeDoc> {
    let mut sources = Vec::new();
    let dir = std::env::var_os("RERAIL_S1_DIR");
    files(dir.as_ref().map_or(pack().join("upstream"), PathBuf::from).as_path(), "py", &mut sources);
    sources.sort();
    let step = if std::env::var_os("RERAIL_S1_ALL").is_some() || dir.is_some() { 1 } else { 8 };
    let most = std::env::var("RERAIL_S1_FILES").ok().and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    let from = std::env::var("RERAIL_S1_FROM").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    sources
        .iter()
        .step_by(step)
        .skip(from)
        .take(most)
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|source| python_tape(&source))
        .collect()
}

/// Whether a gate checks this clause: all, or those whose `policy/clause` contains
/// `RERAIL_S1_CLAUSE`.
fn selected(policy: &str, clause: &str) -> bool {
    std::env::var("RERAIL_S1_CLAUSE").map_or(true, |parts| parts.split('|').any(|part| format!("{policy}/{clause}").contains(part)))
}

fn python_matchers(
    compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
) -> impl Iterator<Item = (&str, &GrammarMatcher)> {
    compiled.inner.clauses.iter().flat_map(|clause| {
        let matchers = match &clause.kind {
            ClauseKind::Forbid(Query::Ast { matchers, .. }) => &matchers[..],
            _ => &[],
        };
        matchers
            .iter()
            .filter(|matcher| matcher.language == GrammarDialect::Python)
            .map(move |matcher| (clause.id.as_str(), matcher))
    })
}

#[test]
fn lowered_python_clauses_match_where_ws_matcher_does() {
    let tapes = fixtures();
    assert!(!tapes.is_empty(), "the fixtures are in git");
    let compiled = clauses();
    let mut total = 0;
    let mut lowered = 0;
    let mut refused = BTreeMap::<String, usize>::new();
    let mut mismatched = Vec::new();
    let mut matched_nodes = 0;
    let (mut oracle_time, mut lowered_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let mut slowest = Vec::new();
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            total += 1;
            let program = match match_tree(grammar) {
                Ok(tree) => LoweredMatcher::lower(&tree)
                    .map_err(|error| format!("compile: {error:?}")),
                Err(unsupported) => Err(unsupported.0.to_owned()),
            };
            let program = match program {
                Ok(program) => program,
                Err(reason) => {
                    if std::env::var_os("RERAIL_S1_LIST").is_some() {
                        eprintln!("  refused {policy}/{clause}: {reason}");
                    }
                    *refused.entry(reason).or_default() += 1;
                    continue;
                }
            };
            lowered += 1;
            let mut first = None;
            let mut clause_time = std::time::Duration::ZERO;
            for (index, tape) in tapes.iter().enumerate() {
                let started = std::time::Instant::now();
                let root = Root::doc(tape.clone());
                let mut work = usize::MAX;
                let expected = (0..tape.transducer_len() as u32)
                    .filter(|&ordinal| {
                        grammar
                            .match_root(node_at(&root, ordinal), &mut work)
                            .ok()
                            .flatten()
                            .is_some()
                    })
                    .collect::<BTreeSet<_>>();
                oracle_time += started.elapsed();
                let started = std::time::Instant::now();
                let actual = program.roots(tape);
                lowered_time += started.elapsed();
                clause_time += started.elapsed();
                matched_nodes += expected.len();
                match actual {
                    Ok(actual) if actual == expected => {}
                    Ok(actual) => {
                        let missing = expected.difference(&actual).next().copied();
                        let extra = actual.difference(&expected).next().copied();
                        let at = missing.or(extra).expect("differ");
                        first = Some(format!(
                            "fixture {index}: {} missing, {} extra; first at node {at} ({}) {:?}",
                            expected.difference(&actual).count(),
                            actual.difference(&expected).count(),
                            if missing.is_some() { "missing" } else { "extra" },
                            tape.transducer_text_str(at).chars().take(60).collect::<String>(),
                        ));
                        break;
                    }
                    Err(error) => {
                        first = Some(format!("fixture {index}: {error:?}"));
                        break;
                    }
                }
            }
            slowest.push((clause_time, format!("{policy}/{clause} ({} passes)", program.passes())));
            if let Some(first) = first {
                mismatched.push(format!("{policy}/{clause} ({} passes): {first}", program.passes()));
            }
        }
    }
    eprintln!(
        "S1 coverage: {lowered}/{total} Python AST clauses lowered, {} exact on {} fixtures \
         ({matched_nodes} matched nodes); refused: {refused:?}",
        lowered - mismatched.len(),
        tapes.len(),
    );
    slowest.sort_by(|left, right| right.0.cmp(&left.0));
    eprintln!("S1 time: W's matcher {oracle_time:?}, lowered {lowered_time:?}; slowest lowered: {:?}", &slowest[..slowest.len().min(5)]);
    for mismatch in &mismatched {
        eprintln!("  mismatch {mismatch}");
    }
    crate::code_rule_dsl::prof::report("S1");
    assert!(mismatched.is_empty(), "{} lowered clauses differ from W's matcher", mismatched.len());
}

/// Widens the first `neighbor` walk to `end`: a lowering this wrong must not pass the gate.
/// How many widened clauses the widened-walk checks must see differ from W's matcher: the pack's clauses give 2
/// on `widened_tapes` (2026-11-17; the CPU's check and Metal's alike). One shows the walk is checked.
const WIDENED_CAUGHT: usize = 2;

/// The widened-walk checks' tapes: every fourth Python file of the pack's upstream sources.
fn widened_tapes() -> Vec<TapeDoc> {
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    sources
        .iter()
        .step_by(4)
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|source| python_tape(&source))
        .collect()
}

fn widen_first_neighbor(tree: &mut MatchTree) -> bool {
    match tree {
        MatchTree::Walk { stop, rule, .. } => {
            if matches!(stop, Stop::Neighbor) {
                *stop = Stop::End;
                true
            } else {
                widen_first_neighbor(rule)
            }
        }
        MatchTree::All(trees) | MatchTree::Any(trees) => trees.iter_mut().any(widen_first_neighbor),
        MatchTree::Not(tree) | MatchTree::Recursive(_, tree) => widen_first_neighbor(tree),
        MatchTree::Count { rule, .. } => widen_first_neighbor(rule),
        MatchTree::Constrained(_, constraints) => {
            constraints.iter_mut().any(|(_, tree)| widen_first_neighbor(tree))
        }
        // A fact's slots are shared with its producer (`Arc`): not widened here.
        MatchTree::Kind(_) | MatchTree::Supertype(..) | MatchTree::Pattern(_) | MatchTree::Regex(_) | MatchTree::Recur(_) | MatchTree::Fact(_) | MatchTree::Never => false,
    }
}

#[test]
fn the_gate_catches_a_widened_walk() {
    let tapes = widened_tapes();
    let compiled = clauses();
    let mut caught = 0;
    for (_, compiled) in &compiled {
        for (_, grammar) in python_matchers(compiled) {
            let Ok(mut tree) = match_tree(grammar) else { continue };
            if !widen_first_neighbor(&mut tree) {
                continue;
            }
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            let differs = tapes.iter().any(|tape| {
                let root = Root::doc(tape.clone());
                let mut work = usize::MAX;
                let expected = (0..tape.transducer_len() as u32)
                    .filter(|&ordinal| {
                        grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some()
                    })
                    .collect::<BTreeSet<_>>();
                program.roots(tape).ok() != Some(expected)
            });
            caught += usize::from(differs);
            if caught >= WIDENED_CAUGHT {
                return;
            }
        }
    }
    panic!("only {caught} widened clauses differ from W's matcher");
}

/// A recursive utility inside another (RUF005's `list-add` inside `outer-operand`) settles round
/// after round within the outer one's group: `x`'s right side is a `list-add` only through its
/// left operand, itself one, which a single round of `list-add` does not reach.
#[test]
fn a_recursive_utility_inside_another_settles() {
    let policy = r#"{
        "format": "rerail.policy/v1", "id": "nested-recursion", "revision": 1, "mode": "steer",
        "description": "A recursive utility inside another.", "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "nested",
            "forbid": { "ast": {
                "language": "python",
                "utils": {
                    "list-expr": { "any": [
                        { "kind": "list" },
                        { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "list-expr" } },
                        { "matches": "list-add" }
                    ] },
                    "list-add": { "kind": "binary_operator", "has": { "stopBy": "neighbor", "matches": "list-expr" } },
                    "outer": { "any": [
                        { "matches": "list-add" },
                        { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "outer" } }
                    ] }
                },
                "rule": { "kind": "assignment", "has": { "stopBy": "neighbor", "matches": "outer" } }
            } },
            "violation": { "anchor": "witness.span", "code": "NESTED", "message": "A list concatenation." }
        }]
    }"#;
    let compiled = CodeRuleDslRuntime::compile(policy.to_owned(), "json".to_owned(), CAPABILITIES.to_owned()).expect("the policy compiles");
    let (_, grammar) = python_matchers(&compiled).next().expect("a Python clause");
    let program = LoweredMatcher::lower(&match_tree(grammar).expect("the clause lowers")).expect("the tree compiles");
    let tape = python_tape("x = ['a'] + b + c\ny = (['a'] + b) + c\nz = d + e\n");
    let root = Root::doc(tape.clone());
    let mut work = usize::MAX;
    let expected = (0..tape.transducer_len() as u32)
        .filter(|&ordinal| grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 2, "W's matcher finds x and y");
    assert_eq!(program.roots(&tape).unwrap(), expected);
}

/// S2's parity gate (O23, F1): every lowered clause's passes run on Metal over the whole batch of
/// fixtures, in one command buffer (plus one per extra round of a recursive group), and give the
/// CPU interpreter's root sets on every fixture.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn lowered_clauses_find_the_same_roots_on_metal() {
    use super::compile_matcher::Backend;
    let tapes = fixtures();
    let compiled = clauses();
    let (mut lowered, mut matched_nodes, mut commands, mut dispatches) = (0, 0, 0, 0);
    let mut waits = [0; 3];
    let mut most = 0;
    let mut differ = Vec::new();
    let (mut cpu_time, mut metal_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            lowered += 1;
            let started = std::time::Instant::now();
            let cpu = program.roots_batch(Backend::Cpu, &tapes);
            cpu_time += started.elapsed();
            let started = std::time::Instant::now();
            let metal = program.roots_metal(&tapes).map(|(roots, outcome)| {
                most = most.max(outcome.dispatches);
                if outcome.dispatches > 100 && std::env::var_os("RERAIL_S2_TRACE").is_some() {
                    eprintln!("  over 100 launches: {policy}/{clause}: {} launches, {} rounds at most, {} passes", outcome.dispatches, outcome.rounds, program.passes());
                }
                commands += outcome.command_buffers;
                dispatches += outcome.dispatches;
                waits.iter_mut().zip(outcome.waits).for_each(|(sum, wait)| *sum += wait);
                roots
            });
            metal_time += started.elapsed();
            match (cpu, metal) {
                (Ok(cpu), Ok(metal)) if cpu == metal => {
                    matched_nodes += cpu.iter().map(BTreeSet::len).sum::<usize>();
                }
                (Ok(cpu), Ok(metal)) => {
                    let at = cpu.iter().zip(&metal).position(|(cpu, metal)| cpu != metal);
                    differ.push(format!("{policy}/{clause}: fixture {at:?} differs"));
                }
                (cpu, metal) => differ.push(format!(
                    "{policy}/{clause}: cpu {:?}, metal {:?}",
                    cpu.err(),
                    metal.err()
                )),
            }
        }
    }
    eprintln!(
        "S2 Metal parity: {} of {lowered} lowered clauses equal on {} fixtures ({matched_nodes} \
         matched nodes); CPU {cpu_time:?}, Metal {metal_time:?} ({commands} command buffers, \
         {dispatches} launches, at most {most} for a clause; host waits for growth, reruns, \
         uploads {waits:?})",
        lowered - differ.len(),
        tapes.len(),
    );
    for differ in &differ {
        eprintln!("  {differ}");
    }
    crate::code_rule_dsl::prof::report("S2");
    assert!(differ.is_empty(), "{} lowered clauses differ between CPU and Metal", differ.len());
}

/// S2 on the interpreter (decision 0032): every pass runs as programs in `run_batch` over the
/// resident tapes (as a run whose walks outgrew their frames does), every fourth clause. Its
/// passes mark their bits as they publish (`packed_batch::MARKS`). A clause refused for the
/// interpreter's per-lane workspace or the launch bound (5 of 39, O38; the packed batch refused the
/// same ones) is counted, not compared: it runs as walks in production.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn lowered_clauses_find_the_same_roots_on_the_resident_interpreter() {
    use super::compile_matcher::Backend;
    use metal_program_backend::label_passes::INTERPRET_ONLY;
    let tapes = fixtures();
    let compiled = clauses();
    let (mut lowered, mut differ, mut refused, mut equal) = (0, Vec::new(), 0, 0);
    INTERPRET_ONLY.set(true);
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            lowered += 1;
            if lowered % 4 != 0 {
                continue;
            }
            let cpu = program.roots_batch(Backend::Cpu, &tapes);
            let metal = program.roots_metal(&tapes).map(|(roots, outcome)| {
                assert!(outcome.interpreted, "{policy}/{clause} ran its walks");
                roots
            });
            match (cpu, metal) {
                (Ok(cpu), Ok(metal)) if cpu == metal => equal += 1,
                (Ok(_), Err(error @ (Error::BackendCapacity | Error::BackendLaunchLimit))) => {
                    refused += 1;
                    eprintln!("  refused {policy}/{clause}: {error:?}");
                }
                (cpu, metal) => differ.push(format!("{policy}/{clause}: cpu {:?}, metal {:?}", cpu.as_ref().err(), metal.as_ref().err())),
            }
        }
    }
    INTERPRET_ONLY.set(false);
    eprintln!("S2 interpreted: {equal} of {} clauses equal, {refused} refused (workspace or launches)", lowered / 4);
    for differ in &differ {
        eprintln!("  {differ}");
    }
    assert!(differ.is_empty(), "{} clauses differ on the resident interpreter", differ.len());
    assert!(equal > 0, "no clause ran on the interpreter");
}

/// Decision 0334: the label sweep under the replanned budget, with every program it walks priced at almost nothing
/// (a step per node, times the slack). Its tasks run over their budgets, the run goes again with twice the budget
/// until they finish, and it finds the full budget's roots.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn a_misplanned_label_sweep_runs_again_and_finds_the_full_budgets_roots() {
    use crate::code_rule_dsl::transducer::packed::evaluation_costs::{self, Rate};
    use metal_program_backend::label_passes::INTERPRET_ONLY;
    let tapes = fixtures();
    let mut matchers = Vec::new();
    for (policy, compiled) in &clauses() {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            if let Ok(matcher) = LoweredMatcher::lower(&tree) {
                matchers.push(matcher);
            }
        }
    }
    let rates = matchers
        .iter()
        .flat_map(LoweredMatcher::walked_programs)
        .map(|program| (program.plain_digest().to_owned(), Rate { steps_per_unit: 1e-6, samples: 1 }))
        .collect();
    evaluation_costs::install(rates);
    INTERPRET_ONLY.set(true);
    let (mut checked, mut replanned) = (0, 0);
    for matcher in matchers.iter().step_by(4).take(12) {
        // SAFETY: nextest runs each test in a process of its own; nothing else reads the variable meanwhile.
        unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "full") };
        let full = matcher.roots_metal(&tapes);
        unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "replan") };
        let replan = matcher.roots_metal(&tapes);
        match (full, replan) {
            (Ok((full, _)), Ok((replan, outcome))) => {
                assert_eq!(replan, full, "the replanned sweep finds the full budget's roots");
                replanned += outcome.replanned;
                checked += 1;
            }
            (Err(Error::BackendCapacity | Error::BackendLaunchLimit), _) => {}
            (full, replan) => panic!("full {:?}, replanned {:?}", full.err(), replan.err()),
        }
    }
    unsafe { std::env::remove_var("RERAIL_DEVICE_BUDGET") };
    INTERPRET_ONLY.set(false);
    eprintln!("{checked} clauses checked, {replanned} reruns with twice the budget");
    assert!(checked > 0, "no clause ran");
    assert!(replanned > 0, "no task ran over a budget of a step per node");
}

/// Decision 0334: a wave clause over its budget is replanned by cost. With its programs priced at almost nothing,
/// each clause's evaluations run over their budgets; where W's matcher is priced lower than the transducer again,
/// the evaluation goes to W (`WaveRun::limited`, its candidates for W to confirm, every root the full run found
/// among them), and where W is priced higher, the transducer runs again with twice the budget until it finishes,
/// and finds the full run's roots.
#[test]
fn a_clause_over_its_budget_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not() {
    use super::compile_matcher::{Backend, LoweredWave};
    use crate::code_rule_dsl::transducer::packed::WorkBudget;
    use crate::code_rule_dsl::transducer::packed::evaluation_costs::{self, Rate};
    // One large file (the fixtures' sources one after another) and a few small ones.
    let mut sources = Vec::new();
    files(pack().join("upstream").as_path(), "py", &mut sources);
    sources.sort();
    let joined: String = sources.iter().step_by(8).filter_map(|path| std::fs::read_to_string(path).ok()).collect::<Vec<_>>().join("\n");
    let mut tapes = vec![python_tape(&joined)];
    tapes.extend(fixtures().into_iter().take(8));
    let mut matchers = Vec::new();
    for (policy, compiled) in &clauses() {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            if let Ok(matcher) = LoweredMatcher::lower(&tree) {
                matchers.push(matcher);
            }
        }
    }
    // The wave's own programs (a wave relabels its clauses'), each at almost nothing, and W at `w` a node.
    let priced = |wave: &LoweredWave, w: f64| {
        let mut rates: std::collections::HashMap<String, Rate> = wave
            .walked_programs()
            .into_iter()
            .map(|program| (program.plain_digest().to_owned(), Rate { steps_per_unit: 1e-6, samples: 1 }))
            .collect();
        rates.insert(wave.w_key(0), Rate { steps_per_unit: w, samples: 1 });
        evaluation_costs::install(rates);
    };
    // A slack of 1: the budget is the prediction's floor (`MIN_PREDICTED`), which the large file's root walks outrun.
    let replan = WorkBudget::Replan { slack: 1, doublings: 0 };
    let active = vec![vec![true; tapes.len()]];
    evaluation_costs::keep(true);
    let (mut checked, mut handed, mut overran) = (0, 0, 0);
    for matcher in matchers.iter().step_by(4).take(16) {
        let wave = LoweredWave::new(&[matcher]).unwrap();
        let Ok((full, full_run)) = wave.roots_batch_budgeted(Backend::Cpu, &tapes, &active, WorkBudget::Full) else { continue };
        priced(&wave, 1e9);
        let (again, again_run) = wave.roots_batch_budgeted(Backend::Cpu, &tapes, &active, replan).unwrap();
        assert_eq!(again, full, "W priced higher: the transducer runs again and finds the full run's roots");
        assert_eq!(again_run.limited, full_run.limited);
        overran += evaluation_costs::drain().0.iter().filter(|seen| seen.over_budget).count();
        priced(&wave, 1e-6);
        let (cheap, cheap_run) = wave.roots_batch_budgeted(Backend::Cpu, &tapes, &active, replan).unwrap();
        let _ = evaluation_costs::drain();
        for (c, per) in cheap.iter().enumerate() {
            for (t, roots) in per.iter().enumerate() {
                if cheap_run.limited.contains(&(c, t)) {
                    assert!(roots.is_superset(&full[c][t]), "W's candidates hold every root the full run found");
                } else {
                    assert_eq!(roots, &full[c][t]);
                }
            }
        }
        handed += cheap_run.limited.len().saturating_sub(full_run.limited.len());
        checked += 1;
    }
    evaluation_costs::keep(false);
    eprintln!("{checked} clauses checked, {overran} overruns run again, {handed} evaluations given to W");
    assert!(checked > 0, "no clause ran");
    assert!(overran > 0, "no evaluation ran over its budget");
    assert!(handed > 0, "no evaluation went to W where W cost less");
}

/// Decision 0334's test for `replan` as the default: every lowered Python clause of the pack, in waves of `width`, over
/// the fixtures (`RERAIL_S1_DIR`, e.g. corpus0), with the full budget and with `replan` (its slack, a cost store
/// empty at the start, so the priors and then the rates the run itself measures). Each run's findings on a tape are
/// its roots, or W's matcher at its candidates where it gave the clause to W there; they must be the same. Also how
/// often each plan was chosen.
fn replan_finds_the_full_budgets_roots(backend: super::compile_matcher::Backend, width: usize) {
    use super::compile_matcher::LoweredWave;
    use crate::code_rule_dsl::transducer::packed::{WorkBudget, evaluation_costs};
    let mut tapes = fixtures();
    assert!(!tapes.is_empty(), "no fixtures");
    // Ten tapes, unless `RERAIL_S1_FILES` says how many: each wave runs twice over them (the Metal check of 491
    // clauses on 10 of corpus0's files takes about 230 s on an M3 Max).
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(10);
    }
    let compiled = clauses();
    let mut lowered = Vec::new();
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            if let Ok(matcher) = LoweredMatcher::lower(&tree) {
                lowered.push((grammar, matcher));
            }
        }
    }
    evaluation_costs::install(Default::default());
    let replan = WorkBudget::Replan { slack: crate::code_rule_dsl::transducer::packed::REPLAN_SLACK, doublings: 0 };
    let (mut waves, mut given, mut checked, mut replanned) = (0, 0, 0, 0);
    for chunk in lowered.chunks(width) {
        let matchers: Vec<&LoweredMatcher> = chunk.iter().map(|(_, matcher)| matcher).collect();
        let Ok(wave) = LoweredWave::new(&matchers) else { continue };
        let active = vec![vec![true; tapes.len()]; matchers.len()];
        let run = |budget: WorkBudget| {
            // SAFETY: nextest runs each test in a process of its own; nothing else reads the variable meanwhile.
            match budget {
                WorkBudget::Full => unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "full") },
                WorkBudget::Replan { .. } => unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "replan") },
            }
            wave.roots_batch_budgeted(backend, &tapes, &active, budget)
        };
        let started = std::time::Instant::now();
        let full = match run(WorkBudget::Full) {
            Ok(found) => found,
            Err(Error::BackendCapacity | Error::BackendLaunchLimit) => continue,
            Err(e) => panic!("full: {e:?}"),
        };
        let full_time = started.elapsed();
        let (found, ran) = run(replan).unwrap();
        eprintln!("wave {waves}: full {full_time:?}, replan {:?}, {} given to W, {} reruns", started.elapsed() - full_time, ran.limited.len(), ran.replanned);
        replanned += ran.replanned;
        // What each run's findings are: its roots, or W's matcher at its candidates where it gave the clause to W.
        let settled = |found: &[Vec<BTreeSet<u32>>], limited: &[(usize, usize)], c: usize, t: usize| -> BTreeSet<u32> {
            if !limited.contains(&(c, t)) {
                return found[c][t].clone();
            }
            let root = Root::doc(tapes[t].clone());
            let mut work = usize::MAX;
            found[c][t].iter().copied().filter(|&n| chunk[c].0.match_root(node_at(&root, n), &mut work).ok().flatten().is_some()).collect()
        };
        for c in 0..found.len() {
            for t in 0..tapes.len() {
                checked += 1;
                given += usize::from(ran.limited.contains(&(c, t)));
                assert_eq!(settled(&found, &ran.limited, c, t), settled(&full.0, &full.1.limited, c, t), "clause {c} of wave {waves}, tape {t}");
            }
        }
        waves += 1;
    }
    unsafe { std::env::remove_var("RERAIL_DEVICE_BUDGET") };
    eprintln!("{backend:?}: {} clauses in {waves} waves on {} tapes: {checked} evaluations, {given} given to W, {replanned} reruns", lowered.len(), tapes.len());
    assert!(waves > 0);
}

#[test]
fn replan_finds_the_full_budgets_roots_on_the_pack() {
    replan_finds_the_full_budgets_roots(super::compile_matcher::Backend::Cpu, 16);
}

#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn on_metal_replan_finds_the_full_budgets_roots_on_the_pack() {
    // Wider waves: a Metal wave's time is mostly its dispatches, not its clauses.
    replan_finds_the_full_budgets_roots(super::compile_matcher::Backend::MetalInterpreter, 48);
}

/// Decision 0334 on Metal: the wave's passes and root walks on the device, every program priced at almost nothing, on
/// the interpreter (no micro walks). Where W is priced higher, a run whose tasks ran over goes again with twice the
/// budget and finds the full run's roots; where W is priced lower, the unfinished (pass, file) go to W
/// (`WaveRun::limited`: the clause's candidates, every root the full run found among them).
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn on_metal_an_unfinished_pass_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not() {
    use super::compile_matcher::{Backend, LoweredWave};
    use crate::code_rule_dsl::transducer::packed::evaluation_costs::{self, Rate};
    use metal_program_backend::label_passes::INTERPRET_ONLY;
    let mut sources = Vec::new();
    files(pack().join("upstream").as_path(), "py", &mut sources);
    sources.sort();
    let joined: String = sources.iter().step_by(8).filter_map(|path| std::fs::read_to_string(path).ok()).collect::<Vec<_>>().join("\n");
    let mut tapes = vec![python_tape(&joined)];
    tapes.extend(fixtures().into_iter().take(8));
    let mut matchers = Vec::new();
    for (policy, compiled) in &clauses() {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            if let Ok(matcher) = LoweredMatcher::lower(&tree) {
                matchers.push(matcher);
            }
        }
    }
    let priced = |wave: &LoweredWave, w: f64| {
        let mut rates: std::collections::HashMap<String, Rate> = wave
            .walked_programs()
            .into_iter()
            .map(|program| (program.plain_digest().to_owned(), Rate { steps_per_unit: 1e-6, samples: 1 }))
            .collect();
        for clause in 0..wave.clauses.len() {
            rates.insert(wave.w_key(clause), Rate { steps_per_unit: w, samples: 1 });
        }
        evaluation_costs::install(rates);
    };
    let active = vec![vec![true; tapes.len()]];
    INTERPRET_ONLY.set(true);
    let (mut checked, mut handed, mut replanned) = (0, 0, 0);
    for matcher in matchers.iter().step_by(4).take(12) {
        let wave = LoweredWave::new(&[matcher]).unwrap();
        // SAFETY: nextest runs each test in a process of its own; nothing else reads the variable meanwhile.
        unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "full") };
        evaluation_costs::install(Default::default());
        let full = match wave.roots_batch(Backend::MetalInterpreter, &tapes, &active) {
            Ok(found) => found,
            Err(Error::BackendCapacity | Error::BackendLaunchLimit) => continue,
            Err(e) => panic!("full: {e:?}"),
        };
        unsafe { std::env::set_var("RERAIL_DEVICE_BUDGET", "replan") };
        priced(&wave, 1e9);
        let (again, again_run) = wave.roots_batch(Backend::MetalInterpreter, &tapes, &active).unwrap();
        replanned += again_run.replanned;
        assert_eq!(again, full.0, "W priced higher: the run goes again and finds the full run's roots");
        assert_eq!(again_run.limited, full.1.limited);
        priced(&wave, 1e-6);
        let (cheap, cheap_run) = wave.roots_batch(Backend::MetalInterpreter, &tapes, &active).unwrap();
        for (c, per) in cheap.iter().enumerate() {
            for (t, roots) in per.iter().enumerate() {
                if cheap_run.limited.contains(&(c, t)) {
                    assert!(roots.is_superset(&full.0[c][t]), "W's candidates hold every root the full run found");
                } else {
                    assert_eq!(roots, &full.0[c][t]);
                }
            }
        }
        handed += cheap_run.limited.len().saturating_sub(full.1.limited.len());
        checked += 1;
    }
    unsafe { std::env::remove_var("RERAIL_DEVICE_BUDGET") };
    INTERPRET_ONLY.set(false);
    eprintln!("{checked} clauses checked, {replanned} reruns, {handed} evaluations given to W");
    assert!(checked > 0, "no clause ran");
    assert!(handed > 0, "no unfinished pass went to W where W cost less");
}

/// The Metal gate's mutation check: a widened walk must differ from W's matcher on Metal too, over the CPU check's
/// tapes (`widened_tapes`), 16 a launch.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn metal_catches_a_widened_walk() {
    let tapes = widened_tapes();
    let mut caught = 0;
    for (_, compiled) in &clauses() {
        for (_, grammar) in python_matchers(compiled) {
            let Ok(mut tree) = match_tree(grammar) else { continue };
            if !widen_first_neighbor(&mut tree) {
                continue;
            }
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            let differs = tapes.chunks(16).any(|tapes| {
                let expected = tapes
                    .iter()
                    .map(|tape| {
                        let root = Root::doc(tape.clone());
                        let mut work = usize::MAX;
                        (0..tape.transducer_len() as u32)
                            .filter(|&ordinal| grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some())
                            .collect::<BTreeSet<_>>()
                    })
                    .collect::<Vec<_>>();
                let metal = program.roots_metal(tapes).ok().map(|(roots, _)| roots);
                let cpu = program.roots_batch(super::compile_matcher::Backend::Cpu, tapes).ok();
                assert_eq!(metal, cpu, "a widened clause must still agree between CPU and Metal");
                metal != Some(expected)
            });
            caught += usize::from(differs);
            if caught >= WIDENED_CAUGHT {
                return;
            }
        }
    }
    panic!("only {caught} widened clauses differ from W's matcher on Metal");
}

#[test]
#[ignore = "a probe of the sweeps a recursive group needs"]
fn recursive_groups_settle_in_few_sweeps() {
    let tapes = fixtures();
    for (policy, compiled) in &clauses() {
        for (clause, grammar) in python_matchers(compiled) {
            if !selected(policy, clause) {
                continue;
            }
            let Ok(tree) = match_tree(grammar) else { continue };
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            let needed = program.sweeps_needed(&tapes).unwrap();
            if !needed.is_empty() {
                eprintln!("{policy}/{clause}: {needed:?}");
            }
        }
    }
}

/// The spike's anchor measure (prior work, ast-index-spike S151: anchor rows per answer, 1.05 there) for the wave:
/// over the fixtures (10 unless `RERAIL_S1_FILES` says), each clause's candidate roots (its domain past its entry,
/// where its root walk starts) against the roots it finds, the root walk's steps, and the rows its label passes
/// start at (each pass's domain). A census, not a check: it prints the totals and the clauses with the most
/// candidates per answer and the most steps.
#[test]
fn anchor_rows_per_answer() {
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(10);
    }
    let (compiled, mut lowered, mut names) = (clauses(), Vec::new(), Vec::new());
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
                names.push(format!("{policy}/{clause}"));
            }
        }
    }
    let nodes: usize = tapes.iter().map(TapeDoc::transducer_len).sum();
    // Per clause: candidates, roots, root-walk steps, label rows, label passes.
    let mut per: Vec<(String, u64, u64, u64, u64, usize)> = Vec::new();
    for (matchers, names) in lowered.chunks(16).zip(names.chunks(16)) {
        let wave = LoweredWave::new(&matchers.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; matchers.len()];
        let Ok(views) = wave.labelled(&tapes, &active) else { continue };
        for (clause, name) in wave.clauses.iter().zip(names) {
            let entry = Entry::of(&clause.program);
            let (mut candidates, mut roots, mut steps, mut label_rows) = (0_u64, 0_u64, 0_u64, 0_u64);
            for view in &views {
                let sets = view.transducer_labels();
                let starts: Vec<u32> = domain(&clause.domain, view, sets).filter(|&n| entry.admits(view, n)).collect();
                candidates += starts.len() as u64;
                if let Ok((outputs, used)) = clause.program.execute_roots_budgeted(view, starts.iter().copied(), usize::MAX) {
                    roots += outputs.iter().map(|o| o.start).collect::<BTreeSet<_>>().len() as u64;
                    steps += used as u64;
                }
                for def in &clause.labels.defs {
                    if let Label::Pattern(_, d) | Label::Constrained(_, d, _) | Label::Roots(_, d) | Label::Targets(_, d) | Label::Count { domain: d, .. } = def {
                        label_rows += domain(d, view, sets).count() as u64;
                    }
                }
            }
            per.push((name.clone(), candidates, roots, steps, label_rows, clause.labels.defs.len()));
        }
    }
    let total = |f: fn(&(String, u64, u64, u64, u64, usize)) -> u64| per.iter().map(f).sum::<u64>();
    let (candidates, roots, steps, label_rows) = (total(|p| p.1), total(|p| p.2), total(|p| p.3), total(|p| p.4));
    eprintln!(
        "anchors: {} clauses on {} tapes ({nodes} nodes): {candidates} candidate roots for {roots} roots ({:.2} a root), {steps} root-walk steps ({:.1} a candidate), {label_rows} label-pass rows ({:.1} a node)",
        per.len(),
        tapes.len(),
        candidates as f64 / roots.max(1) as f64,
        steps as f64 / candidates.max(1) as f64,
        label_rows as f64 / nodes.max(1) as f64,
    );
    let without = per.iter().filter(|p| p.2 == 0);
    eprintln!("clauses that found nothing: {}, with {} candidates", without.clone().count(), without.map(|p| p.1).sum::<u64>());
    let mut by_ratio: Vec<_> = per.iter().filter(|p| p.2 > 0).collect();
    by_ratio.sort_by(|a, b| (b.1 as f64 / b.2 as f64).total_cmp(&(a.1 as f64 / a.2 as f64)));
    for p in by_ratio.iter().take(10) {
        eprintln!("  candidates a root {:>8.1}: {} ({} candidates, {} roots, {} steps)", p.1 as f64 / p.2 as f64, p.0, p.1, p.2, p.3);
    }
    let mut by_steps: Vec<_> = per.iter().collect();
    by_steps.sort_by_key(|p| std::cmp::Reverse(p.3 + p.4));
    for p in by_steps.iter().take(10) {
        eprintln!("  steps {:>10} + label rows {:>8} ({} passes): {} ({} candidates, {} roots)", p.3, p.4, p.5, p.0, p.1, p.2);
    }
    assert!(!per.is_empty());
}

/// The root programs' micro walks give the interpreter's roots (decision 0334 §5's prefilter plan, and Metal's root
/// walks, read text tests as the text layer's bits): every lowered Python clause whose root program is a micro walk
/// that tests text, at every node of the fixtures (4 unless `RERAIL_S1_FILES` says), labelled by the wave's passes.
#[test]
fn root_walks_with_text_are_the_interpreters_roots() {
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(4);
    }
    let (compiled, mut lowered, mut names) = (clauses(), Vec::new(), Vec::new());
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
                names.push(format!("{policy}/{clause}"));
            }
        }
    }
    let (mut checked, mut stopped, mut differ) = (0, 0, Vec::new());
    for (chunk, (matchers, names)) in lowered.chunks(12).zip(names.chunks(12)).enumerate() {
        let wave = LoweredWave::new(&matchers.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; matchers.len()];
        let views = wave.labelled(&tapes, &active).unwrap();
        for ((program, name), c) in wave.programs().iter().zip(names).zip(0..) {
            let Some(walk) = Micro::of(program).filter(|walk| !walk.texts.is_empty()) else { continue };
            for (t, view) in views.iter().enumerate() {
                let Some(texts) = walk.texts(program, view) else { continue };
                let mut frames = Frames::default();
                // Per node: the walk's verdict against the interpreter's; a node either stops at its limit (the
                // interpreter's ceiling, where the wave gives the clause to W; the walk's bound) is not compared.
                let (mut missing, mut extra) = (Vec::new(), Vec::new());
                for node in 0..view.transducer_len() as u32 {
                    let mut walked = false;
                    if walk.run(view, view.transducer_labels(), &texts, node, Emit::Start, &mut frames, &mut |_| walked = true).is_err() {
                        stopped += 1;
                        continue;
                    }
                    let Ok(outputs) = program.execute_roots(view, [node]) else {
                        stopped += 1;
                        continue;
                    };
                    match (outputs.is_empty(), walked) {
                        (false, false) => missing.push(node),
                        (true, true) => extra.push(node),
                        _ => {}
                    }
                }
                checked += 1;
                if !missing.is_empty() || !extra.is_empty() {
                    let at = |node: u32| view.transducer_text_str(node).chars().take(60).collect::<String>();
                    differ.push(format!(
                        "{name} (chunk {chunk}, clause {c}) tape {t}: {} missing (first {:?}), {} extra (first {:?})",
                        missing.len(),
                        missing.first().map(|&n| (n, view.transducer_kind(n), at(n))),
                        extra.len(),
                        extra.first().map(|&n| (n, view.transducer_kind(n), at(n))),
                    ));
                }
            }
        }
    }
    eprintln!("root walks with text: {checked} (clause, tape) checked, {stopped} nodes stopped at a limit");
    assert!(checked > 0);
    assert!(differ.is_empty(), "{} differ: {differ:#?}", differ.len());
}

/// O28's micro walk gives each label program's interpreter outputs, as labels read them, from
/// every node of the fixtures, labelled by the wave's own passes: the starts that emit, or the
/// nodes the outputs captured. Every lowered clause's eligible programs are checked.
#[test]
fn micro_walks_are_the_interpreters_label_outputs() {
    // Every program at every node: 4 fixtures unless `RERAIL_S1_FILES` says.
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(4);
    }
    let mut lowered = Vec::new();
    for (_, compiled) in &clauses() {
        for (_, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
            }
        }
    }
    let wave = LoweredWave::new(&lowered.iter().collect::<Vec<_>>()).unwrap();
    let active = vec![vec![true; tapes.len()]; lowered.len()];
    let views = wave.labelled(&tapes, &active).unwrap();
    let programs = wave.label_programs();
    // The count programs: their marks are tallies, compared with multiplicity.
    let counts = wave.count_programs();
    let counted = programs.iter().filter(|(program, _)| counts.contains(&std::ptr::from_ref(*program))).count();
    let micro = programs.iter().map(|(program, targets)| Micro::of(program).map(|m| (m, *targets))).collect::<Vec<_>>();
    let (mut compared, mut overflowed, mut marked) = (0_usize, 0_usize, 0_usize);
    let mut frames = Frames::default();
    for view in &views {
        for ((program, _), walk) in programs.iter().zip(&micro) {
            let Some((walk, targets)) = walk else { continue };
            let Some(texts) = walk.texts(program, view) else { continue };
            for node in 0..view.transducer_len() as u32 {
                // A label's marks as a set; a count's as a multiset (each output tallies).
                let outputs = program.execute_roots(view, [node]).unwrap();
                let mut expected = outputs
                    .iter()
                    .flat_map(|output| if *targets == Emit::Captures { output.captures.iter().map(|c| c.node).collect() } else { vec![output.start] })
                    .collect::<Vec<_>>();
                if *targets == Emit::Start {
                    expected.truncate(1);
                }
                let mut found = Vec::new();
                let result = walk.run(view, view.transducer_labels(), &texts, node, *targets, &mut frames, &mut |n| found.push(n));
                expected.sort_unstable();
                found.sort_unstable();
                if *targets == Emit::Captures && !counts.contains(&std::ptr::from_ref(*program)) {
                    expected.dedup();
                    found.dedup();
                }
                match result {
                    Ok(()) => {
                        assert_eq!(found, expected, "program {:?} at node {node}", program.words);
                        compared += 1;
                        marked += found.len();
                    }
                    Err(why) => {
                        overflowed += 1;
                        eprintln!("overflow {why:?}: program {:?} at node {node} of {}", program.words.iter().map(|w| split(*w)).map(|(o, m, a)| format!("{o:#x}/{m}/{a}")).collect::<Vec<_>>(), view.transducer_len());
                    }
                }
            }
        }
    }
    let eligible = micro.iter().filter(|m| m.is_some()).count();
    let texts = micro.iter().flatten().filter(|(m, _)| !m.texts.is_empty()).count();
    eprintln!("micro: {texts} of the walks test text; {counted} of the programs count");
    eprintln!("micro: {eligible} of {} label programs; {compared} starts compared, {marked} marks, {overflowed} overflowed", programs.len());
    assert!(eligible * 2 > programs.len(), "most label programs are micro walks");
    assert!(compared > 100_000 && overflowed * 100 < compared);
}

/// A long walk spills its frames to other threads (decision 0125): with a budget of one step, nearly every walk
/// gives frames away (a descendant range halved, a move frame whole), and the wave's findings on Metal are still the
/// CPU's (each side's roots, or W's matcher at its candidates where it gave the clause to W: a backend may stop at
/// its limit where the other does not). A frame lost, walked twice where a tally counts, or given its walk's wrong start shows here.
#[cfg(target_os = "macos")]
#[test]
fn spilled_walks_find_the_cpus_roots() {
    use super::compile_matcher::Backend;
    use metal_program_backend::label_passes::SPILL_BUDGET_TEST;
    let mut tapes = fixtures();
    tapes.truncate(24);
    let (compiled, mut lowered, mut names) = (clauses(), Vec::new(), Vec::new());
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
                names.push((format!("{policy}/{clause}"), grammar));
            }
        }
    }
    // A (clause, tape) a backend gave to W (`WaveRun::limited`: its candidates) is what W's matcher confirms there.
    let confirmed = |grammar: &GrammarMatcher, tape: &TapeDoc, candidates: &BTreeSet<u32>| -> BTreeSet<u32> {
        let root = Root::doc(tape.clone());
        let mut work = usize::MAX;
        candidates.iter().copied().filter(|&n| grammar.match_root(node_at(&root, n), &mut work).ok().flatten().is_some()).collect()
    };
    // Waves of a dozen clauses, as the engine's are (the whole set is over the launch bound).
    let (mut found, mut differ, mut waves) = (0, Vec::new(), 0);
    for (at, clauses) in lowered.chunks(12).enumerate() {
        let wave = LoweredWave::new(&clauses.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; clauses.len()];
        let (mut cpu, cpu_run) = wave.roots_batch(Backend::Cpu, &tapes, &active).unwrap();
        SPILL_BUDGET_TEST.set(Some(1));
        let metal = wave.roots_batch(Backend::MetalInterpreter, &tapes, &active);
        SPILL_BUDGET_TEST.set(None);
        let Ok((mut metal, run)) = metal else { continue };
        assert!(run.metal, "the wave ran on Metal");
        for (found, limited) in [(&mut cpu, &cpu_run.limited), (&mut metal, &run.limited)] {
            for &(c, t) in limited {
                found[c][t] = confirmed(names[at * 12 + c].1, &tapes[t], &found[c][t]);
            }
        }
        waves += 1;
        found += cpu.iter().flatten().map(BTreeSet::len).sum::<usize>();
        let unspilled = || wave.roots_batch(Backend::MetalInterpreter, &tapes, &active).ok().map(|found| found.0);
        for (c, (cpu, metal)) in cpu.iter().zip(&metal).enumerate().filter(|(_, (c, m))| c != m) {
            // Per tape: roots the CPU found that Metal did not, and the other way; and whether Metal without the
            // spill finds the CPU's.
            let tapes: Vec<_> = cpu.iter().zip(metal).enumerate().filter(|(_, (c, m))| c != m).map(|(t, (c, m))| (t, c.difference(m).count(), m.difference(c).count())).collect();
            let without = unspilled().map(|found| found[c] == *cpu);
            let given = |limited: &[(usize, usize)]| limited.iter().filter(|&&(l, _)| l == c).map(|&(_, t)| t).collect::<Vec<_>>();
            differ.push(format!(
                "{} (wave {at}, clause {c}): (tape, missing, extra) {tapes:?}; unspilled agrees: {without:?}; given to W on the CPU {:?}, on Metal {:?}",
                names[at * 12 + c].0,
                given(&cpu_run.limited),
                given(&run.limited)
            ));
        }
    }
    eprintln!("spilled: {waves} waves of {} clauses, {found} roots", lowered.len());
    assert!(waves * 2 > lowered.len() / 12 && found > 1000, "most waves ran on Metal and found roots ({waves} waves, {found} roots)");
    assert!(differ.is_empty(), "{} clauses differ on Metal with every walk spilling: {differ:?}", differ.len());
}

/// The combined graph (list step 15): a sub-rule shared by clauses is one label of the wave, and
/// each clause's roots from the wave's labels are its roots alone. Clauses are active on half the
/// tapes each, so a label an earlier clause owns is computed where a later clause reads it.
#[test]
fn the_combined_graph_shares_labels_and_keeps_each_clauses_roots() {
    let mut tapes = fixtures();
    tapes.truncate(4);
    let mut lowered = Vec::new();
    for (_, compiled) in &clauses() {
        for (_, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
            }
        }
    }
    let wave = LoweredWave::new(&lowered.iter().collect::<Vec<_>>()).unwrap();
    let separate = lowered.iter().map(LoweredMatcher::passes).sum::<usize>();
    eprintln!("combined graph: {} clauses, {separate} labels separately, {} combined", lowered.len(), wave.passes());
    assert!(wave.passes() < separate, "some sub-rule is shared");
    let active = (0..lowered.len()).map(|clause| (0..tapes.len()).map(|tape| (clause + tape) % 2 == 0).collect()).collect::<Vec<Vec<bool>>>();
    let views = wave.labelled(&tapes, &active).unwrap();
    let programs = wave.programs();
    let mut compared = 0;
    for (clause, matcher) in lowered.iter().enumerate() {
        for (tape, view) in views.iter().enumerate() {
            if !active[clause][tape] {
                continue;
            }
            let starts = (0..view.transducer_len() as u32).filter(|&node| wave.starts(clause, view, node));
            let found = programs[clause].execute_roots(view, starts).unwrap().iter().map(|output| output.start).collect::<BTreeSet<_>>();
            assert_eq!(found, matcher.roots(&tapes[tape]).unwrap(), "clause {clause} on tape {tape}");
            compared += 1;
        }
    }
    assert!(compared > 0);
}

/// Patterns of every binding shape: single, repeated, multi before, between and after singles,
/// and none; and Ruff fixtures to run them on, at every node of each tape.
const BINDING_PATTERNS: &[&str] = &[
    "$F($$$A)", "$A = $A", "def $F($$$ARGS): $$$BODY", "$X.format($$$A)", "if $C: $$$B", "[$$$A, $B]",
    "$A + $B", "f($A, $$$, $B)", "return $$$", "$A[$B]", "for $X in $Y: $$$Z", "print($$$A, $B, $$$C)",
];
const BINDING_FIXTURES: &[&str] = &[
    "pyflakes/F601.py", "pyflakes/F632.py", "flake8_simplify/SIM905.py", "flake8_bugbear/B031.py",
    "flake8_bugbear/B006_B008.py", "flake8_return/RET504.py",
];
fn binding_fixtures() -> Vec<(String, TapeDoc)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python/upstream/ruff/fixtures");
    BINDING_FIXTURES
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
            ((*path).to_owned(), python_tape(&source))
        })
        .collect()
}

/// What `execute_ast_pattern` gives, at every node of the fixtures for every binding pattern, is
/// pinned: a change to how bindings are kept must give every capture as before.
#[test]
fn ast_pattern_captures_are_pinned() {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    let mut matched = 0;
    for source in BINDING_PATTERNS {
        let program = Program::compile_ast_grep_pattern(&Pattern::try_new(source, GrammarDialect::Python).unwrap()).unwrap();
        for (path, tape) in binding_fixtures() {
            for node in 0..tape.transducer_len() as u32 {
                let Some(captures) = program.execute_ast_pattern(&tape, node).unwrap() else { continue };
                matched += 1;
                h.update(format!("{source}|{path}|{node}:").as_bytes());
                for c in captures {
                    h.update(format!("{}/{}/{}/{};", c.label, c.occurrence, c.node, c.multi).as_bytes());
                }
            }
        }
    }
    let got: String = h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    assert!(matched > 500, "{matched} matches");
    assert_eq!(got, "ea71d4437b2c2ba12fc3b08d3c7f982c", "the captures moved ({matched} matches)");
}

/// The verdict alone (`matches_ast_pattern`, into bindings reused from node to node) is
/// `execute_ast_pattern`'s, and so is what each name binds: nothing, one node, or several.
#[test]
fn ast_pattern_verdicts_reuse_their_bindings() {
    let mut scratch = AstBindings::default();
    for source in BINDING_PATTERNS {
        let program = Program::compile_ast_grep_pattern(&Pattern::try_new(source, GrammarDialect::Python).unwrap()).unwrap();
        let names: Vec<String> = ["A", "B", "C", "F", "X", "Y", "Z", "ARGS", "BODY", "NONE"].map(String::from).into();
        for (path, tape) in binding_fixtures() {
            for node in 0..tape.transducer_len() as u32 {
                let captures = program.execute_ast_pattern(&tape, node).unwrap();
                let verdict = program.matches_ast_pattern(&tape, node, &mut scratch).unwrap();
                assert_eq!(verdict, captures.is_some(), "{source} at {path}:{node}");
                let Some(captures) = captures else { continue };
                for name in &names {
                    let want = match captures.iter().find(|c| c.label == *name) {
                        None => Bound::Unbound,
                        Some(c) if c.multi => Bound::Multi,
                        Some(c) => Bound::Single(c.node),
                    };
                    let got = program.ast_slot(name).map_or(Bound::Unbound, |slot| scratch.bound(slot));
                    assert_eq!(got, want, "{source} at {path}:{node}: ${name}");
                }
            }
        }
    }
}

