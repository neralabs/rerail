// A build of some languages, or without the addon (`headless`), leaves the others' code and the addon's API unused by
// design: dead-code warnings there say nothing (the default build still warns).
#![cfg_attr(
    any(not(feature = "all-languages"), feature = "headless"),
    allow(dead_code)
)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// Canonical paths without Windows' verbatim prefix: the engine's every one (`rerail_store::Canonical`).
use rerail_store::Canonical as _;
/// `build.rs` compiles this file too; the library builds it only for tests, so `cargo test`
/// checks the identities the build script computes.
#[cfg(test)]
mod build_identity;

/// `build.rs` generates the language tables with this file; the library builds it only for tests,
/// so `cargo test` checks how the build validates the language registry. The rest is the build script's.
#[cfg(test)]
#[allow(dead_code)]
mod build_languages;
// Each test's time limit, enforced inside the test binary (`RERAIL_TEST_TIMEOUT`).
mod code_rule_dsl;
#[cfg(test)]
mod executors_guard_tests;
#[cfg(test)]
mod test_watchdog;
// The test binary's stores flush nothing (`RERAIL_FLUSH=none`, decision 0481) unless the environment says otherwise.
#[cfg(test)]
mod test_flush;
// The test binary's folders are the user's alone (a umask of 077, decision 0574).
#[cfg(test)]
mod test_private_folders;
#[cfg(test)]
mod text_readers_guard_tests;
#[cfg(all(test, not(windows)))]
pub(crate) use code_rule_dsl::prof::purge_allocator_pages;
/// The engine's tools without Node (`rerail-engine dump` / `asm`).
pub use code_rule_dsl::tools;
pub use rerail_store::content_store;
/// Every in-process Git read (`native/src/git_access`): the opener, its setup and its pool are the store's
/// (`rerail-store`), which the workspace discovery and the ignore matcher open through; the Node-API object that
/// installs the setup is the engine's.
pub mod git_access {
    pub use rerail_store::git_access::*;
    pub mod owner;
    #[cfg(test)]
    mod tests;
}
pub mod git_start;
pub use rerail_store::git_workspace;
pub use rerail_store::source_tree;
pub use rerail_workspace::git_ignore;
mod main_instance;
mod model;
pub use rerail_workspace::panic_boundary;
mod path_owner;
pub mod process;
/// The warning on a macOS whose allocator aborts the engine's work (`bin/rerail-engine.rs`).
pub mod os_warning;
pub use rerail_logic::threads;
/// The browser's long-lived engine: `rerail_command` and its buffers (wasm/README.md).
#[cfg(target_family = "wasm")]
pub mod reactor;
mod rules;
pub use rerail_tree_watch::tree_watch;

use model::{ModelFingerprint, shared_model, verify_model};
use napi::{Error, Result, Status, bindgen_prelude::Float32Array};
use napi_derive::napi;
use rules::{CompileSummary, ModelEncoder, NativeMatch, NativeRuleInput, NativeSample, RuleSet};
use std::path::Path;

// jemalloc does not build for MSVC targets; Windows takes mimalloc (below). The test build declares it too
// (`rust_allocations_reach_the_profile_allocator` checks it).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: rerail_prof::heap::Sampled = rerail_prof::heap::Sampled;

#[cfg(all(windows, not(feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(not(any(test, windows, target_family = "wasm")))]
union JemallocConf {
    byte: &'static u8,
    character: &'static std::ffi::c_char,
}

// Freed pages stay for a second so an apply reuses them instead of mapping new ones (immediate
// decay cost a tenth of a storage-heavy benchmark's CPU in mmap/munmap); every apply purges them
// when it ends and again once its response is freed (`purge_allocator_pages`), so nothing dirty
// is kept at rest.
#[cfg(not(any(test, windows, target_family = "wasm")))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static jemalloc_conf: Option<&'static std::ffi::c_char> = Some(unsafe {
    JemallocConf {
        byte: &b"dirty_decay_ms:1000,muzzy_decay_ms:0\0"[0],
    }
    .character
});

/// The engine's SQL functions, for every connection the store opens (its crate, `rerail-store`): the text tests'
/// and the prefilter's tapes; with the build's identities, devices and languages. Once a process, before any store is
/// opened: as the library loads (the allocator's hook; Windows' module init), and where neither runs (the browser's
/// build) at the first command (`tools::cli_in`).
pub(crate) fn install_store_functions() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let grammar = env!("RERAIL_GRAMMAR_BUILD_ID");
        // A test can name another grammar build (32 hex digits), as a store written by another engine build has.
        #[cfg(feature = "test-hooks")]
        let grammar = std::env::var("RERAIL_TEST_GRAMMAR_BUILD_ID")
            .ok()
            .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map_or(grammar, |id| Box::leak(id.into_boxed_str()));
        code_rule_dsl::boot::install(code_rule_dsl::boot::BuildIds { engine: env!("RERAIL_ENGINE_BUILD_ID"), grammar });
    });
}

/// Windows has no jemalloc hook (below): the store's functions alone, as the library loads.
#[cfg(windows)]
#[napi_derive::module_init]
fn install_store_functions_on_load() {
    install_store_functions();
}

