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
use crate::code_rule_dsl::soundness::UNREAD;
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
            clause.violation.ignored_by = None;
            clause.violation.group_transform = None;
            clause.violation.message_template = None;
            clause.violation.fix = None;
        }
        code_rule.clauses.push(clause);
        id
    })
}

/// One file's evidence for a synthetic clause.
#[derive(Clone, PartialEq)]
pub(super) struct Partition {
    pub(super) witnesses: Vec<Witness>,
    /// The static message of the clause's failure in the file: its witnesses there are unknown.
    failure: Option<&'static str>,
    /// The file was read around syntax errors, or (failed `UNREAD`) not read at all: the detail.
    syntax: Option<String>,
}

impl Partition {
    fn failed(&self) -> bool {
        self.failure.is_some()
    }
}

/// The synthetic clause producing an `evidence` relation's rows in each file, answering to the
/// comments its `side` does.
pub(super) fn compile_relation_producer(
    relation: &FactQuery,
    side: evidence::Side,
    template: &CompiledClause,
    index: usize,
    code_rule: &mut CompiledCodeRule,
) -> ClauseId {
    let ast = relation.ast.as_ref();
    let id = (index, code_rule.clauses.len());
    let mut clause = template.clone();
    clause.violation = evidence::producer_violation(template, relation, side);
    clause.kind = ClauseKind::Forbid(ast.query.clone());
    clause.synthetic = true;
    clause.projected_only =
        matches!(&template.kind, ClauseKind::Relational(plan) if !plan.per_file);
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
        let failure = file.failures.get(&id).map(|&(_, message)| message);
        let syntax = match &state.code_rules[id.0].clauses[id.1].kind {
            ClauseKind::Forbid(Query::Ast { matchers, .. })
                if crate::code_rule_dsl::eval::grammar_matcher(matchers, &file.input)
                    .is_some_and(|grammar| grammar.invalid_syntax.is_some()) =>
            {
                None
            }
            _ => file.syntax.clone(),
        };
        let partitions = state.evidence.entry(id).or_default();
        if failure.is_some() || syntax.is_some() || !witnesses.is_empty() {
            let partition = Partition {
                witnesses,
                failure,
                syntax,
            };
            if partitions.get(path) != Some(&partition) {
                partitions.insert(path.to_owned(), partition);
                state.dirty_globals.insert(id.0);
            }
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
    // Lazy source reads can change while every projected metadata row stays equal,
    // including a source with no witnesses. Invalidate those repository scopes.
    for (index, clauses) in &state.globals {
        if state.code_rules[*index].scope.matches(path) && clauses.iter().any(|clause| {
            matches!(&clause.kind, ClauseKind::Relational(plan) if plan.reads_source && !plan.per_file)
        }) { state.dirty_globals.insert(*index); }
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
        if partition.failed() || partition.syntax.is_some() {
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
    snapshot: &Snapshot,
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
        (ClauseKind::Relational(plan), ids) => {
            let rows = plan
                .inputs
                .iter()
                .zip(ids)
                .map(|((_, input), id)| relation_rows(state, *id, input))
                .collect();
            crate::code_rule_dsl::relational::evaluate(code_rule, clause, plan, rows, snapshot)
        }
        _ => unreachable!("relation plans are compiled for relation clauses"),
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

/// A file's partition of a leaf, by path.
type EvidenceRow<'a> = (&'a str, &'a Partition);

struct EvidenceRead<'a> {
    partitions: Option<std::collections::btree_map::Iter<'a, String, Partition>>,
    /// The rows read so far; `None` where the time budget ran out.
    rows: Vec<Option<EvidenceRow<'a>>>,
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

    fn partition(&self, leaf: usize, position: usize) -> EvidenceRow<'a> {
        self.row(leaf, position).expect("the answer read this row")
    }

    /// Row `position` of leaf `leaf`, already read: `None` where the time budget ran out.
    fn row(&self, leaf: usize, position: usize) -> Option<EvidenceRow<'a>> {
        self.leaves[leaf].read.borrow().rows[position]
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
                    leaf.decision && !partition.failed() && !partition.witnesses.is_empty()
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
                failed: partition.failed(),
                partial: !partition.failed() && leaf.ast && partition.syntax.is_some(),
                witnesses: if partition.failed() {
                    0
                } else {
                    partition.witnesses.len()
                },
            },
        }))
    }
}

