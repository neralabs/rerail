//! Compiled regexes, one per pattern for the whole process. A rule generated from a table repeats
//! a few patterns across thousands of terms, each query compiles the shared utilities it reaches
//! again for each grammar and `languageOptions` (`compile::shared`), and the rules of the packs a
//! repository installs write many of the same patterns. One regex built per term held each
//! pattern's automata (about 14 KiB) and match cache pool (about 3.5 KiB) that many times over.
//! Every term, value test and anchor plan of every rule holds its pattern's one `Regex`, a shared
//! handle on the compiled regex: a regex is immutable, and its pool gives each thread that matches
//! with it a cache of its own.
//!
//! The table keeps a regex while a handle on it lives: the last handle to go drops it from the
//! table, so a rule's regexes go with the rule, and those of a workspace's copies of its clauses
//! with the copies, as they did when each term built its own, and a process that drops its last
//! rule holds none. Each drop costs the regexes it lets go of, not a scan of the table. A thread
//! compiling a document (`compiling`) keeps the regexes it lets go of until the compilation ends,
//! so a regex its validation compiled is the one its terms take, and the ones only validation
//! took, or a rejected document's, go then. A handle on another thread that lets go of a regex a
//! document validated before its terms take it drops it; they compile it again.

use parking_lot::RwLock;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::sync::{Arc, LazyLock, Weak};

/// Each pattern compiled and its regex, while a handle on the regex lives. Public, as the handle's field is, for
/// the tests the engine hosts (the crate split).
pub static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(Default::default);

pub type Table = HashMap<Box<str>, Arc<regex::Regex>>;

thread_local! {
    /// While this thread compiles a document, the regexes it let go of the last handle on, which
    /// the table keeps until the compilation ends.
    static LET_GO: RefCell<Option<Vec<Weak<regex::Regex>>>> = const { RefCell::new(None) };
}

/// A compiled regex, the one every holder of its pattern holds. It reads, and prints, as the
/// `regex` crate's `Regex`, which it dereferences to.
#[derive(Clone)]
pub struct Regex(pub ManuallyDrop<Arc<regex::Regex>>);

impl Regex {
    /// `pattern`'s regex, shared with every other holder of the pattern; the `regex` crate's error
    /// where it does not compile.
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        if let Some(regex) = TABLE.read().get(pattern) {
            return Ok(Self::holding(regex));
        }
        // Compiled outside the lock; where another thread added the pattern meanwhile, its regex
        // stays the one.
        let regex = Arc::new(regex::Regex::new(pattern)?);
        let mut table = TABLE.write();
        Ok(Self::holding(table.entry(pattern.into()).or_insert(regex)))
    }

    fn holding(regex: &Arc<regex::Regex>) -> Self {
        Self(ManuallyDrop::new(Arc::clone(regex)))
    }
}

impl Drop for Regex {
    /// The last handle on a regex drops it from the table, or, on a thread compiling a document,
    /// leaves it to the compilation's end.
    fn drop(&mut self) {
        // SAFETY: `drop` runs once, and nothing reads the handle after it.
        let regex = unsafe { ManuallyDrop::take(&mut self.0) };
        // Counted once this handle is gone, so of two last handles going at once on two threads,
        // one sees only the table's.
        let let_go = Arc::downgrade(&regex);
        drop(regex);
        if let_go.strong_count() == 1 && !kept_while_compiling(&let_go) {
            forget([let_go]);
        }
    }
}

impl std::ops::Deref for Regex {
    type Target = regex::Regex;

    fn deref(&self) -> &regex::Regex {
        &self.0
    }
}

impl std::fmt::Debug for Regex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, formatter)
    }
}

/// This thread's compilation of a document, until it drops: the table keeps the regexes the
/// thread lets go of meanwhile, then drops the ones no handle took again.
pub struct Compiling(());

/// Starts this thread's compilation of a document (`Compiling`).
pub fn compiling() -> Compiling {
    LET_GO.with_borrow_mut(|let_go| {
        let_go.get_or_insert_default();
    });
    Compiling(())
}

impl Drop for Compiling {
    fn drop(&mut self) {
        if let Some(let_go) = LET_GO.with_borrow_mut(Option::take) {
            forget(let_go);
        }
    }
}

/// Whether this thread compiles a document, which then keeps `regex` until it ends.
fn kept_while_compiling(regex: &Weak<regex::Regex>) -> bool {
    LET_GO
        .try_with(|let_go| {
            let mut let_go = let_go.borrow_mut();
            let_go
                .as_mut()
                .map(|kept| kept.push(Weak::clone(regex)))
                .is_some()
        })
        .unwrap_or(false)
}

/// Drops from the table each of `regexes` that no handle holds.
fn forget(regexes: impl IntoIterator<Item = Weak<regex::Regex>>) {
    let mut table = TABLE.write();
    let forgotten = regexes
        .into_iter()
        .filter_map(|regex| unheld(&mut table, &regex))
        .collect::<Vec<_>>();
    // Freed outside the lock.
    drop(table);
    drop(forgotten);
}

/// `regex`, taken out of the table, where the table holds it and no handle does.
fn unheld(table: &mut Table, regex: &Weak<regex::Regex>) -> Option<Arc<regex::Regex>> {
    let regex = regex.upgrade()?;
    // The table's and this one: no handle is left to clone it, and `new` waits for the lock.
    let unheld = Arc::strong_count(&regex) == 2
        && table
            .get(regex.as_str())
            .is_some_and(|held| Arc::ptr_eq(held, &regex));
    if unheld {
        table.remove(regex.as_str())
    } else {
        None
    }
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