/// Tree-sitter allocates through jemalloc, as Rust does: a parse allocates many small subtrees,
/// which the system allocator served at 10% of the parse. Installed when the addon loads, before
/// any Tree-sitter call; like Tree-sitter's own allocator, these abort when allocation fails.
#[cfg(not(any(windows, target_family = "wasm")))]
#[napi_derive::module_init]
fn install_tree_sitter_allocator() {
    install_store_functions();
    use std::ffi::c_void;
    unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
        // SAFETY: jemalloc's malloc accepts any size.
        allocated(unsafe { tikv_jemalloc_sys::malloc(size) }, size)
    }
    unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
        // SAFETY: jemalloc's calloc accepts any count and size, and fails on overflow.
        allocated(
            unsafe { tikv_jemalloc_sys::calloc(count, size) },
            count.saturating_mul(size),
        )
    }
    unsafe extern "C" fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
        // SAFETY: Tree-sitter passes null or a pointer these functions returned.
        if rerail_prof::heap::on() {
            rerail_prof::heap::free(pointer as usize);
        }
        allocated(unsafe { tikv_jemalloc_sys::realloc(pointer, size) }, size)
    }
    unsafe extern "C" fn free(pointer: *mut c_void) {
        if rerail_prof::heap::on() {
            rerail_prof::heap::free(pointer as usize);
        }
        // SAFETY: Tree-sitter passes null or a pointer these functions returned.
        unsafe { tikv_jemalloc_sys::free(pointer) }
    }
    // The heap sampler sees Tree-sitter's allocations too (`rerail_prof::heap`).
    fn allocated(pointer: *mut c_void, size: usize) -> *mut c_void {
        if pointer.is_null() && size > 0 {
            std::process::abort();
        }
        if rerail_prof::heap::on() {
            rerail_prof::heap::alloc_by(pointer as usize, size, rerail_prof::heap::TREE_SITTER);
        }
        pointer
    }
    // SAFETY: the four functions are jemalloc's, return no null for a nonzero size, and are
    // installed while the addon loads, before any Tree-sitter object exists.
    #[cfg(not(feature = "system-allocator"))]
    unsafe {
        tree_sitter::set_allocator(Some(tree_sitter::Allocator {
            malloc,
            calloc,
            realloc,
            free,
        }));
    }
}

/// The one block policy, as Node registers the addon's exports, before any of them runs. The pool is each
/// environment's machine's (decision 0442: `workspace::api::environment_machine`), made at its first workspace, never
/// in the library's constructor: a constructor runs inside `dlopen`, which holds glibc's loader lock; rayon's
/// `use_current_thread` waits for every pool thread to start, and each one's first thread-local with a destructor
/// registers it through `__cxa_thread_atexit_impl`, which takes that lock: the load never returned.
#[cfg(not(target_family = "wasm"))]
#[napi(module_exports)]
pub fn configure_thread_pool(_exports: napi::bindgen_prelude::Object) {
    threads::configure();
}

#[napi(object)]
pub struct Health {
    pub ok: bool,
    #[napi(js_name = "kernelVersion")]
    pub engine_version: String,
}

#[napi]
pub fn health() -> Health {
    Health {
        ok: true,
        engine_version: "1".to_owned(),
    }
}

/// What `verify_model` found: `problem` and `file` are absent where the model matches its manifest.
#[napi(object)]
pub struct ModelCheck {
    /// `missing`, `unreadable`, `invalid-manifest` or `checksum-mismatch`.
    pub problem: Option<String>,
    /// The file in the model directory at fault, `manifest.json` included.
    pub file: Option<String>,
}

/// Checks the model directory at `model_path` against its `manifest.json` (every file's SHA-256),
/// as a session's attach does, without loading the model.
#[napi(catch_unwind)]
pub fn verify_model_files(model_path: String) -> ModelCheck {
    match verify_model(Path::new(&model_path)) {
        Ok(()) => ModelCheck {
            problem: None,
            file: None,
        },
        Err(fault) => ModelCheck {
            problem: Some(fault.problem.to_owned()),
            file: Some(fault.file.to_owned()),
        },
    }
}

/// Check a pattern against the regex dialect and size limit structural rule conditions compile with.
#[napi(catch_unwind)]
pub fn validate_regex(pattern: String) -> Result<()> {
    rules::compile_regex(&pattern)
        .map(|_| ())
        .map_err(|reason| Error::new(Status::InvalidArg, reason))
}

/// The content-store file name this binary's schema floor owns (`content_store::STORE_FILE_NAME`):
/// the host joins it to the store directory its build owns, so the name has one source.
#[napi]
pub fn content_store_file_name() -> &'static str {
    content_store::STORE_FILE_NAME
}

