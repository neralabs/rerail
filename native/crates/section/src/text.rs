//! Text-keyed sections (decision 0569 §4): a set of names, and a map from a name to a value, each one section of
//! its names sorted and found by a search of them. Shared by a count (`Arc`): a fork that reads one holds it, and a
//! change makes a new section (`with`, `without`), which the holders after the change read.
use crate::{Archive, Decode, RelStr, Section, Tuple2};
use std::sync::Arc;

/// A set of names, sorted, each once.
#[derive(Clone)]
pub struct TextSet(Arc<Section<Vec<String>>>);

impl Default for TextSet {
    fn default() -> TextSet {
        TextSet(Arc::new(Section::of(&Vec::new())))
    }
}

impl TextSet {
    /// The names of `it`, each once.
    pub fn of<S: AsRef<str>>(it: impl IntoIterator<Item = S>) -> TextSet {
        let mut names: Vec<String> = it.into_iter().map(|s| s.as_ref().to_owned()).collect();
        names.sort_unstable();
        names.dedup();
        TextSet(Arc::new(Section::of(&names)))
    }

    fn names(&self) -> &[RelStr] {
        self.0.get()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names().binary_search_by(|n| n.as_str().cmp(name)).is_ok()
    }

    /// The names, sorted.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> + DoubleEndedIterator + Clone + '_ {
        self.names().iter().map(|n| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.names().len()
    }

    pub fn is_empty(&self) -> bool {
        self.names().is_empty()
    }

    /// It and `more`.
    pub fn with<S: AsRef<str>>(&self, more: impl IntoIterator<Item = S>) -> TextSet {
        let mut more = more.into_iter().peekable();
        if more.peek().is_none() {
            return self.clone();
        }
        // copy: derived: a new set, its names and those added.
        TextSet::of(self.iter().map(str::to_owned).chain(more.map(|s| s.as_ref().to_owned())))
    }

    /// The names `keep` keeps.
    pub fn filtered(&self, keep: impl Fn(&str) -> bool) -> TextSet {
        // copy: derived: a new set, of the names kept.
        TextSet::of(self.iter().filter(|n| keep(n)))
    }

    /// Its names as a set of strings, for a reader of the tree form.
    pub fn to_set(&self) -> std::collections::BTreeSet<String> {
        // copy: not block data: the tree form's set.
        self.iter().map(str::to_owned).collect()
    }
}

impl PartialEq for TextSet {
    fn eq(&self, o: &TextSet) -> bool {
        Arc::ptr_eq(&self.0, &o.0) || self.names() == o.names()
    }
}

impl Eq for TextSet {}

impl std::hash::Hash for TextSet {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.names().hash(h)
    }
}

impl std::fmt::Debug for TextSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for &'a TextSet {
    type Item = &'a str;
    type IntoIter = std::iter::Map<std::slice::Iter<'a, RelStr>, fn(&'a RelStr) -> &'a str>;
    fn into_iter(self) -> Self::IntoIter {
        self.names().iter().map(RelStr::as_str as fn(&'a RelStr) -> &'a str)
    }
}

impl<S: AsRef<str>> FromIterator<S> for TextSet {
    fn from_iter<I: IntoIterator<Item = S>>(it: I) -> TextSet {
        TextSet::of(it)
    }
}

/// A map from a name to a value, sorted by name, each name once.
pub struct TextMap<V: Archive>(Arc<Section<Vec<(String, V)>>>);

impl<V: Archive> Clone for TextMap<V> {
    fn clone(&self) -> Self {
        TextMap(Arc::clone(&self.0))
    }
}

impl<V: Archive> Default for TextMap<V> {
    fn default() -> Self {
        TextMap(Arc::new(Section::of(&Vec::new())))
    }
}

impl<V: Archive> TextMap<V> {
    /// The entries of `it`, by name: of a name given twice, the last.
    pub fn of(it: impl IntoIterator<Item = (String, V)>) -> Self {
        let mut es: Vec<(usize, (String, V))> = it.into_iter().enumerate().collect();
        es.sort_by(|(i, (a, _)), (j, (b, _))| a.cmp(b).then(j.cmp(i)));
        es.dedup_by(|(_, (a, _)), (_, (b, _))| a == b);
        let es: Vec<(String, V)> = es.into_iter().map(|(_, e)| e).collect();
        TextMap(Arc::new(Section::of(&es)))
    }

    fn entries(&self) -> &[Tuple2<RelStr, V::Archived>] {
        self.0.get()
    }

    fn at(&self, name: &str) -> Option<usize> {
        self.entries().binary_search_by(|e| e.0.as_str().cmp(name)).ok()
    }

    /// `name`'s value, as its section holds it.
    pub fn get(&self, name: &str) -> Option<&V::Archived> {
        self.at(name).map(|i| &self.entries()[i].1)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.at(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// Each name and its value, by name.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &V::Archived)> + DoubleEndedIterator + Clone + '_ {
        self.entries().iter().map(|e| (e.0.as_str(), &e.1))
    }

    /// The names, sorted.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> + DoubleEndedIterator + Clone + '_ {
        self.entries().iter().map(|e| e.0.as_str())
    }

    /// The values, by name.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &V::Archived> + DoubleEndedIterator + Clone + '_ {
        self.entries().iter().map(|e| &e.1)
    }

    /// Whether the two are one section.
    pub fn same(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl<V: Decode> TextMap<V> {
    /// It with `more`'s entries, which replace its own of the same names.
    pub fn with(&self, more: impl IntoIterator<Item = (String, V)>) -> Self {
        let mut more = more.into_iter().peekable();
        if more.peek().is_none() {
            return self.clone();
        }
        let n = self.0.names();
        // copy: derived: a new map, its entries and those added.
        TextMap::of(self.iter().map(|(k, v)| (k.to_owned(), V::decode(v, &n))).chain(more))
    }

    /// The entries `keep` keeps.
    pub fn filtered(&self, keep: impl Fn(&str, &V::Archived) -> bool) -> Self {
        let n = self.0.names();
        // copy: derived: a new map, of the entries kept.
        TextMap::of(self.iter().filter(|(k, v)| keep(k, v)).map(|(k, v)| (k.to_owned(), V::decode(v, &n))))
    }

    /// `name`'s value, decoded.
    pub fn decoded(&self, name: &str) -> Option<V> {
        let n = self.0.names();
        self.get(name).map(|v| V::decode(v, &n))
    }
}

impl<V: Archive> PartialEq for TextMap<V>
where
    V::Archived: PartialEq,
{
    fn eq(&self, o: &Self) -> bool {
        self.same(o) || self.entries() == o.entries()
    }
}

impl<V: Archive> std::fmt::Debug for TextMap<V>
where
    V::Archived: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V: Archive> FromIterator<(String, V)> for TextMap<V> {
    fn from_iter<I: IntoIterator<Item = (String, V)>>(it: I) -> Self {
        TextMap::of(it)
    }
}
