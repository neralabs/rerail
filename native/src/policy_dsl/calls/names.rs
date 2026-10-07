//! Dotted names, interned per process: every summary and call graph of every workspace holds a
//! name as a reference to its one entry. A name holds its dotted parent (`a.b` of `a.b.c`), so the
//! graph walks prefixes without building strings. A name nothing else holds is dropped when the
//! table has doubled since it was last swept.

use parking_lot::RwLock;
use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

struct Entry {
    text: Box<str>,
    parent: Option<Name>,
}

/// An interned dotted name. Equal names are the same entry: equality and hashing are by address.
#[derive(Clone)]
pub(crate) struct Name(Arc<Entry>);

impl Name {
    /// Interns `text` and its dotted prefixes.
    pub(crate) fn new(text: &str) -> Self {
        if let Some(Key(name)) = TABLE.read().names.get(text) {
            return name.clone();
        }
        let mut table = TABLE.write();
        let name = table.insert(text);
        if table.names.len() > 2 * table.swept.max(1 << 16) {
            table.sweep();
        }
        name
    }

    /// `text`'s name, if something interned it.
    pub(crate) fn get(text: &str) -> Option<Self> {
        TABLE.read().names.get(text).map(|Key(name)| name.clone())
    }

    pub(crate) fn text(&self) -> &str {
        &self.0.text
    }

    /// The name up to its last dot.
    pub(crate) fn parent(&self) -> Option<&Self> {
        self.0.parent.as_ref()
    }

    /// The name and its dotted prefixes, longest first.
    pub(crate) fn lineage(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |name| name.parent())
    }

    /// The language whose definitions the name names: PHP's fully qualified names start with a
    /// backslash, which no Python name does.
    pub(crate) fn family(&self) -> crate::policy_dsl::language::Family {
        if self.text().starts_with('\\') {
            crate::policy_dsl::language::Family::Php
        } else {
            crate::policy_dsl::language::Family::Python
        }
    }
}

/// The table's key: a name hashed and compared by its text.
struct Key(Name);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.0.text() == other.0.text()
    }
}

impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.text().hash(state);
    }
}

impl Borrow<str> for Key {
    fn borrow(&self) -> &str {
        self.0.text()
    }
}

#[derive(Default)]
struct Table {
    names: HashSet<Key>,
    /// Names left by the last sweep.
    swept: usize,
}

impl Table {
    fn insert(&mut self, text: &str) -> Name {
        if let Some(Key(name)) = self.names.get(text) {
            return name.clone();
        }
        let parent = text.rfind('.').map(|dot| self.insert(&text[..dot]));
        let name = Name(Arc::new(Entry {
            text: text.into(),
            parent,
        }));
        self.names.insert(Key(name.clone()));
        name
    }

    /// Drops the names only the table holds; a dropped name releases its parent, which may then
    /// be dropped too.
    fn sweep(&mut self) {
        loop {
            let before = self.names.len();
            self.names
                .retain(|Key(name)| Arc::strong_count(&name.0) > 1);
            if self.names.len() == before {
                break;
            }
        }
        self.swept = self.names.len();
    }
}

static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(Default::default);

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Name {}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

/// By address: an order for maps within one process, not a lexical one.
impl Ord for Name {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Arc::as_ptr(&self.0).cmp(&Arc::as_ptr(&other.0))
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.text(), formatter)
    }
}
