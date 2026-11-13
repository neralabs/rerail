//! 06-first-slice S1's gate: every Python pack clause the lowering accepts matches, on the pack's
//! fixtures, at exactly the nodes W's matcher does (`GrammarMatcher::match_root` at each node of
//! the same tape). Prints the coverage: clauses lowered, refused by operator, mismatched.

use super::compile_matcher::LoweredMatcher;
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

/// The nodes of `tape` W's matcher matches at (`GrammarMatcher::match_root` at each node), as a file
/// evaluation reads them: the file's scope table built once and shared by its nodes
/// (`FileScopeSlot`, as W's wave installs it), not once per node.
fn ws_roots(grammar: &GrammarMatcher, tape: &TapeDoc) -> BTreeSet<u32> {
    let root = Root::doc(tape.clone());
    let _scopes = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty());
    let mut work = usize::MAX;
    (0..tape.transducer_len() as u32)
        .filter(|&ordinal| grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some())
        .collect()
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
            // Each fixture is a file of its own, W's matcher and the lowering both: the fixtures run in parallel,
            // and are read in order, to the first that differs.
            let runs = {
                use rayon::prelude::*;
                tapes
                    .par_iter()
                    .map(|tape| {
                        let started = std::time::Instant::now();
                        let expected = ws_roots(grammar, tape);
                        let oracle = started.elapsed();
                        let started = std::time::Instant::now();
                        let actual = program.roots(tape);
                        (expected, actual, oracle, started.elapsed())
                    })
                    .collect::<Vec<_>>()
            };
            let mut first = None;
            let mut clause_time = std::time::Duration::ZERO;
            for (index, (tape, (expected, actual, oracle, lowering))) in tapes.iter().zip(runs).enumerate() {
                oracle_time += oracle;
                lowered_time += lowering;
                clause_time += lowering;
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
    slowest.sort_by_key(|right| std::cmp::Reverse(right.0));
    eprintln!("S1 time: W's matcher {oracle_time:?}, lowered {lowered_time:?}; slowest lowered: {:?}", &slowest[..slowest.len().min(5)]);
    for mismatch in &mismatched {
        eprintln!("  mismatch {mismatch}");
    }
    crate::code_rule_dsl::prof::report("S1");
    assert!(mismatched.is_empty(), "{} lowered clauses differ from W's matcher", mismatched.len());
}

/// Widens the first `neighbor` walk to `end`: a lowering this wrong must not pass the gate.
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
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    let tapes = sources
        .iter()
        .step_by(4)
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|source| python_tape(&source))
        .collect::<Vec<_>>();
    let compiled = clauses();
    let mut caught = 0;
    for (_, compiled) in &compiled {
        for (_, grammar) in python_matchers(compiled) {
            let Ok(mut tree) = match_tree(grammar) else { continue };
            if !widen_first_neighbor(&mut tree) {
                continue;
            }
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
            let differs = {
                use rayon::prelude::*;
                tapes.par_iter().any(|tape| program.roots(tape).ok() != Some(ws_roots(grammar, tape)))
            };
            caught += usize::from(differs);
            if caught >= 3 {
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

/// The Metal gate's mutation check: a widened walk must differ from W's matcher on Metal too.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn metal_catches_a_widened_walk() {
    let tapes = fixtures().into_iter().take(16).collect::<Vec<_>>();
    let mut caught = 0;
    for (_, compiled) in &clauses() {
        for (_, grammar) in python_matchers(compiled) {
            let Ok(mut tree) = match_tree(grammar) else { continue };
            if !widen_first_neighbor(&mut tree) {
                continue;
            }
            let Ok(program) = LoweredMatcher::lower(&tree) else { continue };
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
            let metal = program.roots_metal(&tapes).ok().map(|(roots, _)| roots);
            let cpu = program.roots_batch(super::compile_matcher::Backend::Cpu, &tapes).ok();
            assert_eq!(metal, cpu, "a widened clause must still agree between CPU and Metal");
            caught += usize::from(metal != Some(expected));
            if caught >= 3 {
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
/// gives frames away (a descendant range halved, a move frame whole), and the wave's roots on Metal are still the
/// CPU's. A frame lost, walked twice where a tally counts, or given its walk's wrong start shows here.
#[cfg(target_os = "macos")]
#[test]
fn spilled_walks_find_the_cpus_roots() {
    use super::compile_matcher::Backend;
    use metal_program_backend::label_passes::SPILL_BUDGET_TEST;
    let mut tapes = fixtures();
    tapes.truncate(24);
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
    // Waves of a dozen clauses, as the engine's are (the whole set is over the launch bound).
    let (mut found, mut differ, mut waves) = (0, Vec::new(), 0);
    for (at, clauses) in lowered.chunks(12).enumerate() {
        let wave = LoweredWave::new(&clauses.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; clauses.len()];
        let (cpu, _) = wave.roots_batch(Backend::Cpu, &tapes, &active).unwrap();
        SPILL_BUDGET_TEST.set(Some(1));
        let metal = wave.roots_batch(Backend::MetalInterpreter, &tapes, &active);
        SPILL_BUDGET_TEST.set(None);
        let Ok((metal, run)) = metal else { continue };
        assert!(run.metal, "the wave ran on Metal");
        waves += 1;
        found += cpu.iter().flatten().map(BTreeSet::len).sum::<usize>();
        differ.extend(cpu.iter().zip(&metal).enumerate().filter(|(_, (c, m))| c != m).map(|(c, _)| (at, c)));
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

/// The end of children meets a named candidate (RER-131): where the tokens the tape leaves out after
/// the last matched child begin with the goal's, the rest trails, in the packed matcher as in
/// ast-grep-core's. `case $V:` matches `case 1: f(); break;` (ast-grep 0.51.0 does); `f($A)` still
/// refuses `f(a, b)`, whose gap after `a` is `,`.
#[test]
fn ast_pattern_end_of_children_meets_implied_tokens() {
    use ast_grep_core::matcher::MatcherExt as _;
    let source = Arc::new("switch (x) { case 1: f(); break; }\nf(a);\nf(a, b);\n".to_owned());
    let grammar = Grammar::new(GrammarDialect::TypeScript).expect("TypeScript grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false).expect("parser completed").expect("tape image");
    let tape = TapeDoc::restore(source, grammar, &image, &|| false).expect("restored tape");
    let root = Root::doc(tape.clone());
    for (pattern, want) in [
        (Pattern::contextual("switch ($X) { case $V: }", "switch_case", GrammarDialect::TypeScript).unwrap(), ["case 1: f(); break;"]),
        (Pattern::try_new("f($A)", GrammarDialect::TypeScript).unwrap(), ["f(a)"]),
    ] {
        let program = Program::compile_ast_grep_pattern(&pattern).unwrap();
        let (mut packed, mut core) = (Vec::new(), Vec::new());
        for node in root.root().dfs() {
            if program.execute_ast_pattern(&tape, node.node_id() as u32).unwrap().is_some() {
                packed.push(node.text().into_owned());
            }
            if pattern.match_node(node.clone()).is_some() {
                core.push(node.text().into_owned());
            }
        }
        assert_eq!(packed, want, "packed");
        assert_eq!(core, want, "ast-grep-core");
    }
}