/// `f` on the entry point's shared object (the run under way's, else one of its own: `default_shared`), with the Git
/// revisions it was handed pinned there (decision 0442): the runs `f` starts read the paths under their roots from them,
/// and no other entry point's runs do.
#[cfg(not(feature = "headless"))]
fn on_revisions<'s, T>(snapshots: impl IntoIterator<Item = &'s git_workspace::snapshot::Snapshot>, f: impl FnOnce() -> T) -> T {
    use code_rule_dsl::logic::profile;
    let shared = profile::shared_object().unwrap_or_else(profile::default_shared);
    let _in = profile::within(std::sync::Arc::clone(&shared));
    let _pinned: Vec<_> = snapshots.into_iter().map(|snapshot| snapshot.pin(&shared)).collect();
    f()
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn inspect_rr_policy(path: String, source: Option<&rerail_workspace::code_rule_dsl::workspace::api::GitRevisionSource>) -> Result<String> {
    let snapshot = source.map(|source| source.pinned_snapshot());
    on_revisions(snapshot.as_deref(), || code_rule_dsl::logic::plan::corpus::combined::inspect_separately(||
        LoadedRrPolicy::read(&path, snapshot.clone(), None)?.inspection()))
}

#[cfg(not(feature = "headless"))]
#[derive(Clone)]
enum PolicyLocation {
    Worktree { path: std::path::PathBuf, library: std::path::PathBuf },
    Git { snapshot: std::sync::Arc<git_workspace::snapshot::Snapshot>, inputs: Option<code_rule_dsl::logic::rr::GitPolicyInputs> },
}

#[cfg(not(feature = "headless"))]
#[derive(Clone)]
struct LoadedRrPolicy {
    location: PolicyLocation,
    program: std::sync::Arc<code_rule_dsl::logic::program_image::Version>,
    files: Vec<code_rule_dsl::logic::rr::LibraryFile>,
    /// What its load did (`LoadedRrPolicies::load`'s stages: compiles, graph hits): a scan handed it ran that load
    /// first, and its profile counts it (`Prepared::add`).
    prepared: Prepared,
}

/// The stages a run counted (`rerail_prof::stages`) between two readings: a policy load's, carried to the scan.
#[cfg(not(feature = "headless"))]
#[derive(Clone, Default)]
struct Prepared(std::sync::Arc<Vec<(String, std::time::Duration, usize)>>);

#[cfg(not(feature = "headless"))]
impl Prepared {
    fn now() -> std::collections::BTreeMap<String, (std::time::Duration, usize)> {
        rerail_prof::stages().clone()
    }
    /// What the run counted since `before`.
    fn since(before: &std::collections::BTreeMap<String, (std::time::Duration, usize)>) -> Prepared {
        let was = |stage: &str| before.get(stage).copied().unwrap_or_default();
        Prepared(std::sync::Arc::new(Self::now().into_iter().filter_map(|(stage, (time, count))| {
            let (t, c) = was(&stage);
            (time > t || count > c).then(|| (stage, time.saturating_sub(t), count.saturating_sub(c)))
        }).collect()))
    }
    /// Counted again by the run under way.
    fn add(&self) {
        let mut stages = rerail_prof::stages();
        for (stage, time, count) in self.0.iter() {
            let at = stages.entry(stage.clone()).or_default();
            at.0 += *time;
            at.1 += count;
        }
    }
}

#[cfg(not(feature = "headless"))]
impl LoadedRrPolicy {
    fn read(path: &str, snapshot: Option<std::sync::Arc<git_workspace::snapshot::Snapshot>>, inspected: Option<&[code_rule_dsl::logic::rr::LibraryFile]>) -> Result<Self> {
        let path = rr_policy_path(path, snapshot.as_deref())?;
        let code_rule_dsl::logic::rr::ReadProgram { program, files, git } = tools::inspect_policy_after(&path, inspected).map_err(Error::from_reason)?;
        let location = match snapshot {
            Some(snapshot) => PolicyLocation::Git { snapshot, inputs: git },
            None => PolicyLocation::Worktree { path: path.clone(), library: code_rule_dsl::logic::rr::library_root() },
        };
        Ok(Self { location, program, files, prepared: Prepared::default() })
    }

    fn inspection(&self) -> Result<String> {
        let metadata = code_rule_dsl::logic::rr::metadata(&*self.program).map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(serde_json::json!({ "metadata": metadata, "programInputs": tools::policy_inputs(&self.files) }).to_string())
    }
}

/// The programs and includes inspected during one scan's policy selection.
#[cfg(not(feature = "headless"))]
#[napi]
#[derive(Default)]
pub struct LoadedRrPolicies {
    store: Option<std::path::PathBuf>,
    programs: std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, PolicyVersions>>,
}

#[cfg(not(feature = "headless"))]
type PolicyVersions = std::collections::HashMap<Option<git_workspace::snapshot::SnapshotKey>, LoadedRrPolicy>;

#[cfg(not(feature = "headless"))]
#[napi]
impl LoadedRrPolicies {
    #[napi(constructor)]
    pub fn new(store: Option<String>) -> Self {
        Self { store: store.map(Into::into), ..Self::default() }
    }

    #[napi]
    pub fn load(&self, path: String, source: Option<&rerail_workspace::code_rule_dsl::workspace::api::GitRevisionSource>) -> Result<String> {
        let key = policy_file_path(&path)?;
        let read = || code_rule_dsl::logic::plan::corpus::combined::inspect_separately(|| {
            let inspected = if source.is_none() && self.store.is_some() {
                self.programs.lock().unwrap_or_else(|err| err.into_inner()).get(key)
                    .and_then(|versions| versions.get(&None)).map(|policy| policy.files.clone())
            } else { None };
            let before = Prepared::now();
            let mut policy = LoadedRrPolicy::read(&path, source.map(|source| source.pinned_snapshot()), inspected.as_deref())?;
            policy.prepared = Prepared::since(&before);
            let inspection = policy.inspection()?;
            let origin = match &policy.location {
                PolicyLocation::Git { snapshot, .. } => Some(snapshot.source_key()),
                PolicyLocation::Worktree { .. } => None,
            };
            self.programs.lock().unwrap_or_else(|err| err.into_inner()).entry(key.to_owned()).or_default().insert(origin, policy);
            Ok(inspection)
        });
        let snapshot = source.map(|source| source.pinned_snapshot());
        on_revisions(snapshot.as_deref(), || match &self.store {
            Some(store) => code_rule_dsl::logic::plan::corpus::with_tape_cache_path(store, None, || {
                let inspection = read()?;
                // The load is a command (decision 0441): what it queued for the store (its program's version) is in
                // the store as it returns, not applied behind a later command, whose store may by then be another file.
                // The store the load ran under (`tape_cache_path`): none for `:memory:`, which names no file to open.
                if let Some(path) = code_rule_dsl::logic::plan::corpus::tape_cache_path()
                    && let Ok(handle) = content_store::ContentStore::open(&path)
                    && let Err(error) = handle.written()
                {
                    eprintln!("rerail: {}: the policy load's writes: {error}", path.display());
                }
                Ok(inspection)
            }),
            None => read(),
        })
    }

    /// The policy a failed `load` of `path` names: the one `.policy` its own text declares, read without lowering
    /// (`declared_policy_ids`); `null` when it declares none or several, or cannot be read.
    #[napi]
    pub fn failed_policy(&self, path: String, source: Option<&rerail_workspace::code_rule_dsl::workspace::api::GitRevisionSource>) -> Option<String> {
        let snapshot = source.map(|source| source.pinned_snapshot());
        on_revisions(snapshot.as_deref(), || failed_policy_id(declared_policy_ids(&path, snapshot.as_deref())))
    }
}

/// The `.policy` IDs the policy at `path` declares in its own text (`rr::declared_policies`), read where `snapshot`
/// (a Git view, registered: `on_revisions`) or the worktree holds it, without its includes or lowering: what a policy
/// whose program does not load names in its failure. Empty when the text cannot be read.
#[cfg(not(feature = "headless"))]
fn declared_policy_ids(path: &str, snapshot: Option<&git_workspace::snapshot::Snapshot>) -> Vec<String> {
    let Ok(path) = rr_policy_path(path, snapshot) else { return Vec::new() };
    let bytes = if git_workspace::snapshot::view_root(&path).is_some() {
        git_workspace::snapshot::read_bounded(&path, 1024 * 1024).ok()
    } else {
        std::fs::read(&path).ok()
    };
    bytes.and_then(|bytes| String::from_utf8(bytes).ok()).map(|text| code_rule_dsl::logic::rr::declared_policies(&text)).unwrap_or_default()
}

/// The one policy a policy's failure names: its single declared `.policy` (`declared_policy_ids`), else none.
#[cfg(not(feature = "headless"))]
fn failed_policy_id(ids: Vec<String>) -> Option<String> {
    let [id] = <[String; 1]>::try_from(ids).ok()?;
    Some(id)
}

#[cfg(not(feature = "headless"))]
fn policy_file_path(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    if git_workspace::snapshot::requires_directory(path) {
        return Err(Error::from_reason(format!("Expected a policy file at {}", path.display())));
    }
    Ok(path.strip_prefix(".").unwrap_or(path))
}

#[cfg(not(feature = "headless"))]
fn rr_policy_path(path: &str, snapshot: Option<&git_workspace::snapshot::Snapshot>) -> Result<std::path::PathBuf> {
    let path = policy_file_path(path)?;
    match snapshot {
        None => path.canonical().map_err(|error| Error::from_reason(format!("{}: {error}", path.display()))),
        Some(snapshot) if path.is_relative() && path.components().all(|part| matches!(part, std::path::Component::Normal(_))) => Ok(snapshot.root().join(path)),
        Some(_) => Err(Error::from_reason("Expected a repository-relative policy path")),
    }
}

/// What a policy scan reads and where it writes: the policy files, the checkout's root, the content store, the output
/// directory, and the source paths it is held to.
#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct RrScanTarget {
    pub paths: Vec<String>,
    pub root: String,
    pub store: String,
    pub output: String,
    pub source_paths: Option<Vec<String>>,
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn scan_rr_policies_async(
    target: RrScanTarget,
    analysis: Option<&rerail_workspace::code_rule_dsl::workspace::api::AnalysisScopeRun>,
    source: Option<&rerail_workspace::code_rule_dsl::workspace::api::GitRevisionSource>,
    policy_source: Option<&rerail_workspace::code_rule_dsl::workspace::api::GitRevisionSource>,
    loaded: Option<&LoadedRrPolicies>,
) -> Result<napi::bindgen_prelude::AsyncTask<ScanRrPolicies>> {
    let RrScanTarget { paths, root, store, output, source_paths } = target;
    let policy_snapshot = policy_source.map(|source| source.pinned_snapshot());
    let policies = loaded.map(|loaded| {
        let programs = loaded.programs.lock().unwrap_or_else(|error| error.into_inner());
        let origin = policy_snapshot.as_ref().map(|snapshot| snapshot.source_key());
        paths.iter().map(|path| {
            let key = policy_file_path(path).map_err(|error| error.to_string())?;
            let versions = programs.get(key).ok_or_else(|| format!("{path}: policy was not loaded"))?;
            if let Some(policy) = versions.get(&origin) { return Ok(policy.clone()); }
            if origin.is_some() { return Err(format!("{path}: policy was not loaded from the selected Git source")); }
            if versions.len() == 1 { return Ok(versions.values().next().expect("one inspected policy").clone()); }
            Err(format!("{path}: select a policy source for this path inspected in multiple revisions"))
        }).collect()
    });
    let owner = analysis.map(|run| run.owner_for(Path::new(&store), Path::new(&root), source)).transpose()?;
    Ok(napi::bindgen_prelude::AsyncTask::new(ScanRrPolicies {
        paths, root, store, output, owner, source_paths,
        snapshot: source.map(|source| source.pinned_snapshot()),
        policy_snapshot,
        policies,
    }))
}

#[cfg(not(feature = "headless"))]
pub struct ScanRrPolicies {
    paths: Vec<String>,
    root: String,
    store: String,
    output: String,
    owner: Option<content_store::analysis_scope::AnalysisOwner>,
    source_paths: Option<Vec<String>>,
    snapshot: Option<std::sync::Arc<git_workspace::snapshot::Snapshot>>,
    policy_snapshot: Option<std::sync::Arc<git_workspace::snapshot::Snapshot>>,
    policies: Option<Vec<std::result::Result<LoadedRrPolicy, String>>>,
}

#[cfg(not(feature = "headless"))]
impl napi::Task for ScanRrPolicies {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        // The scan's revisions, its loaded policies' among them, are its own (decision 0442).
        let mut snapshots: Vec<_> = self.snapshot.iter().chain(&self.policy_snapshot).cloned().collect();
        snapshots.extend(self.policies.iter().flatten().flatten().filter_map(|policy| match &policy.location {
            PolicyLocation::Git { snapshot, .. } => Some(std::sync::Arc::clone(snapshot)),
            PolicyLocation::Worktree { .. } => None,
        }));
        on_revisions(snapshots.iter().map(|snapshot| &**snapshot), || self.scan())
    }

    fn resolve(&mut self, _env: napi::Env, output: String) -> Result<String> {
        Ok(output)
    }
}

