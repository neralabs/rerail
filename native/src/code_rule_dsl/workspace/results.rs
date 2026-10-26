//! Reporting: result serialization, the ordered result page index, and evicted files' states
//! rebuilt from their stored results.

use super::crossfile::CrossFileScheduler;
use super::ingest::{candidate_dispatch, make_file};
use super::persisted::{candidate_catalog, restore_complete_results};
use super::{CandidateSets, ClauseId, FileState, WorkspaceState};
use crate::WORK_POOL;
use crate::code_rule_dsl::eval::{
    coverage, evaluation_json, failure, file_language, result_order, violation_from_witness,
};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Evaluation, Metrics, Query};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::UNREAD;
use napi::Result;
use rayon::prelude::*;
use serde_json::{Value, json};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

// Per-thread so concurrently running tests cannot perturb each other's counts. A result page task
// computes on the thread that runs it, so a test that runs the task observes its own counts.
#[cfg(test)]
thread_local! {
    pub(super) static RESULT_PAGE_WITNESSES_VISITED: Cell<usize> = const { Cell::new(0) };
    pub(super) static RESULT_PAGE_SORTS: Cell<usize> = const { Cell::new(0) };
    pub(super) static RESULT_PAGE_EVICTED_RESTORES: Cell<usize> = const { Cell::new(0) };
    /// Files whose violations a complete result rendered, and complete results that sorted theirs.
    pub(super) static RESULT_RENDERS: Cell<usize> = const { Cell::new(0) };
    pub(super) static RESULT_SORTS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn count(counter: &'static std::thread::LocalKey<Cell<usize>>, amount: usize) {
    counter.set(counter.get() + amount);
}

/// A file failure's reported message: the static one, but for a file the parser did not read the
/// detail `file.syntax` holds (`SyntaxReading::Unread`: how much the regions cover, and where).
fn failure_message<'a>(file: &'a FileState, message: &'a str) -> &'a str {
    match &file.syntax {
        Some(detail) if message == UNREAD => detail,
        _ => message,
    }
}

/// A serialized result, and whether it is all ASCII: the host then copies it into a string as
/// Latin-1, without decoding UTF-8.
#[derive(Debug)]
pub struct ResultText {
    pub(super) text: String,
    ascii: bool,
}

impl ResultText {
    pub(super) fn of(text: String) -> Self {
        let ascii = text.is_ascii();
        Self { text, ascii }
    }
}

impl std::ops::Deref for ResultText {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl napi::bindgen_prelude::TypeName for ResultText {
    fn type_name() -> &'static str {
        "String"
    }

    fn value_type() -> napi::ValueType {
        napi::ValueType::String
    }
}

impl napi::bindgen_prelude::ToNapiValue for ResultText {
    unsafe fn to_napi_value(
        env: napi::sys::napi_env,
        value: Self,
    ) -> Result<napi::sys::napi_value> {
        if !value.ascii {
            // SAFETY: forwarded from the caller, which holds a live environment.
            return unsafe { String::to_napi_value(env, value.text) };
        }
        let mut result = std::ptr::null_mut();
        // SAFETY: the bytes are valid for their length, and ASCII is Latin-1.
        let status = unsafe {
            napi::sys::napi_create_string_latin1(
                env,
                value.text.as_ptr().cast(),
                value.text.len() as isize,
                &mut result,
            )
        };
        if status != napi::sys::Status::napi_ok {
            return Err(invalid("workspace result string could not be created"));
        }
        Ok(result)
    }
}

pub(super) fn profile_result(
    result: ResultText,
    metrics: &Metrics,
    serialization: Duration,
) -> Result<ResultText> {
    if std::env::var_os("RERAIL_CODE_RULE_PROFILE").is_none() {
        return Ok(result);
    }
    let mut value: Value = serde_json::from_str(&result.text).map_err(json_error)?;
    value.as_object_mut().expect("workspace result").insert(
        "phaseClock".to_owned(),
        crate::code_rule_dsl::phase_clock::snapshot(),
    );
    value.as_object_mut().expect("workspace result").insert(
        "workspaceProfile".to_owned(),
        json!({
            "lineIndexNanoseconds": metrics.line_index_nanoseconds,
            "regexNanoseconds": metrics.regex_nanoseconds,
            "astParseNanoseconds": metrics.ast_parse_nanoseconds,
            "astMatchNanoseconds": metrics.ast_match_nanoseconds,
            "serializationNanoseconds": serialization.as_nanos() as u64,
            "astMatcherCalls": metrics.ast_matcher_calls,
            "storageRetries": metrics.storage_retries,
            "storageRetryWaitNanoseconds": metrics.storage_retry_wait_nanoseconds,
            "storageUnpublishedFiles": metrics.storage_unpublished_files,
            "storageReleasedFiles": metrics.storage_released_files,
            "storageTransactionNanoseconds": metrics.storage_transaction_nanoseconds,
            "storageObjectTransactionNanoseconds": metrics.storage_object_transaction_nanoseconds,
            "storageRelationshipTransactionNanoseconds": metrics.storage_relationship_transaction_nanoseconds,
            "storageReadDataStatements": metrics.storage_read_data_statements,
            "storageDataStatements": metrics.storage_data_statements,
            "storageMaxDataStatementsPerPass": metrics.storage_max_data_statements_per_pass,
            "storageTransactionStatements": metrics.storage_transaction_statements,
            "storageReconciliationPasses": metrics.storage_reconciliation_passes,
            "storageReadNanoseconds": metrics.storage_read_nanoseconds,
            "storageManifestNanoseconds": metrics.storage_manifest_nanoseconds,
            "storageWriteNanoseconds": metrics.storage_write_nanoseconds,
            "reusedSources": metrics.reused_sources,
            "reusedTapes": metrics.reused_tapes,
            "compressedSources": metrics.compressed_sources,
            "compressedTapes": metrics.compressed_tapes,
            "restoredTapes": metrics.restored_tapes,
            "restoredCandidateFiles": metrics.restored_candidate_files,
            "avoidedTapeDecompressions": metrics.avoided_tape_decompressions,
            "tapeCacheHits": metrics.tape_cache_hits,
            "tapeDecompressions": metrics.tape_decompressions,
            "restoredLocalResults": metrics.restored_local_results,
        }),
    );
    serde_json::to_string(&value)
        .map(ResultText::of)
        .map_err(json_error)
}

pub(super) struct ResultPageIndex {
    revision: u32,
    /// Each violation as the pages serialize it, in result order. Serialized once when the index is
    /// built, which holds the JSON alone rather than the witnesses it renders.
    violations: Vec<Box<str>>,
    failures: Vec<Value>,
    /// The clauses' coverage from `failures`, sent with them on the first page.
    coverage: Vec<Value>,
    /// The files no clause read (`blind_file`), in path order, sent on the first page.
    blind_files: Vec<Value>,
}

impl ResultPageIndex {
    /// Resident bytes of the open index, estimated like the workspace's own results.
    pub(super) fn estimated_bytes(&self) -> usize {
        use std::mem::size_of;
        let values = |values: &Vec<Value>| {
            values.capacity() * size_of::<Value>()
                + values
                    .iter()
                    .map(|value| value.to_string().len())
                    .sum::<usize>()
        };
        size_of::<Self>()
            + self.violations.capacity() * size_of::<Box<str>>()
            + self
                .violations
                .iter()
                .map(|violation| violation.len())
                .sum::<usize>()
            + values(&self.failures)
            + values(&self.coverage)
            + values(&self.blind_files)
    }
}

/// The workspace's reported clauses, as (rule id, clause id): each has a coverage entry.
fn reported_clauses(state: &WorkspaceState) -> Vec<(Option<String>, String)> {
    state
        .code_rules
        .iter()
        .flat_map(|code_rule| {
            code_rule
                .clauses
                .iter()
                .filter(|clause| !clause.synthetic)
                .map(|clause| (Some(code_rule.id.clone()), clause.id.clone()))
        })
        .collect()
}

/// Evicted files with their stored results, rebuilt for reporting without making them resident:
/// all of them, or those in `only`. With `complete`, an evicted file whose complete results are
/// not in the store is an error, never an omission; without, it holds the results the store has.
pub(super) fn evicted_file_states(
    state: &WorkspaceState,
    only: Option<&BTreeSet<String>>,
    complete: bool,
) -> Result<Vec<FileState>> {
    let evicted = state
        .evicted_inputs
        .iter()
        .filter(|(path, _)| !state.files.contains_key(*path))
        .filter(|(path, _)| only.is_none_or(|only| only.contains(*path)))
        .filter_map(|(path, input)| {
            let key = state.versions.get(path)?.result_key();
            let stamped = CrossFileScheduler::stamped_evicted_key(state, path, key);
            Some((input, key, stamped))
        })
        .collect::<Vec<_>>();
    if evicted.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(test)]
    count(&RESULT_PAGE_EVICTED_RESTORES, evicted.len());
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| invalid("evicted files need the workspace store"))?;
    let keys = evicted
        .iter()
        .flat_map(|(_, key, stamped)| std::iter::once(*key).chain(*stamped))
        .collect::<Vec<_>>();

    #[cfg(test)]
    if super::TEST_EVICTED_READ_FAILURE.get() {
        return Err(invalid("injected evicted-result storage failure"));
    }
    let results = storage
        .store
        .local_check_results_by_key(&keys, candidate_catalog(&state.code_rules)?)
        .map_err(|error| invalid(error.to_string()))?;
    let mut candidate_sets = CandidateSets::new();
    evicted
        .into_iter()
        .map(|(input, key, stamped)| {
            let mut file = make_file(
                input.clone(),
                state.revision.max(1),
                &state.code_rules,
                &mut candidate_sets,
            );
            // A quiet file's consumers match nothing; they are answered, not stored.
            let quiet = CrossFileScheduler::quiet(state, input, &state.code_rules);
            if !quiet.is_empty() {
                let rest = file
                    .candidates
                    .regex
                    .iter()
                    .chain(&file.candidates.ast)
                    .copied()
                    .filter(|id| !quiet.contains(id))
                    .collect();
                file.candidates = Arc::new(candidate_dispatch(
                    file_language(input),
                    rest,
                    &state.code_rules,
                    input,
                ));
                for id in quiet {
                    file.witnesses.insert(id, Vec::new());
                }
            }
            let mut persisted = results
                .get(&key)
                .map_or_else(Vec::new, |results| results.to_vec());
            if let Some(results) = stamped.and_then(|stamped| results.get(&stamped)) {
                persisted.extend(results.iter().cloned());
            }
            let persisted = persisted.as_slice();
            if restore_complete_results(&mut file, &state.code_rules, persisted).is_none()
                && complete
            {
                return Err(invalid(format!(
                    "stored results for evicted {} are unavailable; apply it again",
                    input.path
                )));
            }
            Ok(file)
        })
        .collect()
}

