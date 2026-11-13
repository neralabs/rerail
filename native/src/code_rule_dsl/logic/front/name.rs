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

/// A program's list (an atom's arguments, a rule's body, a declaration's columns, a fact's row), shared: a clone is
/// a count, and a program copied by a pass copies none of its lists. A list is changed in place where no other
/// program holds it (`make_mut`, `edit`), and copied first where one does.
///
/// The empty list holds nothing, so making one makes nothing. Lists read off stored texts are made through a run's
/// `Lists`, which keeps one of each distinct list: the run's programs repeat most of theirs.
pub struct List<T>(Option<Arc<[T]>>);

impl<T> List<T> {
    pub const fn new() -> List<T> {
        List(None)
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn as_slice(&self) -> &[T] {
        self.0.as_deref().unwrap_or(&[])
    }

    /// Whether `self` and `other` are the one list, not two equal ones.
    pub fn same(&self, other: &List<T>) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: Clone> List<T> {
    /// The items, to change in place: copied first where another holder shares them.
    pub fn make_mut(&mut self) -> &mut [T] {
        match &mut self.0 {
            Some(items) => Arc::make_mut(items),
            None => &mut [],
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.make_mut()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.make_mut().iter_mut()
    }

    /// `f` over the items as a `Vec`, for a change of length: the list is made again, once, after it.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Vec<T>) -> R) -> R {
        let mut items = self.as_slice().to_vec();
        let out = f(&mut items);
        *self = List::from(items);
        out
    }

    pub fn push(&mut self, item: T) {
        self.edit(|v| v.push(item))
    }

    pub fn insert(&mut self, at: usize, item: T) {
        self.edit(|v| v.insert(at, item))
    }

    pub fn remove(&mut self, at: usize) -> T {
        self.edit(|v| v.remove(at))
    }

    pub fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.edit(|v| v.truncate(len))
        }
    }

    /// As `Vec::retain`, `keep` called once for each item in order; the list is made again only where one goes.
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let kept: Vec<bool> = self.iter().map(&mut keep).collect();
        if kept.iter().all(|k| *k) {
            return;
        }
        *self = self.iter().zip(&kept).filter(|(_, k)| **k).map(|(t, _)| t.clone()).collect();
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = T>) {
        self.edit(|v| v.extend(items))
    }

    /// As `Vec::splice`, the items taken out dropped.
    pub fn splice(&mut self, range: impl std::ops::RangeBounds<usize>, items: impl IntoIterator<Item = T>) {
        self.edit(|v| {
            v.splice(range, items);
        })
    }
}

impl<T> Clone for List<T> {
    fn clone(&self) -> List<T> {
        List(self.0.clone())
    }
}

impl<T> Default for List<T> {
    fn default() -> List<T> {
        List(None)
    }
}

impl<T> std::ops::Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> Borrow<[T]> for List<T> {
    fn borrow(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T> AsRef<[T]> for List<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: PartialEq> PartialEq for List<T> {
    fn eq(&self, other: &List<T>) -> bool {
        self.same(other) || self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for List<T> {}

impl<T: PartialEq> PartialEq<Vec<T>> for List<T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: PartialEq> PartialEq<List<T>> for Vec<T> {
    fn eq(&self, other: &List<T>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: PartialEq> PartialEq<[T]> for List<T> {
    fn eq(&self, other: &[T]) -> bool {
        self.as_slice() == other
    }
}

impl<T: PartialEq, const N: usize> PartialEq<[T; N]> for List<T> {
    fn eq(&self, other: &[T; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Hash> Hash for List<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // As the slice, which `Borrow<[T]>` needs.
        self.as_slice().hash(state);
    }
}

impl<T: PartialOrd> PartialOrd for List<T> {
    fn partial_cmp(&self, other: &List<T>) -> Option<std::cmp::Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}

impl<T: Ord> Ord for List<T> {
    fn cmp(&self, other: &List<T>) -> std::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl<T: fmt::Debug> fmt::Debug for List<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_slice(), f)
    }
}

impl<T> From<Vec<T>> for List<T> {
    fn from(items: Vec<T>) -> List<T> {
        if items.is_empty() { List(None) } else { List(Some(Arc::from(items))) }
    }
}

impl<T: Clone> From<&[T]> for List<T> {
    fn from(items: &[T]) -> List<T> {
        if items.is_empty() { List(None) } else { List(Some(Arc::from(items))) }
    }
}

impl<T: Clone> From<&Vec<T>> for List<T> {
    fn from(items: &Vec<T>) -> List<T> {
        List::from(items.as_slice())
    }
}

impl<T, const N: usize> From<[T; N]> for List<T> {
    fn from(items: [T; N]) -> List<T> {
        if N == 0 { List(None) } else { List(Some(Arc::from(items))) }
    }
}

impl<T: Clone> From<List<T>> for Vec<T> {
    fn from(list: List<T>) -> Vec<T> {
        list.as_slice().to_vec()
    }
}

impl<T> FromIterator<T> for List<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> List<T> {
        let items: Arc<[T]> = items.into_iter().collect();
        if items.is_empty() { List(None) } else { List(Some(items)) }
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> std::slice::Iter<'a, T> {
        self.as_slice().iter()
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut List<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> std::slice::IterMut<'a, T> {
        self.iter_mut()
    }
}

/// A list's items by value: each cloned out of the list, which is shared.
pub struct IntoIter<T> {
    list: List<T>,
    at: usize,
    end: usize,
}

impl<T: Clone> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.at == self.end {
            return None;
        }
        self.at += 1;
        Some(self.list.as_slice()[self.at - 1].clone())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.end - self.at, Some(self.end - self.at))
    }
}

impl<T: Clone> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.at == self.end {
            return None;
        }
        self.end -= 1;
        Some(self.list.as_slice()[self.end].clone())
    }
}

