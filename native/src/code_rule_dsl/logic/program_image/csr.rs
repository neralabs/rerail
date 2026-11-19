//! One key to many values, as an image stores it (decision 0173 §4): CSR, `starts[k]..starts[k + 1]` the range of
//! key k's values. Dense in a base (every key from 0); keyed in a delta, whose few keys are any of its stack's, so a
//! delta's index is the size of its edit. `Csr` reads either; `Keyed` writes the keyed form.

/// A CSR's starts, dense (`keys` empty) or over sorted `keys`.
pub struct Csr<'a> {
    keys: &'a [u32],
    starts: &'a [u32],
}

impl<'a> Csr<'a> {
    pub fn new(keys: &'a [u32], starts: &'a [u32]) -> Csr<'a> {
        Csr { keys, starts }
    }
    /// The range of `key`'s values (empty when it has none).
    pub fn range(&self, key: u32) -> std::ops::Range<usize> {
        let k = if self.keys.is_empty() {
            key as usize
        } else {
            match self.keys.binary_search(&key) {
                Ok(k) => k,
                Err(_) => return 0..0,
            }
        };
        match (self.starts.get(k), self.starts.get(k + 1)) {
            (Some(&a), Some(&b)) => a as usize..b as usize,
            _ => 0..0,
        }
    }
}

/// The keyed form of (key, value) pairs: the sorted distinct keys, their starts, and the values grouped by key in
/// the order given.
pub fn keyed(mut pairs: Vec<(u32, u32)>) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    pairs.sort_by_key(|p| p.0);
    let mut keys = Vec::new();
    let mut starts = vec![0u32];
    let mut values = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        if keys.last() != Some(&k) {
            if !keys.is_empty() {
                starts.push(values.len() as u32);
            }
            keys.push(k);
        }
        values.push(v);
    }
    if !keys.is_empty() {
        starts.push(values.len() as u32);
    }
    (keys, starts, values)
}