/// A global `forbid` query answered over the evidence index: its witnesses, or where unknown the
/// failed row it could not do without (`None` where the time budget ran out). `partial` receives
/// the files read around syntax errors the answer relied on.
fn evaluate_plan<'s>(
    plan: &QueryPlan,
    state: &'s WorkspaceState,
    code_rule: &CompiledCodeRule,
    partial: &mut BTreeMap<String, String>,
) -> Result<std::result::Result<Vec<Witness>, Option<EvidenceRow<'s>>>> {
    let leaves = EvidenceLeaves::new(plan, state, code_rule);
    let cap = code_rule
        .capabilities
        .limits
        .max_witnesses
        .saturating_add(1);
    let answer = match query::select_when::answer(plan, cap, &leaves).map_err(invalid)? {
        Ok(answer) => answer,
        Err((leaf, position)) => return Ok(Err(leaves.row(leaf, position))),
    };
    for (leaf, position) in answer.partial {
        let (path, partition) = leaves.partition(leaf, position);
        if let Some(syntax) = &partition.syntax {
            partial.insert(path.to_owned(), syntax.clone());
        }
    }
    Ok(Ok(answer
        .witnesses
        .into_iter()
        .flat_map(|(leaf, position, take)| {
            leaves.partition(leaf, position).1.witnesses[..take]
                .iter()
                .cloned()
        })
        .collect()))
}