/// The AST clauses with a result on `file`: the ones evaluated on its current parse, restored from
/// the store, or evaluated before a cross-file consumer pass narrowed `candidates` to its consumers
/// (each keeps its result, so a clause that ran around the file's syntax errors is reported partial
/// whichever pass ran it last). Ordered by clause, as the stateless evaluation reports them.
fn reporting_ast_candidates<'a>(
    file: &'a FileState,
    code_rules: &'a [Arc<CompiledCodeRule>],
) -> impl Iterator<Item = ClauseId> + 'a {
    file.witnesses.keys().copied().filter(|id| {
        matches!(
            &code_rules[id.0].clauses[id.1].kind,
            ClauseKind::Forbid(Query::Ast { matchers, .. })
                if crate::code_rule_dsl::eval::grammar_matcher(matchers, &file.input)
                    .is_some_and(|grammar| grammar.invalid_syntax.is_none())
        )
    })
}

/// A file no clause read: every clause with a result on it (`candidates`, or a clause a pass
/// before a consumer pass narrowed them ran) failed for a reason of the whole file, its work
/// (`budget-exceeded` or `file-budget-exceeded`, reason `budget`) or its syntax (reason `syntax`,
/// which wins where both occur), with the message of a failure of that reason. Its syntax is a
/// `source-incomplete` failure for the file's syntax errors or a parse that did not read it
/// (`UNREAD`); any other `source-incomplete` (a source a busy store released) is no blind file, so
/// that failure stays listed and fails the audit. `folded` names each clause (`[ruleId,
/// clauseId]`) whose failure the row stands for: only those are folded into it, so a cross-file
/// query's failure on the file stays listed. `None` for a file any clause read, or with no clause
/// at all.
fn blind_file(state: &WorkspaceState, file: &FileState) -> Option<Value> {
    let clauses = file
        .candidates
        .regex
        .iter()
        .chain(&file.candidates.ast)
        .chain(file.witnesses.keys())
        .filter(|id| !state.code_rules[id.0].clauses[id.1].synthetic)
        .collect::<BTreeSet<_>>();
    let (mut budget, mut syntax) = (None, None);
    for id in &clauses {
        let (code, message) = file.failures.get(id)?;
        let first = match (*code, *message) {
            ("budget-exceeded" | crate::code_rule_dsl::model::FILE_BUDGET_EXCEEDED, _) => {
                &mut budget
            }
            ("source-incomplete", "file has syntax errors" | UNREAD) => &mut syntax,
            _ => return None,
        };
        first.get_or_insert(*message);
    }
    let (reason, message) = match (syntax, budget) {
        (Some(message), _) => ("syntax", message),
        (None, Some(message)) => ("budget", message),
        (None, None) => return None,
    };
    let folded = clauses
        .iter()
        .map(|id| {
            let code_rule = &state.code_rules[id.0];
            json!([code_rule.id, code_rule.clauses[id.1].id])
        })
        .collect::<Vec<_>>();
    Some(json!({
        "path": file.input.path,
        "reason": reason,
        "clauses": clauses.len(),
        "message": message,
        "folded": folded,
    }))
}

