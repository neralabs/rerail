//! What the tests read the conformance fixtures with, and the tapes and store paths they make, for every crate's
//! tests (the crate split keeps this below them all: `rerail-lang`). `RERAIL_FIXTURES_DIR` is the repository's
//! `fixtures/` (`.cargo/config.toml`), or the engine build's `RERAIL_ORACLES_DIR` (`native/build.rs`).
use std::path::PathBuf;
use std::sync::Arc;

use ast_grep_core::tree_sitter::StrDoc;

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};

/// The conformance fixtures directory.
#[macro_export]
#[doc(hidden)]
macro_rules! __rerail_fixtures_dir {
    () => {
        env!(
            "RERAIL_FIXTURES_DIR",
            "Rerail fixtures not found: check out the repository's fixtures/ directory, or set RERAIL_ORACLES_DIR"
        )
    };
}

/// A fixture's text, read as the test compiles.
#[macro_export]
#[doc(hidden)]
macro_rules! __rerail_oracle {
    ($path:literal) => {
        include_str!(concat!($crate::__rerail_fixtures_dir!(), "/", $path))
    };
}

pub use crate::__rerail_fixtures_dir as fixtures_dir;
pub use crate::__rerail_oracle as oracle;

/// `source`'s tape in `lang`, restored, and its bytes.
pub fn document(source: &str, lang: GrammarDialect) -> (TapeDoc, Vec<u8>) {
    let native = StrDoc::new(source, lang);
    let grammar = Grammar::new(lang).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
    let bytes = image.to_bytes().unwrap();
    (TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(), bytes)
}

/// A fresh store path: the clock alone is not unique across parallel tests (macOS reads it in
/// microseconds), so a counter disambiguates.
///
/// No test removes its store (many keep it open to the end, and a failing one panics first), so the
/// first call of a run sweeps the stores of test processes that have exited: their files and
/// directories named `rerail-prefilter-<pid>-…`, whose process is no longer alive.
pub fn temp() -> PathBuf {
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(sweep);
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("rerail-prefilter-{}-{nonce}-{n}.sqlite", std::process::id()))
}

/// Removes the stores `temp` made for processes that have exited.
fn sweep() {
    #[cfg(unix)]
    {
        let me = std::process::id();
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
        for e in entries.flatten() {
            let name = e.file_name();
            let Some(rest) = name.to_str().and_then(|n| n.strip_prefix("rerail-prefilter-")) else { continue };
            let Some(pid) = rest.split('-').next().and_then(|p| p.parse::<i32>().ok()) else { continue };
            // SAFETY: signal 0 sends nothing; it only asks whether the process exists.
            let alive = pid as u32 == me || unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
            if alive {
                continue;
            }
            let path = e.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// The store at `path`, opened (created if it is not there).
pub fn open(path: &std::path::Path) -> crate::content_store::ContentStore {
    crate::content_store::ContentStore::open(path).unwrap()
}
