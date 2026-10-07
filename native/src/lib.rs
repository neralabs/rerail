/// `build.rs` compiles this file too; the library builds it only for tests, so `cargo test`
/// checks the identities the build script computes.
#[cfg(test)]
mod build_identity;
pub mod content_store;
pub mod git_workspace;
mod main_instance;
mod model;
mod policy_dsl;
pub mod process;
mod rules;

use model::{ModelFingerprint, load_model};
use napi::{Error, Result, Status, bindgen_prelude::Float32Array};
use napi_derive::napi;
use rules::{CompileSummary, ModelEncoder, NativeMatch, NativeRuleInput, NativeSample, RuleSet};
use std::path::Path;
use std::sync::LazyLock;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

union JemallocConf {
    byte: &'static u8,
    character: &'static std::ffi::c_char,
}

#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static jemalloc_conf: Option<&'static std::ffi::c_char> = Some(unsafe {
    JemallocConf {
        byte: &b"dirty_decay_ms:0,muzzy_decay_ms:0\0"[0],
    }
    .character
});

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
    pub kernel_version: String,
}

#[napi]
pub fn health() -> Health {
    Health {
        ok: true,
        kernel_version: "1".to_owned(),
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

/// Process-global native allocator and SQLite memory counters for benchmark attribution.
#[napi]
pub fn native_memory_profile() -> String {
    let mut sqlite_current = 0_i64;
    let mut sqlite_highwater = 0_i64;
    // SAFETY: SQLite owns the global accounting slots and writes the supplied valid pointers.
    let sqlite_status = unsafe {
        rusqlite::ffi::sqlite3_status64(
            rusqlite::ffi::SQLITE_STATUS_MEMORY_USED,
            &mut sqlite_current,
            &mut sqlite_highwater,
            0,
        )
    };
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
    serde_json::json!({
        "allocator": allocator,
        "sqlite": {
            "memoryUsedBytes": sqlite_current,
            "memoryHighwaterBytes": sqlite_highwater,
            "status": sqlite_status,
        },
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
    // The static configuration uses zero decay where background threads are unavailable.
    let _ = tikv_jemalloc_ctl::background_thread::write(true);
    // SAFETY: arena.4096 is jemalloc's all-arenas selector; purge takes no value.
    let _ = unsafe { tikv_jemalloc_ctl::raw::write(b"arena.4096.purge\0", ()) };
}

#[napi]
pub struct CosineKernel {
    encoder: ModelEncoder,
    fingerprint: ModelFingerprint,
    rules: RuleSet,
}

#[napi]
impl CosineKernel {
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