/// A violation's place in result order (`result_order`) and its serialization.
type PageEntry = (String, u64, String, String, Box<str>);

fn page_entry(violation: &Value) -> Result<PageEntry> {
    let (path, start, code, id) = result_order(violation);
    Ok((
        path.to_owned(),
        start,
        code.to_owned(),
        id.to_owned(),
        serde_json::to_string(violation)
            .map_err(json_error)?
            .into_boxed_str(),
    ))
}

fn build_result_page_index(state: &WorkspaceState) -> Result<ResultPageIndex> {
    let evicted = evicted_file_states(state, None, true)?;
    let mut witnesses = Vec::new();
    let mut failures = Vec::new();
    let mut blind_files = Vec::new();
    for file in state.files.values().chain(&evicted) {
        blind_files.extend(blind_file(state, file));
        for (&id, file_witnesses) in file.witnesses.iter() {
            if file.failures.contains_key(&id) || state.code_rules[id.0].clauses[id.1].synthetic {
                continue;
            }
            let code_rule = &state.code_rules[id.0];
            let clause = &code_rule.clauses[id.1];
            for witness in file_witnesses {
                #[cfg(test)]
                count(&RESULT_PAGE_WITNESSES_VISITED, 1);
                witnesses.push((code_rule, clause, witness));
            }
        }
        for (&id, &(code, message)) in &file.failures {
            if state.code_rules[id.0].clauses[id.1].synthetic {
                continue;
            }
            let mut item = failure(
                &state.code_rules[id.0].clauses[id.1].id,
                code,
                failure_message(file, message),
                Some(&file.input.path),
            );
            item.as_object_mut()
                .expect("failure object")
                .insert("ruleId".to_owned(), json!(state.code_rules[id.0].id));
            failures.push(item);
        }
        if let Some(message) = &file.syntax {
            for id in reporting_ast_candidates(file, &state.code_rules) {
                if file.failures.contains_key(&id) || state.code_rules[id.0].clauses[id.1].synthetic
                {
                    continue;
                }
                let mut item = failure(
                    &state.code_rules[id.0].clauses[id.1].id,
                    "source-partial",
                    message,
                    Some(&file.input.path),
                );
                item.as_object_mut()
                    .expect("failure object")
                    .insert("ruleId".to_owned(), json!(state.code_rules[id.0].id));
                failures.push(item);
            }
        }
    }
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for (rule, _, _) in &witnesses {
        *counts.entry(rule.id.as_str()).or_default() += 1;
    }
    for (&index, global) in &state.global_results {
        *counts
            .entry(state.code_rules[index].id.as_str())
            .or_default() += global.violations.len();
    }
    let allowed = |rule: &crate::code_rule_dsl::model::CompiledCodeRule| {
        rule.report_count_gt
            .is_none_or(|maximum| counts.get(rule.id.as_str()).copied().unwrap_or(0) > maximum)
    };
    // Each witness's violation is rendered and serialized on the worker pool, one at a time, so the
    // index never holds the violations as JSON values.
    let mut entries = WORK_POOL.install(|| {
        witnesses
            .par_iter()
            .filter(|&&(rule, _, _)| allowed(rule))
            .map(|&(code_rule, clause, witness)| {
                page_entry(&violation_from_witness(code_rule, clause, witness.clone()))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    for (&index, global) in &state.global_results {
        for violation in &global.violations {
            if allowed(&state.code_rules[index]) {
                entries.push(page_entry(violation)?);
            }
        }
        for item in &global.failures {
            let mut item = item.clone();
            item.as_object_mut()
                .expect("failure object")
                .insert("ruleId".to_owned(), json!(state.code_rules[index].id));
            failures.push(item);
        }
    }
    #[cfg(test)]
    count(&RESULT_PAGE_SORTS, 1);
    entries.sort_by(|left, right| {
        (&left.0, left.1, &left.2, &left.3).cmp(&(&right.0, right.1, &right.2, &right.3))
    });
    blind_files
        .sort_by(|left: &Value, right: &Value| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(ResultPageIndex {
        revision: state.revision,
        violations: entries.into_iter().map(|entry| entry.4).collect(),
        coverage: coverage(&reported_clauses(state), &failures),
        failures,
        blind_files,
    })
}

pub(super) fn serialize_result_page(
    state: &mut WorkspaceState,
    cursor: usize,
    maximum_bytes: usize,
) -> Result<String> {
    if cursor == 0 {
        state.result_page_index = Some(build_result_page_index(state)?);
    }
    let index = state
        .result_page_index
        .as_ref()
        .filter(|index| index.revision == state.revision)
        .ok_or_else(|| invalid("result page cursor is stale; restart at cursor 0"))?;
    let total = index.violations.len();
    let mut bytes = 0_usize;
    let start = cursor.min(total);
    let mut next = start;
    for violation in &index.violations[start..] {
        if next > start && bytes.saturating_add(violation.len()) > maximum_bytes {
            break;
        }
        bytes = bytes.saturating_add(violation.len());
        next += 1;
    }
    let done = next >= total;
    // The bytes `serde_json` writes for the page as a map (its keys sorted, values compact),
    // with the violations as the index serialized them.
    let mut page = Vec::with_capacity(bytes + 256);
    let empty = Vec::new();
    let (failures, coverage, blind_files) = if cursor == 0 {
        (&index.failures, &index.coverage, &index.blind_files)
    } else {
        (&empty, &empty, &empty)
    };
    page.extend_from_slice(b"{\"blindFiles\":");
    serde_json::to_writer(&mut page, blind_files).map_err(json_error)?;
    page.extend_from_slice(format!(",\"bytes\":{bytes},\"coverage\":").as_bytes());
    serde_json::to_writer(&mut page, coverage).map_err(json_error)?;
    page.extend_from_slice(format!(",\"cursor\":{next},\"done\":{done},\"failures\":").as_bytes());
    serde_json::to_writer(&mut page, failures).map_err(json_error)?;
    page.extend_from_slice(format!(",\"revision\":{},\"violations\":[", state.revision).as_bytes());
    for (position, violation) in index.violations[start..next].iter().enumerate() {
        if position > 0 {
            page.push(b',');
        }
        page.extend_from_slice(violation.as_bytes());
    }
    page.extend_from_slice(b"]}");
    let page = String::from_utf8(page).map_err(|error| invalid(error.to_string()))?;
    if done {
        state.result_page_index = None;
    }
    Ok(page)
}

/// A file's violations as a complete result serializes them, with what they were rendered under
/// besides its witnesses (`rendering_key`).
pub(super) struct Rendered {
    key: u64,
    violations: Vec<RenderedViolation>,
}

impl Rendered {
    /// Bytes the rendering holds, for memory accounting.
    pub(super) fn bytes(&self) -> usize {
        self.violations
            .iter()
            .map(|violation| {
                std::mem::size_of::<RenderedViolation>()
                    + violation.path.capacity()
                    + violation.code.capacity()
                    + violation.id.capacity()
                    + violation.rule_id.capacity()
                    + violation.json.capacity()
            })
            .sum()
    }
}

/// One violation serialized, with the fields results are ordered by (`result_order`).
pub(super) struct RenderedViolation {
    rule_id: String,
    path: String,
    start: u64,
    code: String,
    id: String,
    json: String,
    ascii: bool,
}

impl RenderedViolation {
    fn of(violation: &Value) -> Result<Self> {
        let (path, start, code, id) = result_order(violation);
        let json = serde_json::to_string(violation).map_err(json_error)?;
        Ok(Self {
            rule_id: violation["ruleId"].as_str().unwrap_or_default().to_owned(),
            path: path.to_owned(),
            start,
            code: code.to_owned(),
            id: id.to_owned(),
            ascii: json.is_ascii(),
            json,
        })
    }

    fn order(&self) -> (&str, u64, &str, &str) {
        (&self.path, self.start, &self.code, &self.id)
    }
}

/// What a file's violations render under besides its witnesses: the code rules (by digest) and the
/// clauses that failed in the file, whose witnesses are not reported.
fn rendering_key(code_rules: u64, file: &FileState) -> u64 {
    let mut hasher = DefaultHasher::new();
    code_rules.hash(&mut hasher);
    for id in file.failures.keys() {
        id.hash(&mut hasher);
    }
    hasher.finish()
}

/// `file`'s reported violations, serialized, in result order.
fn render(state: &WorkspaceState, file: &FileState) -> Result<Vec<RenderedViolation>> {
    #[cfg(test)]
    count(&RESULT_RENDERS, 1);
    let mut violations = Vec::new();
    for (&id, witnesses) in file.witnesses.iter() {
        if file.failures.contains_key(&id) || state.code_rules[id.0].clauses[id.1].synthetic {
            continue;
        }
        let code_rule = &state.code_rules[id.0];
        for witness in witnesses {
            let violation =
                violation_from_witness(code_rule, &code_rule.clauses[id.1], witness.clone());
            violations.push(RenderedViolation::of(&violation)?);
        }
    }
    violations.sort_by(|a, b| a.order().cmp(&b.order()));
    Ok(violations)
}

/// A file's rendered violations: kept with its witnesses, or rendered for this result alone where
/// those were rendered under another key.
enum Rendering<'a> {
    Kept(&'a [RenderedViolation]),
    Fresh(Vec<RenderedViolation>),
}

impl<'a> Rendering<'a> {
    fn of(state: &WorkspaceState, file: &'a FileState, code_rules: u64) -> Result<Self> {
        let key = rendering_key(code_rules, file);
        let rendered = &file.witnesses.rendered;
        if rendered.get().is_none() {
            let violations = render(state, file)?;
            let _ = rendered.set(Rendered { key, violations });
        }
        Ok(match rendered.get() {
            Some(kept) if kept.key == key => Rendering::Kept(&kept.violations),
            _ => Rendering::Fresh(render(state, file)?),
        })
    }

    fn violations(&self) -> &[RenderedViolation] {
        match self {
            Rendering::Kept(violations) => violations,
            Rendering::Fresh(violations) => violations,
        }
    }
}

/// Adds to `replaced` the paths whose violations or failures from a global clause differ between its
/// `previous` and `current` evaluation. False where a pathless one differs: no delta carries it.
pub(super) fn replace_global_changes(
    previous: Option<&Evaluation>,
    current: &Evaluation,
    replaced: &mut BTreeSet<String>,
) -> Result<bool> {
    /// Each path's violations and failures, serialized and sorted.
    fn by_path(evaluation: Option<&Evaluation>) -> Result<BTreeMap<Option<&str>, Vec<String>>> {
        let mut items = BTreeMap::<_, Vec<_>>::new();
        if let Some(evaluation) = evaluation {
            let violations = evaluation.violations.iter().map(|item| ('f', item));
            for (kind, item) in violations.chain(evaluation.failures.iter().map(|item| ('x', item)))
            {
                let json = serde_json::to_string(item).map_err(json_error)?;
                items
                    .entry(item["path"].as_str())
                    .or_default()
                    .push(format!("{kind}{json}"));
            }
        }
        for list in items.values_mut() {
            list.sort_unstable();
        }
        Ok(items)
    }
    let (previous, current) = (by_path(previous)?, by_path(Some(current))?);
    let mut bounded = true;
    for path in previous.keys().chain(current.keys()) {
        if previous.get(path) == current.get(path) {
            continue;
        }
        match path {
            Some(path) => {
                replaced.insert((*path).to_owned());
            }
            None => bounded = false,
        }
    }
    Ok(bounded)
}

/// The workspace's result: complete, or a delta of `replaced_paths`. A file's violations are
/// serialized once per set of witnesses (`Witnesses`), so a result after an edit re-serializes
/// only what the edit changed. A delta carries every violation and failure at a replaced path, its
/// global clauses' too, so the result is `complete` only where a change has no path: a global
/// clause's pathless failure.
pub(super) fn serialize_result(
    state: &WorkspaceState,
    metrics: Metrics,
    deferred: bool,
    replaced_paths: &BTreeSet<String>,
    complete: bool,
) -> Result<ResultText> {
    #[cfg(feature = "code-rule-profile")]
    let _serialization_timer = profile::PhaseTimer::start(profile::Phase::Serialization);
    let mut evaluation = Evaluation {
        metrics,
        clauses: reported_clauses(state),
        ..Evaluation::default()
    };
    // Crossing a rule-wide ceiling changes reports in unchanged and evicted files too.
    // A complete result replaces the host's entire previous result at either transition.
    let delta = state.storage.is_some()
        && !replaced_paths.is_empty()
        && !complete
        && state
            .code_rules
            .iter()
            .all(|rule| rule.report_count_gt.is_none());
    let code_rules = {
        let mut hasher = DefaultHasher::new();
        for code_rule in &state.code_rules {
            code_rule.digest.hash(&mut hasher);
        }
        hasher.finish()
    };
    // A complete result covers evicted files too, from their stored results; a delta covers the
    // replaced ones, including consumers whose relation changed while they remained evicted.
    let evicted = evicted_file_states(state, delta.then_some(replaced_paths), true)?;
    let mut renderings = Vec::new();
    // Each file's rendering is in result order: where every violation of a file is at its path and
    // files come in path order (resident ones), so is their concatenation.
    let mut ordered = true;
    let mut previous: Option<&str> = None;
    let mut blind_files = Vec::new();
    for file in state.files.values().chain(&evicted) {
        if delta && !replaced_paths.contains(&file.input.path) {
            continue;
        }
        blind_files.extend(blind_file(state, file));
        let rendering = Rendering::of(state, file, code_rules)?;
        if let (Some(first), Some(last)) = (
            rendering.violations().first(),
            rendering.violations().last(),
        ) {
            let path = file.input.path.as_str();
            ordered &= first.path == path
                && last.path == path
                && previous.is_none_or(|previous| previous < path);
            previous = Some(path);
        }
        renderings.push(rendering);
        for (&id, &(code, message)) in &file.failures {
            if state.code_rules[id.0].clauses[id.1].synthetic {
                continue;
            }
            let mut item = failure(
                &state.code_rules[id.0].clauses[id.1].id,
                code,
                failure_message(file, message),
                Some(&file.input.path),
            );
            item.as_object_mut()
                .expect("failure object")
                .insert("ruleId".to_owned(), json!(state.code_rules[id.0].id));
            evaluation.failures.push(item);
        }
        if let Some(message) = &file.syntax {
            for id in reporting_ast_candidates(file, &state.code_rules) {
                if file.failures.contains_key(&id) || state.code_rules[id.0].clauses[id.1].synthetic
                {
                    continue;
                }
                let mut item = failure(
                    &state.code_rules[id.0].clauses[id.1].id,
                    "source-partial",
                    message,
                    Some(&file.input.path),
                );
                item.as_object_mut()
                    .expect("failure object")
                    .insert("ruleId".to_owned(), json!(state.code_rules[id.0].id));
                evaluation.failures.push(item);
            }
        }
    }
    // A delta carries a global clause's violations and failures at the replaced paths; the pathless
    // ones did not change (else the result is complete), and the host keeps them.
    let reported = |item: &Value| {
        !delta
            || item["path"]
                .as_str()
                .is_some_and(|path| replaced_paths.contains(path))
    };
    let mut global_violations = Vec::new();
    for &index in state.globals.keys() {
        if deferred && state.dirty_globals.contains(&index) {
            continue;
        }
        if let Some(global) = state.global_results.get(&index) {
            for violation in global.violations.iter().filter(|item| reported(item)) {
                global_violations.push(RenderedViolation::of(violation)?);
            }
            for failure in global.failures.iter().filter(|item| reported(item)) {
                let mut failure = failure.clone();
                failure
                    .as_object_mut()
                    .expect("failure object")
                    .insert("ruleId".to_owned(), json!(state.code_rules[index].id));
                evaluation.failures.push(failure);
            }
        }
    }
    let mut violations = renderings
        .iter()
        .flat_map(Rendering::violations)
        .chain(&global_violations)
        .collect::<Vec<_>>();
    if state
        .code_rules
        .iter()
        .any(|rule| rule.report_count_gt.is_some())
    {
        let mut counts = std::collections::HashMap::<&str, usize>::new();
        for violation in &violations {
            *counts.entry(violation.rule_id.as_str()).or_default() += 1;
        }
        let blocked = state
            .code_rules
            .iter()
            .filter(|rule| {
                rule.report_count_gt.is_some_and(|maximum| {
                    counts.get(rule.id.as_str()).copied().unwrap_or(0) <= maximum
                })
            })
            .map(|rule| rule.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        violations.retain(|violation| !blocked.contains(violation.rule_id.as_str()));
    }
    if !ordered || !global_violations.is_empty() {
        #[cfg(test)]
        count(&RESULT_SORTS, 1);
        violations.sort_by(|a, b| a.order().cmp(&b.order()));
    }
    evaluation
        .failures
        .sort_by(|a, b| result_order(a).cmp(&result_order(b)));
    blind_files
        .sort_by(|left: &Value, right: &Value| left["path"].as_str().cmp(&right["path"].as_str()));
    // The violations are spliced into the serialized evaluation, whose own list is empty.
    let text = serde_json::to_string(&json!({
        "revision": state.revision,
        "replacedPaths": delta.then(|| replaced_paths.iter().collect::<Vec<_>>()),
        "blindFiles": blind_files,
        "evaluation": evaluation_json(evaluation, None, &[], &[])
    }))
    .map_err(json_error)?;
    const EMPTY: &str = "\"violations\":[]";
    let at = text
        .find(EMPTY)
        .ok_or_else(|| invalid("serialized evaluation lacks its violations"))?;
    let mut output = String::with_capacity(
        text.len()
            + violations
                .iter()
                .map(|violation| violation.json.len() + 1)
                .sum::<usize>(),
    );
    output.push_str(&text[..at]);
    output.push_str("\"violations\":[");
    for (index, violation) in violations.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&violation.json);
    }
    output.push(']');
    output.push_str(&text[at + EMPTY.len()..]);
    let ascii = text.is_ascii() && violations.iter().all(|violation| violation.ascii);
    Ok(ResultText {
        text: output,
        ascii,
    })
}

#[cfg(test)]
mod tests {
    use super::super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
    use super::super::storage::StorageContext;
    use super::super::tests::{WORKSPACE_TEST_SLOT, result_page};
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION;
    use crate::code_rule_dsl::test_support::{CAPABILITIES, violation};
    use crate::content_store::{ContentStore, GitTree, HashKey};
    use napi::Task;
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// A workspace over a fresh registration of `root` in `store`, holding `code_rule`.
    fn stored_workspace(
        store: &Arc<ContentStore>,
        root: &str,
        code_rule: &str,
    ) -> CodeRuleDslWorkspace {
        let root_tree = HashKey::git_sha1([7; 20]);
        store
            .put_git_trees_bulk(&[GitTree {
                id: root_tree,
                prefix: String::new(),
                entries: Vec::new(),
            }])
            .unwrap();
        let snapshot = store
            .put_snapshot(
                root_tree,
                1,
                HashKey::sha256(b"rerail-grammars-v1"),
                i64::from(TAPE_FORMAT_VERSION),
            )
            .unwrap();
        let owner = format!("coverage-test:{root}:{}", std::process::id());
        let workspace_id = store
            .open_workspace(root, "repository", &owner, snapshot, 1, i64::MAX)
            .unwrap();
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.state.lock().unwrap().storage = Some(StorageContext {
            store: Arc::clone(store),
            repository: None,
            repository_prefix: String::new(),
            workspace_id,
            snapshot_id: snapshot,
            owner,
            _maintenance: None,
        });
        let compiled =
            CodeRuleDslRuntime::compile(code_rule.to_owned(), "json".into(), CAPABILITIES.into())
                .unwrap();
        workspace.add_code_rule(&compiled, None).unwrap();
        workspace
    }

    fn apply(workspace: &CodeRuleDslWorkspace, files: &Value) -> Value {
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
            request_buffer: None,
            request_direct: None,
        };
        serde_json::from_str(&task.compute().unwrap()).unwrap()
    }

    #[test]
    fn coverage_reports_a_broken_files_partial_clause_cold_warm_and_paged() {
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let directory = std::env::temp_dir().join(format!(
            "rerail-coverage-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(ContentStore::open(&directory.join("content.sqlite")).unwrap());
        let ast = |id: &str, language: &str, rule: Value| json!({ "id": id, "forbid": { "ast": { "language": language, "rule": rule } }, "violation": violation() });
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "covered", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                ast("print", "python", json!({ "pattern": "print($A)" })),
                ast("clean", "typescript", json!({ "kind": "debugger_statement" })),
            ],
        })
        .to_string();
        let files =
            json!([{ "path": "broken.py", "language": "python", "source": "print(3)\ndef (:\n" }]);
        let expected = json!([
            { "ruleId": "covered", "clauseId": "print", "status": "partial", "reasons": [
                { "code": "source-partial", "count": 1, "paths": ["broken.py"] },
            ] },
            { "ruleId": "covered", "clauseId": "clean", "status": "complete", "reasons": [] },
        ]);

        let cold = stored_workspace(&store, "/coverage", &code_rule);
        let result = apply(&cold, &files);
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        cold.close();

        let warm = stored_workspace(&store, "/coverage", &code_rule);
        let result = apply(&warm, &files);
        let restored = warm
            .state
            .lock()
            .unwrap()
            .files
            .values()
            .map(|file| file.restored_local_results)
            .sum::<usize>();
        assert!(restored > 0, "the file's results are restored");
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        let page: Value =
            serde_json::from_str(&result_page(&warm, 0, 64 * 1024 * 1024).unwrap()).unwrap();
        assert_eq!(page["coverage"], expected, "{page}");
        warm.close();
        drop(store);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A file no clause read is reported once, by path and reason, cold, restored and paged: one
    /// whose syntax errors failed every clause (`syntax`), one whose every clause ran out of work
    /// (`budget`). A file any clause read is not.
    #[test]
    fn blind_files_are_reported_by_reason_cold_warm_and_paged() {
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let directory = std::env::temp_dir().join(format!(
            "rerail-blind-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(ContentStore::open(&directory.join("content.sqlite")).unwrap());
        // Unsound under syntax errors: a broken file fails it `source-incomplete`.
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "blind", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{ "id": "unguarded", "violation": violation(), "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "debugger_statement",
                    "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } } } } }],
        })
        .to_string();
        let files = json!([
            { "path": "broken.ts", "language": "typescript", "source": "function f( {\n  debugger;\n" },
            { "path": "ok.ts", "language": "typescript", "source": "debugger;\n" },
        ]);
        // Its error regions cover most of it: the parser did not read it (`UNREAD`).
        let syntax = json!({ "clauses": 1, "message": "file did not parse",
            "path": "broken.ts", "reason": "syntax", "folded": [["blind", "unguarded"]] });

        let cold = stored_workspace(&store, "/blind", &code_rule);
        let result = apply(&cold, &files);
        assert_eq!(result["blindFiles"], json!([syntax]), "{result}");
        assert_eq!(
            result["evaluation"]["violations"].as_array().unwrap().len(),
            1
        );
        cold.close();

        let warm = stored_workspace(&store, "/blind", &code_rule);
        let result = apply(&warm, &files);
        assert_eq!(result["blindFiles"], json!([syntax]), "{result}");
        let page: Value =
            serde_json::from_str(&result_page(&warm, 0, 64 * 1024 * 1024).unwrap()).unwrap();
        assert_eq!(page["blindFiles"], json!([syntax]), "{page}");
        warm.close();

        // New content, so no stored result answers it.
        let starved = stored_workspace(&store, "/blind-budget", &code_rule);
        let files = json!([files[0], { "path": "ok.ts", "language": "typescript",
            "source": "debugger;\ndebugger;\n" }]);
        super::super::TEST_CLAUSE_WORK_BUDGET.set(Some(0));
        let result = apply(&starved, &files);
        super::super::TEST_CLAUSE_WORK_BUDGET.set(None);
        assert_eq!(
            result["blindFiles"],
            json!([syntax, { "clauses": 1, "message": "clause work budget exceeded",
                "path": "ok.ts", "reason": "budget", "folded": [["blind", "unguarded"]] }]),
            "{result}"
        );
        starved.close();
        drop(store);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A complete result after an edit renders the edited file's violations alone and sorts none:
    /// unchanged files' renderings are kept, in path order, and the result is a fresh
    /// workspace's.
    #[test]
    fn an_edit_renders_only_its_file_and_keeps_result_order() {
        use super::{RESULT_RENDERS, RESULT_SORTS};
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "rendered", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                { "id": "print", "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }, "violation": violation() },
                { "id": "call", "forbid": { "ast": { "language": "python", "rule": { "kind": "call" } } }, "violation": violation() },
            ],
        })
        .to_string();
        let compiled =
            CodeRuleDslRuntime::compile(code_rule, "json".into(), CAPABILITIES.into()).unwrap();
        let file = |path: &str, source: &str| json!({ "path": path, "language": "python", "source": source });
        let files = [
            file("b.py", "print(1)\nf()\n"),
            file("a.py", "print(2)\n"),
            file("c/d.py", "g(print(3))\n"),
        ];
        let run = |workspace: &CodeRuleDslWorkspace, revision: u32, files: &[Value]| -> Value {
            let mut task = WorkspaceApplyTask {
                state: Arc::clone(&workspace.state),
                closed: Arc::clone(&workspace.closed),
                request_json: json!({ "revision": revision, "files": files, "removed": [] })
                    .to_string(),
                request_buffer: None,
                request_direct: None,
            };
            serde_json::from_str(&task.compute().unwrap()).unwrap()
        };
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(&compiled, None).unwrap();
        run(&workspace, 1, &files);
        let edited = file("b.py", "x = 1\nprint(1)\nf()\n");
        RESULT_RENDERS.set(0);
        RESULT_SORTS.set(0);
        let result = run(&workspace, 2, std::slice::from_ref(&edited));
        assert_eq!((RESULT_RENDERS.get(), RESULT_SORTS.get()), (1, 0));
        let mut fresh = CodeRuleDslWorkspace::new();
        fresh.add_code_rule(&compiled, None).unwrap();
        let expected = run(&fresh, 1, &[files[1].clone(), edited, files[2].clone()]);
        assert_eq!(
            result["evaluation"]["violations"],
            expected["evaluation"]["violations"]
        );
        assert_eq!(
            result["evaluation"]["violations"].as_array().unwrap().len(),
            8
        );
        workspace.close();
        fresh.close();
    }
}
