//! Repository indexing: the host drives the index of the attached Git worktree one bounded chunk
//! per call (`WorkspaceIndexTask`), restoring completed clean directories and reading the rest.
//!
//! File set: the index evaluates exactly the files the host's walk selects (`walk`), which leaves
//! out what Git ignores only when the host follows `.gitignore`; `.rerailignore` and Rerail's
//! built-in ignores decide only which of them the store may restore.
//!
//! Memory bound: a chunk reads at most `INDEX_BATCH_FILES` files and stops reading once it holds
//! `INDEX_BATCH_BYTES` of source, so it holds less than that plus one file (at most
//! `max_file_bytes`). Its apply publishes the batch and releases every source and tape before the
//! chunk returns, so between chunks the workspace retains per-file results only.

use super::api::{WorkspaceApplyTask, cpu_slot};
use super::crossfile::CrossFileScheduler;
use super::global::record_evidence;
use super::ingest::{Request, WireFile, make_file};
use super::persisted::{candidate_catalog, file_context, restore_complete_results};
use super::storage::StorageContext;
use super::{
    Candidates, REUSE_DIRECTORY_RESULTS, REUSE_LOCAL_RESULTS, WorkspaceState, check_closed,
};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::language::adapter::{module_path, package_manifests};
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::package_manifest::{Manifest, Manifests};
use crate::content_store::{
    CandidateCatalogIdentity, DirectoryReuse, HashKey, StoredFile, StoredPathObjects,
};
use crate::git_workspace;
use napi::{Env, Result, Task};
use napi_derive::napi;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

/// Files one index chunk reads at most.
const INDEX_BATCH_FILES: usize = 32;
/// Source bytes after which an index chunk stops reading.
const INDEX_BATCH_BYTES: usize = 1024 * 1024;

#[cfg(test)]
thread_local! {
    /// Runs inside `plan` on this thread once the scoped files are listed, before the restore.
    static PLAN_GATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `gate` inside the next index plan on this thread, before it restores any file.
#[cfg(test)]
pub(super) fn gate_next_plan(gate: impl FnOnce() + 'static) {
    PLAN_GATE.with(|slot| *slot.borrow_mut() = Some(Box::new(gate)));
}

/// The host's bounds for one repository index; every chunk call carries the same.
#[napi(object)]
pub struct RepositoryIndexOptions {
    /// Scoped files the index covers at most (restored or read); beyond them it is over budget.
    pub max_files: u32,
    /// Source bytes the index reads at most in total; beyond them it is over budget.
    pub max_source_bytes: u32,
    /// A larger source is reported as over its per-file budget and not evaluated.
    pub max_file_bytes: u32,
    /// Directory names never indexed, at any depth below the workspace root.
    pub ignored_directories: Vec<String>,
    /// The file label of each extension the host evaluates (`.py` → `python`, `.md` →
    /// `markdown`), keyed lowercase with its dot; files with other extensions are not indexed.
    pub source_languages: HashMap<String, String>,
    /// Folders below the workspace root, workspace-relative, that are checkouts judged by their
    /// own rules: the walk neither lists nor reads them, and reports the ones it reached. Any
    /// other folder is walked, whether or not it holds a `.git` entry.
    pub nested_checkouts: Vec<String>,
    /// Whether files Git ignores (nested `.gitignore`s, `.git/info/exclude`, `core.excludesFile`;
    /// never a tracked file) are left out of the walk, as the host's own walks leave them out.
    pub follow_gitignore: bool,
}

pub struct WorkspaceIndexTask {
    pub(super) state: Arc<Mutex<WorkspaceState>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) options: RepositoryIndexOptions,
}

#[cfg(test)]
impl WorkspaceIndexTask {
    /// One index call over `state`, with sources above `max_file_bytes` refused.
    pub(super) fn for_test(
        state: Arc<Mutex<WorkspaceState>>,
        closed: Arc<AtomicBool>,
        max_file_bytes: u32,
    ) -> Self {
        Self {
            state,
            closed,
            options: RepositoryIndexOptions {
                max_files: u32::MAX,
                max_source_bytes: u32::MAX,
                max_file_bytes,
                ignored_directories: Vec::new(),
                source_languages: [
                    (".py", "python"),
                    (".ts", "typescript"),
                    (".md", "markdown"),
                ]
                .into_iter()
                .map(|(extension, label)| (extension.to_owned(), label.to_owned()))
                .collect(),
                nested_checkouts: Vec::new(),
                follow_gitignore: false,
            },
        }
    }
}

