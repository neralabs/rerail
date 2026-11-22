//! What the observer keeps, and the trait it keeps it through (t33 K2): its answers, canonical forms and settings
//! are kept facts of the content store whose observer it is. The store is above this crate (rerail-store), so the
//! facts are its (`content_store::observe_facts`, `observe/facts.rs`), handed down through [`OPEN`]; boot registers
//! them as it installs, and the store's own tests as they load. Unregistered, an observer keeps nothing, as when its
//! store cannot be opened.

use super::{Meta, Stat};
use std::path::Path;

/// An answer as the table held it: path, kind, stat, digest, listing (encoded), when recorded.
pub type Row = (String, i64, Stat, Vec<u8>, Option<Vec<u8>>, i64);
/// A canonical form: the path, its canonical path, when recorded.
pub type Canon = (String, String, i64);

/// One relation's part of a change: rows put, paths whose rows go, paths whose rows go with every row under them.
pub struct Edit<R> {
    pub put: Vec<R>,
    pub delete: Vec<String>,
    pub delete_under: Vec<String>,
}

impl<R> Default for Edit<R> {
    fn default() -> Self {
        Edit { put: Vec::new(), delete: Vec::new(), delete_under: Vec::new() }
    }
}

impl<R> Edit<R> {
    pub fn is_empty(&self) -> bool {
        self.put.is_empty() && self.delete.is_empty() && self.delete_under.is_empty()
    }
}

/// One change of the observer's kept facts, written as one put.
#[derive(Default)]
pub struct Change {
    pub answers: Edit<Row>,
    pub canon: Edit<Canon>,
    pub meta_put: Vec<(String, rusqlite::types::Value)>,
    pub meta_delete: Vec<String>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty() && self.canon.is_empty() && self.meta_put.is_empty() && self.meta_delete.is_empty()
    }
}

/// An observer's kept facts, in the content store beside its file.
pub trait Kept: Send + Sync {
    /// Every setting kept.
    fn meta(&self) -> Meta;
    /// The answers and canonical forms kept at and under `unit`.
    fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>);
    /// Every answer and canonical form kept (`collect`).
    fn all(&self) -> (Vec<Row>, Vec<Canon>);
    /// Writes `change` as one put, a job called `what`.
    fn apply(&self, change: Change, what: &'static str);
    /// Waits until every put queued is written, reading nothing (so a thread may ask it as it ends).
    fn written(&self);
    /// Waits until every put queued is written, and closes the store (the outermost run's end).
    fn close(&self);
}

/// The kept facts of the observer whose file is `observer` (`<store>.files…`), in the content store beside it; `None`
/// when the store cannot be opened.
pub type Open = fn(&Path) -> Option<Box<dyn Kept>>;

/// The content store's kept facts for an observer (rerail-store's `observe_facts::open`).
pub static OPEN: rerail_registry::Slot<Open> = rerail_registry::Slot::new("the observer's kept facts in the content store");

/// The observer's kept facts, through the store's (`OPEN`).
pub struct Facts(Box<dyn Kept>);

impl Facts {
    /// The facts of the observer whose file is `observer`; `None` when the store cannot be opened, or no store
    /// registered its facts.
    pub fn open(observer: &Path) -> Option<Facts> {
        OPEN.try_get().and_then(|open| open(observer)).map(Facts)
    }

    pub fn meta(&self) -> Meta {
        self.0.meta()
    }

    pub fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>) {
        self.0.under(unit)
    }

    pub fn all(&self) -> (Vec<Row>, Vec<Canon>) {
        self.0.all()
    }

    pub fn apply(&self, change: Change, what: &'static str) {
        self.0.apply(change, what)
    }

    pub fn written(&self) {
        self.0.written()
    }

    pub fn close(&self) {
        self.0.close()
    }
}

/// Every answer's path kept, sorted, for a test.
#[cfg(any(test, feature = "test-hooks"))]
pub fn all_paths(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().0.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}

/// Every canonical form's path kept, sorted, for a test.
#[cfg(any(test, feature = "test-hooks"))]
pub fn all_canonical(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().1.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}
