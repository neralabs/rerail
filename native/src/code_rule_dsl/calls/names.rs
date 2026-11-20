//! Dotted names, interned per process: every summary and call graph of every workspace holds a
//! name as a reference to its one entry. A name holds its dotted parent (`a.b` of `a.b.c`), so the
//! graph walks prefixes without building strings. A name nothing else holds is dropped when its
//! shard has doubled since it was last swept.
//!
//! The table is `SHARDS` shards, a name's by the hash of its family and text, each behind its own
//! lock: the parse workers summarize their files at once, and behind one lock their interning was
//! a third of the summaries' time, waiting (0484's next steps, Q343). A name's parent is interned
//! before its own shard is locked, so no thread holds two shards' locks.

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
pub struct Name(Arc<Entry>);

impl Name {
    /// Interns `text` in `family`'s namespace, along with its dotted prefixes.
    pub fn new(family: Family, text: &str) -> Self {
        let shard = shard(family, text);
        if let Some(name) = shard.read().get(family, text) {
            return name;
        }
        // The parent first, in its own shard, with no lock held here.
        let parent = text.rfind('.').map(|dot| Name::new(family, &text[..dot]));
        let mut table = shard.write();
        let name = table.insert(family, text, parent);
        if table.len() > 2 * table.swept.max(SWEEP_AT) {
            table.sweep();
        }
        name
    }

    /// Interns each of `texts` in `family`'s namespace, with their dotted prefixes, as `new` does one: a file's
    /// names at once (a summary's), each shard's lock taken once a prefix depth (a read for the names it has, a
    /// write for the rest), not once a name. Most of a file's names are its own, new to the table, so one by one
    /// each took a write lock, and the parse workers summarizing their files at once waited on each other's
    /// (0484, Q343). The names come back in `texts`' order.
    pub fn new_all<'t>(family: Family, texts: impl IntoIterator<Item = &'t str>) -> Vec<Self> {
        let texts = texts.into_iter().collect::<Vec<_>>();
        // Every text and dotted prefix, once, by depth (its dots): a prefix's name is made before the names under it.
        let mut wanted: HashMap<&str, Option<Name>> = HashMap::default();
        let mut depths: Vec<Vec<&str>> = Vec::new();
        for &text in &texts {
            let mut end = text.len();
            loop {
                let prefix = &text[..end];
                if wanted.contains_key(prefix) {
                    break;
                }
                wanted.insert(prefix, None);
                let depth = prefix.bytes().filter(|b| *b == b'.').count();
                if depths.len() <= depth {
                    depths.resize_with(depth + 1, Vec::new);
                }
                depths[depth].push(prefix);
                match prefix.rfind('.') {
                    Some(dot) => end = dot,
                    None => break,
                }
            }
        }
        for level in &depths {
            let mut by_shard: Vec<Vec<&str>> = vec![Vec::new(); SHARDS];
            for &text in level {
                by_shard[shard_index(family, text)].push(text);
            }
            // From a shard of this batch's own (its first name's), not shard 0: workers interning their files at
            // once each in shard order would meet at every shard in turn, one behind the other.
            let from = texts.first().map_or(0, |text| shard_index(family, text));
            for index in (0..SHARDS).map(|k| (from + k) % SHARDS) {
                let group = &by_shard[index];
                if group.is_empty() {
                    continue;
                }
                let shard = &TABLE[index];
                let mut missing = Vec::new();
                {
                    let table = shard.read();
                    for &text in group {
                        match table.get(family, text) {
                            Some(name) => *wanted.get_mut(text).expect("a wanted name") = Some(name),
                            None => missing.push(text),
                        }
                    }
                }
                if missing.is_empty() {
                    continue;
                }
                // The entries made before the lock is taken (their parents are interned, a depth before), so the
                // write lock is held for the table's inserts alone; one another thread made first is dropped.
                let made = missing
                    .iter()
                    .map(|&text| {
                        let parent = text.rfind('.').map(|dot| wanted[&text[..dot]].clone().expect("a prefix, made a depth before"));
                        (text, Name::entry(family, text, parent))
                    })
                    .collect::<Vec<_>>();
                let mut table = shard.write();
                for (text, name) in made {
                    let name = table.insert_made(name);
                    *wanted.get_mut(text).expect("a wanted name") = Some(name);
                }
                if table.len() > 2 * table.swept.max(SWEEP_AT) {
                    table.sweep();
                }
            }
        }
        texts.iter().map(|text| wanted[text].clone().expect("every wanted name made")).collect()
    }

    /// A new entry for `text`, not yet in the table.
    fn entry(family: Family, text: &str, parent: Option<Name>) -> Self {
        let root = parent.as_ref().map(|parent| parent.root().clone());
        Name(Arc::new(Entry { family, text: text.into(), parent, root }))
    }

    /// `text`'s name in `family`, if something interned it.
    pub fn get(family: Family, text: &str) -> Option<Self> {
        shard(family, text).read().get(family, text)
    }

    pub fn text(&self) -> &str {
        &self.0.text
    }

    /// The name up to its last dot.
    pub fn parent(&self) -> Option<&Self> {
        self.0.parent.as_ref()
    }

    /// The name and its dotted prefixes, longest first.
    pub fn lineage(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |name| name.parent())
    }

    /// The name's first dotted component: `a` of `a.b.c`, and of `a`.
    pub(crate) fn root(&self) -> &Self {
        self.0.root.as_ref().unwrap_or(self)
    }

    pub fn family(&self) -> Family {
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

    /// `text`'s entry, made with `parent` (its interned dotted prefix) where no other thread made it first.
    fn insert(&mut self, family: Family, text: &str, parent: Option<Name>) -> Name {
        if let Some(name) = self.get(family, text) {
            return name;
        }
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

    /// `name` as the table's entry for its text, unless one is there already (then that one).
    fn insert_made(&mut self, name: Name) -> Name {
        if let Some(held) = self.get(name.family(), name.text()) {
            return held;
        }
        self.names.entry(name.family()).or_default().insert(Key(name.clone()));
        name
    }

    /// Drops the names only the shard holds; a dropped name releases its parent, which may then
    /// be dropped too (by its own shard's next sweep, where it is another's).
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

/// The table's shards: a power of two, so a hash's low bits pick one.
const SHARDS: usize = 256;
/// A shard is swept once it holds twice this many names, or twice what its last sweep left.
const SWEEP_AT: usize = (1 << 16) / SHARDS;

static TABLE: LazyLock<[RwLock<Table>; SHARDS]> = LazyLock::new(|| std::array::from_fn(|_| RwLock::default()));

/// The shard of `text` in `family`.
fn shard(family: Family, text: &str) -> &'static RwLock<Table> {
    &TABLE[shard_index(family, text)]
}

fn shard_index(family: Family, text: &str) -> usize {
    use std::hash::BuildHasher;
    foldhash::fast::FixedState::with_seed(0).hash_one((family, text)) as usize & (SHARDS - 1)
}

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
