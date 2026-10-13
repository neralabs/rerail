use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::json::meta_name;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    FileScopeSlot, GrammarMatcher, UtilLimit, WITNESS_NODE, export_text, exported_span,
    is_span_variable,
};
use crate::code_rule_dsl::model::{
    CodeRuleScope, FileInput, Limits, Metrics, Query, RegexSource, Snapshot, ViolationSpec,
};
use crate::code_rule_dsl::query::select_when::{self, LeafRow, Leaves, Plan};
use crate::code_rule_dsl::soundness::SyntaxErrors;
use crate::code_rule_dsl::violations::{self, Captures, Span, Witness};
use ast_grep_core::Doc;
use ast_grep_core::matcher::NodeMatch;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use napi::Result;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub(crate) enum WitnessError {
    TransformLimit,
}

type WitnessResult<T> = std::result::Result<T, WitnessError>;

#[cfg(test)]
thread_local! {
    /// Lookups of one group in one each-set made on this thread to decide which groups the sets
    /// share (`regrouped`).
    static SHARED_GROUP_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_shared_group_lookups() {
    SHARED_GROUP_LOOKUPS.set(0);
}

#[cfg(test)]
pub(crate) fn shared_group_lookups() -> usize {
    SHARED_GROUP_LOOKUPS.get()
}

/// `violation` is the reporting clause's violation spec; `None` for witnesses that only decide a
/// `select_when` condition and are never reported.
pub(crate) fn witness_from_match<D: Doc>(
    file: &FileInput,
    matched: &NodeMatch<'_, D>,
    language: GrammarDialect,
    violation: Option<&ViolationSpec>,
    span: impl Fn(usize, usize) -> Span,
) -> WitnessResult<Witness> {
    let groups = violation.map_or_else(Vec::new, |violation| {
        violations::node_groups(violation, language, matched.get_node())
    });
    let sets = std::slice::from_ref(&groups);
    let env = regrouped(matched, violation, sets, || false)?
        .pop()
        .flatten();
    witness_with_groups(file, matched, language, violation, &groups, env, span)
}

/// The witnesses one match reports under `violation.each`: one, or one per match of a regex group
/// `each` names (`violations::node_group_sets`). Once `cancelled` holds, the sets not yet enumerated
/// or transformed report nothing.
pub(crate) fn witnesses_from_match<D: Doc>(
    file: &FileInput,
    matched: &NodeMatch<'_, D>,
    language: GrammarDialect,
    violation: &ViolationSpec,
    room: usize,
    cancelled: impl Fn() -> bool,
    span: impl Fn(usize, usize) -> Span,
) -> WitnessResult<Vec<Witness>> {
    let sets = violations::node_group_sets(violation, language, matched, room, &cancelled);
    let envs = regrouped(matched, Some(violation), &sets, &cancelled)?;
    sets.iter()
        .zip(envs)
        .map(|(groups, env)| {
            witness_with_groups(file, matched, language, Some(violation), groups, env, &span)
        })
        .collect()
}

/// The environment each set of the witness's node groups reads, after the violation's group
/// transform ran over it (`None` for a set without groups, or a violation without the transform).
/// The groups the transform reads join the environment as text, as a parsed tree's exports do;
/// every other reader of a group (message, fix range, projection) finds it in the set.
///
/// Where several sets hold a group alike (the same text at the same range), the steps reading it
/// compute the same output for every set: they run once over the group, and each set receives
/// their output instead of its own copy of the group.
///
/// `cancelled` is polled before each set; once it holds, the remaining sets get no environment
/// (the returned list ends early).
fn regrouped<'t, D: Doc>(
    matched: &NodeMatch<'t, D>,
    violation: Option<&ViolationSpec>,
    sets: &[Vec<violations::NodeGroup>],
    cancelled: impl Fn() -> bool,
) -> WitnessResult<Vec<Option<MetaVarEnv<'t, D>>>> {
    let limit = |_| WitnessError::TransformLimit;
    let Some(transform) = violation
        .and_then(|violation| violation.group_transform.as_deref())
        .filter(|_| sets.iter().any(|groups| !groups.is_empty()))
    else {
        return Ok(sets.iter().map(|_| None).collect());
    };
    let exported =
        |name: &str| transform.reads(name) && matched.get_env().get_match(name).is_none();
    let held = |groups: &[violations::NodeGroup], name: &str| {
        #[cfg(test)]
        SHARED_GROUP_LOOKUPS.set(SHARED_GROUP_LOOKUPS.get() + 1);
        groups
            .iter()
            .find(|(group, _, _)| group == name)
            .map(|(_, text, range)| (Arc::as_ptr(text), range.clone()))
    };
    // The exported groups every set holds alike, decided once: a pass over the sets for each
    // exported group of the first, ending at the first set that differs.
    let shared = match sets {
        [first, rest @ ..] if !rest.is_empty() => first
            .iter()
            .map(|(name, _, _)| name.as_str())
            .filter(|name| exported(name))
            .filter(|name| {
                let held_first = held(first, name);
                rest.iter().all(|groups| held(groups, name) == held_first)
            })
            .collect::<HashSet<_>>(),
        _ => HashSet::new(),
    };
    // A step runs per set unless several sets share every group its chain reads.
    let per_set = transform
        .steps
        .iter()
        .map(|(key, _)| {
            sets.len() == 1
                || transform.roots(key).into_iter().any(|root| {
                    exported(root)
                        && sets.iter().flatten().any(|(group, _, _)| group == root)
                        && !shared.contains(root)
                })
        })
        .collect::<Vec<_>>();
    let mut base = matched.get_env().clone();
    // `line` and `lines` place a group in the file through a node of it: the witness.
    base.insert(WITNESS_NODE, matched.get_node().clone());
    if per_set.contains(&false) {
        let mut once = base.clone();
        for (name, text, range) in &sets[0] {
            if shared.contains(name.as_str()) {
                export_text(&mut once, name, text, range.clone());
            }
        }
        transform
            .apply_steps(&mut once, |index| !per_set[index])
            .map_err(limit)?;
        for (index, (key, _)) in transform.steps.iter().enumerate() {
            if let Some(output) = once.get_transformed(key).filter(|_| !per_set[index]) {
                // The output is already reindented to its source: insert it without a source node.
                base.insert_transformation(&MetaVariable::Multiple, key, output.clone());
            }
        }
    }
    let mut base = Some(base);
    let mut envs = Vec::with_capacity(sets.len());
    for (index, groups) in sets.iter().enumerate() {
        if cancelled() {
            break;
        }
        if groups.is_empty() {
            envs.push(None);
            continue;
        }
        let mut env = if index + 1 == sets.len() {
            base.take()
                .expect("the last set takes the base environment")
        } else {
            base.clone()
                .expect("the base environment outlives every set but the last")
        };
        for (name, text, range) in groups {
            if exported(name) && !shared.contains(name.as_str()) {
                export_text(&mut env, name, text, range.clone());
            }
        }
        transform
            .apply_steps(&mut env, |index| per_set[index])
            .map_err(limit)?;
        envs.push(Some(env));
    }
    Ok(envs)
}