#[cfg(not(feature = "headless"))]
impl ScanRrPolicies {
    fn scan(&mut self) -> Result<String> {
        use content_store::analysis_scope::{self, AnalysisRun, RunSource};
        use code_rule_dsl::logic::{plan::corpus, profile};
        // The scan is one run (decision 0441: a command's): every policy's runs join it, and the programs each runs
        // first (its dialects', Git's inventory). Its profile is theirs together (decision 0442: the records are the
        // run's), held here and written once at its end; held by a corpus run, each run's would replace the last's.
        let _run = code_rule_dsl::logic::run::scope(code_rule_dsl::logic::native::width::Settings::from_env);
        let profiled = if profile::on() { profile::hold_records() } else { None };
        if profiled.is_some() {
            profile::reset();
            // The loads it was handed ran first, for it: its profile counts them.
            for policy in self.policies.iter().flatten().flatten() {
                policy.prepared.add();
            }
        }
        let path = Path::new(&self.store);
        let root = self.snapshot.as_ref().map_or_else(|| Path::new(&self.root), |snapshot| snapshot.root());
        let mut warning = None;
        let mut run = None;
        if self.owner.is_none() && path != Path::new(":memory:") && !self.paths.is_empty() {
            let scope = git_workspace::snapshot::analysis_scope(root).unwrap_or_else(|| analysis_scope::directory(root));
            let source = self.snapshot.as_ref().map_or_else(|| RunSource::directory(root), |view| view.run_source());
            match content_store::ContentStore::open(path).and_then(|store| AnalysisRun::begin_batch(store, &scope, source)) {
                Ok(batch) => run = Some(batch),
                Err(error) => warning = Some(error.to_string()),
            }
        }
        let path = if warning.is_some() { Path::new(":memory:") } else { path };
        let owner = self.owner.as_ref().or_else(|| run.as_ref().map(|run| &run.owner)).cloned();
        let ownership = owner.clone().map(|owner| (root.to_owned(), owner));
        let (mut reports, source_complete) = corpus::with_tape_cache_path(path, ownership, || {
            let options = serde_json::json!({ "sourcePaths": self.source_paths, "scanSpans": true, "reuse": path != Path::new(":memory:") });
            let preloaded = self.policies.is_some();
            let policies = self.policies.take().unwrap_or_else(|| self.paths.iter().map(|path|
                LoadedRrPolicy::read(path, self.policy_snapshot.clone(), None).map_err(|error| error.to_string())
            ).collect());
            let mut counts = std::collections::HashMap::new();
            let policies: Vec<_> = policies.into_iter().map(|policy| {
                let policy = policy?;
                let metadata = code_rule_dsl::logic::rr::metadata(&*policy.program).map_err(|err| err.to_string())?;
                let declared = metadata["policies"].as_array().filter(|ids| !ids.is_empty()).ok_or(".policy declaration required")?;
                let mut ids = Vec::new();
                for entry in declared {
                    let id = entry["id"].as_str().filter(|id| !id.trim().is_empty()).ok_or(".policy ID must not be empty")?;
                    ids.push(id.to_owned());
                    *counts.entry(id.to_owned()).or_insert(0usize) += 1;
                }
                Ok::<_, String>((policy, ids))
            }).collect();
            let mut reports = Vec::new();
            let mut ephemeral = path == Path::new(":memory:");
            for (i, policy) in policies.into_iter().enumerate() {
                let (LoadedRrPolicy { program, files, location, .. }, ids) = match policy {
                    Ok(policy) => policy,
                    Err(error) => {
                        // A program that did not load names the policy its own text declares.
                        let id = self.paths.get(i).and_then(|path| failed_policy_id(declared_policy_ids(path, self.policy_snapshot.as_deref())));
                        reports.push(serde_json::json!({"error": error, "ruleId": id}));
                        continue;
                    }
                };
                if let Some(id) = ids.iter().find(|id| counts[*id] > 1) {
                    reports.push(serde_json::json!({"error": format!("duplicate .rerail policy ID {id:?}"), "ruleId": id}));
                    continue;
                }
                let out = if self.output.is_empty() { std::path::PathBuf::new() } else { Path::new(&self.output).join(i.to_string()) };
                let report = if ephemeral {
                    None
                } else {
                    if preloaded {
                        match &location {
                            PolicyLocation::Worktree { path, library } => code_rule_dsl::logic::rr::retain_read_program(path, library, &files, &program),
                            PolicyLocation::Git { snapshot, inputs: Some(inputs) } => inputs.retain(snapshot.root(), &files, &program),
                            PolicyLocation::Git { inputs: None, .. } => {}
                        }
                    }
                    match tools::run_program_checked(program.clone(), &files, root, &out, &options) {
                        Ok(report) => Some(Ok(report)),
                        Err(tools::ProgramRunError::Cache(error)) => {
                            if let Some(owner) = &owner { owner.warn(error.to_string()); }
                            ephemeral = true;
                            None
                        }
                        Err(error) => Some(Err(error)),
                    }
                };
                let report = match report {
                    Some(report) => report,
                    None => {
                        let mut options = options.clone();
                        options["reuse"] = serde_json::Value::Bool(false);
                        code_rule_dsl::logic::plan::corpus::with_tape_cache_path(Path::new(":memory:"), None, ||
                            tools::run_program_checked(program, &files, root, &out, &options)
                        )
                    }
                };
                reports.push(report.unwrap_or_else(|error| serde_json::json!({
                    "error": error.to_string(), "ruleId": ids.first().filter(|_| ids.len() == 1),
                })));
            }
            (reports, corpus::git_source_reads_complete())
        });
        if profiled.is_some() && let Some(dir) = profile::directory() {
            let header = format!("Profile of a policy scan: {} policies, each run and the programs it ran first\n", self.paths.len());
            profile::report::write(&dir, &header).map_err(|e| Error::from_reason(format!("profile: {e}")))?;
        }
        drop(profiled);
        if let Some(mut run) = run {
            let succeeded = source_complete && reports.iter().all(|report| report.get("error").is_none());
            warning = match run.finish(succeeded) {
                Ok(()) => run.publication_warning(),
                Err(error) => Some(error.to_string()),
            };
        }
        if let Some(warning) = warning {
            for report in &mut reports { report["cacheWarnings"] = serde_json::json!([warning]); }
        }
        serde_json::to_string(&reports).map_err(|e| Error::from_reason(e.to_string()))
    }
}