/// The index of one workspace: the scoped files not yet read, and what it has covered so far.
pub(super) struct RepositoryIndex {
    pending: VecDeque<PendingFile>,
    discovered: usize,
    restored: usize,
    reusable_directories: usize,
    read_files: usize,
    read_bytes: u64,
    /// The scoped files or their sources exceeded the host's bounds; the rest is not indexed.
    over_budget: bool,
    /// Directories the plan could not list, reported by the next chunk.
    failures: Vec<Value>,
    /// The host's nested checkouts the walk reached, sorted: each is a separate checkout judged by
    /// its own rules, so this index neither lists nor reads them.
    nested_checkouts: Vec<String>,
    /// The `package.json` files read so far, by directory.
    manifests: Manifests,
}

struct PendingFile {
    repository_path: String,
    path: String,
    language: String,
}

/// The clean trees under the workspace root, keyed by their workspace-relative prefix. A tree's
/// markers cover what its files' context reads outside it: the Python package markers of its
/// ancestors, and the `package.json` its ECMA files inherit from above it.
pub(super) fn reusable_directories(
    repository: &git_workspace::GitWorkspace,
    repository_prefix: &str,
    manifests: &mut Manifests,
) -> Vec<DirectoryReuse> {
    let root = repository_prefix.trim_end_matches('/');
    repository
        .clean_trees()
        .into_iter()
        .filter_map(|(prefix, tree)| {
            let logical_prefix = if root.is_empty() {
                prefix.clone()
            } else if prefix == root {
                String::new()
            } else {
                prefix.strip_prefix(repository_prefix)?.to_owned()
            };
            let mut marker_bytes = b"rerail-package-markers-v1\0".to_vec();
            let mut ancestor = std::path::PathBuf::new();
            marker_bytes.push(u8::from(
                repository.worktree_root.join("__init__.py").exists(),
            ));
            for component in std::path::Path::new(&prefix).components() {
                ancestor.push(component);
                marker_bytes.extend_from_slice(ancestor.to_string_lossy().as_bytes());
                marker_bytes.push(0);
                marker_bytes.push(u8::from(
                    repository
                        .worktree_root
                        .join(&ancestor)
                        .join("__init__.py")
                        .exists(),
                ));
            }
            // A file directly in the tree inherits the manifest at or above it.
            let inherited = manifests.of_directory(&repository.worktree_root.join(&prefix));
            marker_bytes.extend_from_slice(b"\0package\0");
            if let Some(manifest) = inherited {
                marker_bytes.extend_from_slice(manifest.digest());
            }
            Some(DirectoryReuse {
                prefix: logical_prefix,
                tree,
                package_markers: HashKey::sha256(&marker_bytes),
            })
        })
        .collect()
}

/// The module a host would send for a repository file: derived from the package markers in the
/// same worktree the source is read from, so repository and host ingestion agree.
pub(super) fn repository_module(
    repository: &git_workspace::GitWorkspace,
    repository_path: &str,
    language: &str,
) -> Option<Value> {
    module_path(language, &repository.worktree_root.join(repository_path))
}

/// The package a host would send for a repository file: the nearest `package.json` in the same
/// worktree of a file whose family reads one (`package_manifests`), below the workspace root
/// `manifests` reads.
pub(super) fn repository_package(
    manifests: &mut Manifests,
    repository: &git_workspace::GitWorkspace,
    repository_path: &str,
    language: &str,
) -> Option<Arc<Manifest>> {
    package_manifests(language)
        .then(|| manifests.of_file(&repository.worktree_root.join(repository_path)))
        .flatten()
}

