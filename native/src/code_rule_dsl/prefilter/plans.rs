//! Gather the complete rule frontier before lowering SQL. Pointer identities retain shared plans.
//! From Pulling-it-all-together's `matcher/anchors/sql/plans.rs`, without its per-file demand
//! discovery (`for_file`: candidates are selected for a batch of tapes, never one file at a time)
//! and its profile provenance.
use crate::code_rule_dsl::{
    language,
    matcher::AnchorPlan,
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
                            .push(Arc::clone(&matcher.anchors));
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
                ClauseKind::Llm => {}
            }
        }
    }
    out
}
