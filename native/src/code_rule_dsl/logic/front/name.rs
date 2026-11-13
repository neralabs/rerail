//! A relation's name, interned for the process: one allocation per distinct name, shared by every atom that names
//! it. A program is copied whole by several passes (the front, lowered and split forms, a run's parts), and its
//! relation names were a `String` each, copied with every atom; names compared byte by byte where they are equal
//! only when they are the same name. A `Name` clones by a count, and two compare equal by address.
//!
//! It reads as the `str` it holds (`Deref`, `Borrow<str>`, `Display`, a `Debug` like a `String`'s), hashes and
//! orders as that `str`, so a set or map of names keeps the order and lookups by `&str` a set of `String`s had.

use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone)]
pub struct Name(Arc<str>);

/// The process's names. An entry no name holds any more (its count one: the table's own) goes when the table has
/// doubled since the last sweep, so a long-lived process preparing many programs does not keep every name it made.
struct Table {
    names: HashSet<Arc<str>, rustc_hash::FxBuildHasher>,
    swept_at: usize,
}

fn table() -> &'static Mutex<Table> {
    static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(Table { names: HashSet::default(), swept_at: 1 << 16 }))
}

impl Name {
    pub fn new(text: &str) -> Name {
        let mut t = table().lock().expect("the names");
        if let Some(found) = t.names.get(text) {
            return Name(Arc::clone(found));
        }
        if t.names.len() >= 2 * t.swept_at {
            t.names.retain(|n| Arc::strong_count(n) > 1);
            t.swept_at = t.names.len().max(1 << 16);
        }
        let made: Arc<str> = Arc::from(text);
        t.names.insert(Arc::clone(&made));
        Name(made)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Name {
    fn default() -> Name {
        Name::new("")
    }
}

impl std::ops::Deref for Name {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Name) -> bool {
        // Interned: one name, one address.
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Name {}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // As the `str`, which `Borrow<str>` needs.
        self.as_str().hash(state);
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Name) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Name) -> std::cmp::Ordering {
        if Arc::ptr_eq(&self.0, &other.0) { std::cmp::Ordering::Equal } else { self.as_str().cmp(other.as_str()) }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Name> for String {
    fn eq(&self, other: &Name) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Name> for str {
    fn eq(&self, other: &Name) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Name> for &str {
    fn eq(&self, other: &Name) -> bool {
        *self == other.as_str()
    }
}

impl From<&str> for Name {
    fn from(text: &str) -> Name {
        Name::new(text)
    }
}

impl From<String> for Name {
    fn from(text: String) -> Name {
        Name::new(&text)
    }
}

impl From<&String> for Name {
    fn from(text: &String) -> Name {
        Name::new(text)
    }
}

impl From<&Name> for Name {
    fn from(name: &Name) -> Name {
        name.clone()
    }
}

impl From<Name> for String {
    fn from(name: Name) -> String {
        name.as_str().to_owned()
    }
}

impl From<&Name> for String {
    fn from(name: &Name) -> String {
        name.as_str().to_owned()
    }
}

/// A name serializes as the text it holds.
impl serde::Serialize for Name {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// A name deserializes from its text, interned.
impl<'de> serde::Deserialize<'de> for Name {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Ok(Name::new(&text))
    }
}