fn restore_prepared_file(
    state: &mut WorkspaceState,
    input: FileInput,
    stored: StoredPathObjects,
    persisted: &[crate::content_store::StoredLocalResult],
) -> Result<bool> {
    // A `calls` closure reads each Python file's call summary, which only a job records.
    if CrossFileScheduler::needs_job(state, &input) {
        return Ok(false);
    }
    let path = input.path.clone();
    let code_rules = state.code_rules.clone();
    let mut file = make_file(
        input,
        state.revision.max(1),
        &code_rules,
        &mut state.candidate_sets,
    );
    let Some(restored) = restore_complete_results(&mut file, &state.code_rules, persisted) else {
        return Ok(false);
    };
    file.source_digest = stored
        .source_hash
        .sha256_bytes()
        .ok_or_else(|| invalid("stored source canonical hash is not SHA-256"))?;
    file.stored = Some(StoredFile {
        source: stored.objects.source,
        tape: None,
    });
    file.restored_local_results = restored.len();
    file.restored_result_ids = restored;
    file.candidates = Arc::new(Candidates::default());
    record_evidence(state, &path, &file);
    state.insert_file(path, file);
    Ok(true)
}

fn path_in_prefixes(path: &str, prefixes: &HashSet<String>) -> bool {
    if prefixes.contains("") {
        return true;
    }
    let mut end = path.len();
    while let Some(index) = path[..end].rfind('/') {
        if prefixes.contains(&path[..index]) {
            return true;
        }
        end = index;
    }
    false
}