#[napi]
pub fn prepare_analysis_store_async(store_path: String) -> napi::bindgen_prelude::AsyncTask<PrepareAnalysisStore> {
    napi::bindgen_prelude::AsyncTask::new(PrepareAnalysisStore(store_path))
}

pub struct PrepareAnalysisStore(String);

impl napi::Task for PrepareAnalysisStore {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<()> {
        content_store::ContentStore::open(Path::new(&self.0))
            .map(drop)
            .map_err(|error| Error::from_reason(error.to_string()))
    }

    fn resolve(&mut self, _env: napi::Env, _: ()) -> Result<()> { Ok(()) }
}

#[napi]
pub fn prepare_directory_analysis_store_async(store: String, directory: String, home: String) -> napi::bindgen_prelude::AsyncTask<PrepareDirectoryAnalysisStore> {
    napi::bindgen_prelude::AsyncTask::new(PrepareDirectoryAnalysisStore { store, directory, home, recorded: false })
}

pub struct PrepareDirectoryAnalysisStore { store: String, directory: String, home: String, recorded: bool }

#[napi(object)]
pub struct AnalysisStoreRegistration {
    pub identity: String,
    pub cutoff: Option<f64>,
}

impl PrepareDirectoryAnalysisStore {
    fn registration(&self) -> Result<AnalysisStoreRegistration> {
        let store = content_store::ContentStore::open(Path::new(&self.store)).map_err(|error| Error::from_reason(error.to_string()))?;
        let cutoff = if self.recorded { store.recorded_directory_release_cutoff(&self.directory, &self.home) }
            else { store.directory_release_cutoff(Path::new(&self.directory), &self.home) }
            .map_err(|error| Error::from_reason(error.to_string()))?;
        Ok(AnalysisStoreRegistration { identity: store.identity().to_string(), cutoff: cutoff.map(|n| n as f64) })
    }
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn prepare_analysis_store_registration_async(store: String, directory: String, home: String, recorded: Option<bool>) -> napi::bindgen_prelude::AsyncTask<PrepareRegistration> {
    napi::bindgen_prelude::AsyncTask::new(PrepareRegistration(PrepareDirectoryAnalysisStore { store, directory, home, recorded: recorded == Some(true) }))
}

#[cfg(not(feature = "headless"))]
pub struct PrepareRegistration(PrepareDirectoryAnalysisStore);

#[cfg(not(feature = "headless"))]
impl napi::Task for PrepareRegistration {
    type Output = AnalysisStoreRegistration;
    type JsValue = AnalysisStoreRegistration;
    fn compute(&mut self) -> Result<Self::Output> { self.0.registration() }
    fn resolve(&mut self, _env: napi::Env, registration: Self::Output) -> Result<Self::JsValue> { Ok(registration) }
}

impl napi::Task for PrepareDirectoryAnalysisStore {
    type Output = Option<f64>;
    type JsValue = Option<f64>;

