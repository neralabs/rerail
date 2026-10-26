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

/// Source bytes of the resident files in `code_rule`'s scope. A file whose source was freed
/// counts nothing, so the estimate errs low.
fn scoped_source_bytes(state: &WorkspaceState, code_rule: &CompiledCodeRule) -> usize {
    state
        .files
        .iter()
        .filter(|(path, _)| code_rule.scope.matches(path))
        .map(|(_, file)| file.input.source.len())
        .chain(
            state
                .evicted_inputs
                .iter()
                .filter(|(path, _)| code_rule.scope.matches(path))
                .map(|(_, input)| input.source.len()),
        )
        .sum()
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
    mut batched: BTreeMap<String, Evaluation>,
) -> Result<Evaluation> {
    let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::install(std::sync::Arc::clone(
        &state.anchor_corpus,
    ));
    let _syntax = crate::code_rule_dsl::relational::syntax::Pool::install();
    let resident = std::sync::Arc::clone(&state.code_rules[index]);
    let code_rule = resident.as_ref();
    let _graph = state.crossfile.evaluation_slot();
    let mut evaluation = Evaluation::default();
    for clause in &state.globals[&index].clone() {
        if let Some(mut result) = batched.remove(&clause.id) {
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

/// Collect every dirty relational partition before advancing any SQL consumer.
pub(super) fn evaluate_batched_relations(
    state: &mut WorkspaceState,
    indices: &BTreeSet<usize>,
    snapshot: &Snapshot,
    closed: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<BTreeMap<usize, BTreeMap<String, Evaluation>>> {
    use crate::code_rule_dsl::{
        budget::Context,
        relational::{self, Job},
    };
    let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::install(std::sync::Arc::clone(
        &state.anchor_corpus,
    ));
    let _syntax = crate::code_rule_dsl::relational::syntax::Pool::install();
    let _graph = state.crossfile.evaluation_slot();
    let catalog = indices
        .iter()
        .map(|&index| {
            (
                index,
                std::sync::Arc::clone(&state.code_rules[index]),
                state.globals[&index].clone(),
            )
        })
        .collect::<Vec<_>>();
    // Publish the corpus before producing any relational partition. Local AST evaluation
    // normally supplied these entries already; persisted answers may have skipped restoration.
    let paths = catalog
        .iter()
        .flat_map(|(index, _, clauses)| {
            clauses
                .iter()
                .filter(|clause| match &clause.kind {
                    ClauseKind::Relational(plan) => plan
                        .inputs
                        .iter()
                        .any(|(_, input)| matches!(input.ast.query, Query::Ast { .. })),
                    _ => false,
                })
                .flat_map(|clause| state.relation_plans.get(&(*index, clause.id.clone())))
                .flatten()
                .flat_map(|id| state.evidence.get(id))
                .flat_map(|partitions| partitions.keys().cloned())
        })
        .collect::<BTreeSet<_>>();
    // Without the opt-in positional index there is nothing to prepare.
    let paths = if crate::code_rule_dsl::matcher::Corpus::enabled() {
        paths
    } else {
        BTreeSet::new()
    };
    let mut demands = BTreeMap::new();
    for path in paths {
        let input = state
            .files
            .get(&path)
            .map(|file| file.input.clone())
            .or_else(|| state.evicted_inputs.get(&path).cloned());
        let Some(mut input) = input else {
            continue;
        };
        let Some(language) = crate::code_rule_dsl::eval::file_language(&input) else {
            continue;
        };
        let Some(version) = super::documents::current_version(state, &path) else {
            continue;
        };
        let lang =
            u32::from(crate::code_rule_dsl::language::tape_tag(language).expect("tape language"));
        if state
            .anchor_corpus
            .contains(&path, lang, &version.source_digest())
            .map_err(|e| invalid(e.to_string()))?
        {
            continue;
        }
        // Missing source is diagnosed by the individual partition, as before. Index
        // preparation must not turn that diagnosis into a failed workspace apply.
        let (_, document) = match super::documents::load(state, &path) {
            Ok(document) => document,
            Err(error) => {
                eprintln!("SQL AST preparation unavailable for {path}: {error}");
                continue;
            }
        };
        input.source = std::sync::Arc::clone(&document.source);
        let _tapes = crate::code_rule_dsl::eval::TapeCacheSlot::install();
        if let Err(error) =
            crate::code_rule_dsl::eval::TapeCacheSlot::seed(language, &input, &document.image)
        {
            eprintln!("SQL AST preparation unavailable for {path}: {error}");
            continue;
        }
        let Some((doc, _)) = crate::code_rule_dsl::eval::file_tape(language, &input)? else {
            continue;
        };
        let plans = catalog
            .iter()
            .flat_map(|(_, rule, _)| {
                crate::code_rule_dsl::matcher::anchor_plans_for_file(rule, &input, &doc)
            })
            .collect();
        demands.insert(path.clone(), plans);
        if let Err(error) = state.anchor_corpus.put(&path, &doc, &|| {
            closed.load(std::sync::atomic::Ordering::Acquire)
        }) {
            eprintln!("SQL AST preparation unavailable for {path}: {error}");
        }
    }
    #[cfg(feature = "code-rule-profile")]
    state
        .anchor_corpus
        .record_origins(catalog.iter().map(|(_, rule, _)| rule.as_ref()));
    if let Err(error) = state.anchor_corpus.prepare_files(&demands) {
        eprintln!("SQL AST corpus batch unavailable; retaining authoritative candidates: {error}");
    }
    #[cfg(feature = "code-rule-profile")]
    let mut preparation = crate::code_rule_dsl::profile::trace::Span::start(
        "input-preparation",
        "queued-relational-inputs",
    );
    let mut jobs = Vec::new();
    let mut keys = Vec::new();
    let mut results = BTreeMap::new();
    for (index, code_rule, clauses) in &catalog {
        let index = *index;
        let results = results
            .entry(index)
            .or_insert_with(BTreeMap::<String, Evaluation>::new);
        for clause in clauses {
            let ClauseKind::Relational(plan) = &clause.kind else {
                continue;
            };
            results.entry(clause.id.clone()).or_default();
            let producers = state.relation_plans[&(index, clause.id.clone())].clone();
            if plan.per_file {
                let paths = producers
                    .iter()
                    .flat_map(|id| {
                        state
                            .evidence
                            .get(id)
                            .into_iter()
                            .flat_map(|r| r.keys().cloned())
                    })
                    .collect::<BTreeSet<_>>();
                state
                    .file_relation_results
                    .retain(|(owner, name, path), _| {
                        *owner != index || name != &clause.id || paths.contains(path)
                    });
                for path in paths {
                    let key = (index, clause.id.clone(), path.clone());
                    let version = super::documents::current_version(state, &path)
                        .ok_or_else(|| invalid("relational source version unavailable"))?;
                    if let Some((cached, evaluation)) = state
                        .file_relation_results
                        .get(&key)
                        .filter(|(cached, _)| cached == &version)
                    {
                        let _ = cached;
                        let result = results.get_mut(&clause.id).unwrap();
                        result
                            .violations
                            .extend(evaluation.violations.iter().cloned());
                        result.failures.extend(evaluation.failures.iter().cloned());
                        result.metrics.witnesses += evaluation.metrics.witnesses;
                        continue;
                    }
                    let input = state
                        .files
                        .get(&path)
                        .map(|f| f.input.clone())
                        .or_else(|| state.evicted_inputs.get(&path).cloned())
                        .ok_or_else(|| invalid("relational source removed"))?;
                    let budget = Context::work(closed, super::file_work_budget(input.source.len()));
                    jobs.push(Job {
                        code_rule,
                        clause,
                        inputs: relational::Inputs {
                            gate: None,
                            file_local: false,
                            read_bytes: 0,
                            rows: None,
                            facts: Vec::new(),
                            snapshot: Snapshot {
                                revision: snapshot.revision,
                                files: vec![input],
                            },
                        },
                        budget,
                    });
                    keys.push((index, clause.id.clone(), Some((key, version, path))));
                }
            } else {
                let mut files = Vec::new();
                if plan.reads_source {
                    let paths = state
                        .files
                        .keys()
                        .chain(state.evicted_inputs.keys())
                        .filter(|p| code_rule.scope.matches(p))
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    if paths.len() > code_rule.capabilities.limits.max_files {
                        return Err(invalid("relational source snapshot exceeds maxFiles"));
                    }
                    for path in paths {
                        let mut input = state
                            .files
                            .get(&path)
                            .map(|f| f.input.clone())
                            .or_else(|| state.evicted_inputs.get(&path).cloned())
                            .ok_or_else(|| invalid("relational source removed"))?;
                        input.source = super::documents::source(state, &path)?;
                        if input.source.len() > code_rule.capabilities.limits.max_source_bytes {
                            return Err(invalid(
                                "relational source snapshot exceeds maxSourceBytes",
                            ));
                        }
                        files.push(input);
                    }
                } else {
                    files = snapshot.files.clone();
                }
                // A repository clause's work grows with the code in its scope, as a file's does.
                let read_bytes = scoped_source_bytes(state, code_rule);
                jobs.push(Job {
                    code_rule,
                    clause,
                    inputs: relational::Inputs {
                        gate: None,
                        file_local: false,
                        read_bytes,
                        rows: None,
                        facts: Vec::new(),
                        snapshot: Snapshot {
                            revision: snapshot.revision,
                            files,
                        },
                    },
                    budget: Context::work(closed, super::file_work_budget(read_bytes)),
                });
                keys.push((index, clause.id.clone(), None));
            }
        }
    }
    #[cfg(feature = "code-rule-profile")]
    {
        preparation.detail(|| {
            let mut memory = super::memory::census::WitnessMemory::default();
            let mut syntax_seen = std::collections::HashSet::new();
            let mut source_seen = std::collections::HashSet::new();
            let mut syntax_bytes = 0usize;
            let mut source_bytes = 0usize;
            let mut by_clause = BTreeMap::<_, (usize, usize)>::new();
            for job in &jobs {
                for (_, facts) in &job.inputs.facts {
                    if syntax_seen.insert(std::sync::Arc::as_ptr(facts) as usize) {
                        syntax_bytes += facts.estimated_bytes();
                    }
                }
                for file in &job.inputs.snapshot.files {
                    if source_seen.insert(std::sync::Arc::as_ptr(&file.source) as usize) {
                        source_bytes += file.source.capacity();
                    }
                }
                let counts = by_clause.entry((&job.code_rule.id, &job.clause.id)).or_default();
                counts.0 += 1;
                for rows in job.inputs.rows.iter().flatten() {
                    counts.1 += rows.witnesses.len();
                    for witness in &rows.witnesses { memory.visit(witness); }
                }
            }
            serde_json::json!({"jobs": jobs.len(), "ownership": memory,
                "suppliedSyntaxVersions": syntax_seen.len(), "suppliedSyntaxBytesEstimate": syntax_bytes,
                "snapshotSourceBytesUnique": source_bytes,
                "byClause": by_clause.into_iter().map(|((rule, clause), (jobs, rows))|
                    serde_json::json!({"rule": rule, "clause": clause, "jobs": jobs, "rows": rows}))
                    .collect::<Vec<_>>(),
                "deferredInputJobs": jobs.iter().filter(|j|j.inputs.rows.is_none()).count(),
                "accounting": "Queue holds unmaterialized consumer descriptions. Referenced Arc payloads can also belong to the workspace/document cache; do not sum these categories as exclusive RSS. Syntax size uses the existing cache estimate. Census work is included in this diagnostic span."})
        });
        drop(preparation);
    }
    let owners = catalog
        .iter()
        .map(|(index, rule, _)| (std::sync::Arc::as_ptr(rule) as usize, *index))
        .collect::<std::collections::HashMap<_, _>>();
    let evaluations = relational::evaluate_prepared(jobs, |inputs, code_rule, clause, phase| {
        #[cfg(feature = "code-rule-profile")]
        let mut materialization = crate::code_rule_dsl::profile::trace::Span::start(
            "input-preparation",
            "admitted-relational-inputs",
        );
        let ClauseKind::Relational(plan) = &clause.kind else {
            return Err(invalid("expected relational job"));
        };
        if plan.per_file {
            let input = &mut inputs.snapshot.files[0];
            let path = input.path.clone();
            let version = super::documents::current_version(state, &path)
                .ok_or_else(|| invalid("relational source version unavailable"))?;
            let (_, document) = super::documents::load(state, &path)?;
            input.source = std::sync::Arc::clone(&document.source);
            if let Some(language) = crate::code_rule_dsl::eval::file_language(input) {
                crate::code_rule_dsl::eval::TapeCacheSlot::seed(language, input, &document.image)?;
            }
            if phase == relational::Preparation::Body && plan.syntax.is_some() {
                let facts = match document.syntax.get() {
                    Some(f) => std::sync::Arc::clone(f),
                    None => {
                        let f = relational::syntax::Pool::get(input)?;
                        let _ = document.syntax.set(std::sync::Arc::clone(&f));
                        f
                    }
                };
                inputs.facts.push((path, facts.clone()));
                relational::syntax::Pool::seed(input, facts);
                state.documents.account_syntax(&version);
            }
            inputs.file_local = true;
        } else {
            let index = owners[&(std::ptr::from_ref(code_rule) as usize)];
            let producers = &state.relation_plans[&(index, clause.id.clone())];
            if phase == relational::Preparation::Gate {
                let slot = plan.require_nonempty.expect("guarded preparation");
                inputs.gate = Some(relation_rows(state, producers[slot], &plan.inputs[slot].1));
            } else {
                inputs.rows = Some(
                    plan.inputs
                        .iter()
                        .enumerate()
                        .zip(producers)
                        .map(|((slot, (_, query)), id)| {
                            if inputs.gate.is_some() && plan.require_nonempty == Some(slot) {
                                evidence::Rows::default()
                            } else {
                                relation_rows(state, *id, query)
                            }
                        })
                        .collect(),
                );
                if let Some(spec) = &plan.syntax {
                    let gate_slot = plan
                        .inputs
                        .iter()
                        .position(|(name, _)| name == &spec.gate)
                        .expect("syntax gate");
                    let rows = if plan.require_nonempty == Some(gate_slot) {
                        inputs.gate.as_ref().expect("admitted gate")
                    } else {
                        &inputs.rows.as_ref().unwrap()[gate_slot]
                    };
                    let paths = rows
                        .witnesses
                        .iter()
                        .map(|w| w.path.clone())
                        .collect::<BTreeSet<_>>();
                    let paths = paths.into_iter().collect::<Vec<_>>();
                    let file_slots = inputs
                        .snapshot
                        .files
                        .iter()
                        .enumerate()
                        .map(|(slot, file)| (file.path.clone(), slot))
                        .collect::<std::collections::HashMap<_, _>>();
                    let mut remaining = code_rule.capabilities.limits.max_source_bytes;
                    for paths in paths.chunks(128) {
                        let documents = super::documents::load_many(state, paths, remaining)?;
                        for (path, (version, document)) in paths.iter().zip(documents) {
                            remaining = remaining.saturating_sub(document.source.len());
                            let slot = *file_slots
                                .get(path)
                                .ok_or_else(|| invalid("syntax source snapshot is incomplete"))?;
                            let input = &mut inputs.snapshot.files[slot];
                            input.source = std::sync::Arc::clone(&document.source);
                            if let Some(language) = crate::code_rule_dsl::eval::file_language(input)
                            {
                                crate::code_rule_dsl::eval::TapeCacheSlot::seed(
                                    language,
                                    input,
                                    &document.image,
                                )?;
                            }
                            let facts = match document.syntax.get() {
                                Some(facts) => std::sync::Arc::clone(facts),
                                None => {
                                    let facts = relational::syntax::Pool::get(input)?;
                                    let _ = document.syntax.set(std::sync::Arc::clone(&facts));
                                    facts
                                }
                            };
                            inputs.facts.push((path.clone(), facts.clone()));
                            relational::syntax::Pool::seed(input, facts);
                            state.documents.account_syntax(&version);
                        }
                    }
                }
            }
        }
        #[cfg(feature="code-rule-profile")]
        materialization.detail(|| {
            let mut memory=super::memory::census::WitnessMemory::default();
            let count=inputs.rows.iter().flatten().map(|rows|{for witness in &rows.witnesses {memory.visit(witness);}rows.witnesses.len()}).sum::<usize>();
            serde_json::json!({"jobs":1,"ownership":memory,"byClause":[{"rule":code_rule.id,"clause":clause.id,"jobs":1,"rows":count}],"accounting":"Scheduler restores one consumer. File-local columns stream during session import; this boundary retains no file-local projected rows. Source references overlap workspace storage."})
        });
        Ok(())
    });
    for ((index, name, key), evaluation) in keys.into_iter().zip(evaluations) {
        if let Some((key, version, _path)) = key
            && evaluation.failures.is_empty()
        {
            state
                .file_relation_results
                .insert(key, (version, evaluation.clone()));
        }
        let result = results.get_mut(&index).unwrap().get_mut(&name).unwrap();
        result.violations.extend(evaluation.violations);
        result.failures.extend(evaluation.failures);
        result.metrics.merge(evaluation.metrics);
    }
    Ok(results)
}