/// A source the walk refuses as a symlink is not a file of the workspace, as for the host's walk.
#[cfg(unix)]
fn is_symlink(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_symlink(_error: &std::io::Error) -> bool {
    false
}

fn read_failure(path: &str, code: &str, message: &str) -> Value {
    json!({ "path": path, "code": code, "message": message })
}

impl WorkspaceIndexTask {
    fn lock(&self) -> Result<MutexGuard<'_, WorkspaceState>> {
        self.state
            .lock()
            .map_err(|_| invalid("policy workspace is unavailable"))
    }

    /// The host's file label for `path`: its longest extension the host lists (`.blade.php`
    /// before `.php`) with a nonempty name before it, as the host's `languageOfPath` reads it: a
    /// file named only `.php` has no extension, and `.blade.php` is a `.php` file named `.blade`.
    pub(super) fn source_language(&self, path: &str) -> Option<String> {
        let name = std::path::Path::new(path)
            .file_name()?
            .to_str()?
            .to_ascii_lowercase();
        name.match_indices('.')
            .filter(|&(at, _)| at > 0)
            .find_map(|(at, _)| self.options.source_languages.get(&name[at..]))
            .cloned()
    }

    /// The workspace's scoped sources as the host's walk selects them: every regular file below
    /// the workspace root with a host source language and a code-rule scope, outside the host's
    /// ignored directories, outside what Git ignores when the host follows `.gitignore` (a path
    /// whose ignore files cannot be read is kept and reported), and never through a symlink,
    /// sorted by path. `.rerailignore` and Rerail's built-in ignores play no part here; they decide
    /// only which of these files the store may restore. Directories it cannot list become read
    /// failures. A directory the host names as a nested checkout is not descended into, only
    /// reported. Also returns whether the files exceed `max_files`, in which case it stops listing.
    fn walk(
        &self,
        repository: &git_workspace::GitWorkspace,
        prefix: &str,
        scoped: impl Fn(&str) -> bool,
        failures: &mut Vec<Value>,
    ) -> Result<(Vec<PendingFile>, Vec<String>, bool)> {
        let ignored = self
            .options
            .ignored_directories
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let pruned = self
            .options
            .nested_checkouts
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let max_files = self.options.max_files as usize;
        let mut git_ignore = if self.options.follow_gitignore {
            crate::git_ignore::GitIgnore::open(&repository.worktree_root).map_err(invalid)?
        } else {
            None
        };
        let mut files = Vec::new();
        let mut nested_checkouts = BTreeSet::new();
        let mut over_budget = false;
        let mut directories = vec![(repository.worktree_root.join(prefix), String::new())];
        'walk: while let Some((directory, relative)) = directories.pop() {
            check_closed(&self.closed)?;
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) => {
                    failures.push(read_failure(
                        &relative,
                        "source-incomplete",
                        &format!("cannot read workspace directory: {error}"),
                    ));
                    continue;
                }
            };
            for entry in entries {
                let entry = entry.and_then(|entry| entry.file_type().map(|kind| (entry, kind)));
                let (entry, kind) = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        failures.push(read_failure(
                            &relative,
                            "source-incomplete",
                            &format!("cannot read workspace directory: {error}"),
                        ));
                        continue;
                    }
                };
                let name = entry.file_name();
                let path = match relative.as_str() {
                    "" => name.to_string_lossy().into_owned(),
                    relative => format!("{relative}/{}", name.to_string_lossy()),
                };
                // A directory has no language; symlinks and other kinds are never followed.
                let language = if kind.is_dir() {
                    if ignored.contains(name.to_string_lossy().as_ref()) {
                        continue;
                    }
                    None
                } else if kind.is_file() {
                    match self.source_language(&path).filter(|_| scoped(&path)) {
                        Some(language) => Some(language),
                        None => continue,
                    }
                } else {
                    continue;
                };
                if name.to_str().is_none() {
                    // The host's walk cannot open a path it decoded lossily either.
                    failures.push(read_failure(
                        &path,
                        "source-incomplete",
                        "workspace path is not valid UTF-8",
                    ));
                    continue;
                }
                if language.is_none() && pruned.contains(path.as_str()) {
                    // A nested checkout follows its own repository's ignores, not this one's.
                    nested_checkouts.insert(path);
                    continue;
                }
                if let Some(git_ignore) = git_ignore.as_mut() {
                    match git_ignore.ignored(&format!("{prefix}{path}"), language.is_none()) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        // Walked as if not ignored: an unreadable `.gitignore` never hides files.
                        Err(message) => {
                            failures.push(read_failure(&path, "source-incomplete", &message))
                        }
                    }
                }
                match language {
                    None => directories.push((entry.path(), path)),
                    Some(_) if files.len() == max_files => {
                        over_budget = true;
                        break 'walk;
                    }
                    Some(language) => files.push(PendingFile {
                        repository_path: format!("{prefix}{path}"),
                        path,
                        language,
                    }),
                }
            }
        }
        files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        Ok((files, nested_checkouts.into_iter().collect(), over_budget))
    }

    /// The scoped files under the workspace root, with every completed clean directory restored
    /// from the store; the rest is left for the chunks to read. Planning reads the store and
    /// inserts restored files into this scan's own workspace; it evaluates nothing, so it runs
    /// without the CPU slot and interactive applies proceed meanwhile.
    fn plan(
        &self,
        storage: &StorageContext,
        repository: &git_workspace::GitWorkspace,
    ) -> Result<RepositoryIndex> {
        let code_rules = self.lock()?.code_rules.clone();
        let prefix = storage.repository_prefix.as_str();
        let mut failures = Vec::new();
        let (accepted, nested_checkouts, over_budget) = self.walk(
            repository,
            prefix,
            |path| {
                code_rules
                    .iter()
                    .any(|code_rule| code_rule.scope.matches(path))
            },
            &mut failures,
        )?;
        let mut manifests = Manifests::new(repository.worktree_root.join(prefix))
            .reading_server_graphs(
                code_rules
                    .iter()
                    .any(|code_rule| code_rule.reads_server_graph()),
            );
        let directories = reusable_directories(repository, prefix, &mut manifests);
        let catalog = candidate_catalog(&code_rules)?;
        let reusable_prefixes = if REUSE_DIRECTORY_RESULTS {
            storage
                .store
                .reusable_directory_prefixes(catalog, &directories)
                .map_err(|error| invalid(error.to_string()))?
        } else {
            HashSet::new()
        };
        #[cfg(test)]
        if let Some(gate) = PLAN_GATE.with(|slot| slot.borrow_mut().take()) {
            gate();
        }
        let restored = if REUSE_LOCAL_RESULTS {
            self.restore(
                storage,
                repository,
                &accepted,
                &reusable_prefixes,
                catalog,
                &mut manifests,
            )?
        } else {
            HashSet::new()
        };
        let discovered = accepted.len();
        Ok(RepositoryIndex {
            pending: accepted
                .into_iter()
                .filter(|file| !restored.contains(&file.path))
                .collect(),
            discovered,
            restored: restored.len(),
            reusable_directories: reusable_prefixes.len(),
            read_files: 0,
            read_bytes: 0,
            over_budget,
            failures,
            nested_checkouts,
            manifests,
        })
    }

    /// Restore every file of a completed directory that is clean in Git, still holds its committed
    /// blob's bytes, and whose stored results cover all its clauses; returns the restored paths.
    fn restore(
        &self,
        storage: &StorageContext,
        repository: &git_workspace::GitWorkspace,
        accepted: &[PendingFile],
        reusable_prefixes: &HashSet<String>,
        catalog: CandidateCatalogIdentity,
        manifests: &mut Manifests,
    ) -> Result<HashSet<String>> {
        let aliases = accepted
            .iter()
            .filter(|file| path_in_prefixes(&file.path, reusable_prefixes))
            .filter_map(|file| {
                repository
                    .clean_source_alias(&file.repository_path)
                    .map(|alias| (file.path.clone(), alias))
            })
            .collect::<Vec<_>>();
        if aliases.is_empty() {
            return Ok(HashSet::new());
        }
        let stored_paths = storage
            .store
            .stored_sources_by_alias(&aliases)
            .map_err(|error| invalid(error.to_string()))?;
        let mut prepared = Vec::with_capacity(stored_paths.len());
        for file in accepted {
            let Some((stored, length)) = stored_paths.get(&file.path) else {
                continue;
            };
            let module = repository_module(repository, &file.repository_path, &file.language);
            let package =
                repository_package(manifests, repository, &file.repository_path, &file.language);
            let input = FileInput::new(
                file.path.clone(),
                file.language.clone(),
                Arc::new(String::new()),
                module.as_ref(),
                package,
                &file.path,
            )?;
            let key = (stored.source_hash, file_context(&input));
            prepared.push((file, input, stored, *length, key));
        }
        let keys = prepared
            .iter()
            .map(|(_, _, _, _, key)| *key)
            .collect::<Vec<_>>();
        let result_manifest = storage
            .store
            .local_check_results_by_key(&keys, catalog)
            .map_err(|error| invalid(error.to_string()))?;
        // Discovery found these files clean at scan start, but a file may change after it, and a
        // checkout conversion keeps a file clean in Git while its bytes differ from the blob's
        // (whose stored source and results another worktree may have published). Restore only a
        // file that holds the blob's bytes now; verify before taking the workspace lock, since a
        // changed stat costs a hash.
        let verified = prepared
            .into_iter()
            .filter_map(|(file, input, stored, length, key)| {
                let results = result_manifest.get(&key)?;
                repository
                    .holds_blob_bytes(&file.repository_path, length)
                    .then_some((input, stored, results))
            })
            .collect::<Vec<_>>();
        let mut state = self.lock()?;
        let mut restored = HashSet::new();
        for (input, stored, results) in verified {
            let path = input.path.clone();
            if restore_prepared_file(&mut state, input, stored.clone(), results)? {
                restored.insert(path);
            }
        }
        Ok(restored)
    }

    /// Read the next batch of pending sources; unreadable ones become read failures.
    fn read_batch(
        &self,
        repository: &git_workspace::GitWorkspace,
        index: &mut RepositoryIndex,
        failures: &mut Vec<Value>,
    ) -> Result<Vec<WireFile>> {
        let max_file_bytes = u64::from(self.options.max_file_bytes);
        let max_source_bytes = u64::from(self.options.max_source_bytes);
        let mut batch = Vec::new();
        let mut batch_bytes = 0_usize;
        while batch.len() < INDEX_BATCH_FILES && batch_bytes < INDEX_BATCH_BYTES {
            let Some(file) = index.pending.pop_front() else {
                break;
            };
            check_closed(&self.closed)?;
            let bytes = match repository.read_source(&file.repository_path, max_file_bytes) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
                    failures.push(read_failure(
                        &file.path,
                        "budget-exceeded",
                        "Source exceeds the per-file byte budget",
                    ));
                    continue;
                }
                Err(error) if is_symlink(&error) => continue,
                Err(error) => {
                    failures.push(read_failure(
                        &file.path,
                        "source-incomplete",
                        &format!("cannot read workspace source: {error}"),
                    ));
                    continue;
                }
            };
            if bytes.contains(&0) {
                failures.push(read_failure(
                    &file.path,
                    "source-incomplete",
                    "Binary source cannot be evaluated",
                ));
                continue;
            }
            let length = bytes.len() as u64;
            if index.read_bytes + length > max_source_bytes {
                index.over_budget = true;
                index.pending.clear();
                break;
            }
            index.read_files += 1;
            index.read_bytes += length;
            batch_bytes += bytes.len();
            // As the host decodes a file it reads: invalid UTF-8 becomes replacement characters.
            let source = String::from_utf8(bytes)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
            let module = repository_module(repository, &file.repository_path, &file.language);
            let package = repository_package(
                &mut index.manifests,
                repository,
                &file.repository_path,
                &file.language,
            );
            batch.push(WireFile {
                path: file.path,
                language: file.language,
                source,
                module,
                package,
            });
        }
        Ok(batch)
    }
}

