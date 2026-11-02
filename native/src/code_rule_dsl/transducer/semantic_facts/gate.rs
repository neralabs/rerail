//! The semantic producers' differential gates (O24): for every form of a predicate the Python
//! pack uses, on every fixture (a sample by default; all in the `#[ignore]`d runs) and at every
//! node, the label a producer's rows give equals W's verdicts for that form, under no module path
//! and inside a package. A form the rows do not answer is counted as refused, by name.

use super::binds::{self, Verdict};
use super::harness::{
    clause_of, file_evaluation, fixtures, modules, policies, python_matchers, semantic_leaves,
    verdict_at, verdicts,
};
use super::resolves::{self, Kinds};
use super::scopes::{Scopes, reading};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, Resolves, Semantic};
use crate::code_rule_dsl::scope::Reading;
use ast_grep_core::AstGrep as Root;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// A form's parameters, which tell forms apart (two clauses may share one).
fn resolves_key(form: &Resolves) -> String {
    format!(
        "globals={:?} imports={:?} imported={:?} prefix={} relative_text={} global_any={} \
         import_any={} unresolved={:?} before_local={} seen_before={} declared={} python={:?} \
         typing={} kinds={:?}/{:?}/{:?}/{:?}",
        form.globals,
        form.imports,
        form.imported,
        form.prefix,
        form.relative_text,
        form.global_any,
        form.import_any,
        form.unresolved,
        form.before_local,
        form.seen_before,
        form.declared,
        form.python.as_ref().map(|python| python.minor),
        form.python
            .as_ref()
            .is_some_and(|python| python.typing.is_some()),
        form.reference_kinds,
        form.attribute_kinds,
        form.dotted_kinds,
        form.string_kinds,
    )
}

/// Each distinct form of `pick`'s predicate in the pack, with a clause that reads it.
fn forms<T>(
    pick: impl Fn(&Semantic) -> Option<T>,
    key: impl Fn(&T) -> String,
) -> Vec<(String, T, GrammarMatcher)> {
    let mut found = BTreeMap::new();
    for (policy, compiled) in &policies() {
        for (clause, grammar) in python_matchers(compiled) {
            for leaf in semantic_leaves(&grammar.matcher) {
                if let Some(form) = pick(&leaf) {
                    found
                        .entry(key(&form))
                        .or_insert_with(|| (format!("{policy}/{clause}"), form, grammar.clone()));
                }
            }
        }
    }
    found.into_values().collect()
}

#[derive(Default)]
struct Tally {
    forms: usize,
    refused: BTreeMap<&'static str, usize>,
    checked: usize,
    nodes: usize,
    matched: usize,
    unknown: usize,
    rows: usize,
    documents: usize,
    produce: Duration,
    label: Duration,
    oracle: Duration,
    differences: Vec<String>,
}

impl Tally {
    fn compare(&mut self, what: &str, expected: &BTreeSet<u32>, actual: &BTreeSet<u32>) {
        self.matched += expected.len();
        if expected != actual {
            let missing = expected.difference(actual).count();
            let extra = actual.difference(expected).count();
            let first = expected.symmetric_difference(actual).next();
            self.differences.push(format!(
                "{what}: {missing} missing, {extra} extra, first at node {first:?}"
            ));
        }
    }

    fn report(&self, predicate: &str, fixtures: usize) {
        eprintln!(
            "{predicate}: {} forms ({} refused {:?}) on {fixtures} fixtures × 2 module paths: {} \
             (form, document) pairs, {} nodes, {} matched, {} unknown, {} differences; producer \
             {} rows on {} documents, {:?} ({:?}/document), labels {:?}, W {:?}",
            self.forms,
            self.refused.values().sum::<usize>(),
            self.refused,
            self.checked,
            self.nodes,
            self.matched,
            self.unknown,
            self.differences.len(),
            self.rows,
            self.documents,
            self.produce,
            self.produce / self.documents.max(1) as u32,
            self.label,
            self.oracle,
        );
        for difference in self.differences.iter().take(20) {
            eprintln!("  differs {difference}");
        }
    }
}

