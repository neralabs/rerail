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
use crate::code_rule_dsl::origin::InFile;
use crate::code_rule_dsl::query::select_when::{self, LeafRow, Leaves, Plan};
use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading};
use crate::code_rule_dsl::violations::{self, Captures, Witness};
use ast_grep_core::matcher::NodeMatch;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
use napi::Result;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
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
) -> WitnessResult<Witness> {
    let groups = violation.map_or_else(Vec::new, |violation| {
        violations::node_groups(violation, language, matched.get_node())
    });
    let sets = std::slice::from_ref(&groups);
    let env = regrouped(matched, violation, sets, || false)?
        .pop()
        .flatten();
    witness_with_groups(file, matched, language, violation, &groups, env)
}

/// The witnesses one match reports under `violation.each`: one, or one per match of a regex group
/// `each` names (`violations::each_node_group_set`), each passed to `keep` as it is built. `keep`
/// drops the witnesses it refuses (those a suppression comment silences), which spend none of
/// `room`: building ends once `room` witnesses were kept. Once `cancelled` holds, the sets not
/// yet enumerated or transformed report nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn witnesses_from_match<D: Doc>(
    file: &FileInput,
    matched: &NodeMatch<'_, D>,
    language: GrammarDialect,
    violation: &ViolationSpec,
    room: usize,
    cancelled: impl Fn() -> bool,
    mut keep: impl FnMut(&mut Witness) -> bool,
) -> WitnessResult<Vec<Witness>> {
    let mut kept = Vec::new();
    // The sets are transformed and built a batch at a time, each batch as large as the room
    // left: where `keep` refuses none, one batch holds every set the room allows.
    let mut build = |batch: &mut Vec<Vec<violations::NodeGroup>>, kept: &mut Vec<Witness>| {
        let envs = regrouped(matched, Some(violation), batch, &cancelled)?;
        for (groups, env) in batch.iter().zip(envs) {
            let mut witness =
                witness_with_groups(file, matched, language, Some(violation), groups, env)?;
            if keep(&mut witness) {
                kept.push(witness);
            }
        }
        batch.clear();
        WitnessResult::Ok(())
    };
    let mut batch = Vec::new();
    let mut failed = Ok(());
    violations::each_node_group_set(violation, language, matched, &cancelled, |groups| {
        batch.push(groups);
        if kept.len() + batch.len() < room {
            return true;
        }
        failed = build(&mut batch, &mut kept);
        failed.is_ok() && kept.len() < room
    });
    failed?;
    build(&mut batch, &mut kept)?;
    Ok(kept)
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
) -> WitnessResult<Witness> {
    // Every position reported is the file's (`origin`): an embedded program's nodes stand for
    // the file bytes they were built from.
    let place = InFile::of(matched.get_node()).expect("a witness's document is a file's");
    let span = |range: Range<usize>| place.span(place.range(range));
    let node_span = |node: &Node<'t, D>| place.span(place.node(node));
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
                    captures.insert(name, vec![node_span(capture)]);
                } else if !is_span_variable(&name)
                    && let Some(range) = exported_span(matched.get_env(), &name)
                {
                    captures.insert(name, vec![span(range)]);
                }
            }
            MetaVariable::MultiCapture(name) => {
                let spans = matched
                    .get_env()
                    .get_multiple_matches(&name)
                    .into_iter()
                    .filter(|capture| capture.is_named())
                    .map(|capture| node_span(&capture))
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
            .or_insert_with(|| vec![span(range.clone())]);
    }
    let regrouped = regrouped.map(|env| NodeMatch::new(node.clone(), env));
    let matched = regrouped.as_ref().unwrap_or(matched);
    let (message, fix) = violation.map_or((None, None), |violation| {
        violations::ast_output(violation, language, matched, groups, &place)
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
        span: node_span(node),
        captures,
        message,
        fix,
        ignored: false,
    })
}

/// Where a query could not answer: the first file it did not read, with the detail a file the
/// parser did not read reports (`SyntaxReading::Unread`).
pub(crate) struct Incomplete {
    pub(crate) path: String,
    pub(crate) unread: Option<String>,
}