impl Task for WorkspaceIndexTask {
    type Output = String;
    type JsValue = String;

    /// One chunk: plan on the first call, then read one batch and apply it with global clauses
    /// deferred (the host's closing apply evaluates them). Only the read and apply hold the CPU
    /// slot: an interactive apply waits for one batch at most, and if it holds the slot the
    /// chunk keeps its plan and reports busy, so the host retries.
    fn compute(&mut self) -> Result<String> {
        check_closed(&self.closed)?;
        let (storage, index) = {
            let mut state = self.lock()?;
            let storage = state
                .storage
                .clone()
                .ok_or_else(|| invalid("repository indexing requires storage"))?;
            (storage, state.repository_index.take())
        };
        let repository = storage
            .repository
            .clone()
            .ok_or_else(|| invalid("repository indexing requires Git discovery"))?;
        let mut index = match index {
            Some(index) => index,
            None => self.plan(&storage, &repository)?,
        };
        let Ok(_slot) = cpu_slot().try_lock() else {
            self.lock()?.repository_index = Some(index);
            return Err(invalid("WORKSPACE_BUSY"));
        };
        let mut failures = std::mem::take(&mut index.failures);
        let batch = self.read_batch(&repository, &mut index, &mut failures)?;
        let evaluation = if batch.is_empty() {
            Value::Null
        } else {
            let revision = self.lock()?.revision.saturating_add(1);
            let mut apply = WorkspaceApplyTask {
                state: Arc::clone(&self.state),
                closed: Arc::clone(&self.closed),
                request_json: String::new(),
                request_buffer: None,
                request_direct: Some(Request {
                    revision,
                    files: batch,
                    removed: Vec::new(),
                    defer_global: true,
                    quiet: false,
                }),
            };
            let mut output: Value =
                serde_json::from_str(&apply.apply_with_slot()?.text).map_err(json_error)?;
            output["evaluation"].take()
        };
        let mut state = self.lock()?;
        check_closed(&self.closed)?;
        let done = index.pending.is_empty();
        let response = json!({
            "revision": state.revision,
            "done": done,
            "overBudget": done && index.over_budget,
            "evaluation": evaluation,
            "failures": failures,
            "discoveredFiles": index.discovered,
            "restoredFiles": index.restored,
            "readFiles": index.read_files,
            "readBytes": index.read_bytes,
            "reusableDirectories": index.reusable_directories,
            "nestedCheckouts": index.nested_checkouts,
        });
        state.repository_index = Some(index);
        serde_json::to_string(&response).map_err(json_error)
    }