fn gate_resolves(step: usize, mutate: &dyn Fn(&mut resolves::ResolvesRows)) -> Tally {
    let forms = forms(
        |leaf| match leaf {
            Semantic::Resolves(form) => Some(form.clone()),
            _ => None,
        },
        |form| resolves_key(form),
    );
    let mut tally = Tally {
        forms: forms.len(),
        ..Tally::default()
    };
    let admitted = forms
        .iter()
        .filter(|(_, form, _)| match resolves::admits(form) {
            Ok(()) => true,
            Err(refused) => {
                *tally.refused.entry(refused.0).or_default() += 1;
                false
            }
        })
        .collect::<Vec<_>>();
    let kinds = Kinds::python();
    let tapes = fixtures(step);
    for (path, tape) in &tapes {
        for module in modules() {
            let _evaluation = file_evaluation(tape, module.as_ref());
            let root = Root::doc(tape.clone());
            tally.nodes += tape.transducer_len();
            let mut produced: Vec<(Reading, resolves::ResolvesRows)> = Vec::new();
            for (clause, form, grammar) in &admitted {
                let reading = reading(form.python.as_deref(), false);
                if !produced.iter().any(|(known, _)| *known == reading) {
                    let started = Instant::now();
                    let scopes =
                        Scopes::build(&root, reading, module.as_ref(), usize::MAX, &|| false)
                            .expect("the fixture's table builds");
                    let mut rows = resolves::rows(&root, &scopes, module.as_ref(), &kinds);
                    mutate(&mut rows);
                    tally.produce += started.elapsed();
                    tally.rows += rows.len();
                    tally.documents += 1;
                    produced.push((reading, rows));
                }
                let rows = &produced
                    .iter()
                    .find(|(known, _)| *known == reading)
                    .expect("produced")
                    .1;
                let started = Instant::now();
                let actual = resolves::label(form, rows, &root).expect("admitted");
                tally.label += started.elapsed();
                let started = Instant::now();
                let rule = AstMatcher::Semantic(Semantic::Resolves(form.clone()));
                let (expected, unknown) = verdicts(grammar, &rule, &root);
                tally.oracle += started.elapsed();
                tally.checked += 1;
                tally.unknown += unknown.len();
                tally.compare(
                    &format!(
                        "{clause} on {} (module {:?})",
                        path.display(),
                        module.as_ref().map(|module| module.text())
                    ),
                    &expected,
                    &actual,
                );
            }
        }
    }
    tally.report("resolves", tapes.len());
    tally
}

fn exact(tally: &Tally) {
    assert!(
        tally.differences.is_empty(),
        "{} (form, document) pairs differ from W",
        tally.differences.len()
    );
}

#[test]
fn resolves_rows_give_ws_verdicts_on_a_sample() {
    let tally = gate_resolves(8, &|_| {});
    exact(&tally);
    assert_eq!(tally.unknown, 0, "Python's `resolves` is never unknown");
}

#[test]
#[ignore = "every fixture: run with --ignored"]
fn resolves_rows_give_ws_verdicts_on_every_fixture() {
    let tally = gate_resolves(1, &|_| {});
    exact(&tally);
    assert_eq!(tally.unknown, 0, "Python's `resolves` is never unknown");
}

/// Rows that forget shadowing (every read's head unshadowed) must fail the gate.
#[test]
fn the_resolves_gate_catches_rows_that_forget_shadowing() {
    let tally = gate_resolves(32, &|rows| {
        rows.reads.iter_mut().for_each(|read| read.shadows = false)
    });
    assert!(
        !tally.differences.is_empty(),
        "the gate passed rows that forget shadowing"
    );
}