fn witness_with_groups<'t, D: Doc>(
    file: &FileInput,
    matched: &NodeMatch<'t, D>,
    language: GrammarDialect,
    violation: Option<&ViolationSpec>,
    groups: &[violations::NodeGroup],
    regrouped: Option<MetaVarEnv<'t, D>>,
    span: impl Fn(usize, usize) -> Span,
) -> WitnessResult<Witness> {
    let mut captures = BTreeMap::new();
    let mut variables = matched
        .get_env()
        .get_matched_variables()
        .collect::<Vec<_>>();
    variables.sort_by_key(meta_name);
    for variable in variables {
        match variable {
            MetaVariable::Capture(name, _) => {
                if let Some(capture) = matched.get_env().get_match(&name) {
                    captures.insert(name, vec![span(capture.range().start, capture.range().end)]);
                } else if !is_span_variable(&name)
                    && let Some(range) = exported_span(matched.get_env(), &name)
                {
                    captures.insert(name, vec![span(range.start, range.end)]);
                }
            }
            MetaVariable::MultiCapture(name) => {
                let spans = matched
                    .get_env()
                    .get_multiple_matches(&name)
                    .into_iter()
                    .filter(|capture| capture.is_named())
                    .map(|capture| span(capture.range().start, capture.range().end))
                    .collect::<Vec<_>>();
                captures.insert(name, spans);
            }
            MetaVariable::Dropped(_) | MetaVariable::Multiple => {}
        }
    }
    let node = matched.get_node();
    for (name, _, range) in groups {
        captures
            .entry(name.clone())
            .or_insert_with(|| vec![span(range.start, range.end)]);
    }
    let regrouped = regrouped.map(|env| NodeMatch::new(node.clone(), env));
    let matched = regrouped.as_ref().unwrap_or(matched);
    let (message, fix) = violation.map_or((None, None), |violation| {
        violations::ast_output(violation, language, matched, groups, &span)
    });
    let mut captures = Captures::from_spans(captures);
    if let Some(violation) = violation.filter(|violation| !violation.project.is_empty()) {
        captures.set_projection(Some(super::evidence::project(
            &violation.project,
            file,
            matched,
            groups,
        )));
    }
    Ok(Witness {
        path: file.path.clone(),
        span: span(node.range().start, node.range().end),
        captures,
        message,
        fix,
    })
}