impl<T: Clone> ExactSizeIterator for IntoIter<T> {}

impl<T: Clone> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        let end = self.len();
        IntoIter { list: self, at: 0, end }
    }
}

impl<T: serde::Serialize> serde::Serialize for List<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(s)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for List<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(List::from(Vec::<T>::deserialize(d)?))
    }
}

/// An item of a list a run's `Lists` makes, hashed and compared shallowly: each name and each list in it by its
/// address. A list made through the table holds names (interned) and lists (the table's) that are the one of their
/// value, so two such items are equal exactly when their parts are the same: equal lists are found without walking
/// what they hold. An item holding a list made elsewhere is only found less often, never wrongly.
pub trait Shallow {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher);
    fn shallow_eq(&self, other: &Self) -> bool;
}

impl Name {
    /// Its address: one name, one address.
    pub fn address(&self) -> usize {
        Arc::as_ptr(&self.0) as *const u8 as usize
    }
}

impl<T> List<T> {
    /// Its address, or 0 for the empty list.
    pub fn address(&self) -> usize {
        self.0.as_ref().map_or(0, |items| Arc::as_ptr(items) as *const T as usize)
    }
}

impl Shallow for (Name, Name) {
    fn shallow_hash(&self, h: &mut rustc_hash::FxHasher) {
        h.write_usize(self.0.address());
        h.write_usize(self.1.address());
    }
    fn shallow_eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == other.1
    }
}

fn shallow_hash_of<T: Shallow>(items: &[T]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    h.write_usize(items.len());
    for item in items {
        item.shallow_hash(&mut h);
    }
    h.finish()
}

/// One table of distinct lists of `T`, by their items' shallow hash.
struct ListTable<T>(Mutex<hashbrown::HashTable<List<T>>>);

impl<T> Default for ListTable<T> {
    fn default() -> ListTable<T> {
        ListTable(Mutex::new(hashbrown::HashTable::new()))
    }
}

impl<T: Clone + Shallow> ListTable<T> {
    fn get(&self, items: &[T]) -> List<T> {
        if items.is_empty() {
            return List::new();
        }
        let hash = shallow_hash_of(items);
        let mut t = self.0.lock().expect("the lists");
        let entry = t.entry(hash, |l| l.len() == items.len() && l.iter().zip(items).all(|(a, b)| a.shallow_eq(b)), |l| shallow_hash_of(l));
        entry.or_insert_with(|| List::from(items)).get().clone()
    }

    fn len(&self) -> usize {
        self.0.lock().expect("the lists").len()
    }
}

/// A run's distinct lists, one table for each kind: the programs a run reads from their stored texts share every
/// list they have in common. The run holds it, and it goes with the run.
#[derive(Default)]
pub struct Lists {
    terms: ListTable<crate::code_rule_dsl::logic::front::ast::Term>,
    literals: ListTable<crate::code_rule_dsl::logic::front::ast::Literal>,
    columns: ListTable<(Name, Name)>,
    values: ListTable<crate::code_rule_dsl::logic::front::ast::Value>,
    /// The stacks a read gathers its lists on, kept between reads so their room is made once a run.
    stacks: Mutex<Stacks>,
}

