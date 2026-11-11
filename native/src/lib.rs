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
/// so `cargo test` checks how the build validates the language registry.
#[cfg(test)]
mod build_languages;
// Each test's time limit, enforced inside the test binary (`RERAIL_TEST_TIMEOUT`).
#[cfg(test)]
mod test_watchdog;
mod code_rule_dsl;
/// The engine's tools without Node (`rerail-engine dump` / `asm`).
pub use code_rule_dsl::tools;
#[cfg(test)]
pub(crate) use code_rule_dsl::prof::purge_allocator_pages;
pub use rerail_store::content_store;
pub use rerail_w::git_ignore;
pub use rerail_store::git_workspace;
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

// jemalloc does not build for MSVC targets; Windows keeps the system allocator. A test build links the engine
// again as its own dev-dependency (`test-hooks`), whose copy declares the allocator.
#[cfg(not(any(test, windows, target_family = "wasm", feature = "system-allocator")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(not(any(windows, target_family = "wasm")))]
union JemallocConf {
    byte: &'static u8,
    character: &'static std::ffi::c_char,
}

// Freed pages stay for a second so an apply reuses them instead of mapping new ones (immediate
// decay cost a tenth of a storage-heavy benchmark's CPU in mmap/munmap); every apply purges them
// when it ends and again once its response is freed (`purge_allocator_pages`), so nothing dirty
// is kept at rest.
#[cfg(not(any(windows, target_family = "wasm")))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static jemalloc_conf: Option<&'static std::ffi::c_char> = Some(unsafe {
    JemallocConf {
        byte: &b"dirty_decay_ms:1000,muzzy_decay_ms:0\0"[0],
    }
    .character
});

/// The engine's SQL functions, for every connection the store opens (its crate, `rerail-store`): the text tests'
/// and the prefilter's tapes. Set as the library loads, before any store is opened.
fn install_store_functions() {
    code_rule_dsl::build_ids::set(env!("RERAIL_ENGINE_BUILD_ID"), env!("RERAIL_GRAMMAR_BUILD_ID"));
    code_rule_dsl::install_devices();
    code_rule_dsl::install_languages();
    content_store::set_language_tags(code_rule_dsl::stored_tape_tag, code_rule_dsl::path_tape_tag);
    content_store::set_connection_functions(|db| {
        code_rule_dsl::text::install(db)?;
        code_rule_dsl::prefilter::tapes::install(db)
    });
    // And where its SQL hazards go: the engine's profile (decision 0104).
    content_store::hazards::set(content_store::hazards::Hazards {
        on: code_rule_dsl::sql_hazards::on,
        sql_in_wave: code_rule_dsl::sql_hazards::sql_in_wave,
        wait_in_wave: code_rule_dsl::sql_hazards::wait_in_wave,
        behind_the_run: code_rule_dsl::sql_hazards::behind_the_run,
        statement: code_rule_dsl::sql_hazards::strict_statement,
    });
    code_rule_dsl::sql_hazards::strict_on_every_connection();
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
        allocated(unsafe { tikv_jemalloc_sys::realloc(pointer, size) }, size)
    }
    unsafe extern "C" fn free(pointer: *mut c_void) {
        // SAFETY: Tree-sitter passes null or a pointer these functions returned.
        unsafe { tikv_jemalloc_sys::free(pointer) }
    }
    fn allocated(pointer: *mut c_void, size: usize) -> *mut c_void {
        if pointer.is_null() && size > 0 {
            std::process::abort();
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

#[cfg(all(test, not(windows)))]
mod tests {
    use tikv_jemalloc_ctl::{epoch, raw};
    use tikv_jemalloc_sys::{MALLOCX_ARENA, MALLOCX_TCACHE_NONE, dallocx, mallocx};

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