/// `partial` receives each file evaluated around syntax errors, with its `source-partial` message;
/// `util_limit` the file where `matches` recursion hit a limit, which ends the query.
#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate_query(
    query: &Query,
    snapshot: &Snapshot,
    _bindings: &BTreeMap<String, &Map<String, Value>>,
    scope: &CodeRuleScope,
    metrics: &mut Metrics,
    limits: &Limits,
    decision: bool,
    partial: &mut BTreeMap<String, String>,
    violation: Option<&ViolationSpec>,
    util_limit: &mut Option<(String, UtilLimit)>,
) -> Result<Option<Vec<Witness>>> {
    let leaf = Leaf {
        snapshot,
        scope,
        limits,
    };
    if let Query::SelectWhen { .. } = query {
        return leaf.select_when(query, metrics, decision, partial, violation, util_limit);
    }
    let mut witnesses = Vec::new();
    for file in &snapshot.files {
        match leaf.file(query, file, metrics, decision, violation, util_limit)? {
            FileStep::Skipped => {}
            FileStep::Failed => return Ok(None),
            FileStep::Read {
                witnesses: found,
                partial: message,
                decided,
            } => {
                if let Some(message) = message {
                    partial.insert(file.path.clone(), message);
                }
                witnesses.extend(found);
                if decided {
                    return Ok(Some(witnesses));
                }
            }
        }
    }
    Ok(Some(witnesses))
}

/// What one file contributes to a regex or AST query.
enum FileStep {
    /// The query does not read the file: it is outside the scope, or no grammar of it applies.
    Skipped,
    /// The file's witnesses, with its `source-partial` message where it was read around syntax
    /// errors; `decided` where a decision needs nothing more.
    Read {
        witnesses: Vec<Witness>,
        partial: Option<String>,
        decided: bool,
    },
    /// The query cannot answer: syntax errors it is not sound under, an unfinished text scan, or
    /// a limit (left in `util_limit`).
    Failed,
}

/// Where a query's leaves read: the snapshot's files in the code rule's scope.
#[derive(Clone, Copy)]
struct Leaf<'a> {
    snapshot: &'a Snapshot,
    scope: &'a CodeRuleScope,
    limits: &'a Limits,
}

