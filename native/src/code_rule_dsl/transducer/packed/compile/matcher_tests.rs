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
fn fixtures() -> Vec<TapeDoc> {
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    let step = if std::env::var_os("RERAIL_S1_ALL").is_some() { 1 } else { 8 };
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
    std::env::var("RERAIL_S1_CLAUSE").map_or(true, |part| format!("{policy}/{clause}").contains(&part))
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
        MatchTree::Kind(_) | MatchTree::Pattern(_) | MatchTree::Regex(_) | MatchTree::Recur(_) | MatchTree::Never => false,
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
            if caught >= 3 {
                return;
            }
        }
    }
    panic!("only {caught} widened clauses differ from W's matcher");
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
    assert!(differ.is_empty(), "{} lowered clauses differ between CPU and Metal", differ.len());
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
    let micro = programs.iter().map(|(program, targets)| Micro::of(program).map(|m| (m, *targets))).collect::<Vec<_>>();
    let (mut compared, mut overflowed, mut marked) = (0_usize, 0_usize, 0_usize);
    let mut frames = Frames::default();
    for view in &views {
        for ((program, _), walk) in programs.iter().zip(&micro) {
            let Some((walk, targets)) = walk else { continue };
            for node in 0..view.transducer_len() as u32 {
                let expected = program
                    .execute_roots(view, [node])
                    .unwrap()
                    .iter()
                    .flat_map(|output| if *targets { output.captures.iter().map(|c| c.node).collect() } else { vec![output.start] })
                    .collect::<BTreeSet<_>>();
                let mut found = BTreeSet::new();
                match walk.run(view, view.transducer_labels(), node, *targets, &mut frames, &mut |n| {
                    found.insert(n);
                }) {
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
    eprintln!("micro: {eligible} of {} label programs; {compared} starts compared, {marked} marks, {overflowed} overflowed", programs.len());
    assert!(eligible * 2 > programs.len(), "most label programs are micro walks");
    assert!(compared > 100_000 && overflowed * 100 < compared);
}
