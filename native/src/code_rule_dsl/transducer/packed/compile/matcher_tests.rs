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

/// The lowerings of t45's plan (decisions 0424, 0425), each a clause: a bare `$X` is the named-node test, not a
/// pattern label; a clause that is one `inside … end` copies its label; the labels made inside an `all` that tests a
/// neighbor (`inside` or `has`, by a field or not) start within the set of the nodes it holds at, a formula, not a
/// seed label. Each finds W's roots.
fn seeded_clauses() -> (crate::code_rule_dsl::CompiledCodeRuleDsl, TapeDoc) {
    let clause = |id: &str, rule: &str| {
        format!(
            r#"{{"id": "{id}", "forbid": {{"ast": {{"language": "python", "rule": {rule}}}}}, "violation": {{"anchor": "witness.span", "code": "X", "message": "x"}}}}"#
        )
    };
    let clauses = [
        clause("named", r#"{"all": [{"pattern": "$X"}, {"kind": "identifier"}, {"inside": {"kind": "parameters", "stopBy": "neighbor"}}]}"#),
        clause("in-function", r#"{"inside": {"kind": "function_definition", "stopBy": "end"}}"#),
        clause("top", r#"{"kind": "expression_statement", "inside": {"kind": "module", "stopBy": "neighbor"}, "not": {"has": {"kind": "call", "stopBy": "neighbor"}}}"#),
        clause("named-function", r#"{"kind": "function_definition", "has": {"field": "name", "kind": "identifier", "stopBy": "neighbor"}}"#),
        clause("function-name", r#"{"kind": "identifier", "inside": {"kind": "function_definition", "field": "name", "stopBy": "neighbor"}}"#),
        clause("lone", r#"{"not": {"inside": {"kind": "argument_list", "stopBy": "neighbor"}}}"#),
        // A neighbor tested by text (ruff F541's shape): its set has no text test, since a text slot is its own
        // program's (0425).
        clause(
            "fstring",
            r#"{"kind": "string", "has": {"kind": "string_start", "regex": "^[fF]", "stopBy": "neighbor"}, "not": {"has": {"kind": "interpolation", "stopBy": "neighbor"}}}"#,
        ),
    ];
    let policy = format!(
        r#"{{"format": "rerail.policy/v1", "id": "seeded", "revision": 1, "mode": "steer", "description": "t45's lowerings.",
            "scope": {{"include": ["**/*"], "exclude": []}}, "clauses": [{}]}}"#,
        clauses.join(", ")
    );
    let compiled = CodeRuleDslRuntime::compile(policy, "json".to_owned(), CAPABILITIES.to_owned()).expect("the policy compiles");
    let tape = python_tape("import os\nx = 1\nprint(x)\n\ndef f(a, b=2, *c):\n    g(a, b)\n    return a\n\nclass K:\n    def m(self):\n        x\n\ns = f\"plain\"\nt = f\"{x}\"\nu = \"{x}\"\nv = f\"{1:>10}\"\n");
    (compiled, tape)
}

#[test]
fn seeded_and_copied_labels_find_ws_roots() {
    let (compiled, tape) = seeded_clauses();
    let root = Root::doc(tape.clone());
    let mut seen = BTreeSet::new();
    for (clause, grammar) in python_matchers(&compiled) {
        let program = LoweredMatcher::lower(&match_tree(grammar).expect("the clause lowers")).expect("the tree compiles");
        let mut work = usize::MAX;
        let expected = (0..tape.transducer_len() as u32)
            .filter(|&ordinal| grammar.match_root(node_at(&root, ordinal), &mut work).ok().flatten().is_some())
            .collect::<BTreeSet<_>>();
        assert!(!expected.is_empty(), "{clause}: W's matcher finds a root");
        assert_eq!(program.roots(&tape).unwrap(), expected, "{clause}");
        let census = program.census();
        let patterns = census.iter().filter(|(kind, _)| *kind == "pattern").count();
        // A neighbor's set is a formula a pass's anchor reads (0425), not a targets label of its own (0424's seed). A
        // reach (`inside … stopBy end`) is a targets label: the nodes below a match, which `in-function` copies.
        let targets = census.iter().filter(|(kind, _)| *kind == "targets").count();
        assert_eq!(targets, usize::from(clause == "in-function"), "{clause}: no seed label");
        let within = program.labels.defs.iter().any(|label| matches!(label, Label::Roots(_, super::compile_matcher::Domain::Within(..))));
        match clause {
            "named" => assert_eq!(patterns, 0, "named: `$X` is a node test, not a pattern pass"),
            "in-function" => assert!(program.label_copy().is_some(), "in-function: its roots copy the descendant label"),
            "top" => assert!(within, "top: the `not has` label starts within the module's children"),
            "named-function" | "function-name" | "lone" | "fstring" => {}
            other => panic!("an unexpected clause {other}"),
        }
        seen.insert(clause.to_owned());
    }
    assert_eq!(seen.len(), 7);
}

/// The same clauses on Metal: a label-domain pass starts only at the label's members, read from its row on the
/// device (`root_mask_label`), and a clause that copies a label reads its row (decision 0424).
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn seeded_and_copied_labels_match_on_metal() {
    use super::compile_matcher::Backend;
    let (compiled, tape) = seeded_clauses();
    let tapes = vec![tape.clone(), tape];
    for (clause, grammar) in python_matchers(&compiled) {
        let program = LoweredMatcher::lower(&match_tree(grammar).expect("the clause lowers")).expect("the tree compiles");
        let cpu = program.roots_batch(Backend::Cpu, &tapes).unwrap();
        let (metal, _) = program.roots_metal(&tapes).unwrap();
        assert_eq!(metal, cpu, "{clause}");
    }
}

/// Facts on Metal read their findings where the host wrote them, in the batch's fact block (Q251): rows with a slot
/// (a node and its target), rows with none, and a node set as bits (`nthChild`). Each finds W's roots, and the
/// device's are the CPU's.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn facts_on_metal_are_read_in_their_block() {
    use super::compile_matcher::Backend;
    let clause = |id: &str, rule: &str| {
        format!(
            r#"{{"id": "{id}", "forbid": {{"ast": {{"language": "python", "rule": {rule}}}}}, "violation": {{"anchor": "witness.span", "code": "X", "message": "x"}}}}"#
        )
    };
    let clauses = [
        clause("slot", r#"{"kind": "identifier", "binds": {"form": "assignment", "value": {"kind": "integer"}}}"#),
        clause("no-slot", r#"{"kind": "identifier", "binds": {"form": "assignment"}}"#),
        clause("bits", r#"{"kind": "identifier", "nthChild": 1}"#),
    ];
    let policy = format!(
        r#"{{"format": "rerail.policy/v1", "id": "facts", "revision": 1, "mode": "steer", "description": "Q251's facts.",
            "scope": {{"include": ["**/*"], "exclude": []}}, "clauses": [{}]}}"#,
        clauses.join(", ")
    );
    let compiled = CodeRuleDslRuntime::compile(policy, "json".to_owned(), CAPABILITIES.to_owned()).expect("the policy compiles");
    let tapes = vec![
        python_tape("x = 1\ny = \"s\"\nz = 2\nprint(x, y)\n"),
        python_tape("def f(a, b):\n    c = 3\n    d = a\n    return c\n"),
    ];
    let mut seen = BTreeSet::new();
    for (clause, grammar) in python_matchers(&compiled) {
        let program = LoweredMatcher::lower(&match_tree(grammar).expect("the clause lowers")).expect("the tree compiles");
        assert!(program.labels.defs.iter().any(|label| matches!(label, Label::Fact(..))), "{clause}: a fact label");
        let cpu = program.roots_batch(Backend::Cpu, &tapes).unwrap();
        assert!(cpu.iter().all(|roots| !roots.is_empty()), "{clause}: roots on every tape");
        let (metal, _) = program.roots_metal(&tapes).unwrap();
        assert_eq!(metal, cpu, "{clause}");
        seen.insert(clause.to_owned());
    }
    assert_eq!(seen.len(), 3);
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

/// Decision 0365: the wave's walks started within their anchors (the root walks, and the label passes' walks in the
/// levels past the recursive groups) find what they find without them, on the CPU, for every clause on the fixtures
/// (10 unless `RERAIL_S1_FILES` says): the same roots, and the same (clause, tape) given to W. Prints the starts the
/// anchors kept and dropped and each way's time.
#[test]
fn anchored_walks_find_the_unanchored_roots() {
    use super::compile_matcher::Backend;
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(10);
    }
    let compiled = clauses();
    let lowered: Vec<_> = compiled
        .iter()
        .flat_map(|(_, compiled)| python_matchers(compiled).filter_map(|(_, grammar)| Some((grammar, LoweredMatcher::lower(&match_tree(grammar).ok()?).ok()?))).collect::<Vec<_>>())
        .collect();
    let chunks: Vec<_> = lowered.chunks(16).collect();
    let waves: Vec<LoweredWave> = chunks.iter().map(|chunk| LoweredWave::new(&chunk.iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap()).collect();
    let run = |mode: &str| {
        // SAFETY: nextest runs each test in a process of its own.
        unsafe { std::env::set_var("RERAIL_ANCHORS", mode) };
        drop(std::mem::take(&mut *crate::code_rule_dsl::prof::stages()));
        let cpu = || {
            // SAFETY: getrusage writes the struct it is given.
            let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
            let time = |t: libc::timeval| std::time::Duration::from_micros(t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64);
            time(usage.ru_utime) + time(usage.ru_stime)
        };
        let cpu_before = cpu();
        let started = std::time::Instant::now();
        let found: Vec<_> = waves
            .iter()
            .map(|wave| {
                let active = vec![vec![true; tapes.len()]; wave.clauses.len()];
                wave.roots_batch(Backend::Cpu, &tapes, &active).map(|(roots, run)| (roots, run.limited)).unwrap()
            })
            .collect();
        let elapsed = (started.elapsed(), cpu() - cpu_before);
        // The counters count where `RERAIL_PROF` is set.
        let counts = std::mem::take(&mut *crate::code_rule_dsl::prof::stages());
        let count = |name: &str| counts.get(name).map_or(0, |&(_, calls)| calls);
        let mut stages: Vec<_> = counts.iter().filter(|(_, (time, _))| !time.is_zero()).collect();
        stages.sort_by_key(|(_, (time, _))| std::cmp::Reverse(*time));
        for (stage, (time, calls)) in stages.iter().take(14) {
            eprintln!("  {mode:>3} {stage:<36} {:>9.1} ms CPU {calls:>8} calls", time.as_secs_f64() * 1e3);
        }
        for (stage, (_, ms)) in counts.iter().filter(|(stage, _)| stage.starts_with("cpu_ms.")) {
            eprintln!("  {mode:>3} {stage:<36} {ms:>9} ms process CPU");
        }
        (found, elapsed, count("anchor.kept"), count("anchor.dropped"), count("sweep.anchor_dropped"))
    };
    let (unanchored, plain, _, _, _) = run("off");
    let (anchored, with, kept, dropped, swept) = run("on");
    unsafe { std::env::remove_var("RERAIL_ANCHORS") };
    // A run's findings: its roots, or W's matcher at its candidates where it gave the clause to W.
    let settled = |found: &(Vec<Vec<BTreeSet<u32>>>, Vec<(usize, usize)>), w: usize, c: usize, t: usize| -> BTreeSet<u32> {
        if !found.1.contains(&(c, t)) {
            return found.0[c][t].clone();
        }
        let root = Root::doc(tapes[t].clone());
        let mut work = usize::MAX;
        found.0[c][t].iter().copied().filter(|&n| chunks[w][c].0.match_root(node_at(&root, n), &mut work).ok().flatten().is_some()).collect()
    };
    let (mut differ, mut given) = (Vec::new(), (0, 0));
    for (w, (a, u)) in anchored.iter().zip(&unanchored).enumerate() {
        given.0 += a.1.len();
        given.1 += u.1.len();
        for c in 0..a.0.len() {
            for t in 0..tapes.len() {
                let (with, without) = (settled(a, w, c, t), settled(u, w, c, t));
                if with != without {
                    differ.push(format!("wave {w} clause {c} tape {t}: {} with, {} without", with.len(), without.len()));
                }
            }
        }
    }
    eprintln!(
        "anchors: {kept} starts kept, {dropped} dropped; {} waves on {} tapes: (wall, CPU) {plain:?} without, {with:?} with; given to W: {} with, {} without",
        waves.len(),
        tapes.len(),
        given.0,
        given.1
    );
    assert!(differ.is_empty(), "anchored walks differ: {differ:#?}");
    assert!(!crate::code_rule_dsl::prof::on() || dropped > 0, "the anchors dropped no start");
    // A recursive group's sweep starts within its passes' anchors, the group's own labels read as every node (Q133).
    eprintln!("the recursive groups' sweeps: {swept} starts outside their anchors");
    assert!(!crate::code_rule_dsl::prof::on() || swept > 0, "the recursive groups' anchors dropped no start");
}

/// Decision 0365 on Metal: the anchors evaluated on the device (`label_phase`), for the label passes past the
/// recursive groups, the wave's listed roots and the root records' walks, their text tests reading the walks' rows; and
/// the formula passes as device formulas (§9). With and without them (`RERAIL_ANCHORS`, `RERAIL_SETS=walk`), on Metal
/// both: the settled findings of the wave's roots are the same, and so are the root records from every node.
#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn on_metal_anchored_walks_find_the_unanchored_roots() {
    use super::compile_matcher::Backend;
    use crate::code_rule_dsl::transducer::packed::RootJobs;
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(6);
    }
    let compiled = clauses();
    let lowered: Vec<_> = compiled
        .iter()
        .flat_map(|(_, compiled)| python_matchers(compiled).filter_map(|(_, grammar)| Some((grammar, LoweredMatcher::lower(&match_tree(grammar).ok()?).ok()?))).collect::<Vec<_>>())
        .collect();
    // Every third wave of a dozen clauses: a few seconds a mode.
    let chunks: Vec<_> = lowered.chunks(12).step_by(3).collect();
    let waves: Vec<LoweredWave> = chunks.iter().map(|chunk| LoweredWave::new(&chunk.iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap()).collect();
    let every: Vec<Vec<u32>> = tapes.iter().map(|tape| (0..tape.transducer_len() as u32).collect()).collect();
    type Records = Vec<Option<Vec<BTreeSet<String>>>>;
    let run = |mode: &str| {
        // SAFETY: nextest runs each test in a process of its own.
        // Off: no anchors, and every formula pass walked (`RERAIL_SETS=walk`).
        unsafe { std::env::set_var("RERAIL_ANCHORS", mode) };
        unsafe { std::env::set_var("RERAIL_SETS", if mode == "off" { "walk" } else { "formulas" }) };
        drop(std::mem::take(&mut *crate::code_rule_dsl::prof::stages()));
        let started = std::time::Instant::now();
        let found: Vec<_> = waves
            .iter()
            .map(|wave| {
                let active = vec![vec![true; tapes.len()]; wave.clauses.len()];
                let roots = wave.roots_batch(Backend::MetalInterpreter, &tapes, &active).ok().map(|(roots, run)| (roots, run.limited, run.metal));
                // The root records from every node (as the engine's demand runs them, `waves::matches`).
                let programs = wave.programs();
                let jobs = RootJobs { programs: programs.iter().collect(), starts: vec![every.clone(); programs.len()] };
                let records: Option<Records> = wave.labelled_metal(&tapes, &active, Some(&jobs)).ok().map(|(_, outcome)| {
                    outcome
                        .records
                        .into_iter()
                        .map(|program| program.map(|files| files.into_iter().map(|records| records.into_iter().map(|record| format!("{record:?}")).collect()).collect()))
                        .collect()
                });
                (roots, records)
            })
            .collect();
        let elapsed = started.elapsed();
        // Under `RERAIL_PROF` (and `RERAIL_GPU_KERNEL_TIMES`, each kernel's device time): where the time went.
        let counts = std::mem::take(&mut *crate::code_rule_dsl::prof::stages());
        let mut kernels: Vec<_> = counts.iter().filter(|(name, _)| name.starts_with("kernel_us.") || name.starts_with("gpu.") || name.starts_with("formula.")).map(|(name, &(time, calls))| (name.clone(), time, calls)).collect();
        kernels.sort_by_key(|(_, time, calls)| std::cmp::Reverse((*time, *calls)));
        for (name, time, calls) in kernels.iter().take(30) {
            eprintln!("  {mode:>3} {name:<36} {:>9.1} ms {calls:>10}", time.as_secs_f64() * 1e3);
        }
        (found, elapsed)
    };
    // Warm: the pipelines and programs compiled before either timed run.
    let _ = run("on");
    let (unanchored, plain) = run("off");
    let (anchored, with) = run("on");
    unsafe { std::env::remove_var("RERAIL_ANCHORS") };
    unsafe { std::env::remove_var("RERAIL_SETS") };
    let settled = |found: &(Vec<Vec<BTreeSet<u32>>>, Vec<(usize, usize)>, bool), w: usize, c: usize, t: usize| -> BTreeSet<u32> {
        if !found.1.contains(&(c, t)) {
            return found.0[c][t].clone();
        }
        let root = Root::doc(tapes[t].clone());
        let mut work = usize::MAX;
        found.0[c][t].iter().copied().filter(|&n| chunks[w][c].0.match_root(node_at(&root, n), &mut work).ok().flatten().is_some()).collect()
    };
    let (mut differ, mut compared, mut roots, mut records) = (Vec::new(), 0, 0, 0);
    for (w, ((a, ar), (u, ur))) in anchored.iter().zip(&unanchored).enumerate() {
        if let (Some(a), Some(u)) = (a, u)
            && a.2
            && u.2
        {
            compared += 1;
            for c in 0..a.0.len() {
                for t in 0..tapes.len() {
                    let (with, without) = (settled(a, w, c, t), settled(u, w, c, t));
                    roots += without.len();
                    if with != without {
                        differ.push(format!("wave {w} clause {c} tape {t}: {} roots with, {} without", with.len(), without.len()));
                    }
                }
            }
        }
        if let (Some(a), Some(u)) = (ar, ur) {
            for (c, (a, u)) in a.iter().zip(u).enumerate() {
                let (Some(a), Some(u)) = (a, u) else { continue };
                records += u.iter().map(BTreeSet::len).sum::<usize>();
                for (t, (a, u)) in a.iter().zip(u).enumerate().filter(|(_, (a, u))| a != u) {
                    differ.push(format!("wave {w} program {c} tape {t}: {} records with, {} without", a.len(), u.len()));
                }
            }
        }
    }
    eprintln!("metal anchors: {compared} of {} waves compared on {} tapes, {roots} roots, {records} records; {plain:?} without, {with:?} with", waves.len(), tapes.len());
    assert!(compared * 2 > waves.len() && roots > 0 && records > 0, "most waves ran on Metal ({compared} of {}), with roots ({roots}) and records ({records})", waves.len());
    assert!(differ.is_empty(), "anchored walks differ on Metal: {differ:#?}");
}

/// The spike's anchor measure (prior work, ast-index-spike S151: anchor rows per answer, 1.05 there) for the wave:
/// over the fixtures (10 unless `RERAIL_S1_FILES` says), each clause's candidate roots (its domain past its entry,
/// where its root walk starts) against the roots it finds, the root walk's steps, and the rows its label passes
/// start at (each pass's domain). A census, not a check: it prints the totals and the clauses with the most
/// candidates per answer and the most steps.
/// The members of `set` on `tape`, node by node from what each test and step means: the reference the formula reader
/// (`compile_sets::Reader`) is checked against, whatever order or shortcut it evaluates in.
fn set_members(set: &super::compile_sets::Set, tape: &TapeDoc, labels: &[Vec<u64>], texts: &[Vec<u64>]) -> Vec<bool> {
    use super::compile_sets::{Set, Step};
    use crate::code_rule_dsl::tape::DERIVED_CLASS_BASE;
    let len = tape.transducer_len() as u32;
    let bit = |row: &[u64], node: u32| row.get(node as usize / 64).is_some_and(|word| word >> (node % 64) & 1 != 0);
    let named = |node: u32, named: bool| !named || tape.transducer_flags(node) & crate::code_rule_dsl::transducer::NAMED != 0;
    let each = |f: &dyn Fn(u32) -> bool| (0..len).map(f).collect::<Vec<_>>();
    let of = |set: &Set| set_members(set, tape, labels, texts);
    match set {
        Set::All => vec![true; len as usize],
        Set::Kind(kind) => each(&|node| tape.transducer_kind(node) == *kind),
        Set::Field(field) => each(&|node| tape.transducer_field(node) == *field),
        Set::Flags { mode, arg } => each(&|node| tape.transducer_flags(node) & mode == *mode && tape.transducer_flags(node) & arg == 0),
        Set::Class(class) if *class >= DERIVED_CLASS_BASE => each(&|node| labels.get(usize::from(class - DERIVED_CLASS_BASE)).is_some_and(|row| bit(row, node))),
        Set::Class(class) => each(&|node| tape.transducer_has_supertype(node, *class)),
        Set::Text(slot) => each(&|node| texts.get(usize::from(*slot)).is_some_and(|row| bit(row, node))),
        Set::And(parts) => parts.iter().map(of).fold(vec![true; len as usize], |a, b| a.iter().zip(b).map(|(a, b)| *a && b).collect()),
        Set::Or(parts) => parts.iter().map(of).fold(vec![false; len as usize], |a, b| a.iter().zip(b).map(|(a, b)| *a || b).collect()),
        Set::Child { named: n, of: inner } => {
            let inner = of(inner);
            each(&|node| tape.transducer_children(node).any(|child| inner[child as usize] && named(child, *n)))
        }
        Set::Descendant { named: n, of: inner } => {
            let inner = of(inner);
            each(&|node| tape.transducer_descendants(node).any(|below| inner[below as usize] && named(below, *n)))
        }
        Set::Following { named: n, of: inner } => {
            let inner = of(inner);
            each(&|node| {
                tape.transducer_parent(node).is_some_and(|parent| tape.transducer_children(parent).skip_while(|&c| c != node).skip(1).any(|c| inner[c as usize] && named(c, *n)))
            })
        }
        Set::Parent(inner) => {
            let inner = of(inner);
            each(&|node| tape.transducer_parent(node).is_some_and(|parent| inner[parent as usize]))
        }
        Set::Chain { named: n, through, to } => {
            let (through, to) = (of(through), of(to));
            // A node with a child that ends the chain: in `to`, or in `through` with such a child itself.
            let mut reach = vec![false; len as usize];
            for node in (0..len).rev() {
                reach[node as usize] = tape.transducer_children(node).any(|c| named(c, *n) && (to[c as usize] || (through[c as usize] && reach[c as usize])));
            }
            reach
        }
        Set::Reached { step, named: n, from } => {
            let from = of(from);
            match step {
                Step::Child => each(&|node| named(node, *n) && tape.transducer_parent(node).is_some_and(|p| from[p as usize])),
                Step::Descendant => each(&|node| {
                    named(node, *n) && std::iter::successors(tape.transducer_parent(node), |&p| tape.transducer_parent(p)).any(|a| from[a as usize])
                }),
                Step::Parent => each(&|node| tape.transducer_children(node).any(|c| from[c as usize])),
                Step::Following => each(&|node| {
                    named(node, *n) && tape.transducer_parent(node).is_some_and(|p| tape.transducer_children(p).take_while(|&c| c != node).any(|c| from[c as usize]))
                }),
                Step::Chain(through) => {
                    let through = of(through);
                    let mut reached = vec![false; len as usize];
                    for node in 0..len {
                        reached[node as usize] = named(node, *n)
                            && tape.transducer_parent(node).is_some_and(|p| from[p as usize] || (reached[p as usize] && through[p as usize]));
                    }
                    reached
                }
            }
        }
    }
}

/// The formula reader's sets are their members (`set_members`): each label program's anchor, its anchor with its
/// recursive group's labels read as every node (Q133), and its formula where it is one, on real tapes with their
/// labels settled. The reader tests a conjunction's steps within its tests and keeps the steps over every node; the
/// members are the same.
#[test]
fn formula_rows_are_the_sets_members() {
    use super::compile_sets::{Index, Reader, Set};
    let mut tapes = fixtures();
    tapes.truncate(3);
    let compiled = clauses();
    let mut lowered = Vec::new();
    for (_, compiled) in &compiled {
        for (_, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                lowered.push(matcher);
            }
        }
    }
    let (mut sets, mut differ) = (0, Vec::new());
    for matchers in lowered.chunks(16) {
        let wave = LoweredWave::new(&matchers.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; matchers.len()];
        let Ok(views) = wave.labelled(&tapes, &active) else { continue };
        for (t, view) in views.iter().enumerate() {
            let index = Index::default();
            let labels = view.transducer_labels();
            for clause in &wave.clauses {
                for (at, def) in clause.labels.defs.iter().enumerate() {
                    let (Label::Roots(program, _) | Label::Targets(program, _) | Label::Count { program, .. }) = def else { continue };
                    let Some(walk) = super::compile_micro::Micro::of(program) else { continue };
                    let texts = walk.texts(program, view).unwrap_or_default();
                    let id = (crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as usize + clause.labels.offset() + at) as u16;
                    let group = clause.labels.group_of(id).unwrap_or(&[]);
                    let unknown = |label: u16| group.contains(&(crate::code_rule_dsl::tape::DERIVED_CLASS_BASE + label));
                    let anchor = Set::anchor(&walk.words, &|slot| usize::from(slot) < texts.len());
                    for set in [Some(anchor.clone()), Some(anchor.labels_unknown(&unknown)), Set::of(&walk.words)].into_iter().flatten() {
                        let read = Reader { tape: view, index: &index, labels, texts: &texts }.eval(&set);
                        let want = set_members(&set, view, labels, &texts);
                        sets += 1;
                        if let Some(node) = (0..want.len()).find(|&node| want[node] != (read[node / 64] >> (node % 64) & 1 != 0)) {
                            differ.push(format!("tape {t} node {node}: {set:?}"));
                        }
                    }
                }
            }
        }
    }
    eprintln!("formula rows: {sets} sets, {} differ", differ.len());
    assert!(sets > 1000, "the clauses' label programs give sets ({sets})");
    assert!(differ.is_empty(), "{} sets differ from their members: {:?}", differ.len(), &differ[..differ.len().min(5)]);
}

/// The reads of recursive utilities in `tree` (`Recur`), and those not under a move (`Walk`, `Count`, a fact's slot or
/// a constraint, each tested at another node): `counts`' two halves.
fn unmoved_recurs(tree: &crate::code_rule_dsl::matcher::lower::MatchTree, moved: bool, counts: &mut (usize, usize)) {
    use crate::code_rule_dsl::matcher::lower::MatchTree as T;
    match tree {
        T::Recur(_) => {
            counts.0 += 1;
            counts.1 += usize::from(!moved);
        }
        T::All(trees) | T::Any(trees) => trees.iter().for_each(|tree| unmoved_recurs(tree, moved, counts)),
        T::Not(tree) | T::Recursive(_, tree) => unmoved_recurs(tree, moved, counts),
        T::Walk { rule, .. } | T::Count { rule, .. } => unmoved_recurs(rule, true, counts),
        T::Constrained(_, constraints) => constraints.iter().for_each(|(_, tree)| unmoved_recurs(tree, true, counts)),
        T::Fact(fact) => fact.slots.iter().for_each(|tree| unmoved_recurs(tree, true, counts)),
        T::Kind(_) | T::Supertype(..) | T::Pattern(_) | T::Regex(_) | T::Never => {}
    }
}

#[test]
fn anchor_rows_per_answer() {
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(10);
    }
    let (compiled, mut lowered, mut names) = (clauses(), Vec::new(), Vec::new());
    let mut recursions = (0_usize, 0_usize);
    for (policy, compiled) in &compiled {
        for (clause, grammar) in python_matchers(compiled) {
            if let Ok(tree) = match_tree(grammar)
                && let Ok(matcher) = LoweredMatcher::lower(&tree)
            {
                unmoved_recurs(&tree, false, &mut recursions);
                lowered.push(matcher);
                names.push(format!("{policy}/{clause}"));
            }
        }
    }
    // Q133 (a): a recursive utility's label narrowed by the kinds wanted where it is read (0390) is narrowed only where
    // no read of it is under a move (a move wants every kind at the node it reaches).
    eprintln!("recursions: {} reads, {} of them not under a move", recursions.0, recursions.1);
    let nodes: usize = tapes.iter().map(TapeDoc::transducer_len).sum();
    // Per clause: candidates, roots, root-walk steps, label rows, label passes, anchored candidates.
    let mut per: Vec<(String, u64, u64, u64, u64, usize, u64)> = Vec::new();
    let names_py = super::rir::Names::of(GrammarDialect::Python);
    // By (label kind, route): passes, starts, anchored starts, answers.
    let mut by_route = BTreeMap::<(&str, &str), (u64, u64, u64, u64)>::new();
    let mut passes = BTreeMap::<String, (u64, u64, u64, String, &str, String)>::new();
    // The anchor of a walk's program, evaluated on a tape: its members (`Set::anchor`, decision 0365).
    // A recursive group's pass reads its group's labels as every node (`unknown`): its anchor holds whatever they
    // settle to, so one evaluation before the group's rounds serves every round.
    let anchored_with = |program: &super::Program, view: &TapeDoc, index: &super::compile_sets::Index, unknown: &dyn Fn(u16) -> bool| -> Vec<u64> {
        use super::compile_sets::{Reader, Set};
        let walk = super::compile_micro::Micro::of(program);
        let texts = walk.as_ref().and_then(|walk| walk.texts(program, view));
        let set = match (&walk, &texts) {
            (Some(walk), Some(_)) => Set::anchor(&walk.words, &|_| true),
            _ => Set::anchor(&program.words, &|_| false),
        }
        .labels_unknown(unknown);
        let texts = texts.unwrap_or_default();
        Reader { tape: view, index, labels: view.transducer_labels(), texts: &texts }.eval(&set)
    };
    let anchored = |program: &super::Program, view: &TapeDoc, index: &super::compile_sets::Index| anchored_with(program, view, index, &|_| false);

    let get = |bits: &[u64], node: u32| bits.get(node as usize / 64).is_some_and(|w| w >> (node % 64) & 1 != 0);
    for (matchers, names) in lowered.chunks(16).zip(names.chunks(16)) {
        let wave = LoweredWave::new(&matchers.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; matchers.len()];
        let Ok(views) = wave.labelled(&tapes, &active) else { continue };
        let indexes: Vec<super::compile_sets::Index> = views.iter().map(|_| Default::default()).collect();
        for (clause, name) in wave.clauses.iter().zip(names) {
            let entry = Entry::of(&clause.program);
            let (mut candidates, mut roots, mut steps, mut label_rows, mut anchor_rows) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
            for (view, index) in views.iter().zip(&indexes) {
                let sets = view.transducer_labels();
                let starts: Vec<u32> = domain(&clause.domain, view, sets).filter(|&n| entry.admits(view, n)).collect();
                candidates += starts.len() as u64;
                let anchor = anchored(&clause.program, view, index);
                anchor_rows += starts.iter().filter(|&&n| get(&anchor, n)).count() as u64;
                if let Ok((outputs, used)) = clause.program.execute_roots_budgeted(view, starts.iter().copied(), usize::MAX) {
                    let found = outputs.iter().map(|o| o.start).collect::<BTreeSet<_>>();
                    // The anchor is a superset: every root is in it.
                    assert!(found.iter().all(|&n| get(&anchor, n)), "{name}: a root outside its anchor");
                    roots += found.len() as u64;
                    steps += used as u64;
                }
                for (at, def) in clause.labels.defs.iter().enumerate() {
                    // The recursive group the label is in, if any: its step of the clause's labels.
                    let id = (crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as usize + clause.labels.offset() + at) as u16;
                    let group = clause.labels.group_of(id);
                    let (kind, program, d) = match def {
                        Label::Roots(p, d) => ("roots", p, d),
                        Label::Targets(p, d) => ("targets", p, d),
                        Label::Count { program, domain: d, .. } => ("count", program, d),
                        Label::Pattern(p, d) | Label::Constrained(p, d, _) => ("pattern", p, d),
                        _ => continue,
                    };
                    let walk = super::compile_micro::Micro::of(program);
                    let formula = match (def, &walk) {
                        (Label::Roots(..), Some(walk)) => super::compile_sets::Set::of(&walk.words).is_some(),
                        (Label::Targets(..), Some(walk)) => super::compile_sets::Set::targets(&walk.words, super::compile_sets::Set::All).is_some(),
                        _ => false,
                    };
                    let route = match (kind, formula, walk.is_some(), group.is_some()) {
                        ("pattern", ..) => "pattern",
                        (_, true, _, true) => "group, formula",
                        (.., true) if walk.is_some() => "group, micro walk",
                        (.., true) => "group, interpreter",
                        (_, true, ..) => "formula",
                        (_, false, true, _) => "micro walk",
                        _ => "interpreter",
                    };
                    let pass_entry = Entry::of(program);
                    let pass_starts: Vec<u32> = domain(d, view, sets).filter(|&n| pass_entry.admits(view, n)).collect();
                    let rows = pass_starts.len() as u64;
                    let answers = sets.get(clause.labels.offset() + at).map_or(0, |bits| bits.iter().map(|w| u64::from(w.count_ones())).sum());
                    // Anchored starts where a pass walks from each start (a roots, targets or count walk).
                    let anchor = if let Some(group) = group.filter(|_| matches!(def, Label::Roots(..) | Label::Targets(..) | Label::Count { .. })) {
                        // The group's anchor (its labels unknown): no start outside it emits with the labels settled,
                        // nor in any round before (it reads none of them).
                        let anchor = anchored_with(program, view, index, &|label| group.contains(&((crate::code_rule_dsl::tape::DERIVED_CLASS_BASE as usize + usize::from(label)) as u16)));
                        let outside: Vec<u32> = pass_starts.iter().copied().filter(|&n| !get(&anchor, n)).collect();
                        if let Ok((outputs, _)) = program.execute_roots_budgeted(view, outside.iter().copied(), usize::MAX) {
                            assert!(outputs.is_empty(), "{name}: a recursive group's start outside its anchor emitted");
                        }
                        // The same pass's anchor reading its group's settled labels: what a per-round anchor could
                        // drop at best.
                        let settled = anchored(program, view, index);
                        let r = by_route.entry((kind, "group, settled labels")).or_default();
                        r.0 += 1;
                        r.1 += rows;
                        r.2 += pass_starts.iter().filter(|&&n| get(&settled, n)).count() as u64;
                        r.3 += answers;
                        pass_starts.iter().filter(|&&n| get(&anchor, n)).count() as u64
                    } else if route != "formula" && route != "pattern" && matches!(def, Label::Roots(..) | Label::Targets(..) | Label::Count { .. }) {
                        let anchor = anchored(program, view, index);
                        if matches!(def, Label::Count { .. }) {
                            // A count's tally is over every node, so an unwalked start must count 0: no start outside the
                            // anchor emits.
                            let outside = pass_starts.iter().copied().filter(|&n| !get(&anchor, n));
                            if let Ok((outputs, _)) = program.execute_roots_budgeted(view, outside, usize::MAX) {
                                assert!(outputs.is_empty(), "{name}: a count's start outside its anchor emitted");
                            }
                        }
                        pass_starts.iter().filter(|&&n| get(&anchor, n)).count() as u64
                    } else {
                        rows
                    };
                    if route != "formula" {
                        label_rows += rows;
                    }
                    let r = by_route.entry((kind, route)).or_default();
                    r.0 += 1;
                    r.1 += rows;
                    r.2 += anchor;
                    r.3 += answers;
                    if route != "formula" {
                        let pass = passes.entry(program.plain_digest().to_owned()).or_insert_with(|| (0, 0, 0, name.clone(), route, super::rir::disassemble(program, Some(&names_py))));
                        pass.0 += rows;
                        pass.1 += anchor;
                        pass.2 += answers;
                    }
                }
            }
            per.push((name.clone(), candidates, roots, steps, label_rows, clause.labels.defs.len(), anchor_rows));
        }
    }
    let total = |f: fn(&(String, u64, u64, u64, u64, usize, u64)) -> u64| per.iter().map(f).sum::<u64>();
    let (candidates, roots, steps, label_rows, anchor_rows) = (total(|p| p.1), total(|p| p.2), total(|p| p.3), total(|p| p.4), total(|p| p.6));
    eprintln!("anchored root walks: {anchor_rows} starts for {roots} roots ({:.2} a root), of {candidates}", anchor_rows as f64 / roots.max(1) as f64);
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
        eprintln!("  candidates a root {:>8.1}: {} ({} candidates, {} anchored, {} roots, {} steps)", p.1 as f64 / p.2 as f64, p.0, p.1, p.6, p.2, p.3);
    }
    let mut by_steps: Vec<_> = per.iter().collect();
    by_steps.sort_by_key(|p| std::cmp::Reverse(p.3 + p.4));
    for p in by_steps.iter().take(10) {
        eprintln!("  steps {:>10} + label rows {:>8} ({} passes): {} ({} candidates, {} roots)", p.3, p.4, p.5, p.0, p.1, p.2);
    }
    for ((kind, route), (n, rows, anchor, answers)) in &by_route {
        eprintln!("  label passes {kind:>8} {route:>11}: {n:>6} passes, {rows:>10} starts, {anchor:>10} anchored, {answers:>9} answers");
    }
    let mut heavy: Vec<_> = passes.values().collect();
    heavy.sort_by_key(|p| std::cmp::Reverse(p.0));
    eprintln!("label passes outside the formulas: {} distinct programs", heavy.len());
    for p in heavy.iter().take(12) {
        eprintln!("== {} starts, {} anchored, {} answers, {} ({}):\n{}", p.0, p.1, p.2, p.3, p.4, p.5);
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
    // A loop of child steps through a test is one move (`compile_micro::CLOSURE`, Q56): some fixtures' walks hold one.
    let closures = micro.iter().flatten().filter(|(m, _)| m.words.iter().any(|&w| split(w).0 == compile_micro::CLOSURE)).count();
    eprintln!("micro: {closures} of the walks read a closure");
    assert!(closures > 0, "a walk reads a closure");
    let texts = micro.iter().flatten().filter(|(m, _)| !m.texts.is_empty()).count();
    eprintln!("micro: {texts} of the walks test text; {counted} of the programs count");
    eprintln!("micro: {eligible} of {} label programs; {compared} starts compared, {marked} marks, {overflowed} overflowed", programs.len());
    assert!(eligible * 2 > programs.len(), "most label programs are micro walks");
    assert!(compared > 100_000 && overflowed * 100 < compared);
}

/// Whether a walk's first move, reached from its start through saves, forks and jumps only, is `after` with a tail
/// that marks the start alone (tests, a `load` and its tests, a capture, the emit): the shape the device answers once
/// for a parent's children (`label_passes.uglykl`'s `sibling_pass`, work queue Q182).
fn sibling_shape(words: &[u32]) -> bool {
    let (mut pc, mut seen) = (0, 0);
    while pc < words.len() && seen <= words.len() {
        seen += 1;
        let (op, _, arg) = split(words[pc]);
        match op {
            0x15 | 0x30 => pc += 1,
            0x31 => pc = arg as usize,
            0x14 => break,
            _ => return false,
        }
    }
    let test = |op: u8| matches!(op, 0x01 | 0x02 | 0x03 | 0x04 | 0x0a);
    let mut at = pc + 1;
    while at < words.len() && test(split(words[at]).0) {
        at += 1;
    }
    if at >= words.len() || split(words[at]).0 != 0x16 {
        return at < words.len() && split(words[at]).0 == 0x40;
    }
    at += 1;
    while at < words.len() && test(split(words[at]).0) {
        at += 1;
    }
    if at < words.len() && split(words[at]).0 == 0x20 {
        at += 1;
    }
    at < words.len() && split(words[at]).0 == 0x40
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
    let (mut found, mut differ, mut waves, mut shared) = (0, Vec::new(), 0, 0);
    for (at, clauses) in lowered.chunks(12).enumerate() {
        let wave = LoweredWave::new(&clauses.iter().collect::<Vec<_>>()).unwrap();
        shared += wave.label_programs().iter().filter_map(|(program, _)| Micro::of(program)).filter(|walk| sibling_shape(&walk.words)).count();
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
    eprintln!("spilled: {waves} waves of {} clauses, {found} roots, {shared} walks of a shared sibling scan", lowered.len());
    assert!(shared > 0, "some walk's sibling scan is shared (the device's `sibling_pass`)");
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
    assert_eq!(got, "2f1444d906c39b0c99a4e20b5b9a56cb", "the captures moved ({matched} matches)");
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


/// A tape's presence bitmap (the codes it holds, in its header): of every (program, file) pair, how many the file's
/// kinds and fields alone rule out before any label is known (`possible`), against how many have an empty anchor
/// with every label read as every node (what any file-level test could rule out at most), by pairs and by the
/// nodes a pass over those files would read.
#[test]
#[ignore = "a census, run by name"]
fn presence_bitmap_census() {
    use super::compile_sets::Set;
    use std::collections::HashSet;
    fn possible(set: &Set, kinds: &HashSet<u16>, fields: &HashSet<u16>) -> bool {
        match set {
            Set::Kind(k) => kinds.contains(k),
            Set::Field(f) => fields.contains(f),
            Set::And(parts) => parts.iter().all(|p| possible(p, kinds, fields)),
            Set::Or(parts) => parts.iter().any(|p| possible(p, kinds, fields)),
            Set::Child { of, .. } | Set::Descendant { of, .. } | Set::Parent(of) | Set::Following { of, .. } | Set::Reached { from: of, .. } => possible(of, kinds, fields),
            Set::Chain { to, .. } => possible(to, kinds, fields),
            Set::All | Set::Flags { .. } | Set::Class(_) | Set::Text(_) => true,
        }
    }
    let mut tapes = fixtures();
    if std::env::var_os("RERAIL_S1_FILES").is_none() {
        tapes.truncate(20);
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
    let present: Vec<(HashSet<u16>, HashSet<u16>)> = tapes
        .iter()
        .map(|t| (0..t.transducer_len() as u32).map(|n| (t.transducer_kind(n), t.transducer_field(n))).unzip())
        .collect();
    let distinct: Vec<usize> = present.iter().map(|(k, _)| k.len()).collect();
    eprintln!("presence: {} files, {} nodes, kinds a file: min {} median {} max {}", tapes.len(), tapes.iter().map(TapeDoc::transducer_len).sum::<usize>(), distinct.iter().min().unwrap_or(&0), {
        let mut d = distinct.clone();
        d.sort_unstable();
        d.get(d.len() / 2).copied().unwrap_or(0)
    }, distinct.iter().max().unwrap_or(&0));
    // (role) -> pairs, pairs ruled out by presence, pairs with an empty anchor; the same weighted by the file's nodes.
    let mut counts = BTreeMap::<&str, [u64; 6]>::new();
    for chunk in lowered.chunks(16) {
        let wave = LoweredWave::new(&chunk.iter().collect::<Vec<_>>()).unwrap();
        let active = vec![vec![true; tapes.len()]; chunk.len()];
        let Ok(views) = wave.labelled(&tapes, &active) else { continue };
        let indexes: Vec<super::compile_sets::Index> = views.iter().map(|_| Default::default()).collect();
        for clause in &wave.clauses {
            let mut programs = vec![("clause", &clause.program)];
            for def in &clause.labels.defs {
                match def {
                    Label::Roots(p, _) | Label::Targets(p, _) | Label::Pattern(p, _) | Label::Constrained(p, _, _) => programs.push(("label", p)),
                    Label::Count { program, .. } => programs.push(("label", program)),
                    Label::Complement(_) | Label::Fact(..) | Label::Empty => {}
                }
            }
            for (role, program) in programs {
                let walk = super::compile_micro::Micro::of(program);
                for ((view, index), (kinds, fields)) in views.iter().zip(&indexes).zip(&present) {
                    let texts = walk.as_ref().and_then(|walk| walk.texts(program, view));
                    let set = match (&walk, &texts) {
                        (Some(walk), Some(_)) => Set::anchor(&walk.words, &|_| true),
                        _ => Set::anchor(&program.words, &|_| false),
                    }
                    .labels_unknown(&|_| true);
                    let texts = texts.unwrap_or_default();
                    let empty = super::compile_sets::Reader { tape: view, index, labels: view.transducer_labels(), texts: &texts }.eval(&set).iter().all(|w| *w == 0);
                    let ruled_out = !possible(&set, kinds, fields);
                    assert!(!ruled_out || empty, "a set ruled out by presence has members");
                    let nodes = view.transducer_len() as u64;
                    let c = counts.entry(role).or_default();
                    c[0] += 1;
                    c[1] += u64::from(ruled_out);
                    c[2] += u64::from(empty);
                    c[3] += nodes;
                    c[4] += if ruled_out { nodes } else { 0 };
                    c[5] += if empty { nodes } else { 0 };
                }
            }
        }
    }
    for (role, c) in &counts {
        let pct = |a: u64, b: u64| 100.0 * a as f64 / b.max(1) as f64;
        eprintln!(
            "presence: {role:<6} {} pairs: ruled out by presence {} ({:.1}%), empty anchor {} ({:.1}%); by nodes: {:.1}% and {:.1}%",
            c[0], c[1], pct(c[1], c[0]), c[2], pct(c[2], c[0]), pct(c[4], c[3]), pct(c[5], c[3])
        );
    }
}

/// The end of children meets a named candidate (RER-131): where the tokens the tape leaves out after
/// the last matched child begin with the goal's, the rest trails, in the packed matcher as in
/// ast-grep-core's. `case $V:` matches `case 1: f(); break;` (ast-grep 0.51.0 does); `f($A)` still
/// refuses `f(a, b)`, whose gap after `a` is `,`.
/// PR #508's 65ff68df5 (t47).
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
