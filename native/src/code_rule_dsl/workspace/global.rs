//! Cross-file clauses: global `forbid` query plans and `evidence` relations, answered over the
//! per-file evidence index.

use super::crossfile::CrossFileScheduler;
use super::{ClauseId, FileState, WorkspaceState};
use crate::code_rule_dsl::budget::matcher_budget_exhausted;
use crate::code_rule_dsl::eval::{evaluate_clauses, evidence, failure, violation_from_witness};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Query, Snapshot,
};
use crate::code_rule_dsl::query;
use crate::code_rule_dsl::violations::Witness;
use napi::Result;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

pub(super) type QueryPlan = query::select_when::Plan<ClauseId>;

/// A global `forbid` query's plan; each regex or AST leaf becomes a synthetic clause whose
/// witnesses are recorded as evidence. A leaf under an `exists` side only answers whether it has a
/// witness: reporting configuration is dropped from it, and its evaluation in a file stops at the
/// first witness.
pub(super) fn compile_query_plan(
    query: &Query,
    template: &CompiledClause,
    index: usize,
    code_rule: &mut CompiledCodeRule,
) -> QueryPlan {
    QueryPlan::new(query, false, |leaf, reported| {
        let id = (index, code_rule.clauses.len());
        let mut clause = template.clone();
        clause.kind = ClauseKind::Forbid(leaf.clone());
        clause.synthetic = true;
        clause.decision = !reported;
        if !reported {
            // An existence leaf only answers whether the query has a witness. Reporting
            // configuration can neither suppress that answer nor enumerate regex groups.
            clause.violation.each = None;
            clause.violation.node_regexes.clear();
            clause.violation.suppress = None;
            clause.violation.group_transform = None;
            clause.violation.message_template = None;
            clause.violation.fix = None;
        }
        code_rule.clauses.push(clause);
        id
    })
}

/// One file's evidence for a synthetic clause.
#[derive(Clone)]
pub(super) struct Partition {
    pub(super) witnesses: Vec<Witness>,
    /// The clause failed in the file: its witnesses there are unknown.
    failed: bool,
    /// The file was read around syntax errors.
    syntax: Option<String>,
}

/// The synthetic clause producing an `evidence` relation's rows in each file.
pub(super) fn compile_relation_producer(
    relation: &FactQuery,
    template: &CompiledClause,
    index: usize,
    code_rule: &mut CompiledCodeRule,
) -> ClauseId {
    let ast = relation.ast.as_ref();
    let id = (index, code_rule.clauses.len());
    let mut clause = template.clone();
    clause.violation = evidence::producer_violation(template, relation);
    clause.kind = ClauseKind::Forbid(ast.query.clone());
    clause.synthetic = true;
    code_rule.clauses.push(clause);
    id
}

/// Records `file`'s evidence for every synthetic clause it was evaluated for. A code rule whose
/// evidence gains or loses a partition has its cross-file clauses re-run; a file with no evidence
/// before or after changes none of them.
pub(super) fn record_evidence(state: &mut WorkspaceState, path: &str, file: &FileState) {
    // Synthetic producers owned by another adapter are recorded by that adapter instead.
    let ids = file
        .candidates
        .ast
        .iter()
        .chain(&file.candidates.regex)
        .chain(file.witnesses.keys())
        .chain(file.failures.keys())
        .copied()
        .filter(|id| {
            state.code_rules[id.0].clauses[id.1].synthetic
                && !CrossFileScheduler::is_producer(state, id)
        })
        .collect::<BTreeSet<_>>();
    for id in ids {
        let witnesses = file.witnesses.get(&id).cloned().unwrap_or_default();
        let failed = file.failures.contains_key(&id);
        let partitions = state.evidence.entry(id).or_default();
        if failed || file.syntax.is_some() || !witnesses.is_empty() {
            partitions.insert(
                path.to_owned(),
                Partition {
                    witnesses,
                    failed,
                    syntax: file.syntax.clone(),
                },
            );
            state.dirty_globals.insert(id.0);
        } else if partitions.remove(path).is_some() {
            state.dirty_globals.insert(id.0);
        }
    }
}

