//! Process-local compiled regex programs.
//!
//! A regex program depends only on its source. Policy utilities previously rebuilt the same
//! regex-automata program for every query, dialect, and utility-table pass.

use parking_lot::Mutex;
use regex::Regex;
use rustc_hash::FxHashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static PROGRAMS: LazyLock<Mutex<FxHashMap<String, Regex>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
const MAX_PROGRAMS: usize = 4_096;
static HITS: AtomicUsize = AtomicUsize::new(0);
static MISSES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn compile(source: &str) -> Result<Regex, regex::Error> {
    if let Some(compiled) = PROGRAMS.lock().get(source).cloned() {
        HITS.fetch_add(1, Ordering::Relaxed);
        return Ok(compiled);
    }
    let compiled = Regex::new(source)?;
    let mut programs = PROGRAMS.lock();
    if let Some(existing) = programs.get(source) {
        HITS.fetch_add(1, Ordering::Relaxed);
        return Ok(existing.clone());
    }
    if programs.len() == MAX_PROGRAMS {
        programs.clear();
    }
    programs.insert(source.to_owned(), compiled.clone());
    MISSES.fetch_add(1, Ordering::Relaxed);
    Ok(compiled)
}

pub(crate) fn stats() -> (usize, usize) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}