    fn compute(&mut self) -> Result<Self::Output> {
        Ok(self.registration()?.cutoff)
    }

    fn resolve(&mut self, _env: napi::Env, cutoff: Self::Output) -> Result<Self::JsValue> {
        Ok(cutoff)
    }
}

#[napi]
pub fn collect_analysis_cache_async(store_path: String, identities: Option<Vec<String>>) -> napi::bindgen_prelude::AsyncTask<CollectAnalysisCache> {
    napi::bindgen_prelude::AsyncTask::new(CollectAnalysisCache(store_path, identities.unwrap_or_default()))
}

pub struct CollectAnalysisCache(String, Vec<String>);

impl napi::Task for CollectAnalysisCache {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<()> {
        let path = Path::new(&self.0);
        if self.1.is_empty() && !path.try_exists().map_err(|error| Error::from_reason(error.to_string()))? { return Ok(()); }
        let expected = self.1.iter().map(|id| id.parse::<i64>()).collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::new(Status::InvalidArg, "invalid analysis store identity"))?;
        let collect = || -> content_store::StoreResult<()> {
            let store = content_store::ContentStore::open_for_collection(path, &expected)?;
            let policy = content_store::CorpusPolicy::default();
            let now = content_store::unix_seconds()?;
            if store.corpus_pass_wanted(&policy, now)? {
                let started = std::time::Instant::now();
                for _ in 0..8 {
                    if !store.collect_corpus_pass(&policy, now)?.did_work() || started.elapsed().as_millis() >= 200 { break; }
                }
            }
            Ok(())
        };
        collect().map_err(|error| Error::from_reason(error.to_string()))
    }

    fn resolve(&mut self, _env: napi::Env, _: ()) -> Result<()> { Ok(()) }
}

#[napi]
pub fn existing_directory_analysis_ownership(store_path: String, directory: String, home: String) -> Result<bool> {
    content_store::analysis_scope::existing_directory_ownership(Path::new(&store_path), Path::new(&directory), &home)
        .map_err(|error| Error::from_reason(error.to_string()))
}

#[napi]
pub fn owning_directory_analysis_store(store_path: String, directory: String, home: String, recorded: Option<bool>) -> Result<Option<String>> {
    use content_store::analysis_scope::{owning_directory_store, owning_recorded_directory_store};
    let lookup = if recorded == Some(true) { owning_recorded_directory_store } else { owning_directory_store };
    lookup(Path::new(&store_path), Path::new(&directory), &home)
        .map(|owner| owner.map(|identity| identity.map_or_else(String::new, |id| id.to_string())))
        .map_err(|error| Error::from_reason(error.to_string()))
}

#[cfg(any(test, feature = "test-hooks"))]
#[napi]
pub fn retire_upkeep_under(directory: String) {
    let path = Path::new(&directory);
    let root = path.canonical().unwrap_or_else(|_| path.to_owned());
    rerail_accel::upkeep::retire_files_under(&root);
}

