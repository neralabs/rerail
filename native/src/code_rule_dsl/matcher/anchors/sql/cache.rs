//! Bounded ownership of compiled SQL. Entries are disposable; source facts live in SQLite.
use std::{collections::HashMap, hash::Hash};

struct Entry<V> {
    value: V,
    bytes: usize,
    used: u64,
}
pub(super) struct Cache<K, V> {
    entries: HashMap<K, Entry<V>>,
    limit: usize,
    bytes: usize,
    tick: u64,
}
impl<K: Eq + Hash + Clone, V> Cache<K, V> {
    pub fn new(limit: usize) -> Self {
        Self { entries: HashMap::new(), limit, bytes: 0, tick: 0 }
    }
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let entry = self.entries.get_mut(key)?;
        entry.used = self.tick;
        Some(&entry.value)
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if let Some(previous) = self.entries.remove(&key) { self.bytes -= previous.bytes; }
        self.tick += 1;
        self.bytes += bytes;
        self.entries.insert(key, Entry { value, bytes, used: self.tick });
    }
    pub fn values(&self) -> impl Iterator<Item=&V> { self.entries.values().map(|e| &e.value) }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn bytes(&self) -> usize { self.bytes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn eviction_releases_the_least_recently_used_statement() {
        let mut cache = Cache::new(20);
        let first = Arc::new(String::from("first"));
        let second = Arc::new(String::from("second"));
        cache.insert(1, first.clone(), 10);
        cache.insert(2, second.clone(), 10);
        assert_eq!(cache.get(&1).unwrap().as_str(), "first");
        cache.insert(3, Arc::new(String::from("third")), 10);
        assert!(cache.get(&2).is_none(), "least-recently-used SQL stays retained");
        assert_eq!(Arc::strong_count(&second), 1);
        assert_eq!(cache.get(&1).unwrap().as_str(), "first");
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), 20);
    }
    #[test]
    fn oversized_statements_do_not_displace_the_working_set() {
        let mut cache = Cache::new(20);
        cache.insert(1, "hot", 10);
        cache.insert(2, "oversized", 30);
        assert_eq!(cache.get(&1), Some(&"hot"));
        assert!(cache.get(&2).is_none());
        assert_eq!(cache.bytes(), 10);
        cache.insert(1, "replacement", 15);
        assert_eq!(cache.bytes(), 15);
    }
}
