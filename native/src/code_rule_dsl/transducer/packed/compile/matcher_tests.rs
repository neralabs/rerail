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
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    // Every fourth fixture keeps the gate in seconds; RERAIL_S1_ALL=1 runs them all.
    let step = if std::env::var_os("RERAIL_S1_ALL").is_some() { 1 } else { 4 };
    let tapes = sources
        .iter()
        .step_by(step)
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|source| python_tape(&source))
        .collect::<Vec<_>>();
    assert!(tapes.len() > 50, "the fixtures are in git: {}", tapes.len());
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
