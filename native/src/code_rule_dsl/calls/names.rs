//! Dotted names, interned per process: every summary and call graph of every workspace holds a
//! name as a reference to its one entry. A name holds its dotted parent (`a.b` of `a.b.c`), so the
//! graph walks prefixes without building strings. A name nothing else holds is dropped when the
//! table has doubled since it was last swept.

use crate::code_rule_dsl::language::Family;
use foldhash::{HashMap, HashSet};
use parking_lot::RwLock;
use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

struct Entry {
    family: Family,
    text: Box<str>,
    parent: Option<Name>,
    /// The first dotted component, where it is not the name itself.
    root: Option<Name>,
}

/// An interned dotted name. Equal names are the same entry: equality and hashing are by address.
/// Each family's names are its own namespace. A new family's `CallRules` must create names with
/// its own `Family` and choose that family's spelling.
#[derive(Clone)]
pub(crate) struct Name(Arc<Entry>);

impl Name {
    /// Interns `text` in `family`'s namespace, along with its dotted prefixes.
    pub(crate) fn new(family: Family, text: &str) -> Self {
        if let Some(name) = TABLE.read().get(family, text) {
            return name;
        }
        let mut table = TABLE.write();
        let name = table.insert(family, text);
        if table.len() > 2 * table.swept.max(1 << 16) {
            table.sweep();
        }
        name
    }

    /// `text`'s name in `family`, if something interned it.
    pub(crate) fn get(family: Family, text: &str) -> Option<Self> {
        TABLE.read().get(family, text)
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

    /// The name's first dotted component: `a` of `a.b.c`, and of `a`.
    pub(crate) fn root(&self) -> &Self {
        self.0.root.as_ref().unwrap_or(self)
    }

    pub(crate) fn family(&self) -> Family {
        self.0.family
    }
}

/// The table's key: a name hashed and compared by its text within one family.
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
    names: HashMap<Family, HashSet<Key>>,
    /// Names left by the last sweep.
    swept: usize,
}

impl Table {
    fn get(&self, family: Family, text: &str) -> Option<Name> {
        self.names
            .get(&family)?
            .get(text)
            .map(|Key(name)| name.clone())
    }

    fn len(&self) -> usize {
        self.names.values().map(HashSet::len).sum()
    }

    fn insert(&mut self, family: Family, text: &str) -> Name {
        if let Some(name) = self.get(family, text) {
            return name;
        }
        let parent = text.rfind('.').map(|dot| self.insert(family, &text[..dot]));
        let root = parent.as_ref().map(|parent| parent.root().clone());
        let name = Name(Arc::new(Entry {
            family,
            text: text.into(),
            parent,
            root,
        }));
        self.names
            .entry(family)
            .or_default()
            .insert(Key(name.clone()));
        name
    }

    /// Drops the names only the table holds; a dropped name releases its parent, which may then
    /// be dropped too.
    fn sweep(&mut self) {
        loop {
            let before = self.len();
            self.names.retain(|_, names| {
                names.retain(|Key(name)| Arc::strong_count(&name.0) > 1);
                !names.is_empty()
            });
            if self.len() == before {
                break;
            }
        }
        self.swept = self.len();
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