/// Where a read gathers each kind of list (`program_text::from_tape`), empty between reads.
#[derive(Default)]
pub struct Stacks {
    pub terms: Vec<crate::code_rule_dsl::logic::front::ast::Term>,
    pub literals: Vec<crate::code_rule_dsl::logic::front::ast::Literal>,
    pub columns: Vec<(Name, Name)>,
    pub values: Vec<crate::code_rule_dsl::logic::front::ast::Value>,
}

impl Lists {
    pub fn terms(&self, items: &[crate::code_rule_dsl::logic::front::ast::Term]) -> List<crate::code_rule_dsl::logic::front::ast::Term> {
        self.terms.get(items)
    }

    pub fn literals(&self, items: &[crate::code_rule_dsl::logic::front::ast::Literal]) -> List<crate::code_rule_dsl::logic::front::ast::Literal> {
        self.literals.get(items)
    }

    pub fn columns(&self, items: &[(Name, Name)]) -> List<(Name, Name)> {
        self.columns.get(items)
    }

    pub fn values(&self, items: &[crate::code_rule_dsl::logic::front::ast::Value]) -> List<crate::code_rule_dsl::logic::front::ast::Value> {
        self.values.get(items)
    }

    /// The stacks, for one read: the run's, when no other read holds them.
    pub fn take_stacks(&self) -> Stacks {
        std::mem::take(&mut *self.stacks.lock().expect("the stacks"))
    }

    /// The stacks back, emptied, their room kept.
    pub fn give_stacks(&self, mut stacks: Stacks) {
        stacks.terms.clear();
        stacks.literals.clear();
        stacks.columns.clear();
        stacks.values.clear();
        *self.stacks.lock().expect("the stacks") = stacks;
    }

    /// How many distinct lists of each kind it holds: terms, literals, columns, values.
    pub fn sizes(&self) -> [usize; 4] {
        [self.terms.len(), self.literals.len(), self.columns.len(), self.values.len()]
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;
    use crate::code_rule_dsl::logic::front::ast::Term;

    fn var(v: &str) -> Term {
        Term::Var(Name::new(v))
    }

    #[test]
    fn a_shared_list_is_copied_before_it_changes() {
        let a: List<Term> = [var("x"), var("y")].into();
        let mut b = a.clone();
        assert!(a.same(&b), "a clone is the one list");
        b.make_mut()[0] = var("z");
        assert!(!a.same(&b));
        assert_eq!(a, [var("x"), var("y")]);
        assert_eq!(b, [var("z"), var("y")]);
        // Held once, it changes in place.
        let before = b.address();
        b.make_mut()[1] = var("w");
        assert_eq!(b.address(), before);
        b.push(var("v"));
        b.retain(|t| *t != var("w"));
        assert_eq!(b, [var("z"), var("v")]);
        let kept = b.address();
        b.retain(|_| true);
        assert_eq!(b.address(), kept, "a retain that keeps every item makes nothing");
        b.clear();
        assert!(b.is_empty() && b.address() == 0 && b == List::new());
        assert_eq!(a.clone().into_iter().rev().collect::<Vec<_>>(), [var("y"), var("x")]);
    }

    #[test]
    fn a_runs_table_finds_a_list_by_its_parts() {
        let lists = Lists::default();
        let inner = lists.terms(&[var("a"), Term::Int(1)]);
        let call = |args: &List<Term>| Term::Call(Name::new("f"), args.clone());
        let one = lists.terms(&[call(&inner), var("b")]);
        let two = lists.terms(&[call(&inner), var("b")]);
        assert!(one.same(&two), "the same parts: the one list");
        // Equal arguments made elsewhere: an equal list, found less often but never wrongly.
        let elsewhere: List<Term> = [var("a"), Term::Int(1)].into();
        let three = lists.terms(&[call(&elsewhere), var("b")]);
        assert_eq!(three, one);
        assert!(!lists.terms(&[call(&inner), var("c")]).same(&one));
        assert_eq!(lists.sizes()[0], 4);
        assert!(lists.terms(&[]).is_empty() && lists.sizes()[0] == 4, "the empty list is not kept");
    }
}