pub(super) fn evaluate_global(
    state: &mut WorkspaceState,
    index: usize,
    snapshot: &Snapshot,
    closed: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Evaluation> {
    let _syntax = crate::code_rule_dsl::relational::syntax::Pool::install();
    let resident = std::sync::Arc::clone(&state.code_rules[index]);
    let code_rule = resident.as_ref();
    let _graph = state.crossfile.evaluation_slot();
    let mut evaluation = Evaluation::default();
    for clause in &state.globals[&index].clone() {
        if let ClauseKind::Relational(plan) = &clause.kind
            && plan.per_file
        {
            let mut result = evaluate_file_relations(
                state,
                index,
                code_rule,
                clause,
                snapshot.revision,
                closed,
            )?;
            evaluation.violations.append(&mut result.violations);
            evaluation.failures.append(&mut result.failures);
            evaluation.metrics.merge(result.metrics);
            continue;
        }
        if let Some(plan) = state.query_plans.get(&(index, clause.id.clone())) {
            let mut partial = BTreeMap::new();
            let result = evaluate_plan(plan, state, code_rule, &mut partial)?;
            if result.is_ok() {
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
                // The file the answer could not do without; one the parser did not read reports how
                // much of it the errors cover, as its local clauses do (`results::failure_message`).
                Err(row) => evaluation.failures.push(failure(
                    &clause.id,
                    "source-incomplete",
                    row.and_then(|(_, partition)| {
                        partition
                            .syntax
                            .as_deref()
                            .filter(|_| partition.failure == Some(UNREAD))
                    })
                    .unwrap_or("A cross-file query source is incomplete"),
                    row.map(|(path, _)| path),
                )),
                Ok(witnesses) if witnesses.len() > code_rule.capabilities.limits.max_witnesses => {
                    evaluation.failures.push(failure(
                        &clause.id,
                        crate::code_rule_dsl::model::WITNESS_LIMIT,
                        "global witness budget exceeded",
                        None,
                    ));
                }
                Ok(witnesses) => {
                    evaluation.metrics.witnesses += witnesses.len();
                    evaluation.violations.extend(
                        witnesses
                            .into_iter()
                            .map(|witness| violation_from_witness(code_rule, clause, witness)),
                    );
                }
            }
        } else if let Some(producers) = state
            .relation_plans
            .get(&(index, clause.id.clone()))
            .cloned()
        {
            let source_snapshot = if matches!(&clause.kind,ClauseKind::Relational(plan) if plan.reads_source)
            {
                let paths = state
                    .files
                    .keys()
                    .chain(state.evicted_inputs.keys())
                    .filter(|path| code_rule.scope.matches(path))
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if paths.len() > code_rule.capabilities.limits.max_files {
                    return Err(invalid("relational source snapshot exceeds maxFiles"));
                }
                let mut files = Vec::with_capacity(paths.len());
                for path in paths {
                    if crate::code_rule_dsl::budget::step_budget_exhausted() {
                        return Err(invalid("interrupted: relational source load"));
                    }
                    let mut input = state
                        .files
                        .get(&path)
                        .map(|file| file.input.clone())
                        .or_else(|| state.evicted_inputs.get(&path).cloned())
                        .ok_or_else(|| invalid("relational source was removed"))?;
                    input.source = super::documents::source(state, &path)?;
                    if input.source.len() > code_rule.capabilities.limits.max_source_bytes {
                        return Err(invalid("relational source snapshot exceeds maxSourceBytes"));
                    }
                    files.push(input);
                }
                Some(Snapshot {
                    revision: snapshot.revision,
                    files,
                })
            } else {
                None
            };
            let mut result = evaluate_relations(
                state,
                code_rule,
                clause,
                &producers,
                source_snapshot.as_ref().unwrap_or(snapshot),
            );
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

fn evaluate_file_relations(
    state: &mut WorkspaceState,
    index: usize,
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    revision: u32,
    closed: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Evaluation> {
    let paths = state.relation_plans[&(index, clause.id.clone())]
        .iter()
        .flat_map(|id| {
            state
                .evidence
                .get(id)
                .into_iter()
                .flat_map(|rows| rows.keys().cloned())
        })
        .collect::<BTreeSet<_>>();
    state
        .file_relation_results
        .retain(|(owner, name, path), _| {
            *owner != index || name != &clause.id || paths.contains(path)
        });
    let mut result = Evaluation::default();
    for path in paths {
        #[cfg(feature = "code-rule-profile")]
        let _context = crate::code_rule_dsl::profile::trace::Context::enter(
            &code_rule.id,
            &clause.id,
            Some(&path),
        );
        #[cfg(feature = "code-rule-profile")]
        let mut span =
            crate::code_rule_dsl::profile::trace::Span::start("file-relation", "evaluate");
        let key = (index, clause.id.clone(), path.clone());
        let version = super::documents::current_version(state, &path)
            .ok_or_else(|| invalid(format!("{path} has no current source version")))?;
        let cached = state
            .file_relation_results
            .get(&key)
            .is_some_and(|(cached, _)| cached == &version);
        #[cfg(feature = "code-rule-profile")]
        span.detail(|| serde_json::json!({"cached":cached}));
        if !cached {
            let mut input = state
                .files
                .get(&path)
                .map(|file| file.input.clone())
                .or_else(|| state.evicted_inputs.get(&path).cloned())
                .ok_or_else(|| invalid(format!("{path} is not a file of this workspace")))?;
            // Each file is an independent bounded evaluation, like a local AST clause.
            let budget = crate::code_rule_dsl::budget::MatchBudget::work(
                closed,
                super::file_work_budget(input.source.len()),
            );
            let (_, document) = super::documents::load(state, &path)?;
            input.source = std::sync::Arc::clone(&document.source);
            let _tapes = crate::code_rule_dsl::eval::TapeCacheSlot::install();
            if let Some(language) = crate::code_rule_dsl::eval::file_language(&input) {
                crate::code_rule_dsl::eval::TapeCacheSlot::seed(language, &input, &document.image)?;
            }
            let _syntax = crate::code_rule_dsl::relational::syntax::Pool::install();
            if matches!(&clause.kind, ClauseKind::Relational(plan) if plan.syntax.is_some()) {
                let facts = match document.syntax.get() {
                    Some(facts) => std::sync::Arc::clone(facts),
                    None => {
                        let facts = crate::code_rule_dsl::relational::syntax::Pool::get(&input)?;
                        let _ = document.syntax.set(std::sync::Arc::clone(&facts));
                        facts
                    }
                };
                crate::code_rule_dsl::relational::syntax::Pool::seed(&input, facts);
                state.documents.account_syntax(&version);
            }
            let snapshot = Snapshot {
                revision,
                files: vec![input],
            };
            let mut evaluated =
                evaluate_clauses(code_rule, &snapshot, "report", std::slice::from_ref(clause))?;
            if budget.observed() {
                evaluated.failures.push(failure(
                    &clause.id,
                    "budget-exceeded",
                    "file relation work budget exceeded",
                    Some(&path),
                ));
                result.violations.append(&mut evaluated.violations);
                result.failures.append(&mut evaluated.failures);
                continue;
            }
            state
                .file_relation_results
                .insert(key.clone(), (version, evaluated));
        }
        let evaluated = &state.file_relation_results[&key].1;
        result
            .violations
            .extend(evaluated.violations.iter().cloned());
        result.failures.extend(evaluated.failures.iter().cloned());
        result.metrics.witnesses += evaluated.metrics.witnesses;
    }
    Ok(result)
}
