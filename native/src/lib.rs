// A build of some languages, or without the addon (`headless`), leaves the others' code and the addon's API unused by
// design: dead-code warnings there say nothing (the default build still warns).
#![cfg_attr(any(not(feature = "all-languages"), feature = "headless"), allow(dead_code))]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
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
#[cfg(test)]
mod test_watchdog;
#[cfg(test)]
mod executors_guard_tests;
#[cfg(test)]
mod text_readers_guard_tests;
mod code_rule_dsl;
/// The engine's tools without Node (`rerail-engine dump` / `asm`).
pub use code_rule_dsl::tools;
#[cfg(test)]
pub(crate) use code_rule_dsl::prof::purge_allocator_pages;
pub use rerail_store::content_store;
pub use rerail_w::git_ignore;
pub use rerail_store::git_workspace;
pub use rerail_store::source_tree;
mod main_instance;
mod model;
pub use rerail_w::panic_boundary;
pub mod process;
pub use rerail_logic::threads;
/// The browser's long-lived engine: `rerail_command` and its buffers (wasm/README.md).
#[cfg(target_family = "wasm")]
pub mod reactor;
mod rules;

use model::{ModelFingerprint, load_model, verify_model};
use napi::{Error, Result, Status, bindgen_prelude::Float32Array};
use napi_derive::napi;
use rules::{CompileSummary, ModelEncoder, NativeMatch, NativeRuleInput, NativeSample, RuleSet};
use std::path::Path;

// jemalloc does not build for MSVC targets; Windows keeps the system allocator. The test build declares it too
// (`rust_allocations_reach_the_profile_allocator` checks it).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: Sampled = Sampled;

/// jemalloc, with the heap sampler around it (`rerail_prof::heap`, `RERAIL_HEAP_SAMPLE`): off, one load more an
/// allocation. Tree-sitter's allocations go to jemalloc directly and are not sampled.
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
struct Sampled;

