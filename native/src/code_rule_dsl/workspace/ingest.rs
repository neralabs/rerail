//! Request ingestion: JSON and binary requests, file state and candidate dispatch, Python package
//! context, and applying a request to the workspace state.

use super::api::check_revision;
use super::crossfile::{CrossFileFileState, CrossFileScheduler};
use super::documents;
use super::evaluate::candidate_grammar;
use super::{CandidateSets, Candidates, ClauseId, FileState, WorkspaceState};
use crate::code_rule_dsl::eval::{file_language, grammar_matcher};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::{package_manifests, packages};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, FileInput, Query, package_marker_directory,
    python_module_in,
};
use crate::code_rule_dsl::module_graph;
use crate::code_rule_dsl::package_manifest::{
    Manifest, deserialize_wire, manifest_directory, manifest_in,
};
use crate::content_store::StoreError;
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
pub(super) const BINARY_REQUEST_MAGIC: &[u8; 8] = b"RERAILB1";

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
    /// The same package a JSON request's file carries (see `FileInput::new`).
    #[serde(default, deserialize_with = "deserialize_wire")]
    package: Option<Arc<Manifest>>,
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
            package: file.package,
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
    /// An ECMA file's nearest `package.json` (see `FileInput::new`); without it the workspace's
    /// `package.json` entries decide (`refresh_manifest_context`).
    #[serde(default, deserialize_with = "deserialize_wire")]
    pub(super) package: Option<Arc<Manifest>>,
}
impl WireFile {
    pub(super) fn into_input(self, index: usize) -> Result<FileInput> {
        FileInput::new(
            self.path,
            self.language,
            Arc::new(self.source),
            self.module.as_ref(),
            self.package,
            &format!("files/{index}"),
        )
    }
}

pub(super) fn is_file_local(clause: &CompiledClause) -> bool {
    matches!(
        clause.kind,
        ClauseKind::Forbid(Query::Regex { .. } | Query::Ast { .. })
    )
}

pub(super) fn make_file(
    input: FileInput,
    source_revision: u32,
    code_rules: &[Arc<CompiledCodeRule>],
    candidate_sets: &mut CandidateSets,
) -> FileState {
    let language = file_language(&input);
    let mut regex = Vec::new();
    let mut ast = Vec::new();
    for (code_rule_index, code_rule) in code_rules.iter().enumerate() {
        if !code_rule.scope.matches(&input.path) {
            continue;
        }
        for (clause_index, clause) in code_rule.clauses.iter().enumerate() {
            let id = (code_rule_index, clause_index);
            match &clause.kind {
                ClauseKind::Forbid(Query::Regex { .. }) => regex.push(id),
                ClauseKind::Forbid(Query::Ast { matchers, .. })
                    if grammar_matcher(matchers, &input).is_some() =>
                {
                    ast.push(id);
                }
                _ => {}
            }
        }
    }
    // Code rules cannot change after evaluation starts. Files with the same
    // eligible clauses and grammar can share immutable dispatch metadata, but never nodes.
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
                    let anchors = &candidate_grammar(&code_rules[id.0], id.1, &input).anchors;
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
        unpublished: false,
        results_only: false,
        unstored: HashSet::new(),
        candidates,
        witnesses: Default::default(),
        failures: BTreeMap::new(),
        syntax: None,
        crossfile: CrossFileFileState::default(),
    }
}

