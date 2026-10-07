//! Cross-file checks: global `forbid` query plans and `evidence` relations, answered over the
//! per-file evidence index.

use super::crossfile::CrossFileScheduler;
use super::{CheckId, FileState, WorkspaceState};
use crate::policy_dsl::budget::matcher_budget_exhausted;
use crate::policy_dsl::eval::{evaluate_checks, evidence, failure, finding_from_witness};
use crate::policy_dsl::findings::Witness;
use crate::policy_dsl::json::invalid;
use crate::policy_dsl::model::{
    CheckKind, CompiledCheck, CompiledPolicy, Evaluation, FactQuery, Query, Snapshot,
};
use crate::policy_dsl::query;
use napi::Result;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

pub(super) type QueryPlan = query::select_when::Plan<CheckId>;

/// A global `forbid` query's plan; each regex or AST leaf becomes a synthetic check whose
/// witnesses are recorded as evidence. A leaf under an `exists` side only answers whether it has a
/// witness: reporting configuration is dropped from it, and its evaluation in a file stops at the
/// first witness.
pub(super) fn compile_query_plan(
    query: &Query,
    template: &CompiledCheck,
    index: usize,
    policy: &mut CompiledPolicy,
) -> QueryPlan {
    QueryPlan::new(query, false, |leaf, reported| {
        let id = (index, policy.checks.len());
        let mut check = template.clone();
        check.kind = CheckKind::Forbid(leaf.clone());
        check.synthetic = true;
        check.decision = !reported;
        if !reported {
            // An existence leaf only answers whether the query has a witness. Reporting
            // configuration can neither suppress that answer nor enumerate regex groups.
            check.finding.each = None;
            check.finding.node_regexes.clear();
            check.finding.suppress = None;
            check.finding.group_transform = None;
            check.finding.message_template = None;
            check.finding.fix = None;
        }
        policy.checks.push(check);
        id
    })
}

/// One file's evidence for a synthetic check.
#[derive(Clone)]
pub(super) struct Partition {
    pub(super) witnesses: Vec<Witness>,
    /// The check failed in the file: its witnesses there are unknown.
    failed: bool,
    /// The file was read around syntax errors.
    syntax: Option<String>,
}

/// The synthetic check producing an `evidence` relation's rows in each file.
pub(super) fn compile_relation_producer(
    relation: &FactQuery,
    template: &CompiledCheck,
    index: usize,
    policy: &mut CompiledPolicy,
) -> CheckId {
    let ast = relation.ast.as_ref();
    let id = (index, policy.checks.len());
    let mut check = template.clone();
    check.finding = evidence::producer_finding(template, relation);
    check.kind = CheckKind::Forbid(ast.query.clone());
    check.synthetic = true;
    policy.checks.push(check);
    id
}

/// Records `file`'s evidence for every synthetic check it was evaluated for. A policy whose
/// evidence gains or loses a partition has its cross-file checks re-run; a file with no evidence
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
            state.policies[id.0].checks[id.1].synthetic
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

/// Forgets `path`'s evidence: it was removed or changed. The policies it had evidence for have
/// their cross-file checks re-run.
pub(super) fn forget_evidence(state: &mut WorkspaceState, path: &str) {
    for (id, partitions) in &mut state.evidence {
        if partitions.remove(path).is_some() {
            state.dirty_globals.insert(id.0);
        }
    }
}

/// A relation's rows: every partition of its producer in the relation's domain. A failed file, or
/// one read around syntax errors, has unknown rows.
fn relation_rows(state: &WorkspaceState, id: CheckId, relation: &FactQuery) -> evidence::Rows {
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
    policy: &CompiledPolicy,
    check: &CompiledCheck,
    producers: &[CheckId],
) -> Evaluation {
    match (&check.kind, producers) {
        (CheckKind::RequireEach { subject, evidence }, &[subject_id, evidence_id]) => {
            evidence::require_each(
                policy,
                check,
                (subject, relation_rows(state, subject_id, subject)),
                (evidence, &relation_rows(state, evidence_id, evidence)),
            )
        }
        (CheckKind::Budget { query, .. }, &[id]) => {
            evidence::budget(policy, check, query, relation_rows(state, id, query))
        }
        _ => unreachable!("relation plans are compiled for require_each and budget"),
    }
}

/// A plan's leaves read from the evidence index: one row per file with evidence, in path order.
/// A file whose check failed has unknown witnesses; only an AST leaf reads a file around its
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
    fn new(plan: &QueryPlan, state: &'a WorkspaceState, policy: &CompiledPolicy) -> Self {
        let leaves = plan
            .leaves
            .iter()
            .map(|leaf| EvidenceLeaf {
                decision: leaf.decision,
                ast: matches!(
                    policy.checks[leaf.id.1].kind,
                    CheckKind::Forbid(Query::Ast { .. })
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
    policy: &CompiledPolicy,
    partial: &mut BTreeMap<String, String>,
) -> Result<Option<Vec<Witness>>> {
    let leaves = EvidenceLeaves::new(plan, state, policy);
    let cap = policy.capabilities.limits.max_witnesses.saturating_add(1);
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
    let policy = &state.policies[index];
    let mut evaluation = Evaluation::default();
    for check in &state.globals[&index] {
        if let Some(plan) = state.query_plans.get(&(index, check.id.clone())) {
            let mut partial = BTreeMap::new();
            let result = evaluate_plan(plan, state, policy, &mut partial)?;
            if result.is_some() {
                for (path, message) in &partial {
                    evaluation.failures.push(failure(
                        &check.id,
                        "source-partial",
                        message,
                        Some(path),
                    ));
                }
            }
            match result {
                None => evaluation.failures.push(failure(
                    &check.id,
                    "source-incomplete",
                    "A cross-file query source is incomplete",
                    None,
                )),
                Some(witnesses) if witnesses.len() > policy.capabilities.limits.max_witnesses => {
                    evaluation.failures.push(failure(
                        &check.id,
                        "budget-exceeded",
                        "global witness budget exceeded",
                        None,
                    ));
                }
                Some(witnesses) => {
                    evaluation.metrics.witnesses += witnesses.len();
                    evaluation.findings.extend(
                        witnesses
                            .into_iter()
                            .map(|witness| finding_from_witness(policy, check, witness)),
                    );
                }
            }
        } else if let Some(producers) = state.relation_plans.get(&(index, check.id.clone())) {
            let mut result = evaluate_relations(state, policy, check, producers);
            evaluation.findings.append(&mut result.findings);
            evaluation.failures.append(&mut result.failures);
            evaluation.metrics.witnesses += result.metrics.witnesses;
        } else {
            // Every other kind has a plan (`add_policy`): an `llm` check, which matches nothing.
            let mut result =
                evaluate_checks(policy, snapshot, "diagnostic", std::slice::from_ref(check))?;
            evaluation.findings.append(&mut result.findings);
            evaluation.failures.append(&mut result.failures);
            evaluation.metrics.witnesses += result.metrics.witnesses;
        }
    }
    Ok(evaluation)
}