impl<'a> Leaf<'a> {
    /// One file of a regex or AST `query`.
    fn file(
        self,
        query: &Query,
        file: &FileInput,
        metrics: &mut Metrics,
        decision: bool,
        violation: Option<&ViolationSpec>,
        util_limit: &mut Option<(String, UtilLimit)>,
    ) -> Result<FileStep> {
        if !self.scope.matches(&file.path) {
            return Ok(FileStep::Skipped);
        }
        let mut witnesses = Vec::new();
        match query {
            Query::Regex { regex, source } => {
                let haystack = match source {
                    RegexSource::Content => &file.source,
                    RegexSource::Path => &file.path,
                };
                let lines = LineTable::new(
                    haystack,
                    file_language(file)
                        .map(Terminators::of)
                        .unwrap_or(Terminators::Eslint),
                );
                let span = |start, end| lines.span(start, end);
                let (matches, complete) = crate::code_rule_dsl::query::regex_matches(
                    regex,
                    haystack,
                    self.limits.max_witnesses.saturating_add(1),
                );
                for range in matches {
                    let (start, end) = (range.start, range.end);
                    let found = regex
                        .captures_at(haystack, start)
                        .expect("a regex match has captures");
                    assert_eq!(found.get(0).expect("whole match").range(), start..end);
                    metrics.witnesses += 1;
                    if regex.capture_names().flatten().next().is_none()
                        && violation.is_none_or(|violation| violation.fix.is_none())
                    {
                        witnesses.push(Witness {
                            path: file.path.clone(),
                            span: span(start, end),
                            captures: Captures::default(),
                            message: None,
                            fix: None,
                        });
                    } else {
                        witnesses.push(violations::regex_witness(
                            &file.path, regex, &found, violation, span,
                        ));
                    }
                    if decision {
                        return Ok(FileStep::Read {
                            witnesses,
                            partial: None,
                            decided: true,
                        });
                    }
                }
                if !complete {
                    return Ok(FileStep::Failed);
                }
                Ok(FileStep::Read {
                    witnesses,
                    partial: None,
                    decided: false,
                })
            }
            Query::Ast { matchers, .. } => {
                let Some(grammar) = grammar_matcher(matchers, file) else {
                    return Ok(FileStep::Skipped);
                };
                let (doc, lines) = file_tape(grammar.language, file)?;
                // From the tape: a Blade file's embedded PHP is part of it (`blade::view`).
                let errors = SyntaxErrors::from_ranges(doc.syntax_error_ranges());
                let mut partial = None;
                if !errors.is_empty() {
                    if !grammar.sound_under_errors {
                        return Ok(FileStep::Failed);
                    }
                    partial = Some(errors.partial_message(&lines));
                }
                let mut util_work = self.limits.max_visited_nodes;
                let _scopes =
                    FileScopeSlot::install(Vec::new(), file.module.clone(), !errors.is_empty())
                        .path(&file.path);
                let _calls = crate::code_rule_dsl::calls::FileCallsSlot::for_path(&file.path);
                let decided = match evaluate_file_clause(
                    grammar,
                    file,
                    doc,
                    Arc::clone(&lines),
                    &errors,
                    metrics,
                    self.limits,
                    decision,
                    violation,
                    &mut util_work,
                    &mut witnesses,
                )? {
                    FileOutcome::Continue | FileOutcome::Stopped => false,
                    FileOutcome::Decided => true,
                    FileOutcome::Limit(limit) => {
                        *util_limit = Some((file.path.clone(), limit));
                        return Ok(FileStep::Failed);
                    }
                };
                Ok(FileStep::Read {
                    witnesses,
                    partial,
                    decided,
                })
            }
            Query::SelectWhen { .. } => unreachable!("a select_when is not a leaf"),
        }
    }

    /// A `select_when` answered by `query::select_when`, each leaf read from the snapshot as the
    /// answer asks for its rows.
    fn select_when(
        self,
        query: &'a Query,
        metrics: &mut Metrics,
        decision: bool,
        partial: &mut BTreeMap<String, String>,
        violation: Option<&ViolationSpec>,
        util_limit: &mut Option<(String, UtilLimit)>,
    ) -> Result<Option<Vec<Witness>>> {
        let plan = Plan::new(query, decision, |leaf, _| leaf);
        let leaves = SnapshotLeaves {
            leaf: self,
            plan: &plan,
            violation,
            work: RefCell::new((metrics, util_limit)),
            reads: plan.leaves.iter().map(|_| RefCell::default()).collect(),
            error: RefCell::new(None),
        };
        let cap = self.limits.max_witnesses.saturating_add(1);
        let answer = match select_when::answer(&plan, cap, &leaves) {
            Ok(answer) => answer,
            Err(message) => return Err(leaves.error.take().unwrap_or_else(|| invalid(message))),
        };
        let Some(answer) = answer else {
            return Ok(None);
        };
        for (leaf, position) in answer.partial {
            let read = leaves.reads[leaf].borrow();
            let row = &read.rows[position];
            if let Some(message) = &row.partial {
                partial.insert(self.snapshot.files[row.file].path.clone(), message.clone());
            }
        }
        Ok(Some(
            answer
                .witnesses
                .into_iter()
                .flat_map(|(leaf, position, take)| {
                    leaves.reads[leaf].borrow().rows[position].witnesses[..take].to_vec()
                })
                .collect(),
        ))
    }
}

/// A `select_when`'s leaves, each evaluated file by file, in snapshot order, as the answer
/// reads its rows. A leaf ends where its evaluation would: at a failure, or (read for a decision)
/// at its first witness.
struct SnapshotLeaves<'a, 'w> {
    leaf: Leaf<'a>,
    plan: &'a Plan<&'a Query>,
    violation: Option<&'a ViolationSpec>,
    work: RefCell<(&'w mut Metrics, &'w mut Option<(String, UtilLimit)>)>,
    reads: Vec<RefCell<SnapshotRead>>,
    /// The error that ended the answer, returned in its place.
    error: RefCell<Option<napi::Error>>,
}

#[derive(Default)]
struct SnapshotRead {
    /// The next snapshot file to evaluate.
    next: usize,
    ended: bool,
    rows: Vec<SnapshotRow>,
}

