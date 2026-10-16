/// `build.rs` compiles this file too; the library builds it only for tests, so `cargo test`
/// checks the identities the build script computes.
#[cfg(test)]
mod build_identity;
/// `build.rs` generates the language tables with this file; the library builds it only for tests,
/// so `cargo test` checks how the build validates the language registry.
#[cfg(test)]
mod build_languages;
mod code_rule_dsl;
pub mod content_store;
pub mod git_workspace;
mod main_instance;
mod model;
pub mod process;
mod rules;

use model::{ModelFingerprint, load_model};
use napi::{Error, Result, Status, bindgen_prelude::Float32Array};
use napi_derive::napi;
use rules::{CompileSummary, ModelEncoder, NativeMatch, NativeRuleInput, NativeSample, RuleSet};
use std::path::Path;
use std::sync::LazyLock;

// jemalloc does not build for MSVC targets; Windows keeps the system allocator.
#[cfg(not(windows))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(not(windows))]
union JemallocConf {
    byte: &'static u8,
    character: &'static std::ffi::c_char,
}

// Freed pages stay for a second so an apply reuses them instead of mapping new ones (immediate
// decay cost a tenth of the data-layer benchmark's CPU in mmap/munmap); every apply purges them
// when it ends and again once its response is freed (`purge_allocator_pages`), so nothing dirty
// is kept at rest.
#[cfg(not(windows))]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static jemalloc_conf: Option<&'static std::ffi::c_char> = Some(unsafe {
    JemallocConf {
        byte: &b"dirty_decay_ms:1000,muzzy_decay_ms:0\0"[0],
    }
    .character
});

/// Tree-sitter allocates through jemalloc, as Rust does: a parse allocates many small subtrees,
/// which the system allocator served at 10% of the parse. Installed when the addon loads, before
/// any Tree-sitter call; like Tree-sitter's own allocator, these abort when allocation fails.
#[cfg(not(windows))]
#[napi_derive::module_init]
fn install_tree_sitter_allocator() {
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
    unsafe {
        tree_sitter::set_allocator(Some(tree_sitter::Allocator {
            malloc,
            calloc,
            realloc,
            free,
        }));
    }
}

pub(crate) const WORKER_THREADS: usize = 4;
pub(crate) static WORK_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(WORKER_THREADS)
        .thread_name(|index| format!("rerail-worker-{index}"))
        .build()
        .expect("build Rerail worker pool")
});

#[napi(object)]
pub struct Health {
    pub ok: bool,
    #[napi(js_name = "kernelVersion")]
    pub engine_version: String,
}

/// The benchmark-only `unbounded-work` build says so, so a benchmark can prove what it ran.
#[cfg(not(feature = "unbounded-work"))]
const ENGINE_VERSION: &str = "1";
#[cfg(feature = "unbounded-work")]
const ENGINE_VERSION: &str = "1+unbounded-work";

#[napi]
pub fn health() -> Health {
    Health {
        ok: true,
        engine_version: ENGINE_VERSION.to_owned(),
    }
}

/// Check a pattern against the regex dialect and size limit structural rule conditions compile with.
#[napi]
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
    #[cfg(not(windows))]
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
    #[cfg(windows)]
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
            "threads": WORKER_THREADS,
        },
    })
    .to_string()
}

/// Tell SQLite and jemalloc to discard free pages after a workspace eviction.
pub(crate) fn release_allocator_memory() {
    // SAFETY: SQLite owns its global allocator; the maximum request asks every connection to
    // release reclaimable page-cache memory.
    unsafe { rusqlite::ffi::sqlite3_release_memory(i32::MAX) };
    // Background threads, where available, decay pages between applies too.
    #[cfg(not(windows))]
    let _ = tikv_jemalloc_ctl::background_thread::write(true);
    purge_allocator_pages();
}

/// Return every arena's freed pages to the system.
pub(crate) fn purge_allocator_pages() {
    // SAFETY: arena.4096 is jemalloc's all-arenas selector; purge reads and writes no value, so
    // jemalloc requires null value pointers (`raw::write` passes a non-null one and is refused).
    #[cfg(not(windows))]
    unsafe {
        tikv_jemalloc_sys::mallctl(
            c"arena.4096.purge".as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        );
    }
}

#[napi]
pub struct ConversationEngine {
    encoder: ModelEncoder,
    fingerprint: ModelFingerprint,
    rules: RuleSet,
}

#[napi]
impl ConversationEngine {
    #[napi(factory)]
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
    #[napi]
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
    #[napi]
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

    #[napi]
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

    #[napi]
    pub fn replace_rules(&mut self, rules: Vec<NativeRuleInput>) -> Result<CompileSummary> {
        self.rules.replace(&self.encoder, rules)
    }

    #[napi]
    pub fn upsert_rules(&mut self, rules: Vec<NativeRuleInput>) -> Result<CompileSummary> {
        self.rules.upsert(&self.encoder, rules)
    }

    #[napi]
    pub fn remove_rules(&mut self, ids: Vec<String>) -> u32 {
        self.rules.remove(ids)
    }

    #[napi]
    pub fn match_text(&self, sample: NativeSample) -> Result<Vec<NativeMatch>> {
        self.rules.match_sample(&self.encoder, sample)
    }
}

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
        let flags = MALLOCX_ARENA(arena as usize) | MALLOCX_TCACHE_NONE;
        let size = 4 << 20;
        unsafe {
            let block = mallocx(size, flags).cast::<u8>();
            assert!(!block.is_null());
            std::ptr::write_bytes(block, 1, size);
            dallocx(block.cast(), flags);
        }
        assert!(
            dirty() > 0,
            "the freed block's pages are dirty until purged"
        );
        super::purge_allocator_pages();
        assert_eq!(dirty(), 0);
    }
}