/// The query's witnesses, or where it could not answer (`Incomplete`). `partial` receives each
/// file evaluated around syntax errors, with its `source-partial` message; `util_limit` the file
/// where `matches` recursion hit a limit, which ends the query.
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
) -> Result<std::result::Result<Vec<Witness>, Incomplete>> {
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
            FileStep::Failed(unread) => {
                return Ok(Err(Incomplete {
                    path: file.path.clone(),
                    unread,
                }));
            }
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
                    return Ok(Ok(witnesses));
                }
            }
        }
    }
    Ok(Ok(witnesses))
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
    /// The query cannot answer: syntax errors it is not sound under, an unfinished text scan, a
    /// limit (left in `util_limit`), or a file the parser did not read, with the detail it reports
    /// (`SyntaxReading::Unread`).
    Failed(Option<String>),
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
                            ignored: false,
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
                    return Ok(FileStep::Failed(None));
                }
                Ok(FileStep::Read {
                    witnesses,
                    partial: None,
                    decided: false,
                })
            }
            Query::Ast { matchers, .. } => {
                if grammar_matcher(matchers, file).is_none() {
                    return Ok(FileStep::Skipped);
                }
                let language = file_language(file).expect("a matcher's file has a dialect");
                let (host, lines) = file_tape(language, file)?;
                // The document the clause reads (`embed`): the file's own tree, or the program it
                // embeds; a file without the program the clause reads holds no witness of it.
                let guests = host.guests();
                let Some((doc, grammar)) = std::iter::once(host.clone())
                    .chain(
                        guests
                            .iter()
                            .flat_map(|guests| guests.iter().map(|guest| guest.doc.clone())),
                    )
                    .find_map(|doc| {
                        document_matcher(matchers, file, &doc).map(|grammar| (doc, grammar))
                    })
                else {
                    return Ok(FileStep::Read {
                        witnesses,
                        partial: None,
                        decided: false,
                    });
                };
                // Each document has its own syntax errors, as ranges of the file
                // (`SyntaxErrors::of_doc`): a Blade view reads the template's too.
                let errors = SyntaxErrors::of_doc(&doc);
                if !errors.is_empty() {
                    // A file the parser did not read is not read by any clause, sound or not.
                    if let SyntaxReading::Unread(detail) = errors.reading(&lines, file.source.len())
                    {
                        return Ok(FileStep::Failed(Some(detail)));
                    }
                    if errors.incomplete(grammar) {
                        return Ok(FileStep::Failed(None));
                    }
                }
                // A file read around syntax errors in any of its documents reports the first
                // document's on its clauses, as the workspace does (`FileState::syntax`).
                let partial = std::iter::once(host.clone())
                    .chain(
                        guests
                            .iter()
                            .flat_map(|guests| guests.iter().map(|guest| guest.doc.clone())),
                    )
                    .map(|doc| SyntaxErrors::of_doc(&doc))
                    .find(|errors| !errors.is_empty())
                    .map(|errors| match errors.reading(&lines, file.source.len()) {
                        SyntaxReading::Partial(message) | SyntaxReading::Unread(message) => message,
                    });
                let mut util_work = self.limits.max_visited_nodes;
                let _scopes =
                    FileScopeSlot::install(Vec::new(), file.module.clone(), errors.hide_bindings())
                        .path(&file.path);
                let _calls = crate::code_rule_dsl::calls::FileCallsSlot::for_path(&file.path);
                let decided = match evaluate_file_clause(
                    grammar,
                    file,
                    doc,
                    &errors,
                    metrics,
                    self.limits,
                    decision,
                    violation,
                    self.scope.honours_suppressions_at(&file.path),
                    &mut util_work,
                    &mut witnesses,
                )? {
                    FileOutcome::Continue | FileOutcome::Stopped => false,
                    FileOutcome::Decided => true,
                    FileOutcome::Limit(limit) => {
                        *util_limit = Some((file.path.clone(), limit));
                        return Ok(FileStep::Failed(None));
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
    ) -> Result<std::result::Result<Vec<Witness>, Incomplete>> {
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
            Ok(Ok(answer)) => answer,
            Ok(Err((leaf, position))) => {
                let read = leaves.reads[leaf].borrow();
                let row = &read.rows[position];
                return Ok(Err(Incomplete {
                    path: self.snapshot.files[row.file].path.clone(),
                    unread: row.unread.clone(),
                }));
            }
            Err(message) => return Err(leaves.error.take().unwrap_or_else(|| invalid(message))),
        };
        for (leaf, position) in answer.partial {
            let read = leaves.reads[leaf].borrow();
            let row = &read.rows[position];
            if let Some(message) = &row.partial {
                partial.insert(self.snapshot.files[row.file].path.clone(), message.clone());
            }
        }
        Ok(Ok(answer
            .witnesses
            .into_iter()
            .flat_map(|(leaf, position, take)| {
                leaves.reads[leaf].borrow().rows[position].witnesses[..take].to_vec()
            })
            .collect()))
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
    /// A failed row's detail, where the parser did not read the file.
    unread: Option<String>,
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
                Ok(FileStep::Failed(unread)) => (
                    SnapshotRow {
                        file,
                        row: LeafRow {
                            failed: true,
                            partial: false,
                            witnesses: 0,
                        },
                        witnesses: Vec::new(),
                        partial: None,
                        unread,
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
                        unread: None,
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
/// `maxVisitedNodes` in preorder, witnesses in syntax-error regions are dropped, and so are
/// suppressed ones when `honour_suppressions`, and those a `rerail-ignore` names the code rule
/// over always; the witness budget ends the file.
#[allow(clippy::too_many_arguments)]
fn evaluate_file_clause(
    grammar: &GrammarMatcher,
    file: &FileInput,
    doc: crate::code_rule_dsl::tape::TapeDoc,
    errors: &SyntaxErrors,
    metrics: &mut Metrics,
    limits: &Limits,
    decision: bool,
    violation: Option<&ViolationSpec>,
    honour_suppressions: bool,
    util_work: &mut usize,
    witnesses: &mut Vec<Witness>,
) -> Result<FileOutcome> {
    use crate::code_rule_dsl::json::invalid;
    use crate::code_rule_dsl::matcher::{Stopped, enumerate, node_at, run_clause, with_file_memo};
    let count = doc.transducer_len();
    let target = ast_grep_core::AstGrep::doc(doc.clone());
    let place = InFile::tape(&doc);
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
                // One binding of the match: false ends the clause. A suppressed witness spends
                // none of the violation's allowance, so the enumeration, and a regex capture's
                // groups, go on past suppressed ones until the reported ones overflow it.
                let mut visit = |env, work: &mut usize| {
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
                    if errors.overlaps(&place.node(matched.get_node())) {
                        return Ok(true);
                    }
                    let matched_node = matched.get_node();
                    let keep = |witness: &mut Witness| {
                        if honour_suppressions
                            && violations::suppresses(violation, matched_node, witness)
                        {
                            return false;
                        }
                        if violations::rerail_ignores(violation, matched_node, witness) {
                            if !violation.is_some_and(|violation| violation.ignore_marks) {
                                return false;
                            }
                            witness.ignored = true;
                        }
                        true
                    };
                    let found = match violation.filter(|_| each.is_some()) {
                        Some(violation) => witnesses_from_match(
                            file,
                            &matched,
                            grammar.language,
                            violation,
                            (limits.max_witnesses + 1).saturating_sub(metrics.witnesses),
                            || false,
                            keep,
                        ),
                        None => witness_from_match(file, &matched, grammar.language, violation)
                            .map(|mut witness| {
                                if keep(&mut witness) {
                                    vec![witness]
                                } else {
                                    Vec::new()
                                }
                            }),
                    };
                    let found = match found {
                        Ok(found) => found,
                        Err(WitnessError::TransformLimit) => {
                            outcome = FileOutcome::Limit(UtilLimit::Work);
                            return Ok(false);
                        }
                    };
                    for witness in found {
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
                    Ok(true)
                };
                match each {
                    Some(name) => enumerate(&target, grammar, node, first, work, name, visit),
                    None => visit(first, work),
                }
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
        let reads = match grammar.document {
            None => grammar.language == language,
            // A clause over an embedded program (`ast.embedded`) reads files whose dialect
            // embeds one through its `via` in its dialect.
            Some(via) => language.embeds().contains(&(via, grammar.language)),
        };
        reads && applies(grammar, file)
    })
}

/// Whether a matcher's `when` and `file` conditions hold for `file`.
fn applies(grammar: &GrammarMatcher, file: &FileInput) -> bool {
    grammar.applies
        && grammar
            .file
            .as_ref()
            .is_none_or(|condition| condition.holds(&file.path, file.module.as_deref()))
}

/// The matcher of `matchers` that reads `doc`, a document of `file` (`embed`): the file's own tree,
/// or the program it embeds through a `via`, in the program's dialect. `None` where none reads it.
pub(crate) fn document_matcher<'a>(
    matchers: &'a [GrammarMatcher],
    file: &FileInput,
    doc: &crate::code_rule_dsl::tape::TapeDoc,
) -> Option<&'a GrammarMatcher> {
    let language = *ast_grep_core::Doc::get_lang(doc);
    matchers.iter().find(|grammar| {
        grammar.document == doc.via() && grammar.language == language && applies(grammar, file)
    })
}