/// Forgets `path`'s evidence: it was removed or changed. The code rules it had evidence for have
/// their cross-file clauses re-run.
pub(super) fn forget_evidence(state: &mut WorkspaceState, path: &str) {
    for (id, partitions) in &mut state.evidence {
        if partitions.remove(path).is_some() {
            state.dirty_globals.insert(id.0);
        }
    }
}

/// A relation's rows: every partition of its producer in the relation's domain. A failed file, or
/// one read around syntax errors, has unknown rows.
fn relation_rows(state: &WorkspaceState, id: ClauseId, relation: &FactQuery) -> evidence::Rows {
    let mut rows = evidence::Rows::default();
    let domain = relation.ast.domain.as_ref();
    for (path, partition) in state.evidence.get(&id).into_iter().flatten() {
        if domain.is_some_and(|domain| !domain.matches(path)) {
            continue;
        }
        if partition.failed || partition.syntax.is_some() {
            rows.incomplete.insert(path.clone());
        } else {
            rows.witnesses.extend(partition.witnesses.iter().cloned());
        }
    }
    rows
}

fn evaluate_relations(
    state: &WorkspaceState,
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    producers: &[ClauseId],
) -> Evaluation {
    match (&clause.kind, producers) {
        (ClauseKind::RequireEach { subject, evidence }, &[subject_id, evidence_id]) => {
            evidence::require_each(
                code_rule,
                clause,
                (subject, relation_rows(state, subject_id, subject)),
                (evidence, &relation_rows(state, evidence_id, evidence)),
            )
        }
        (ClauseKind::Budget { query, .. }, &[id]) => {
            evidence::budget(code_rule, clause, query, relation_rows(state, id, query))
        }
        _ => unreachable!("relation plans are compiled for require_each and budget"),
    }
}

/// A plan's leaves read from the evidence index: one row per file with evidence, in path order.
/// A file whose clause failed has unknown witnesses; only an AST leaf reads a file around its
/// syntax errors (a regex reads the text whole). A leaf read for a decision ends with its first
/// file holding a witness, and any leaf ends, unknown, once the time budget is spent.
struct EvidenceLeaves<'a> {
    leaves: Vec<EvidenceLeaf<'a>>,
}

struct EvidenceLeaf<'a> {
    decision: bool,
    ast: bool,
    read: RefCell<EvidenceRead<'a>>,
}

struct EvidenceRead<'a> {
    partitions: Option<std::collections::btree_map::Iter<'a, String, Partition>>,
    /// The rows read so far; `None` where the time budget ran out.
    rows: Vec<Option<(&'a str, &'a Partition)>>,
}

impl<'a> EvidenceLeaves<'a> {
    fn new(plan: &QueryPlan, state: &'a WorkspaceState, code_rule: &CompiledCodeRule) -> Self {
        let leaves = plan
            .leaves
            .iter()
            .map(|leaf| EvidenceLeaf {
                decision: leaf.decision,
                ast: matches!(
                    code_rule.clauses[leaf.id.1].kind,
                    ClauseKind::Forbid(Query::Ast { .. })
                ),
                read: RefCell::new(EvidenceRead {
                    partitions: state.evidence.get(&leaf.id).map(BTreeMap::iter),
                    rows: Vec::new(),
                }),
            })
            .collect();
        Self { leaves }
    }

    fn partition(&self, leaf: usize, position: usize) -> (&'a str, &'a Partition) {
        self.leaves[leaf].read.borrow().rows[position].expect("the answer read this row")
    }
}

