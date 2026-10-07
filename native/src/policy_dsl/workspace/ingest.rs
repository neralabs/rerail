//! Request ingestion: JSON and binary requests, file state and candidate dispatch, Python package
//! context, and applying a request to the workspace state.

use super::api::check_revision;
use super::crossfile::{CrossFileFileState, CrossFileScheduler};
use super::documents;
use super::evaluate::candidate_grammar;
use super::{CandidateSets, Candidates, CheckId, FileState, WorkspaceState};
use crate::policy_dsl::eval::{file_language, grammar_matcher};
use crate::policy_dsl::json::{invalid, json_error};
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::model::{
    CheckKind, CompiledCheck, CompiledPolicy, FileInput, Query, package_marker_directory,
    python_module_in,
};
use napi::Result;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::Ordering;

#[cfg(test)]
pub(super) static PACKAGE_CONTEXT_PATH_VISITS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

const MAX_BINARY_REQUEST_BYTES: usize = 40 * 1024 * 1024;
pub(super) const BINARY_REQUEST_MAGIC: &[u8; 8] = b"TEMPORB1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Request {
    pub(super) revision: u32,
    pub(super) files: Vec<WireFile>,
    pub(super) removed: Vec<String>,
    #[serde(default)]
    pub(super) defer_global: bool,
    #[serde(default)]
    pub(super) quiet: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BufferRequest {
    revision: u32,
    files: Vec<BufferFile>,
    removed: Vec<String>,
    #[serde(default)]
    defer_global: bool,
    #[serde(default)]
    quiet: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BufferFile {
    path: String,
    language: String,
    source_bytes: usize,
    /// The same Python module a JSON request's file carries (see `FileInput::new`).
    #[serde(default)]
    module: Option<Value>,
}

pub(super) fn decode_buffer_request(buffer: &[u8]) -> Result<Request> {
    if buffer.len() > MAX_BINARY_REQUEST_BYTES {
        return Err(invalid("workspace buffer exceeds 40 MiB"));
    }
    if buffer.len() < 12 || &buffer[..8] != BINARY_REQUEST_MAGIC {
        return Err(invalid("workspace buffer header is invalid"));
    }
    let metadata_length = u32::from_le_bytes(
        buffer[8..12]
            .try_into()
            .expect("workspace metadata length has four bytes"),
    ) as usize;
    let metadata_end = 12_usize
        .checked_add(metadata_length)
        .filter(|end| *end <= buffer.len())
        .ok_or_else(|| invalid("workspace buffer metadata length is invalid"))?;
    let metadata: BufferRequest =
        serde_json::from_slice(&buffer[12..metadata_end]).map_err(json_error)?;
    let mut offset = metadata_end;
    let mut files = Vec::with_capacity(metadata.files.len());
    for file in metadata.files {
        let end = offset
            .checked_add(file.source_bytes)
            .filter(|end| *end <= buffer.len())
            .ok_or_else(|| invalid("workspace buffer source length is invalid"))?;
        let source = std::str::from_utf8(&buffer[offset..end])
            .map_err(|_| invalid("workspace buffer source is not UTF-8"))?
            .to_owned();
        files.push(WireFile {
            path: file.path,
            language: file.language,
            source,
            module: file.module,
        });
        offset = end;
    }
    if offset != buffer.len() {
        return Err(invalid("workspace buffer has trailing bytes"));
    }
    Ok(Request {
        revision: metadata.revision,
        files,
        removed: metadata.removed,
        defer_global: metadata.defer_global,
        quiet: metadata.quiet,
    })
}

#[derive(Deserialize)]
pub(super) struct WireFile {
    pub(super) path: String,
    pub(super) language: String,
    pub(super) source: String,
    /// A Python file's module path, dotted or as a list of names (see `FileInput::new`).
    #[serde(default)]
    pub(super) module: Option<Value>,
}
impl WireFile {
    pub(super) fn into_input(self, index: usize) -> Result<FileInput> {
        FileInput::new(
            self.path,
            self.language,
            Arc::new(self.source),
            self.module.as_ref(),
            &format!("files/{index}"),
        )
    }
}

pub(super) fn is_file_local(check: &CompiledCheck) -> bool {
    matches!(
        check.kind,
        CheckKind::Forbid(Query::Regex { .. } | Query::Ast { .. })
    )
}

pub(super) fn make_file(
    input: FileInput,
    source_revision: u32,
    policies: &[Arc<CompiledPolicy>],
    candidate_sets: &mut CandidateSets,
) -> FileState {
    let language = file_language(&input);
    let mut regex = Vec::new();
    let mut ast = Vec::new();
    for (policy_index, policy) in policies.iter().enumerate() {
        if !policy.scope.matches(&input.path) {
            continue;
        }
        for (check_index, check) in policy.checks.iter().enumerate() {
            let id = (policy_index, check_index);
            match &check.kind {
                CheckKind::Forbid(Query::Regex { .. }) => regex.push(id),
                CheckKind::Forbid(Query::Ast { matchers, .. })
                    if grammar_matcher(matchers, &input).is_some() =>
                {
                    ast.push(id);
                }
                _ => {}
            }
        }
    }
    // Policies cannot change after evaluation starts. Files with the same
    // eligible checks and grammar can share immutable dispatch metadata, but never nodes.
    let candidates = Arc::clone(
        candidate_sets
            .entry((language, regex, ast))
            .or_insert_with_key(|(language, regex, ast)| {
                let mut candidates = Candidates {
                    regex: regex.clone(),
                    ast: ast.clone(),
                    ..Candidates::default()
                };
                // The dialect's whole catalog: a Blade rule's PHP root kinds follow Blade's own.
                let kind_count = language.map_or(0, GrammarDialect::kind_count);
                for (slot, id) in candidates.ast.iter().enumerate() {
                    let anchors = &candidate_grammar(&policies[id.0], id.1, &input).anchors;
                    if let Some(kinds) = anchors.root_kinds() {
                        for &kind in kinds {
                            let kind = usize::from(kind);
                            // An ERROR root always overlaps a syntax-error region, so its
                            // witnesses would be suppressed anyway.
                            if kind >= kind_count {
                                continue;
                            }
                            if candidates.by_kind.len() <= kind {
                                candidates.by_kind.resize_with(kind + 1, Vec::new);
                            }
                            candidates.by_kind[kind].push(slot);
                        }
                    } else {
                        candidates.unrestricted.push(slot);
                    }
                }
                Arc::new(candidates)
            }),
    );
    let source_digest = Sha256::digest(input.source.as_bytes()).into();
    FileState {
        input,
        source_digest,
        source_revision,
        tape: None,
        scopes: None,
        stored: None,
        selector_skipped: false,
        tape_cache_hit: false,
        tape_decompressed: false,
        restored_local_results: 0,
        restored_result_ids: HashSet::new(),
        transient_failure: false,
        candidates,
        witnesses: Default::default(),
        failures: BTreeMap::new(),
        syntax: None,
        crossfile: CrossFileFileState::default(),
    }
}

pub(super) fn candidate_dispatch(
    language: Option<GrammarDialect>,
    selected: Vec<CheckId>,
    policies: &[Arc<CompiledPolicy>],
    input: &FileInput,
) -> Candidates {
    let mut candidates = Candidates::default();
    for id in selected {
        match &policies[id.0].checks[id.1].kind {
            CheckKind::Forbid(Query::Regex { .. }) => candidates.regex.push(id),
            CheckKind::Forbid(Query::Ast { .. }) => candidates.ast.push(id),
            _ => {}
        }
    }
    let kind_count = language.map_or(0, GrammarDialect::kind_count);
    for (slot, id) in candidates.ast.iter().enumerate() {
        let anchors = &candidate_grammar(&policies[id.0], id.1, input).anchors;
        if let Some(kinds) = anchors.root_kinds() {
            for &kind in kinds {
                let kind = usize::from(kind);
                if kind >= kind_count {
                    continue;
                }
                if candidates.by_kind.len() <= kind {
                    candidates.by_kind.resize_with(kind + 1, Vec::new);
                }
                candidates.by_kind[kind].push(slot);
            }
        } else {
            candidates.unrestricted.push(slot);
        }
    }
    candidates
}

pub(super) fn file_needs_update(state: &WorkspaceState, input: &FileInput) -> bool {
    if let Some(current) = state.files.get(&input.path) {
        let source_digest: [u8; 32] = Sha256::digest(input.source.as_bytes()).into();
        current.transient_failure
            || current.source_digest != source_digest
            || current.input.language != input.language
            || current.input.module != input.module
    } else {
        state
            .policies
            .iter()
            .any(|policy| policy.scope.matches(&input.path))
    }
}

pub(super) fn reclaim_candidate_sets(candidate_sets: &mut CandidateSets) {
    candidate_sets.retain(|_, candidates| Arc::strong_count(candidates) > 1);
    if candidate_sets.is_empty() {
        // clear/retain alone keep the buckets from a formerly large workspace.
        *candidate_sets = CandidateSets::new();
    } else if candidate_sets.capacity() > candidate_sets.len().saturating_mul(4).max(16) {
        // Shrink only after substantial contraction, rather than reallocating on each removal.
        candidate_sets.shrink_to_fit();
    }
}

fn path_is_below(path: &str, directory: &str) -> bool {
    directory.is_empty()
        || path
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// The module of `input`'s path as `FileInput` normalizes it, from the workspace's markers where
/// `is_package` says a directory holds one.
fn module_under(input: &FileInput, is_package: impl Fn(&str) -> bool) -> Result<FileInput> {
    FileInput::new(
        input.path.clone(),
        input.language.clone(),
        Arc::new(String::new()),
        python_module_in(&input.path, is_package).as_ref(),
        &input.path,
    )
}

impl WorkspaceState {
    /// Makes `file` the resident state of `path`. Every insertion of a workspace path goes through
    /// here (a request, a bulk restore, the inline path, an evaluation's put-back), so
    /// `package_directories` covers every package marker the workspace holds.
    pub(super) fn insert_file(&mut self, path: String, file: FileState) -> Option<FileState> {
        if let Some(directory) = package_marker_directory(&path)
            && !self.package_directories.contains(directory)
        {
            self.package_directories.insert(directory.to_owned());
        }
        self.files.insert(path, file)
    }

    /// Removes `path` from the workspace, resident or evicted, with the package it marks; returns
    /// its resident state.
    fn remove_path(&mut self, path: &str) -> Option<FileState> {
        if let Some(directory) = package_marker_directory(path) {
            self.package_directories.remove(directory);
        }
        self.evicted_inputs.remove(path);
        self.files.remove(path)
    }
}

/// Expand a package-marker edit to the Python files whose module context it changes. The workspace
/// is the logical filesystem for JSON/buffer overlays: resident and evicted paths contribute
/// package markers alike (`package_directories`), then the request atomically replaces its paths.
fn refresh_package_context(
    state: &mut WorkspaceState,
    inputs: &mut Vec<FileInput>,
    unhosted: &HashSet<String>,
    removed: &[String],
) -> Result<BTreeSet<String>> {
    // A marker is created or deleted only when its path is new to, or leaves, the workspace; an
    // edit of an existing `__init__.py` changes no module.
    let known =
        |path: &str| state.files.contains_key(path) || state.evicted_inputs.contains_key(path);
    let marker_directories = inputs
        .iter()
        .filter(|input| !known(&input.path))
        .filter_map(|input| package_marker_directory(&input.path))
        .chain(
            removed
                .iter()
                .filter(|path| known(path))
                .filter_map(|path| package_marker_directory(path)),
        )
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if marker_directories.is_empty() && unhosted.is_empty() {
        return Ok(BTreeSet::new());
    }
    let requested_paths = inputs
        .iter()
        .map(|input| input.path.clone())
        .collect::<HashSet<_>>();
    // The markers after this request, as a change to `package_directories` (which `insert_file`
    // and `remove_path` make as the request applies): its removals, then its files.
    let dropped = removed
        .iter()
        .filter_map(|path| package_marker_directory(path))
        .collect::<HashSet<_>>();
    let added = inputs
        .iter()
        .filter_map(|input| package_marker_directory(&input.path))
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let after = |directories: &BTreeSet<String>, directory: &str| {
        added.contains(directory)
            || (!dropped.contains(directory) && directories.contains(directory))
    };
    // A module the host supplied (Ruff's package walk on its disk) is authoritative; only a module
    // the workspace's own markers derived follows them. Derived means: equal to the derivation
    // from the markers before this request.
    let derived = |directories: &BTreeSet<String>, input: &FileInput| -> Result<bool> {
        Ok(
            module_under(input, |directory| directories.contains(directory))?.module
                == input.module,
        )
    };
    let removed_paths = removed.iter().collect::<HashSet<_>>();
    let mut paths = BTreeSet::new();
    for directory in &marker_directories {
        let start = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        for (path, _) in state.files.range(start.clone()..) {
            #[cfg(test)]
            PACKAGE_CONTEXT_PATH_VISITS.fetch_add(1, Ordering::Relaxed);
            if !path_is_below(path, directory) {
                break;
            }
            if !requested_paths.contains(path) && !removed_paths.contains(path) {
                paths.insert(path.clone());
            }
        }
        for (path, _) in state.evicted_inputs.range(start..) {
            #[cfg(test)]
            PACKAGE_CONTEXT_PATH_VISITS.fetch_add(1, Ordering::Relaxed);
            if !path_is_below(path, directory) {
                break;
            }
            if !requested_paths.contains(path) && !removed_paths.contains(path) {
                paths.insert(path.clone());
            }
        }
    }
    let mut stale = Vec::new();
    for path in paths {
        let input = match state.files.get(&path) {
            Some(file) => &file.input,
            None => &state.evicted_inputs[&path],
        };
        if input.language != "python" || !derived(&state.package_directories, input)? {
            continue;
        }
        // The module depends on the path alone: compare before reading an evicted file's source.
        let refreshed = module_under(input, |directory| {
            after(&state.package_directories, directory)
        })?;
        if refreshed.module != input.module {
            stale.push((path, input.clone(), refreshed.module));
        }
    }
    let mut refreshed = BTreeSet::new();
    for (path, mut input, module) in stale {
        if input.source.is_empty() {
            input.source = Arc::clone(&documents::load(state, &path)?.1.source);
        }
        input.module = module;
        inputs.push(input);
        refreshed.insert(path);
    }
    // A requested file without a host module is named by the markers, as a derived one follows
    // them.
    for input in inputs.iter_mut() {
        if input.language == "python"
            && (unhosted.contains(&input.path)
                || (marker_directories
                    .iter()
                    .any(|directory| path_is_below(&input.path, directory))
                    && derived(&state.package_directories, input)?))
        {
            input.module = module_under(input, |directory| {
                after(&state.package_directories, directory)
            })?
            .module;
        }
    }
    Ok(refreshed)
}

pub(super) fn apply_request(
    state: &mut WorkspaceState,
    request: Request,
) -> Result<BTreeSet<String>> {
    check_revision(state, request.revision)?;
    let unhosted = request
        .files
        .iter()
        .filter(|wire| wire.language == "python" && wire.module.is_none())
        .map(|wire| wire.path.clone())
        .collect::<HashSet<_>>();
    let mut inputs = request
        .files
        .into_iter()
        .enumerate()
        .map(|(index, wire)| wire.into_input(index))
        .collect::<Result<Vec<_>>>()?;
    let refreshed = refresh_package_context(state, &mut inputs, &unhosted, &request.removed)?;
    let mut released_candidates = false;
    if let Some(storage) = &state.storage {
        storage
            .store
            .remove_overlay_files(storage.workspace_id, &request.removed)
            .map_err(|error| invalid(error.to_string()))?;
    }
    for path in request.removed {
        state.versions.remove(&path);
        // This reaches both resident and evicted producer state.
        CrossFileScheduler::forget(state, &path);
        if state.remove_path(&path).is_some() {
            released_candidates = true;
        }
        state.dirty_files.remove(&path);
    }
    for input in inputs {
        if !file_needs_update(state, &input) {
            continue;
        }
        let path = input.path.clone();
        state.evicted_inputs.remove(&path);
        state.versions.remove(&path);
        let file = make_file(
            input,
            request.revision,
            &state.policies,
            &mut state.candidate_sets,
        );
        let candidates = Arc::clone(&file.candidates);
        if let Some(previous) = state.insert_file(path.clone(), file) {
            // Replacement (e.g. a grammar change) may orphan the previous dispatch metadata.
            released_candidates |= !Arc::ptr_eq(&previous.candidates, &candidates);
        }
        state.dirty_files.insert(path.clone());
        CrossFileScheduler::forget(state, &path);
    }
    state.result_page_index = None;
    state.revision = request.revision;
    if released_candidates {
        reclaim_candidate_sets(&mut state.candidate_sets);
    }
    Ok(refreshed)
}

/// Whether a file is a package marker new to the workspace, which changes its siblings' modules
/// (`refresh_package_context`).
fn adds_package_marker(state: &WorkspaceState, file: &WireFile) -> bool {
    package_marker_directory(&file.path).is_some()
        && !state.files.contains_key(&file.path)
        && !state.evicted_inputs.contains_key(&file.path)
}

/// The inline path's one file (`is_cheap`) as `apply_request` reads it: a Python file without a
/// host module is named by the workspace's markers. The file is no new marker, so no sibling's
/// module changes.
pub(super) fn inline_input(state: &mut WorkspaceState, file: WireFile) -> Result<FileInput> {
    let unhosted = (file.language == "python" && file.module.is_none())
        .then(|| file.path.clone())
        .into_iter()
        .collect::<HashSet<_>>();
    let mut inputs = vec![file.into_input(0)?];
    refresh_package_context(state, &mut inputs, &unhosted, &[])?;
    Ok(inputs.swap_remove(0))
}

pub(super) fn is_cheap(state: &WorkspaceState, request: &Request) -> bool {
    state.storage.is_none() && !request.defer_global && !request.quiet && request.removed.is_empty()
        && request.files.len() == 1 && state.dirty_files.is_empty() && state.globals.is_empty()
        && state.files.len() <= 4 && request.files[0].source.len() <= 1024
        && !adds_package_marker(state, &request.files[0])
        && state.files.values().map(|file| file.input.source.len()).sum::<usize>() <= 4096
        && state.policies.iter().map(|policy| policy.checks.len()).sum::<usize>() <= 4
        && state.policies.iter().all(|policy| policy.checks.iter().all(|check|
            matches!(&check.kind, CheckKind::Forbid(Query::Regex { regex, .. }) if regex.as_str().len() <= 128)))
}