struct SnapshotRow {
    /// The row's snapshot file.
    file: usize,
    row: LeafRow,
    witnesses: Vec<Witness>,
    partial: Option<String>,
}

impl Leaves for SnapshotLeaves<'_, '_> {
    fn row(&self, leaf: usize, position: usize) -> std::result::Result<Option<LeafRow>, String> {
        let spec = &self.plan.leaves[leaf];
        let mut read = self.reads[leaf].borrow_mut();
        while read.rows.len() <= position && !read.ended {
            let file = read.next;
            let Some(input) = self.leaf.snapshot.files.get(file) else {
                read.ended = true;
                break;
            };
            read.next += 1;
            let mut work = self.work.borrow_mut();
            let (metrics, util_limit) = &mut *work;
            let step = self.leaf.file(
                spec.id,
                input,
                metrics,
                spec.decision,
                self.violation.filter(|_| spec.reported),
                util_limit,
            );
            let (row, ends) = match step {
                Err(error) => {
                    let message = error.to_string();
                    *self.error.borrow_mut() = Some(error);
                    return Err(message);
                }
                Ok(FileStep::Skipped) => continue,
                Ok(FileStep::Failed) => (
                    SnapshotRow {
                        file,
                        row: LeafRow {
                            failed: true,
                            partial: false,
                            witnesses: 0,
                        },
                        witnesses: Vec::new(),
                        partial: None,
                    },
                    true,
                ),
                Ok(FileStep::Read {
                    witnesses,
                    partial,
                    decided,
                }) => (
                    SnapshotRow {
                        file,
                        row: LeafRow {
                            failed: false,
                            partial: partial.is_some(),
                            witnesses: witnesses.len(),
                        },
                        witnesses,
                        partial,
                    },
                    decided,
                ),
            };
            read.rows.push(row);
            read.ended = ends;
        }
        Ok(read.rows.get(position).map(|row| row.row))
    }
}

pub(super) fn query_failure_path<'a>(
    query: &Query,
    snapshot: &'a Snapshot,
    scope: &CodeRuleScope,
) -> Option<&'a str> {
    match query {
        Query::Ast { matchers, .. } => snapshot
            .files
            .iter()
            .find(|file| {
                scope.matches(&file.path)
                    && grammar_matcher(matchers, file).is_some_and(|grammar| {
                        !grammar.sound_under_errors
                            && file_tape(grammar.language, file)
                                .is_ok_and(|(doc, _)| !doc.syntax_error_ranges().is_empty())
                    })
            })
            .map(|file| file.path.as_str()),
        Query::SelectWhen { select, exists } => query_failure_path(exists, snapshot, scope)
            .or_else(|| query_failure_path(select, snapshot, scope)),
        Query::Regex { .. } => None,
    }
}

pub(crate) fn file_language(file: &FileInput) -> Option<GrammarDialect> {
    crate::code_rule_dsl::language::dialect_by_label(&file.language)
}

/// What the stateless evaluator does after one file's clause.
enum FileOutcome {
    /// Go on with the next file.
    Continue,
    /// A decision needed only this witness.
    Decided,
    /// The witness budget ran out in this file; go on with the next.
    Stopped,
    /// A utility, scope or value limit was hit.
    Limit(UtilLimit),
}

/// A file's tape, as the workspace stores it, and its line table.
pub(super) fn file_tape(
    language: GrammarDialect,
    file: &FileInput,
) -> Result<(crate::code_rule_dsl::tape::TapeDoc, Arc<LineTable>)> {
    use crate::code_rule_dsl::tape;
    let never = || false;
    let tape_grammar = tape::Grammar::new(language).map_err(invalid)?;
    let image = tape::TapeImage::parse(&tape_grammar, &file.source, &never)
        .ok_or_else(|| invalid("file could not be parsed"))?
        .map_err(invalid)?;
    let lines = Arc::new(LineTable::new(&file.source, Terminators::of(language)));
    #[cfg(test)]
    crate::code_rule_dsl::matcher::note_line_table_construction();
    let doc = tape::TapeDoc::restore_with_lines(
        Arc::clone(&file.source),
        tape_grammar,
        &image,
        Arc::clone(&lines),
        &never,
    )
    .map_err(invalid)?;
    Ok((doc, lines))
}