impl query::select_when::Leaves for EvidenceLeaves<'_> {
    fn row(
        &self,
        leaf: usize,
        position: usize,
    ) -> std::result::Result<Option<query::select_when::LeafRow>, String> {
        let leaf = &self.leaves[leaf];
        let mut read = leaf.read.borrow_mut();
        while read.rows.len() <= position {
            let Some(partitions) = &mut read.partitions else {
                break;
            };
            let next = partitions.next();
            let ends = match next {
                None => true,
                Some(_) if matcher_budget_exhausted() => {
                    read.rows.push(None);
                    true
                }
                Some((path, partition)) => {
                    read.rows.push(Some((path.as_str(), partition)));
                    leaf.decision && !partition.failed && !partition.witnesses.is_empty()
                }
            };
            if ends {
                read.partitions = None;
            }
        }
        Ok(read.rows.get(position).map(|row| match row {
            None => query::select_when::LeafRow {
                failed: true,
                partial: false,
                witnesses: 0,
            },
            Some((_, partition)) => query::select_when::LeafRow {
                failed: partition.failed,
                partial: !partition.failed && leaf.ast && partition.syntax.is_some(),
                witnesses: if partition.failed {
                    0
                } else {
                    partition.witnesses.len()
                },
            },
        }))
    }
}

/// A global `forbid` query answered over the evidence index: its witnesses, or `None` where
/// unknown. `partial` receives the files read around syntax errors the answer relied on.
fn evaluate_plan(
    plan: &QueryPlan,
    state: &WorkspaceState,
    code_rule: &CompiledCodeRule,
    partial: &mut BTreeMap<String, String>,
) -> Result<Option<Vec<Witness>>> {
    let leaves = EvidenceLeaves::new(plan, state, code_rule);
    let cap = code_rule
        .capabilities
        .limits
        .max_witnesses
        .saturating_add(1);
    let Some(answer) = query::select_when::answer(plan, cap, &leaves).map_err(invalid)? else {
        return Ok(None);
    };
    for (leaf, position) in answer.partial {
        let (path, partition) = leaves.partition(leaf, position);
        if let Some(syntax) = &partition.syntax {
            partial.insert(path.to_owned(), syntax.clone());
        }
    }
    Ok(Some(
        answer
            .witnesses
            .into_iter()
            .flat_map(|(leaf, position, take)| {
                leaves.partition(leaf, position).1.witnesses[..take]
                    .iter()
                    .cloned()
            })
            .collect(),
    ))
}

pub(super) fn evaluate_global(
    state: &WorkspaceState,
    index: usize,
    snapshot: &Snapshot,
) -> Result<Evaluation> {
    let code_rule = &state.code_rules[index];
    let mut evaluation = Evaluation::default();
    for clause in &state.globals[&index] {
        if let Some(plan) = state.query_plans.get(&(index, clause.id.clone())) {
            let mut partial = BTreeMap::new();
            let result = evaluate_plan(plan, state, code_rule, &mut partial)?;
            if result.is_some() {
                for (path, message) in &partial {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "source-partial",
                        message,
                        Some(path),
                    ));
                }
            }
            match result {
                None => evaluation.failures.push(failure(
                    &clause.id,
                    "source-incomplete",
                    "A cross-file query source is incomplete",
                    None,
                )),
                Some(witnesses)
                    if witnesses.len() > code_rule.capabilities.limits.max_witnesses =>
                {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "budget-exceeded",
                        "global witness budget exceeded",
                        None,
                    ));
                }
                Some(witnesses) => {
                    evaluation.metrics.witnesses += witnesses.len();
                    evaluation.violations.extend(
                        witnesses
                            .into_iter()
                            .map(|witness| violation_from_witness(code_rule, clause, witness)),
                    );
                }
            }
        } else if let Some(producers) = state.relation_plans.get(&(index, clause.id.clone())) {
            let mut result = evaluate_relations(state, code_rule, clause, producers);
            evaluation.violations.append(&mut result.violations);
            evaluation.failures.append(&mut result.failures);
            evaluation.metrics.witnesses += result.metrics.witnesses;
        } else {
            // Every other kind has a plan (`add_code_rule`): an `llm` clause, which matches nothing.
            let mut result =
                evaluate_clauses(code_rule, snapshot, "report", std::slice::from_ref(clause))?;
            evaluation.violations.append(&mut result.violations);
            evaluation.failures.append(&mut result.failures);
            evaluation.metrics.witnesses += result.metrics.witnesses;
        }
    }
    Ok(evaluation)
}