fn gate_binds(step: usize, mutate: &dyn Fn(&mut binds::BindsRows)) -> Tally {
    // A leaf's sub-rules tell its forms apart: every leaf is a form of its own.
    let forms = forms(
        |leaf| match leaf {
            Semantic::Binds(form) => Some(form.clone()),
            _ => None,
        },
        |form| format!("{:p}", &**form),
    );
    let mut tally = Tally {
        forms: forms.len(),
        ..Tally::default()
    };
    let admitted = forms
        .iter()
        .filter(|(_, form, _)| match binds::admits(form) {
            Ok(()) => true,
            Err(refused) => {
                *tally.refused.entry(refused.0).or_default() += 1;
                false
            }
        })
        .collect::<Vec<_>>();
    let tapes = fixtures(step);
    for (path, tape) in &tapes {
        for module in modules() {
            let _evaluation = file_evaluation(tape, module.as_ref());
            let root = Root::doc(tape.clone());
            tally.nodes += tape.transducer_len();
            let mut produced: Vec<(Reading, binds::BindsRows)> = Vec::new();
            for (clause, form, grammar) in &admitted {
                let reading = reading(form.python.as_deref(), false);
                if !produced.iter().any(|(known, _)| *known == reading) {
                    let started = Instant::now();
                    let scopes =
                        Scopes::build(&root, reading, module.as_ref(), usize::MAX, &|| false)
                            .expect("the fixture's table builds");
                    let mut rows = binds::rows(&root, &scopes);
                    mutate(&mut rows);
                    tally.produce += started.elapsed();
                    tally.rows += rows.len();
                    tally.documents += 1;
                    produced.push((reading, rows));
                }
                let rows = &produced
                    .iter()
                    .find(|(known, _)| *known == reading)
                    .expect("produced")
                    .1;
                // The sub-rules' labels, read where the join reaches them: W's verdicts there.
                let mut sub_rules = Vec::<(*const AstMatcher, _)>::new();
                let mut oracle = Duration::ZERO;
                let mut sub_rule = |rule: &AstMatcher, node: u32| {
                    let started = Instant::now();
                    let at = match sub_rules
                        .iter()
                        .position(|(known, _)| std::ptr::eq(*known, rule))
                    {
                        Some(at) => at,
                        None => {
                            sub_rules.push((rule as *const _, clause_of(grammar, rule)));
                            sub_rules.len() - 1
                        }
                    };
                    let verdict = match verdict_at(&sub_rules[at].1, &root, node) {
                        Some(true) => Verdict::True,
                        Some(false) => Verdict::False,
                        None => Verdict::Unknown,
                    };
                    oracle += started.elapsed();
                    verdict
                };
                let started = Instant::now();
                let (actual, actual_unknown) =
                    binds::label(form, rows, &mut sub_rule, &|_| None).expect("admitted");
                tally.label += started.elapsed() - oracle;
                tally.oracle += oracle;
                let started = Instant::now();
                let rule = AstMatcher::Semantic(Semantic::Binds(form.clone()));
                let (expected, unknown) = verdicts(grammar, &rule, &root);
                tally.oracle += started.elapsed();
                tally.checked += 1;
                tally.unknown += unknown.len();
                let what = format!(
                    "{clause} on {} (module {:?})",
                    path.display(),
                    module.as_ref().map(|module| module.text())
                );
                tally.compare(&what, &expected, &actual);
                tally.compare(&format!("{what}, unknown"), &unknown, &actual_unknown);
            }
        }
    }
    tally.report("binds", tapes.len());
    tally
}

#[test]
fn binds_rows_give_ws_verdicts_on_a_sample() {
    exact(&gate_binds(8, &|_| {}));
}

#[test]
#[ignore = "every fixture: run with --ignored"]
fn binds_rows_give_ws_verdicts_on_every_fixture() {
    exact(&gate_binds(1, &|_| {}));
}

/// Rows that read every binding as its name's latest must fail the gate.
#[test]
fn the_binds_gate_catches_rows_that_forget_rebinding() {
    let tally = gate_binds(32, &|rows| {
        rows.bindings
            .values_mut()
            .for_each(|binding| binding.latest = true)
    });
    assert!(
        !tally.differences.is_empty(),
        "the gate passed rows that forget rebinding"
    );
}
