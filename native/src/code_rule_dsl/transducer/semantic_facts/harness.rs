//! What the semantic producers' gates read: the Python pack's AST clauses, the semantic leaves in
//! them, and the pack's fixtures as tapes (as S1's gate reads them,
//! `transducer::packed::compile::matcher_tests`).

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, FileScopeSlot, GrammarMatcher, Semantic, node_at, stop_rule,
};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use ast_grep_core::AstGrep as Root;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn pack() -> PathBuf {
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

pub(super) fn python_tape(source: &str) -> TapeDoc {
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    TapeDoc::restore(source, grammar, &image, &|| false).expect("restored tape")
}

/// The pack's fixture paths, sorted: every `step`-th.
pub(super) fn fixture_paths(step: usize) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    files(&pack().join("upstream"), "py", &mut sources);
    sources.sort();
    sources.into_iter().step_by(step).collect()
}

/// The fixtures a gate reads, as (path, tape): every `step`-th.
pub(super) fn fixtures(step: usize) -> Vec<(PathBuf, TapeDoc)> {
    fixture_paths(step)
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).ok()?;
            let tape = python_tape(&source);
            Some((path, tape))
        })
        .collect()
}

/// The Python pack's policies (rules and candidates), compiled: (policy, compiled).
pub(super) fn policies() -> Vec<(String, crate::code_rule_dsl::CompiledCodeRuleDsl)> {
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    documents
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let compiled =
                CodeRuleDslRuntime::compile(text, "json".to_owned(), CAPABILITIES.to_owned())
                    .ok()?;
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some((name, compiled))
        })
        .collect()
}

/// A policy's Python `ast` matchers: (clause, matcher).
pub(super) fn python_matchers(
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

/// Visits every term `matcher` reaches, utilities and semantic sub-rules included, each utility
/// once (`seen`, by table and index).
pub(super) fn walk(
    matcher: &AstMatcher,
    seen: &mut BTreeSet<(usize, usize)>,
    visit: &mut dyn FnMut(&AstMatcher),
) {
    visit(matcher);
    let util = match matcher {
        AstMatcher::Util(util) => Some(util),
        AstMatcher::Call(call) => Some(&call.util),
        _ => None,
    };
    if let Some(util) = util {
        if seen.insert((util.table.as_ptr() as usize, util.index)) {
            util.with_target(|rule| walk(rule, seen, visit));
        }
        return;
    }
    for child in children(matcher) {
        walk(child, seen, visit);
    }
}

/// A term's direct sub-rules, a relation's stop rule and a semantic predicate's included;
/// utilities aside.
fn children(matcher: &AstMatcher) -> Vec<&AstMatcher> {
    match matcher {
        AstMatcher::Semantic(semantic) => match semantic {
            Semantic::Binding(test) => test.sub_rules().collect(),
            Semantic::Binds(test) => test.sub_rules().collect(),
            Semantic::Host(test) => vec![&test.rule],
            Semantic::Parse(test) => vec![&test.rule],
            Semantic::Embedded(test) => test.rules().collect(),
            Semantic::FormatArgs(test) => test.unused.iter().collect(),
            Semantic::Calls(_)
            | Semantic::Resolves(_)
            | Semantic::Value(_)
            | Semantic::Arguments(_)
            | Semantic::Capture(_)
            | Semantic::Layout(_)
            | Semantic::FilePath(_) => Vec::new(),
        },
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_)
        | AstMatcher::Supertype(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Never
        | AstMatcher::Compare(_)
        | AstMatcher::Util(_)
        | AstMatcher::Call(_) => Vec::new(),
        AstMatcher::TestUnknown(_) => Vec::new(),
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().collect(),
        AstMatcher::Not(item) => vec![&**item],
        AstMatcher::Inside(item, stop)
        | AstMatcher::Has(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            [item.rule()].into_iter().chain(stop_rule(stop)).collect()
        }
        AstMatcher::Field(relation) => [&relation.rule]
            .into_iter()
            .chain(stop_rule(&relation.stop))
            .collect(),
        AstMatcher::Count(count) => vec![&count.relation],
        AstMatcher::NthChild(nth) => nth.of_rule.iter().collect(),
        AstMatcher::Constrained(constrained) => [&constrained.rule]
            .into_iter()
            .chain(constrained.constraints.iter().map(|(_, rule)| rule))
            .collect(),
    }
}

/// Every semantic leaf `matcher` reaches (`walk`).
pub(super) fn semantic_leaves(matcher: &AstMatcher) -> Vec<Semantic> {
    let mut found = Vec::new();
    walk(matcher, &mut BTreeSet::new(), &mut |term| {
        if let AstMatcher::Semantic(semantic) = term {
            found.push(semantic.clone());
        }
    });
    found
}

/// `template`'s clause with `rule` as its rule, read three-valued: (the rule, its negation).
pub(super) fn clause_of(
    template: &GrammarMatcher,
    rule: &AstMatcher,
) -> (GrammarMatcher, GrammarMatcher) {
    let mut positive = template.clone();
    positive.matcher = rule.clone();
    positive.three_valued = true;
    positive.transform = None;
    let mut negative = positive.clone();
    negative.matcher = AstMatcher::Not(Box::new(rule.clone()));
    (positive, negative)
}

/// W's three-valued verdict of a `clause_of` pair at one node: `Some(true)` where the rule
/// matches, `Some(false)` where its negation does, `None` where neither (unknown).
pub(super) fn verdict_at(
    clause: &(GrammarMatcher, GrammarMatcher),
    root: &Root<TapeDoc>,
    ordinal: u32,
) -> Option<bool> {
    let holds = |grammar: &GrammarMatcher| {
        let mut work = usize::MAX;
        grammar
            .match_root(node_at(root, ordinal), &mut work)
            .ok()
            .flatten()
            .is_some()
    };
    if holds(&clause.0) {
        Some(true)
    } else if holds(&clause.1) {
        Some(false)
    } else {
        None
    }
}

/// W's three-valued verdicts for `rule` at every node of `root`'s document, read as `template`'s
/// clause reads them (its utilities): the nodes it matches, and those whose verdict is unknown.
/// Runs inside the caller's file evaluation (`file_evaluation`), so tables are built once per
/// file.
pub(super) fn verdicts(
    template: &GrammarMatcher,
    rule: &AstMatcher,
    root: &Root<TapeDoc>,
) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let clause = clause_of(template, rule);
    let (mut matched, mut unknown) = (BTreeSet::new(), BTreeSet::new());
    for ordinal in 0..root.root().get_doc().transducer_len() as u32 {
        match verdict_at(&clause, root, ordinal) {
            Some(true) => {
                matched.insert(ordinal);
            }
            Some(false) => {}
            None => {
                unknown.insert(ordinal);
            }
        }
    }
    (matched, unknown)
}

/// A file evaluation of `tape` (W's), with `module` as its module path.
pub(super) fn file_evaluation(tape: &TapeDoc, module: Option<&ModulePath>) -> FileScopeSlot {
    FileScopeSlot::install(
        Vec::new(),
        module.cloned().map(Arc::new),
        !tape.syntax_error_ranges().is_empty(),
    )
}

/// The module paths a gate reads each fixture under: none (as S1's gate), and inside a package,
/// which qualifies definitions and relative imports.
pub(super) fn modules() -> [Option<ModulePath>; 2] {
    [None, ModulePath::dotted("pkg.sub.fixture")]
}