    fn resolve(&mut self, _env: Env, output: String) -> Result<String> {
        check_closed(&self.closed)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index routes a name as the host's `languageOfPath` does: the longest listed extension
    /// with a nonempty name before it, case-insensitively.
    #[test]
    fn source_language_routes_php_and_blade_as_the_host_does() {
        let mut task = WorkspaceIndexTask::for_test(
            Arc::new(Mutex::new(WorkspaceState::default())),
            Arc::new(AtomicBool::new(false)),
            u32::MAX,
        );
        task.options.source_languages = [(".php", "php"), (".blade.php", "blade")]
            .into_iter()
            .map(|(extension, label)| (extension.to_owned(), label.to_owned()))
            .collect();
        for (path, expected) in [
            ("a.php", Some("php")),
            ("app/Http/Kernel.php", Some("php")),
            ("A.PHP", Some("php")),
            ("resources/views/page.blade.php", Some("blade")),
            ("deep/nested/dir/Page.Blade.PHP", Some("blade")),
            ("archive.v1.blade.php", Some("blade")),
            ("blade.php", Some("php")),
            ("x.blade.php.bak", None),
            ("page.phtml", None),
            (".php", None),
            ("dir/.php", None),
            (".blade.php", Some("php")),
        ] {
            assert_eq!(task.source_language(path).as_deref(), expected, "{path}");
        }
    }
}
