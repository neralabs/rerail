//! One compiled regex per pattern per process. `regex::Regex` clones share the compiled program
//! (an `Arc`) and get their own match-cache pool, so every rule, input, utility and SQL `rx_*`
//! call that names the same pattern uses one compilation, whichever worker compiled it first.
//! Compilation is a pure function of the pattern text, so sharing changes no result.
use parking_lot::Mutex;
use regex::Regex;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

static GLOBAL: LazyLock<Mutex<FxHashMap<String, Regex>>> = LazyLock::new(Default::default);

thread_local! {
    /// Per-thread handles (one match-cache pool per thread, warm across files) for the per-row
    /// lookups of SQL `rx_*` functions.
    static LOCAL: RefCell<FxHashMap<String, Rc<Regex>>> = RefCell::default();
}

/// The process's compiled regex for `pattern`.
pub fn get(pattern: &str) -> Result<Regex, regex::Error> {
    let mut g = GLOBAL.lock();
    if let Some(r) = g.get(pattern) {
        return Ok(r.clone());
    }
    let r = Regex::new(pattern)?;
    g.insert(pattern.to_owned(), r.clone());
    Ok(r)
}

/// This thread's handle on the process's compiled regex for `pattern`.
pub fn local(pattern: &str) -> Result<Rc<Regex>, regex::Error> {
    if let Some(r) = LOCAL.with(|l| l.borrow().get(pattern).cloned()) {
        return Ok(r);
    }
    let r = Rc::new(get(pattern)?);
    LOCAL.with(|l| l.borrow_mut().insert(pattern.to_owned(), r.clone()));
    Ok(r)
}
