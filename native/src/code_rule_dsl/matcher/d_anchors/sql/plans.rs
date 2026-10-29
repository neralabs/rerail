//! Gather the complete rule frontier before lowering SQL. Pointer identities retain shared plans.
use super::super::AnchorPlan;
use crate::code_rule_dsl::{
    language,
    model::{ClauseKind, CompiledCodeRule, Query},
};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

pub(in crate::code_rule_dsl) fn plans<'a>(
    rules: impl IntoIterator<Item = &'a CompiledCodeRule>,
) -> BTreeMap<u32, Vec<Arc<AnchorPlan>>> {
    fn query(q: &Query, out: &mut BTreeMap<u32, Vec<Arc<AnchorPlan>>>, seen: &mut HashSet<usize>) {
        match q {
            Query::Ast { matchers, .. } => {
                for matcher in matchers {
                    if seen.insert(Arc::as_ptr(&matcher.anchors) as usize) {
                        let lang =
                            u32::from(language::tape_tag(matcher.language).expect("tape language"));
                        out.entry(lang)
                            .or_default()
                            .push(Arc::new(AnchorPlan::compile(&matcher.matcher)));
                    }
                }
            }
            Query::SelectWhen { select, exists } => {
                query(select, out, seen);
                query(exists, out, seen);
            }
            Query::Regex { .. } => {}
        }
    }
    let mut out = BTreeMap::new();
    let mut seen = HashSet::new();
    for rule in rules {
        for clause in &rule.clauses {
            match &clause.kind {
                ClauseKind::Forbid(q) => query(q, &mut out, &mut seen),
                ClauseKind::RequireEach { subject, evidence } => {
                    query(&subject.ast.query, &mut out, &mut seen);
                    query(&evidence.ast.query, &mut out, &mut seen);
                }
                ClauseKind::Budget { query: input, .. } => {
                    query(&input.ast.query, &mut out, &mut seen)
                }
                ClauseKind::Relational(plan) => {
                    for (_, input) in &plan.inputs {
                        query(&input.ast.query, &mut out, &mut seen);
                    }
                    if let Some(reference) = &plan.references {
                        query(&reference.query, &mut out, &mut seen);
                    }
                }
                ClauseKind::Llm => {}
            }
        }
    }
    out
}

/// Discover file demand without running selectors. Conditional scheduling remains the caller's job.
pub(in crate::code_rule_dsl) fn for_file(
    rule: &CompiledCodeRule,
    file: &crate::code_rule_dsl::model::FileInput,
    doc: &crate::code_rule_dsl::tape::TapeDoc,
) -> Vec<Arc<AnchorPlan>> {
    use crate::code_rule_dsl::{
        eval::grammar_matcher, matcher::FileGates, soundness::SyntaxErrors,
    };
    let limits = &rule.capabilities.limits;
    if !rule.scope.matches(&file.path)
        || limits.max_files == 0
        || file.source.len() > limits.max_source_bytes
    {
        return Vec::new();
    }
    let root = ast_grep_core::AstGrep::doc(doc.clone());
    let gates = FileGates::new(file, root.root());
    let _errors = SyntaxErrors::of_doc(doc);
    let mut searched = rustc_hash::FxHashMap::default();
    let mut leaves = Vec::new();
    fn query<'a>(q: &'a Query, leaves: &mut Vec<&'a Query>) {
        match q {
            Query::Ast { .. } => leaves.push(q),
            Query::SelectWhen { select, exists } => {
                query(exists, leaves);
                query(select, leaves);
            }
            Query::Regex { .. } => {}
        }
    }
    fn relation<'a>(
        input: &'a crate::code_rule_dsl::model::FactQuery,
        file: &crate::code_rule_dsl::model::FileInput,
        leaves: &mut Vec<&'a Query>,
    ) {
        if input
            .ast
            .domain
            .as_ref()
            .is_none_or(|domain| domain.matches(&file.path))
        {
            query(&input.ast.query, leaves);
        }
    }
    // Gather borrowed leaves first, then share text searches and file facts across their matchers.
    for clause in &rule.clauses {
        match &clause.kind {
            ClauseKind::Forbid(q) => query(q, &mut leaves),
            ClauseKind::RequireEach { subject, evidence } => {
                relation(subject, file, &mut leaves);
                relation(evidence, file, &mut leaves);
            }
            ClauseKind::Budget { query: input, .. } => relation(input, file, &mut leaves),
            ClauseKind::Relational(plan) => {
                for (_, input) in &plan.inputs {
                    relation(input, file, &mut leaves);
                }
                if let Some(reference) = &plan.references {
                    query(&reference.query, &mut leaves);
                }
            }
            ClauseKind::Llm => {}
        }
    }
    let mut seen = HashSet::new();
    leaves
        .into_iter()
        .filter_map(|q| {
            let Query::Ast { matchers, .. } = q else {
                unreachable!()
            };
            let grammar = grammar_matcher(matchers, file)?;
            // W's GrammarMatcher has no `admits_source` nor D's `invalid_syntax` gate (O18): its
            // required text, as D's `admits_source` reads it.
            (grammar.text.as_ref().is_none_or(|text| {
                text.holds_with(&file.source, &mut |source: &str, literal| {
                    *searched.entry(literal).or_insert_with(|| {
                        memchr::memmem::find(source.as_bytes(), literal.as_bytes()).is_some()
                    })
                })
            })
                && gates.admit(grammar)
                && seen.insert(Arc::as_ptr(&grammar.anchors) as usize))
            .then(|| Arc::new(AnchorPlan::compile(&grammar.matcher)))
        })
        .collect()
}

/// Diagnostic provenance stays outside binding identity and does not activate any plan.
#[cfg(feature = "code-rule-profile")]
pub(super) fn origins<'a>(
    rules: impl IntoIterator<Item = &'a CompiledCodeRule>,
) -> std::collections::HashMap<usize, std::collections::BTreeSet<String>> {
    fn query(
        q: &Query,
        origin: &str,
        out: &mut std::collections::HashMap<usize, std::collections::BTreeSet<String>>,
    ) {
        match q {
            Query::Ast { matchers, .. } => {
                for matcher in matchers {
                    out.entry(Arc::as_ptr(&matcher.anchors) as usize)
                        .or_default()
                        .insert(origin.into());
                }
            }
            Query::SelectWhen { select, exists } => {
                query(select, origin, out);
                query(exists, origin, out);
            }
            Query::Regex { .. } => {}
        }
    }
    let mut out = std::collections::HashMap::new();
    for rule in rules {
        for clause in &rule.clauses {
            let origin = format!(
                "rule {} clause {} policy {}",
                rule.id, clause.id, rule.digest
            );
            match &clause.kind {
                ClauseKind::Forbid(q) => query(q, &origin, &mut out),
                ClauseKind::RequireEach { subject, evidence } => {
                    query(&subject.ast.query, &origin, &mut out);
                    query(&evidence.ast.query, &origin, &mut out);
                }
                ClauseKind::Budget { query: input, .. } => {
                    query(&input.ast.query, &origin, &mut out)
                }
                ClauseKind::Relational(plan) => {
                    for (_, input) in &plan.inputs {
                        query(&input.ast.query, &origin, &mut out);
                    }
                    if let Some(reference) = &plan.references {
                        query(&reference.query, &origin, &mut out);
                    }
                }
                ClauseKind::Llm => {}
            }
        }
    }
    out
}