#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
unsafe impl std::alloc::GlobalAlloc for Sampled {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { tikv_jemallocator::Jemalloc.alloc(layout) };
        if rerail_prof::heap::on() {
            rerail_prof::heap::alloc(p as usize, layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { tikv_jemallocator::Jemalloc.alloc_zeroed(layout) };
        if rerail_prof::heap::on() {
            rerail_prof::heap::alloc(p as usize, layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if rerail_prof::heap::on() {
            rerail_prof::heap::free(ptr as usize);
        }
        unsafe { tikv_jemallocator::Jemalloc.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        let q = unsafe { tikv_jemallocator::Jemalloc.realloc(ptr, layout, new_size) };
        if rerail_prof::heap::on() && !q.is_null() {
            rerail_prof::heap::free(ptr as usize);
            rerail_prof::heap::alloc(q as usize, new_size);
        }
        q
    }
}

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
        #[cfg(feature = "test-hooks")]
        let grammar = std::env::var("RERAIL_TEST_GRAMMAR_BUILD_ID").ok()
            .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map_or(grammar, |id| Box::leak(id.into_boxed_str()));
        code_rule_dsl::build_ids::set(env!("RERAIL_ENGINE_BUILD_ID"), grammar);
        code_rule_dsl::install_devices();
        code_rule_dsl::install_languages();
        code_rule_dsl::logic::plan::corpus::install_git_inventory();
        content_store::set_language_tags(code_rule_dsl::stored_tape_tag, code_rule_dsl::path_tape_tag);
        content_store::set_connection_functions(|db| {
            code_rule_dsl::text::install(db)?;
            code_rule_dsl::prefilter::tapes::install(db)
        });
        code_rule_dsl::sql_hazards::strict_on_every_connection();
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
    // Library callers and the test runner can park their main thread. Their pool needs
    // actual workers, including with RERAIL_THREADS=1. The CLI registers its own main thread
    // explicitly, before its first job (`threads::configure(true)`), headless or not: a
    // constructor that made the pool first left the CLI's main thread out of it, so every
    // parallel loop it ran queued its work and slept, and its waits ran no jobs.
    #[cfg(not(feature = "headless"))]
    if !std::env::current_exe().is_ok_and(|e| e.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rerail-engine"))) {
        threads::configure(false);
    }
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

/// The one pool, of the most threads the process uses (`RERAIL_THREADS`), the loading thread one of them when it is
/// the process's main thread: made as Node registers the addon's exports, before any of them runs, and not in the
/// library's constructor. A constructor runs inside `dlopen`, which holds glibc's loader lock; rayon's
/// `use_current_thread` waits for every pool thread to start, and each one's first thread-local with a destructor
/// registers it through `__cxa_thread_atexit_impl`, which takes that lock: the load never returned.
#[cfg(not(target_family = "wasm"))]
#[napi(module_exports)]
pub fn configure_thread_pool(_exports: napi::bindgen_prelude::Object) {
    threads::configure(threads::is_main_thread());
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

#[cfg(not(feature = "headless"))]
#[napi]
pub fn inspect_rr_policy(path: String, source: Option<&rerail_w::code_rule_dsl::workspace::api::GitRevisionSource>) -> Result<String> {
    LoadedRrPolicy::read(&path, source.map(|source| source.pinned_snapshot()).as_deref())?.inspection()
}

#[cfg(not(feature = "headless"))]
#[derive(Clone)]
struct LoadedRrPolicy {
    program: rerail_rr_syntax::code_rule_dsl::logic::front::ast::Program,
    files: Vec<code_rule_dsl::logic::rr::LibraryFile>,
}

#[cfg(not(feature = "headless"))]
impl LoadedRrPolicy {
    fn read(path: &str, snapshot: Option<&git_workspace::snapshot::Snapshot>) -> Result<Self> {
        let path = rr_policy_path(path, snapshot)?;
        let (program, files) = tools::read_policy(&path).map_err(Error::from_reason)?;
        Ok(Self { program, files })
    }

    fn inspection(&self) -> Result<String> {
        let metadata = code_rule_dsl::logic::rr::metadata(&self.program).map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(serde_json::json!({ "metadata": metadata, "programInputs": tools::policy_inputs(&self.files) }).to_string())
    }
}

/// The programs and includes inspected during one scan's policy selection.
#[cfg(not(feature = "headless"))]
#[napi]
#[derive(Default)]
pub struct LoadedRrPolicies(std::sync::Mutex<std::collections::HashMap<String, LoadedRrPolicy>>);

#[cfg(not(feature = "headless"))]
#[napi]
impl LoadedRrPolicies {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    #[napi]
    pub fn load(&self, path: String, source: Option<&rerail_w::code_rule_dsl::workspace::api::GitRevisionSource>) -> Result<String> {
        let policy = LoadedRrPolicy::read(&path, source.map(|source| source.pinned_snapshot()).as_deref())?;
        let inspection = policy.inspection()?;
        self.0.lock().unwrap_or_else(|error| error.into_inner()).insert(path, policy);
        Ok(inspection)
    }
}

#[cfg(not(feature = "headless"))]
fn rr_policy_path(path: &str, snapshot: Option<&git_workspace::snapshot::Snapshot>) -> Result<std::path::PathBuf> {
    let path = Path::new(path);
    match snapshot {
        None => Ok(path.to_owned()),
        Some(snapshot) if path.is_relative() && path.components().all(|part| matches!(part, std::path::Component::Normal(_))) => Ok(snapshot.root().join(path)),
        Some(_) => Err(Error::from_reason("Expected a repository-relative policy path")),
    }
}

#[cfg(not(feature = "headless"))]
#[napi]
pub fn scan_rr_policies_async(
    paths: Vec<String>,
    root: String,
    store: String,
    output: String,
    analysis: Option<&rerail_w::code_rule_dsl::workspace::api::AnalysisScopeRun>,
    source_paths: Option<Vec<String>>,
    source: Option<&rerail_w::code_rule_dsl::workspace::api::GitRevisionSource>,
    policy_source: Option<&rerail_w::code_rule_dsl::workspace::api::GitRevisionSource>,
    loaded: Option<&LoadedRrPolicies>,
) -> Result<napi::bindgen_prelude::AsyncTask<ScanRrPolicies>> {
    let policies = loaded.map(|loaded| {
        let programs = loaded.0.lock().unwrap_or_else(|error| error.into_inner());
        paths.iter().map(|path| programs.get(path).cloned().ok_or_else(|| format!("{path}: policy was not loaded"))).collect()
    });
    Ok(napi::bindgen_prelude::AsyncTask::new(ScanRrPolicies {
        paths, root, store, output, owner: analysis.map(|run| run.owner()), source_paths,
        snapshot: source.map(|source| source.pinned_snapshot()),
        policy_snapshot: policy_source.map(|source| source.pinned_snapshot()),
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
        let path = Path::new(&self.store);
        let root = self.snapshot.as_ref().map_or_else(|| Path::new(&self.root), |snapshot| snapshot.root());
        let ownership = self.owner.clone().map(|owner| (root.to_owned(), owner));
        code_rule_dsl::logic::plan::corpus::with_tape_cache_path(path, ownership, || {
            let options = serde_json::json!({ "sourcePaths": self.source_paths, "scanSpans": true, "reuse": path != Path::new(":memory:") });
            let policies = self.policies.take().unwrap_or_else(|| self.paths.iter().map(|path|
                LoadedRrPolicy::read(path, self.policy_snapshot.as_deref()).map_err(|error| error.to_string())
            ).collect());
            let mut counts = std::collections::HashMap::new();
            let policies: Vec<_> = policies.into_iter().map(|policy| {
                let policy = policy?;
                let metadata = code_rule_dsl::logic::rr::metadata(&policy.program).map_err(|err| err.to_string())?;
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
                let (LoadedRrPolicy { program, files }, ids) = match policy {
                    Ok(policy) => policy,
                    Err(error) => {
                        reports.push(serde_json::json!({"error": error, "ruleId": null}));
                        continue;
                    }
                };
                if let Some(id) = ids.iter().find(|id| counts[*id] > 1) {
                    reports.push(serde_json::json!({"error": format!("duplicate .rr policy ID {id:?}"), "ruleId": id}));
                    continue;
                }
                let out = if self.output.is_empty() { std::path::PathBuf::new() } else { Path::new(&self.output).join(i.to_string()) };
                let report = if ephemeral {
                    None
                } else {
                    match tools::run_program_checked(program.clone(), &files, root, &out, &options) {
                        Ok(report) => Some(Ok(report)),
                        Err(tools::ProgramRunError::Cache(error)) => {
                            if let Some(owner) = &self.owner { owner.warn(error.to_string()); }
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
            serde_json::to_string(&reports).map_err(|e| Error::from_reason(e.to_string()))
        })
    }

    fn resolve(&mut self, _env: napi::Env, output: String) -> Result<String> {
        Ok(output)
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
pub fn collect_analysis_cache_async(store_path: String) -> napi::bindgen_prelude::AsyncTask<CollectAnalysisCache> {
    napi::bindgen_prelude::AsyncTask::new(CollectAnalysisCache(store_path))
}

pub struct CollectAnalysisCache(String);

impl napi::Task for CollectAnalysisCache {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<()> {
        let path = Path::new(&self.0);
        if !path.exists() { return Ok(()); }
        let collect = || -> content_store::StoreResult<()> {
            let store = content_store::ContentStore::open(path)?;
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

#[cfg(any(test, feature = "test-hooks"))]
#[napi]
pub fn retire_upkeep_under(directory: String) {
    let path = Path::new(&directory);
    let root = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    rerail_accel::upkeep::retire_files_under(&root);
}

#[napi]
pub fn delete_analysis_scope(store_path: String, value: String, directory: bool, release: Option<String>) -> Result<()> {
    use content_store::analysis_scope;
    let path = std::path::Path::new(&store_path);
    if !path.exists() && !(directory && release.as_deref().is_some_and(|id| id.contains(':'))) {
        return Ok(());
    }
    let store = content_store::ContentStore::open(path).map_err(|err| Error::new(Status::GenericFailure, err.to_string()))?;
    if directory {
        store.delete_directory_analysis_scopes(std::path::Path::new(&value), release.as_deref())
    } else {
        let scope = analysis_scope::named(&value).ok_or_else(|| Error::new(Status::InvalidArg, "analysis scope name must not be empty"))?;
        store.delete_analysis_scope(&scope)
    }.map_err(|err| Error::new(Status::GenericFailure, err.to_string()))?;
    Ok(())
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
#[cfg(all(any(target_os = "macos", target_os = "linux"), not(feature = "system-allocator")))]
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
    fingerprint: ModelFingerprint,
    rules: RuleSet,
}

#[napi]
impl ConversationEngine {
    #[napi(factory, catch_unwind)]
    pub fn load(model_path: String, max_input_tokens: u32) -> Result<Self> {
        if max_input_tokens == 0 {
            return Err(Error::new(
                Status::InvalidArg,
                "max_input_tokens must be positive",
            ));
        }
        let loaded = load_model(Path::new(&model_path))?;
        let dimensions = loaded.fingerprint.dimensions as usize;
        Ok(Self {
            encoder: ModelEncoder {
                model: loaded.model,
                tokenizer: loaded.tokenizer,
                max_input_tokens: max_input_tokens as usize,
            },
            fingerprint: loaded.fingerprint,
            rules: RuleSet::empty(dimensions),
        })
    }

    #[napi]
    pub fn fingerprint(&self) -> ModelFingerprint {
        self.fingerprint.clone()
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
        let vectors = self.encoder.model.encode_with_args(
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
        let dimensions = self.fingerprint.dimensions as usize;
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
        assert!(std::ptr::eq(ex, crate::code_rule_dsl::logic::lanes::executor_for(false)), "not the run's executor");
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
        assert!(after - before >= bytes.len() as u64,
            "Rust allocations must use production jemalloc and reach its profile counters: {} bytes recorded for {} allocated", after - before, bytes.len());
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
        unsafe { raw::write::<isize>(format!("arena.{arena}.dirty_decay_ms\0").as_bytes(), -1) }.unwrap();
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
        assert!(seen, "a freed block's pages are dirty until purged, and none after");
    }
}