/// Evaluates one file with its grammar matcher on the file's tape: nodes count towards
/// `maxVisitedNodes` in preorder, witnesses in syntax-error regions and suppressed ones are
/// dropped, the witness budget ends the file.
#[allow(clippy::too_many_arguments)]
fn evaluate_file_clause(
    grammar: &GrammarMatcher,
    file: &FileInput,
    doc: crate::code_rule_dsl::tape::TapeDoc,
    lines: Arc<LineTable>,
    errors: &SyntaxErrors,
    metrics: &mut Metrics,
    limits: &Limits,
    decision: bool,
    violation: Option<&ViolationSpec>,
    util_work: &mut usize,
    witnesses: &mut Vec<Witness>,
) -> Result<FileOutcome> {
    use crate::code_rule_dsl::json::invalid;
    use crate::code_rule_dsl::matcher::{Stopped, enumerate, node_at, run_clause, with_file_memo};
    let count = doc.transducer_len();
    let target = ast_grep_core::AstGrep::doc(doc);
    let visited = metrics.visited_nodes;
    let each = violation.and_then(|violation| violation.each.as_deref());
    let mut outcome = FileOutcome::Continue;
    let result = with_file_memo(&target, || {
        let _first = each.map(crate::code_rule_dsl::matcher::EachScope::start);
        run_clause(
            &target,
            grammar,
            None,
            limits.max_visited_nodes.saturating_sub(visited),
            util_work,
            |node, first, work| {
                let matches = match each {
                    Some(name) => {
                        let room = (limits.max_witnesses + 1).saturating_sub(metrics.witnesses);
                        enumerate(&target, grammar, node, first, work, name, room)?
                    }
                    None => vec![first],
                };
                for env in matches {
                    let mut matched = NodeMatch::new(node_at(&target, node), env);
                    if let Some(transform) = &grammar.transform {
                        crate::code_rule_dsl::matcher::begin_clause(grammar, *work);
                        if transform.apply(matched.get_env_mut()).is_err() {
                            *work = crate::code_rule_dsl::matcher::end_clause().0;
                            outcome = FileOutcome::Limit(UtilLimit::Work);
                            return Ok(false);
                        }
                        *work = crate::code_rule_dsl::matcher::end_clause().0;
                    }
                    if errors.overlaps(&matched.get_node().range()) {
                        continue;
                    }
                    let span = |start, end| lines.span(start, end);
                    let room = (limits.max_witnesses + 1).saturating_sub(metrics.witnesses);
                    let found = match violation.filter(|_| each.is_some()) {
                        Some(violation) => witnesses_from_match(
                            file,
                            &matched,
                            grammar.language,
                            violation,
                            room,
                            || false,
                            span,
                        ),
                        None => {
                            witness_from_match(file, &matched, grammar.language, violation, span)
                                .map(|witness| vec![witness])
                        }
                    };
                    let found = match found {
                        Ok(found) => found,
                        Err(WitnessError::TransformLimit) => {
                            outcome = FileOutcome::Limit(UtilLimit::Work);
                            return Ok(false);
                        }
                    };
                    for witness in found {
                        if crate::code_rule_dsl::violations::suppresses(
                            violation,
                            matched.get_node(),
                            &witness,
                        ) {
                            continue;
                        }
                        metrics.witnesses += 1;
                        if metrics.witnesses > limits.max_witnesses {
                            metrics.visited_nodes = visited + node as usize + 1;
                            outcome = FileOutcome::Stopped;
                            return Ok(false);
                        }
                        witnesses.push(witness);
                        if decision {
                            metrics.visited_nodes = visited + node as usize + 1;
                            outcome = FileOutcome::Decided;
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            },
        )
        .and_then(|result| result)
    });
    match result {
        Ok(()) => {}
        Err(Stopped::Limit(limit)) => return Ok(FileOutcome::Limit(limit)),
        Err(Stopped::Deadline) => return Err(invalid("evaluation deadline exhausted")),
    }
    if !matches!(outcome, FileOutcome::Continue) {
        return Ok(outcome);
    }
    metrics.visited_nodes = visited + count;
    if metrics.visited_nodes > limits.max_visited_nodes {
        return Ok(FileOutcome::Limit(UtilLimit::Work));
    }
    Ok(FileOutcome::Continue)
}

pub(crate) fn grammar_matcher<'a>(
    matchers: &'a [GrammarMatcher],
    file: &FileInput,
) -> Option<&'a GrammarMatcher> {
    let language = file_language(file)?;
    matchers.iter().find(|grammar| {
        grammar.language == language
            && grammar.applies
            && grammar
                .file
                .as_ref()
                .is_none_or(|condition| condition.holds(&file.path, file.module.as_deref()))
    })
}