pub(super) fn candidate_dispatch(
    language: Option<GrammarDialect>,
    selected: Vec<ClauseId>,
    code_rules: &[Arc<CompiledCodeRule>],
    input: &FileInput,
) -> Candidates {
    let mut candidates = Candidates::default();
    for id in selected {
        match &code_rules[id.0].clauses[id.1].kind {
            ClauseKind::Forbid(Query::Regex { .. }) => candidates.regex.push(id),
            ClauseKind::Forbid(Query::Ast { .. }) => candidates.ast.push(id),
            _ => {}
        }
    }
    let kind_count = language.map_or(0, GrammarDialect::kind_count);
    for (slot, id) in candidates.ast.iter().enumerate() {
        let anchors = &candidate_grammar(&code_rules[id.0], id.1, input).anchors;
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
            || current.input.package != input.package
    } else {
        state
            .code_rules
            .iter()
            .any(|code_rule| code_rule.scope.matches(&input.path))
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
        None,
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
        if !packages(&input.language) || !derived(&state.package_directories, input)? {
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
            match documents::load(state, &path) {
                Ok((_, document)) => input.source = Arc::clone(&document.source),
                Err(error) => {
                    // A file freed while the store was busy that nothing can supply again keeps
                    // its module and reports its clauses incomplete: a busy store costs reuse,
                    // never the apply.
                    let Some(file) = state.files.get_mut(&path).filter(|file| file.results_only)
                    else {
                        return Err(error);
                    };
                    let clauses = file
                        .candidates
                        .regex
                        .iter()
                        .chain(&file.candidates.ast)
                        .copied()
                        .collect::<Vec<_>>();
                    file.fail_released_source(clauses);
                    refreshed.insert(path);
                    continue;
                }
            }
        }
        input.module = module;
        inputs.push(input);
        refreshed.insert(path);
    }
    // A requested file without a host module is named by the markers, as a derived one follows
    // them.
    for input in inputs.iter_mut() {
        if packages(&input.language)
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

/// A request's `package.json` changes, by directory: the manifest after it, or `None` where it
/// removes the one there.
type ManifestChanges = BTreeMap<String, Option<Arc<Manifest>>>;

/// Expand a `package.json` edit to the ECMA files whose nearest manifest it changes, and give an
/// ECMA file the host sent no `package` for the workspace's nearest one. As package markers are
/// the packages of JSON/buffer overlays, the workspace's `package.json` entries (whatever their
/// language) are their manifests, read as on disk (`Manifest::parse`) with the `jsxImportSource`
/// of the `tsconfig.json`-style entries beside them (a change of one is a change of the
/// manifest). A resident file follows them while its `package` equals their derivation before
/// the request, so a host's own fact stays authoritative. The changes are the workspace's once
/// the request applies.
fn refresh_manifest_context(
    state: &mut WorkspaceState,
    inputs: &mut Vec<FileInput>,
    unhosted: &HashSet<String>,
    removed: &[String],
) -> Result<(BTreeSet<String>, ManifestChanges)> {
    let mut changes = ManifestChanges::new();
    for path in removed {
        if let Some(directory) = manifest_directory(path)
            && state.manifests.contains_key(directory)
        {
            changes.insert(directory.to_owned(), None);
        }
    }
    let config = |state: &WorkspaceState, directory: &str| {
        module_graph::Config::read(directory, &|path| {
            module_config_text(state, inputs, removed, path)
        })
    };
    for input in inputs.iter() {
        if let Some(directory) = manifest_directory(&input.path) {
            let manifest = Manifest::parse(directory.to_owned(), input.source.as_bytes())
                .with_jsx_import_source(config(state, directory).jsx_import_source());
            if state
                .manifests
                .get(directory)
                .is_some_and(|current| **current == manifest)
            {
                changes.remove(directory);
            } else {
                changes.insert(directory.to_owned(), Some(Arc::new(manifest)));
            }
        }
    }
    if inputs
        .iter()
        .map(|input| input.path.as_str())
        .chain(removed.iter().map(String::as_str))
        .any(module_graph::is_module_config)
    {
        for (directory, current) in &state.manifests {
            if changes.contains_key(directory) {
                continue;
            }
            let config = config(state, directory);
            if current.jsx_import_source() != config.jsx_import_source() {
                let updated = (**current)
                    .clone()
                    .with_jsx_import_source(config.jsx_import_source());
                changes.insert(directory.clone(), Some(Arc::new(updated)));
            }
        }
    }
    if state
        .code_rules
        .iter()
        .any(|code_rule| code_rule.reads_server_graph())
    {
        for path in removed {
            state.graph_entries.remove(path);
        }
        for input in inputs.iter() {
            if unhosted.contains(&input.path) {
                let summary = module_graph::summarize_bounded(&input.path, &input.source);
                state.graph_entries.insert(input.path.clone(), summary);
            } else if !package_manifests(&input.language) {
                state.graph_entries.insert(input.path.clone(), None);
            }
        }
        if !unhosted.is_empty() || !state.server_graph_directories.is_empty() || !changes.is_empty()
        {
            refresh_server_graphs(state, inputs, unhosted, removed, &mut changes)?;
        }
    }
    if changes.is_empty() && unhosted.is_empty() {
        return Ok((BTreeSet::new(), changes));
    }
    let manifests = &state.manifests;
    let before = |directory: &str| manifests.get(directory);
    let after = |directory: &str| match changes.get(directory) {
        Some(change) => change.as_ref(),
        None => manifests.get(directory),
    };
    let requested = inputs
        .iter()
        .map(|input| input.path.clone())
        .collect::<HashSet<_>>();
    let mut paths = BTreeSet::new();
    for directory in changes.keys() {
        let start = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let resident = state.files.range(start.clone()..).map(|(path, _)| path);
        let evicted = state.evicted_inputs.range(start..).map(|(path, _)| path);
        for path in resident
            .take_while(|path| path_is_below(path, directory))
            .chain(evicted.take_while(|path| path_is_below(path, directory)))
        {
            if !requested.contains(path) && !removed.contains(path) {
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
        if !package_manifests(&input.language) || input.package != manifest_in(&path, before) {
            continue;
        }
        let package = manifest_in(&path, after);
        if package != input.package {
            stale.push((path, input.clone(), package));
        }
    }
    for input in inputs.iter_mut() {
        if unhosted.contains(&input.path) {
            input.package = manifest_in(&input.path, after);
        }
    }
    let mut refreshed = BTreeSet::new();
    for (path, mut input, package) in stale {
        if input.source.is_empty() {
            input.source = Arc::clone(&documents::load(state, &path)?.1.source);
        }
        input.package = package;
        inputs.push(input);
        refreshed.insert(path);
    }
    Ok((refreshed, changes))
}

/// Gives each workspace manifest naming `next` the server graph of the files the workspace holds
/// below it, the request applied (`module_graph::in_file_set`), for files it derives `package`
/// for: a directory some file of the request without a host `package` lies below joins
/// `server_graph_directories`, and a joined directory's graph is derived again whenever the
/// request changes what it reads (its manifest, a source below it, a project configuration
/// anywhere, since one may be another's `extends` base), removals included. A manifest whose
/// graph the request changes is a change of the manifest, so the files that read it are
/// evaluated again.
fn refresh_server_graphs(
    state: &mut WorkspaceState,
    inputs: &[FileInput],
    unhosted: &HashSet<String>,
    removed: &[String],
    changes: &mut ManifestChanges,
) -> Result<()> {
    let after = state
        .manifests
        .iter()
        .filter(|(directory, _)| !changes.contains_key(*directory))
        .map(|(directory, manifest)| (directory.clone(), Some(Arc::clone(manifest))))
        .chain(
            changes
                .iter()
                .map(|(directory, change)| (directory.clone(), change.clone())),
        )
        .collect::<BTreeMap<_, _>>();
    let manifest_directories = after
        .iter()
        .filter(|(_, manifest)| manifest.is_some())
        .map(|(directory, _)| directory.clone())
        .collect::<HashSet<_>>();
    let requested = inputs
        .iter()
        .map(|input| input.path.as_str())
        .collect::<HashSet<_>>();
    let touched_paths = requested
        .iter()
        .copied()
        .chain(removed.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let configs_changed = touched_paths
        .iter()
        .any(|path| module_graph::is_module_config(path));
    for (directory, manifest) in after {
        let Some(manifest) = manifest.filter(|manifest| manifest.has_server_graph()) else {
            state.server_graph_directories.remove(&directory);
            continue;
        };
        // The workspace derives the graph for files it gives `package`: ones of this request,
        // or, when the manifest changes, ones it already holds (a manifest that gains `next`
        // after its sources arrived).
        let needed = state.server_graph_directories.contains(&directory)
            || unhosted.iter().any(|path| path_is_below(path, &directory))
            || (changes.contains_key(&directory) && derives_package_below(state, &directory));
        // A nested manifest created or removed moves the boundary of this package's files, and any
        // file below its root `app/` may make that folder present or absent (it hides `src/app`).
        let root_app = if directory.is_empty() {
            "app".to_owned()
        } else {
            format!("{directory}/app")
        };
        let touched = changes.contains_key(&directory)
            || manifest.server_graph_members().is_none()
            || configs_changed
            || changes
                .keys()
                .any(|nested| nested != &directory && path_is_below(nested, &directory))
            || touched_paths.iter().any(|path| {
                path_is_below(path, &directory)
                    && (module_graph::is_source(path) || path_is_below(path, &root_app))
            });
        if !needed || !touched {
            continue;
        }
        state.server_graph_directories.insert(directory.clone());
        let start = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let config = module_graph::Config::read(&directory, &|path| {
            module_config_text(state, inputs, removed, path)
        });
        let members = module_graph::from_summaries(
            &directory,
            state
                .graph_entries
                .range(start..)
                .take_while(|(path, _)| path_is_below(path, &directory))
                .map(|(path, summary)| (path.as_str(), summary.clone())),
            &|folder| manifest_directories.contains(folder),
            &config,
        );
        let updated = Arc::new((*manifest).clone().with_server_graph(members));
        if state
            .manifests
            .get(&directory)
            .is_some_and(|current| **current == *updated)
        {
            changes.remove(&directory);
        } else {
            changes.insert(directory, Some(updated));
        }
    }
    Ok(())
}

/// Whether the workspace holds a file below `directory` whose `package` it derived from its own
/// manifests (no host `package`), rather than one a host sent.
fn derives_package_below(state: &WorkspaceState, directory: &str) -> bool {
    let start = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    let derived = |path: &String, input: &FileInput| {
        package_manifests(&input.language)
            && input.package == manifest_in(path, |folder| state.manifests.get(folder))
    };
    state
        .files
        .range(start.clone()..)
        .take_while(|(path, _)| path_is_below(path, directory))
        .any(|(path, file)| derived(path, &file.input))
        || state
            .evicted_inputs
            .range(start..)
            .take_while(|(path, _)| path_is_below(path, directory))
            .any(|(path, input)| derived(path, input))
}

/// The text of the project configuration at `path` once the request applies: the request's own,
/// else the workspace's (`WorkspaceState::module_configs`) unless the request removes it.
fn module_config_text(
    state: &WorkspaceState,
    inputs: &[FileInput],
    removed: &[String],
    path: &str,
) -> Option<String> {
    if let Some(input) = inputs.iter().find(|input| input.path == path) {
        return Some(input.source.as_str().to_owned());
    }
    if removed.iter().any(|removed| removed == path) {
        return None;
    }
    state
        .module_configs
        .get(path)
        .map(|text| text.as_str().to_owned())
}

/// The ECMA files of a request the host sent no `package` for (`refresh_manifest_context`).
fn unhosted_manifests(files: &[WireFile]) -> HashSet<String> {
    files
        .iter()
        .filter(|wire| wire.package.is_none() && package_manifests(&wire.language))
        .map(|wire| wire.path.clone())
        .collect()
}

/// Applies `request` to `state`: the paths outside it whose module context it refreshed, and
/// whether the store is busy for this apply: deleting its removals found it busy past the write
/// deadline, or an earlier apply did and its window is still open (`store_cooling_down`).
pub(super) fn apply_request(
    state: &mut WorkspaceState,
    request: Request,
) -> Result<(BTreeSet<String>, bool)> {
    check_revision(state, request.revision)?;
    let unhosted = request
        .files
        .iter()
        .filter(|wire| packages(&wire.language) && wire.module.is_none())
        .map(|wire| wire.path.clone())
        .collect::<HashSet<_>>();
    let unhosted_packages = unhosted_manifests(&request.files);
    let mut inputs = request
        .files
        .into_iter()
        .enumerate()
        .map(|(index, wire)| wire.into_input(index))
        .collect::<Result<Vec<_>>>()?;
    let mut refreshed = refresh_package_context(state, &mut inputs, &unhosted, &request.removed)?;
    let (repackaged, manifest_changes) =
        refresh_manifest_context(state, &mut inputs, &unhosted_packages, &request.removed)?;
    refreshed.extend(repackaged);
    for input in &inputs {
        if module_graph::is_module_config(&input.path) {
            state
                .module_configs
                .insert(input.path.clone(), Arc::clone(&input.source));
        }
    }
    for path in &request.removed {
        state.module_configs.remove(path);
    }
    let mut released_candidates = false;
    let mut store_busy = false;
    if state.storage.is_some() {
        if state.store_cooling_down() {
            store_busy = true;
        } else if let Some(storage) = &state.storage {
            match storage
                .store
                .remove_overlay_files(storage.workspace_id, &request.removed)
            {
                Ok(()) => {}
                // A removed file's overlay row only keeps its objects from collection, so a busy
                // store keeps it until a later apply deletes it (`pending_removals`).
                Err(StoreError::Busy) => store_busy = true,
                Err(error) => return Err(invalid(error.to_string())),
            }
            if store_busy {
                state.note_store_busy("while removing files");
            }
        }
    }
    for path in request.removed {
        state.versions.remove(&path);
        // This reaches both resident and evicted producer state.
        CrossFileScheduler::forget(state, &path);
        if state.remove_path(&path).is_some() {
            released_candidates = true;
        }
        state.dirty_files.remove(&path);
        if store_busy {
            state.pending_removals.insert(path);
        }
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
            &state.code_rules,
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
    for (directory, change) in manifest_changes {
        match change {
            Some(manifest) => state.manifests.insert(directory, manifest),
            None => state.manifests.remove(&directory),
        };
    }
    state.result_page_index = None;
    state.revision = request.revision;
    if released_candidates {
        reclaim_candidate_sets(&mut state.candidate_sets);
    }
    Ok((refreshed, store_busy))
}

/// Whether a file is a package marker new to the workspace, which changes its siblings' modules
/// (`refresh_package_context`).
fn adds_package_marker(state: &WorkspaceState, file: &WireFile) -> bool {
    package_marker_directory(&file.path).is_some()
        && !state.files.contains_key(&file.path)
        && !state.evicted_inputs.contains_key(&file.path)
}

/// The inline path's one file (`is_cheap`) as `apply_request` reads it: a file whose family
/// names modules from package markers (Python) without a host module is named by the
/// workspace's markers, and one whose family reads manifests without a host package reads its
/// manifests. The file is no new marker and no manifest, so no sibling's context changes.
pub(super) fn inline_input(state: &mut WorkspaceState, file: WireFile) -> Result<FileInput> {
    let unhosted = (packages(&file.language) && file.module.is_none())
        .then(|| file.path.clone())
        .into_iter()
        .collect::<HashSet<_>>();
    let unhosted_packages = unhosted_manifests(std::slice::from_ref(&file));
    let mut inputs = vec![file.into_input(0)?];
    refresh_package_context(state, &mut inputs, &unhosted, &[])?;
    refresh_manifest_context(state, &mut inputs, &unhosted_packages, &[])?;
    Ok(inputs.swap_remove(0))
}

pub(super) fn is_cheap(state: &WorkspaceState, request: &Request) -> bool {
    state.storage.is_none() && !request.defer_global && !request.quiet && request.removed.is_empty()
        && request.files.len() == 1 && state.dirty_files.is_empty() && state.globals.is_empty()
        && state.files.len() <= 4 && request.files[0].source.len() <= 1024
        && !adds_package_marker(state, &request.files[0])
        && manifest_directory(&request.files[0].path).is_none()
        && state.files.values().map(|file| file.input.source.len()).sum::<usize>() <= 4096
        && state.code_rules.iter().map(|code_rule| code_rule.clauses.len()).sum::<usize>() <= 4
        && state.code_rules.iter().all(|code_rule| code_rule.clauses.iter().all(|clause|
            matches!(&clause.kind, ClauseKind::Forbid(Query::Regex { regex, .. }) if regex.as_str().len() <= 128)))
}