#[napi]
pub fn delete_analysis_scope(store_path: String, value: String, directory: bool, release: Option<String>, expected_store: Option<String>, recorded: Option<bool>) -> Result<()> {
    use content_store::analysis_scope;
    let path = std::path::Path::new(&store_path);
    let receipt = directory.then(|| analysis_scope::DirectoryRelease::parse(release.as_deref())).transpose()
        .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
    if !path.try_exists().map_err(|error| Error::from_reason(error.to_string()))?
        && !(directory && release.is_some()) {
        return Ok(());
    }
    if let Some(receipt) = receipt {
        let expected = expected_store.as_deref().map(str::parse::<i64>).transpose().map_err(|_| Error::new(Status::InvalidArg, "invalid analysis store identity"))?;
        if recorded == Some(true) { receipt.deliver_recorded(path, Path::new(&value), expected) }
        else { receipt.deliver(path, Path::new(&value), expected) }
    } else {
        let scope = analysis_scope::named(&value).ok_or_else(|| Error::new(Status::InvalidArg, "analysis scope name must not be empty"))?;
        let store = content_store::ContentStore::open(path).map_err(|err| Error::new(Status::GenericFailure, err.to_string()))?;
        store.delete_analysis_scope(&scope)
    }.map_err(|err| Error::new(Status::GenericFailure, err.to_string()))?;
    Ok(())
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct NamedScopeRelease { pub identity: String, pub had_members: bool }

#[cfg(not(feature = "headless"))]
#[napi]
pub fn named_analysis_acquisition_state(store: String, name: String, identity: String, acquisition: String, scan: String) -> Result<String> {
    let number = |value: &str| value.parse::<i64>().map_err(|_| Error::new(Status::InvalidArg, "invalid analysis acquisition identity"));
    content_store::analysis_scope::named_acquisition_state(Path::new(&store), &name, number(&identity)?, number(&acquisition)?, &scan)
        .map(str::to_owned).map_err(|e| Error::from_reason(e.to_string()))
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn legacy_named_analysis_route_state(store: String, name: String, identity: String) -> Result<String> {
    let identity = identity.parse().map_err(|_| Error::new(Status::InvalidArg, "invalid analysis store identity"))?;
    content_store::analysis_scope::legacy_named_route_state(Path::new(&store), &name, identity)
        .map(str::to_owned).map_err(|e| Error::from_reason(e.to_string()))
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn release_named_analysis_scope(store: String, name: String, identities: Vec<String>) -> Result<Option<NamedScopeRelease>> {
    let file = Path::new(&store);
    if identities.is_empty() && !file.try_exists().map_err(|e| Error::from_reason(e.to_string()))? { return Ok(None); }
    let expected = identities.iter().map(|id| id.parse::<i64>()).collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid analysis store identity"))?;
    let (identity, had_members) = content_store::analysis_scope::release_named_scope(file, &name, &expected)
        .map_err(|e| Error::from_reason(format!("could not release scope {name} in {store}: {e}")))?;
    Ok(Some(NamedScopeRelease { identity: identity.to_string(), had_members }))
}

/// Inspect real POSIX session identity; allocator-backed ps tokens are not ownership.
#[napi]
pub fn process_session_id(pid: i32) -> Result<Option<i32>> {
    if pid <= 0 {
        return Err(Error::new(Status::InvalidArg, "pid must be positive"));
    }
    #[cfg(unix)]
    {
        // SAFETY: getsid accepts an integer PID and does not access caller memory.
        let session = unsafe { libc::getsid(pid) };
        if session >= 0 {
            return Ok(Some(session));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        Err(Error::new(Status::GenericFailure, error.to_string()))
    }
    #[cfg(not(unix))]
    {
        Err(Error::new(
            Status::GenericFailure,
            "POSIX process sessions are unavailable on this platform",
        ))
    }
}

/// Process-global native allocator counters for benchmark attribution. SQLite keeps no memory
/// statistics (`SQLITE_DEFAULT_MEMSTATUS=0`, `.cargo/config.toml`), so it reports none, and
/// neither does the Windows system allocator: its counters read zero.
#[napi]
pub fn native_memory_profile() -> String {
    #[cfg(not(any(windows, target_family = "wasm")))]
    let allocator = {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        serde_json::json!({
            "allocatedBytes": tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0),
            "activeBytes": tikv_jemalloc_ctl::stats::active::read().unwrap_or(0),
            "residentBytes": tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0),
            "mappedBytes": tikv_jemalloc_ctl::stats::mapped::read().unwrap_or(0),
            "retainedBytes": tikv_jemalloc_ctl::stats::retained::read().unwrap_or(0),
        })
    };
    #[cfg(any(windows, target_family = "wasm"))]
    let allocator = serde_json::json!({
        "allocatedBytes": 0,
        "activeBytes": 0,
        "residentBytes": 0,
        "mappedBytes": 0,
        "retainedBytes": 0,
    });
    serde_json::json!({
        "allocator": allocator,
        "workerPool": {
            "threads": threads::max(),
        },
    })
    .to_string()
}

/// SQLite's allocator set when the library loads, before anything can initialize SQLite
/// (`content_store::sqlite_on_jemalloc`).
#[cfg(all(
    any(target_os = "macos", target_os = "linux"),
    not(feature = "system-allocator")
))]
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static SQLITE_ALLOCATOR: extern "C" fn() = {
    extern "C" fn install() {
        content_store::sqlite_on_jemalloc();
    }
    install
};

#[napi]
pub struct ConversationEngine {
    encoder: ModelEncoder,
    rules: RuleSet,
}

#[napi]
impl ConversationEngine {
    /// The engine of the model at `model_path`. Its caller is the Node environment: its engines share one copy of each
    /// model, held by the environment's object (`api::environment`, decision 0442).
    #[napi(factory, catch_unwind)]
    pub fn load(env: napi::Env, model_path: String, max_input_tokens: u32) -> Result<Self> {
        if max_input_tokens == 0 {
            return Err(Error::new(
                Status::InvalidArg,
                "max_input_tokens must be positive",
            ));
        }
        let environment = rerail_workspace::code_rule_dsl::workspace::api::environment(&env)?;
        let models = environment.shared.get::<model::Models>();
        let model = shared_model(&models, Path::new(&model_path))?;
        let dimensions = model.fingerprint.dimensions as usize;
        Ok(Self {
            encoder: ModelEncoder {
                model,
                max_input_tokens: max_input_tokens as usize,
            },
            rules: RuleSet::empty(dimensions),
        })
    }

    #[napi]
    pub fn fingerprint(&self) -> ModelFingerprint {
        self.encoder.model.fingerprint.clone()
    }
    #[napi(catch_unwind)]
    pub fn embed_texts(&self, texts: Vec<String>) -> Result<Vec<Float32Array>> {
        if texts.iter().any(|text| text.trim().is_empty()) {
            return Err(Error::new(
                Status::InvalidArg,
                "embedding text must not be empty",
            ));
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let vectors = self.encoder.model.model.encode_with_args(
            &texts,
            Some(self.encoder.max_input_tokens),
            texts.len(),
        );
        if vectors.len() != texts.len() {
            return Err(Error::new(
                Status::GenericFailure,
                "embedding count did not match text count",
            ));
        }
        let dimensions = self.encoder.model.fingerprint.dimensions as usize;
        vectors
            .into_iter()
            .map(|vector| {
                if vector.len() != dimensions {
                    return Err(Error::new(
                        Status::GenericFailure,
                        format!(
                            "embedding dimension {} did not match model dimension {dimensions}",
                            vector.len()
                        ),
                    ));
                }
                Ok(Float32Array::from(vector))
            })
            .collect()
    }
    #[napi(catch_unwind)]
    pub fn token_counts(&self, texts: Vec<String>) -> Result<Vec<u32>> {
        texts
            .iter()
            .map(|text| {
                let encoding = self
                    .encoder
                    .model
                    .tokenizer
                    .encode(text.as_str(), false)
                    .map_err(|error| {
                        Error::new(
                            Status::InvalidArg,
                            format!("failed to tokenize text: {error}"),
                        )
                    })?;
                u32::try_from(encoding.len())
                    .map_err(|_| Error::new(Status::GenericFailure, "token count exceeded u32"))
            })
            .collect()
    }

    #[napi(catch_unwind)]
    pub fn fit_texts(
        &self,
        texts: Vec<String>,
        maximum_tokens: u32,
        overflow: String,
    ) -> Result<Vec<String>> {
        if maximum_tokens == 0 {
            return Err(Error::new(
                Status::InvalidArg,
                "maximum_tokens must be positive",
            ));
        }
        if !matches!(
            overflow.as_str(),
            "truncate-start" | "truncate-end" | "reject"
        ) {
            return Err(Error::new(
                Status::InvalidArg,
                format!("unsupported token overflow mode: {overflow}"),
            ));
        }
        let maximum_tokens = maximum_tokens as usize;
        texts
            .into_iter()
            .map(|text| {
                if text.trim().is_empty() {
                    return Err(Error::new(
                        Status::InvalidArg,
                        "embedding text must not be empty",
                    ));
                }
                let encoding = self
                    .encoder
                    .model
                    .tokenizer
                    .encode(text.as_str(), false)
                    .map_err(|error| {
                        Error::new(
                            Status::InvalidArg,
                            format!("failed to tokenize text: {error}"),
                        )
                    })?;
                if encoding.len() <= maximum_tokens {
                    return Ok(text);
                }
                if overflow == "reject" {
                    return Err(Error::new(
                        Status::InvalidArg,
                        format!(
                            "projection text has {} tokens and exceeds limit {maximum_tokens}",
                            encoding.len()
                        ),
                    ));
                }
                let offsets = encoding.get_offsets();
                let (start, end) = if overflow == "truncate-start" {
                    (offsets[offsets.len() - maximum_tokens].0, text.len())
                } else {
                    (0, offsets[maximum_tokens - 1].1)
                };
                text.get(start..end).map(str::to_owned).ok_or_else(|| {
                    Error::new(
                        Status::GenericFailure,
                        "tokenizer returned an invalid UTF-8 text offset",
                    )
                })
            })
            .collect()
    }

    #[napi(catch_unwind)]
    pub fn replace_rules(&mut self, rules: Vec<NativeRuleInput>) -> Result<CompileSummary> {
        self.rules.replace(&self.encoder, rules)
    }

    #[napi(catch_unwind)]
    pub fn upsert_rules(&mut self, rules: Vec<NativeRuleInput>) -> Result<CompileSummary> {
        self.rules.upsert(&self.encoder, rules)
    }

    #[napi(catch_unwind)]
    pub fn remove_rules(&mut self, ids: Vec<String>) -> u32 {
        self.rules.remove(ids)
    }

    #[napi(catch_unwind)]
    pub fn match_text(&self, sample: NativeSample) -> Result<Vec<NativeMatch>> {
        self.rules.match_sample(&self.encoder, sample)
    }
}

pub(crate) use code_rule_dsl::prof::lower_hex;

#[cfg(test)]
mod executor_tests {
    /// The run's executors are installed as the library loads (`install_store_functions`), and the facts' lookup of a
    /// binding's executor is the run's (2026-11-15). Its fallback, an executor of every lane, was made on every
    /// lookup (`map_or`'s default is evaluated first): every CPU run compiled the GPU's kernels, prewarmed its sorts
    /// and built the Neural Engine's lane.
    #[test]
    fn the_facts_executor_of_the_cpu_binding_is_the_runs_and_asks_for_no_device_lane() {
        let before = rerail_accel::lanes::device_lanes_asked();
        let ex = rerail_facts::code_rule_dsl::facts::core::executor_for(false);
        assert_eq!(rerail_accel::lanes::device_lanes_asked(), before, "the lookup asked for a device lane");
        assert!(std::sync::Arc::ptr_eq(&ex, &crate::code_rule_dsl::logic::lanes::executor_for(false)), "not the run's executor");
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use tikv_jemalloc_ctl::{epoch, raw};
    use tikv_jemalloc_sys::{MALLOCX_ARENA, MALLOCX_TCACHE_NONE, dallocx, mallocx};

    #[cfg(not(any(target_family = "wasm", feature = "system-allocator")))]
    #[test]
    fn rust_allocations_reach_the_profile_allocator() {
        let allocated = || unsafe { raw::read::<u64>(b"thread.allocated\0") }.unwrap();
        let before = allocated();
        let bytes = std::hint::black_box(vec![0u8; 1 << 20]);
        let after = allocated();
        assert!(
            after - before >= bytes.len() as u64,
            "Rust allocations must use production jemalloc and reach its profile counters: {} bytes recorded for {} allocated",
            after - before,
            bytes.len()
        );
    }

    #[test]
    fn purge_returns_freed_pages() {
        // An arena of its own, so no other thread's frees move its dirty page count.
        let arena = unsafe { raw::read::<u32>(b"arenas.create\0") }.unwrap();
        let dirty = || {
            epoch::advance().unwrap();
            let key = format!("stats.arenas.{arena}.pdirty\0");
            unsafe { raw::read::<usize>(key.as_bytes()) }.unwrap()
        };
        // Its pages kept dirty until purged: no decay by time (the default purges after 10 s, which a loaded run's
        // pause can reach).
        unsafe { raw::write::<isize>(format!("arena.{arena}.dirty_decay_ms\0").as_bytes(), -1) }
            .unwrap();
        let flags = MALLOCX_ARENA(arena as usize) | MALLOCX_TCACHE_NONE;
        let size = 4 << 20;
        // Other tests' runs purge every arena when they end (`purge_allocator_pages`): one of them can purge this
        // arena between the free and its count. A few tries: one free seen dirty, then purged, is what is asked.
        let seen = (0..20).any(|_| {
            unsafe {
                let block = mallocx(size, flags).cast::<u8>();
                assert!(!block.is_null());
                std::ptr::write_bytes(block, 1, size);
                dallocx(block.cast(), flags);
            }
            if dirty() == 0 {
                return false;
            }
            super::purge_allocator_pages();
            dirty() == 0
        });
        assert!(
            seen,
            "a freed block's pages are dirty until purged, and none after"
        );
    }
}
