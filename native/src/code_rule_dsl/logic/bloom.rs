//! A blocked Bloom filter of a column's keys (decision 0477): the probe screen's (`native::setfix::join_filter`) and
//! the statistics' (a column's membership and the keys two columns share).
//!
//! ```text
//! h     = hash(key)                       // setfix's sketch hash: the same keys, the same h, everywhere
//! word  = (h >> 40) & (words - 1)         // one 64-bit word holds all of a key's bits
//! mask  = bit(h) | bit(h >> 6) | ... (5)  // 5 bits of the word, from 6-bit fields of h
//! add:  words[word] |= mask               // a device builds it with 32-bit atomics: bit b of word w is bit b & 31
//! test: words[word] & mask == mask        //   of the 32-bit word 2w + b / 32, the same bytes
//! ```
//!
//! A test is one hash, one load and one compare. A miss is certain; a hit is a candidate an exact probe checks.
//!
//! Two kinds hold the words:
//! - `Filter`, made whole at once: a screen's own, the heavy values' (`plan::table::HeavyCol`) and the device's;
//! - `Kept`, a column's filter kept as its rows come (`native::setfix::sketch::Sketch`), grown from the column as its
//!   keys pass what it is sized for. Its sketch adds to it, and the planner and the screens read it where it is.
//!
//! A filter of `2w` words folds to `w` (word `i` is words `i` and `i + w` OR-ed): the words a key takes are the low
//! bits of the same field. So any two filters compare at the smaller one's size (`shared`).
/// Bits of a word a key sets.
pub const K: u32 = 5;
/// The bits a filter is sized to give each distinct key.
pub const BITS_PER_KEY: usize = 16;
/// The fewest and most words a filter has (256 B to 16 KiB).
pub const MIN_WORDS: usize = 32;
pub const MAX_WORDS: usize = 2048;
/// The bits a kept filter gives each distinct key: a byte, about 3% of absent keys passing. It is held for as long
/// as its column, so it is sized at an eighth of the column's 8 bytes a distinct value at most.
pub const KEPT_BITS_PER_KEY: usize = 8;
/// The most words a kept filter has (1 MiB, a million keys at a byte each). Past it, it fills and tells less.
pub const MAX_KEPT_WORDS: usize = 1 << 17;

/// Setfix's sketch hash (`native::setfix::sketch::mix`), which the HyperLogLog sketches hash by too.
#[inline]
pub fn hash(v: i64) -> u64 {
    let mut x = v as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A key's word (of `words`, a power of two) and its bits there.
#[inline]
pub fn place(h: u64, words: usize) -> (usize, u64) {
    let mask = (0..K).fold(0u64, |m, i| m | 1 << ((h >> (6 * i)) & 63));
    ((h >> 40) as usize & (words - 1), mask)
}

/// The words for `keys` distinct keys: `BITS_PER_KEY` each, a power of two within the bounds.
pub fn words_for(keys: usize) -> usize {
    (keys.saturating_mul(BITS_PER_KEY) / 64).next_power_of_two().clamp(MIN_WORDS, MAX_WORDS)
}

/// The words a kept filter takes for `keys` distinct keys.
pub fn kept_words_for(keys: usize) -> usize {
    (keys.saturating_mul(KEPT_BITS_PER_KEY) / 64).next_power_of_two().clamp(MIN_WORDS, MAX_KEPT_WORDS)
}

/// A filter's words, read one at a time: both kinds' (`Filter`, `Kept`), for the estimates.
pub trait Words {
    fn len(&self) -> usize;
    fn word(&self, i: usize) -> u64;
    /// The bits set.
    fn ones(&self) -> u64 {
        (0..self.len()).map(|i| u64::from(self.word(i).count_ones())).sum()
    }
    /// The distinct keys added, estimated from the bits set (Swamidass and Baldi): `-(m / k) ln(1 - x / m)`.
    fn keys(&self) -> f64 {
        keys_of(self.ones(), self.len())
    }
    /// Whether most absent keys would pass: past 90% of its bits set, what it says of keys means little.
    fn full(&self) -> bool {
        self.ones() as f64 > 0.9 * (self.len() * 64) as f64
    }
}

/// Word `i` of `f` folded to `to` words (`to` a power of two, at most its words).
fn folded(f: &dyn Words, to: usize, i: usize) -> u64 {
    (i..f.len()).step_by(to).fold(0, |w, j| w | f.word(j))
}

/// The distinct keys two filters both hold, estimated from their bits: `None` where the one with more keys, folded
/// to the smaller's size (`folded`), is past half full, and its bits tell little of a key. Of the bits of the one
/// with fewer keys (`s`), a share `q` is set by a shared key; the rest are set in the other (`o`) only by its own
/// keys, at their fill `g`, which is `o`'s fill `f` less the shared keys' (`q bits(s) / m`):
///
/// ```text
/// both / bits(s) = q + (1 - q) g     g = (f - q bits(s) / m) / (1 - q bits(s) / m)     (solved by iterating)
/// shared keys    = keys_of(q bits(s))       the keys a filter of q bits(s) bits holds
/// ```
pub fn shared(a: &dyn Words, b: &dyn Words) -> Option<f64> {
    let w = a.len().min(b.len());
    let (ka, kb) = (a.keys(), b.keys());
    let (s, o) = if ka <= kb { (a, b) } else { (b, a) };
    let (mut bits_s, mut bits_o, mut both) = (0u64, 0u64, 0u64);
    for i in 0..w {
        let (x, y) = (folded(s, w, i), folded(o, w, i));
        bits_s += u64::from(x.count_ones());
        bits_o += u64::from(y.count_ones());
        both += u64::from((x & y).count_ones());
    }
    let m = (w * 64) as f64;
    let f = bits_o as f64 / m;
    if f > 0.5 {
        return None;
    }
    if bits_s == 0 {
        return Some(0.0);
    }
    let (held, bits) = (both as f64 / bits_s as f64, bits_s as f64);
    let mut q = ((held - f) / (1.0 - f)).clamp(0.0, 1.0);
    for _ in 0..16 {
        let mine = q * bits / m;
        let g = ((f - mine) / (1.0 - mine)).max(0.0);
        q = ((held - g) / (1.0 - g)).clamp(0.0, 1.0);
    }
    Some(keys_of((q * bits).round() as u64, w).min(ka.min(kb)))
}

/// Whether the planner and the screens read the columns' kept keys. `RERAIL_KEPT_KEYS=off` (a measurement aid): the
/// sketches keep them still, and nothing reads them, so a run compares their use at the same cost of keeping them.
pub fn kept_keys_read() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_KEPT_KEYS").as_deref() != Ok("off"))
}

/// The keys a sketch samples (`Sample`): its distinct keys of the least ranks.
pub const SAMPLE: usize = 128;

/// A key's rank in a sample: its hash scrambled (bijectively), so the sample is not the keys of the filter's first
/// words, as the least hashes would be (their high bits pick the word).
#[inline]
pub fn rank(h: u64) -> u64 {
    (h ^ (h >> 31)).wrapping_mul(0xd6e8_feb8_6659_fd93).rotate_left(32)
}

/// A uniform sample of a column's distinct keys (a bottom-k sketch): the hashes of the `SAMPLE` keys of the least
/// ranks, least first, and the distinct keys it says (`distinct`). Another column's filter tested on it says the
/// share of these keys that column holds, whatever the two filters' sizes.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    hashes: Vec<u64>,
}

impl Sample {
    /// Takes the key of hash `h`, new to the column (its filter said so: a key it held already is passed over).
    #[inline]
    pub fn offer(&mut self, h: u64) {
        let r = rank(h);
        if self.hashes.len() == SAMPLE && r >= rank(self.hashes[SAMPLE - 1]) {
            return;
        }
        let at = self.hashes.partition_point(|&x| rank(x) < r);
        if self.hashes.get(at) == Some(&h) {
            return;
        }
        self.hashes.insert(at, h);
        self.hashes.truncate(SAMPLE);
    }

    pub fn hashes(&self) -> &[u64] {
        &self.hashes
    }

    /// The distinct keys, estimated (`(k - 1) 2^64 / rank_k`); exact while it holds fewer than `SAMPLE`.
    pub fn distinct(hashes: &[u64]) -> f64 {
        match hashes.last() {
            Some(&h) if hashes.len() == SAMPLE => (SAMPLE - 1) as f64 * 2f64.powi(64) / rank(h).max(1) as f64,
            _ => hashes.len() as f64,
        }
    }
}

/// A filter's words. Its bytes are the device's, as built (`place`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub words: Box<[u64]>,
}

impl Filter {
    /// An empty filter of `words` words (a power of two).
    pub fn new(words: usize) -> Filter {
        assert!(words.is_power_of_two(), "a filter's words are a power of two");
        Filter { words: vec![0; words].into_boxed_slice() }
    }

    #[inline]
    pub fn add(&mut self, key: i64) {
        let (w, m) = place(hash(key), self.words.len());
        self.words[w] |= m;
    }

    /// Whether `key` may be in: false is certain.
    #[inline]
    pub fn may_hold(&self, key: i64) -> bool {
        let (w, m) = place(hash(key), self.words.len());
        self.words[w] & m == m
    }

    /// The bits set.
    pub fn ones(&self) -> u64 {
        Words::ones(self)
    }

    /// The distinct keys added, estimated (`Words::keys`).
    pub fn keys(&self) -> f64 {
        Words::keys(self)
    }

    /// The distinct keys both filters hold, estimated (`shared`).
    pub fn shared(&self, other: &dyn Words) -> Option<f64> {
        shared(self, other)
    }
}

impl Words for Filter {
    fn len(&self) -> usize {
        self.words.len()
    }
    fn word(&self, i: usize) -> u64 {
        self.words[i]
    }
}

/// A column's filter kept as its rows come (decision 0477). One writer adds to it, the column's sketch, while it is
/// read where it is: by the planner's statistics and the screens, which hold it (`Arc`) and never copy it. A word
/// is a plain load and store (relaxed atomics: no read-modify-write, the writer is one), and a reader reads words as
/// they are: a key added is in every reading after the add. A screen reads only a filter its rows are all in.
pub struct Kept {
    words: Box<[std::sync::atomic::AtomicU64]>,
}

impl Kept {
    /// An empty filter of `words` words (a power of two).
    pub fn new(words: usize) -> Kept {
        assert!(words.is_power_of_two(), "a filter's words are a power of two");
        Kept { words: (0..words).map(|_| std::sync::atomic::AtomicU64::new(0)).collect() }
    }

    /// Adds the key whose hash is `h`, and says whether it was new (not every one of its bits set before). Its one
    /// writer only (the sketch that keeps it, by `&mut` of its own).
    #[inline]
    pub fn add_hashed(&self, h: u64) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        let (w, m) = place(h, self.words.len());
        let word = &self.words[w];
        let was = word.load(Relaxed);
        if was & m == m {
            return false;
        }
        word.store(was | m, Relaxed);
        true
    }

    /// Whether `key` may be in: false is certain.
    #[inline]
    pub fn may_hold(&self, key: i64) -> bool {
        self.may_hold_hashed(hash(key))
    }

    #[inline]
    pub fn may_hold_hashed(&self, h: u64) -> bool {
        let (w, m) = place(h, self.words.len());
        self.words[w].load(std::sync::atomic::Ordering::Relaxed) & m == m
    }


    /// Whether it holds more keys than it is sized for and may grow (`kept_words_for`).
    pub fn outgrown(&self) -> bool {
        self.words.len() < MAX_KEPT_WORDS && Words::keys(self) * KEPT_BITS_PER_KEY as f64 > (self.words.len() * 64) as f64
    }

    pub fn bytes(&self) -> usize {
        self.words.len() * 8
    }
}

impl Words for Kept {
    fn len(&self) -> usize {
        self.words.len()
    }
    fn word(&self, i: usize) -> u64 {
        self.words[i].load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A new filter of the same words: a cloned sketch writes its own (two writers of one filter could lose a bit).
impl Clone for Kept {
    fn clone(&self) -> Kept {
        Kept { words: (0..self.words.len()).map(|i| std::sync::atomic::AtomicU64::new(Words::word(self, i))).collect() }
    }
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Kept({} words, {} bits set)", self.words.len(), Words::ones(self))
    }
}

/// A column's kept filter as its statistics read it: the filter (where it is kept, not copied), a sample of its
/// keys, and its bits set when read. An estimate costs the sample's tests, not the filter's words: the planner asks
/// for one each time it prices a probe.
#[derive(Debug)]
pub struct Reading {
    pub filter: std::sync::Arc<Kept>,
    pub sample: Box<[u64]>,
    ones: u64,
}

/// Filters of at most this many words are compared by their bits (`shared`): a few hundred loads.
const COMPARED_WORDS: usize = 256;

impl Reading {
    pub fn new(filter: std::sync::Arc<Kept>, sample: &[u64]) -> Reading {
        let ones = Words::ones(&*filter);
        Reading { filter, sample: sample.into(), ones }
    }

    fn fill(&self) -> f64 {
        self.ones as f64 / (self.filter.len() * 64) as f64
    }

    /// Past 90% of its bits set, it tells little of a key.
    pub fn full(&self) -> bool {
        self.fill() > 0.9
    }

    /// The distinct keys: the filter's estimate, or the sample's where the filter is full.
    pub fn distinct(&self) -> f64 {
        if self.full() { Sample::distinct(&self.sample) } else { keys_of(self.ones, self.filter.len()) }
    }

    /// The share of absent keys it passes: each of a key's bits set, at its fill.
    fn false_hits(&self) -> f64 {
        self.fill().powi(K as i32)
    }
}

/// A column's keys as its statistics hold them: its few values exactly (a sketch's exact counts, `SMALL` of them at
/// most), or its kept filter as read (`Reading`).
#[derive(Clone, Debug)]
pub enum Keys {
    Few(std::sync::Arc<[i64]>),
    Filter(std::sync::Arc<Reading>),
}

impl Keys {
    /// The keys of `values` (distinct or not) as a sketch keeps them: few exactly, else a filter sized for twice
    /// them (as a sketch's grows) and a sample.
    pub fn of(values: impl IntoIterator<Item = i64>) -> Keys {
        let mut v: Vec<i64> = values.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        if v.len() <= 64 {
            return Keys::Few(v.into());
        }
        let (f, mut s) = (Kept::new(kept_words_for(2 * v.len())), Sample::default());
        for &x in &v {
            f.add_hashed(hash(x));
            s.offer(hash(x));
        }
        Keys::Filter(std::sync::Arc::new(Reading::new(std::sync::Arc::new(f), s.hashes())))
    }

    /// Whether `key` may be in: false is certain.
    pub fn may_hold(&self, key: i64) -> bool {
        match self {
            Keys::Few(v) => v.contains(&key),
            Keys::Filter(r) => r.filter.may_hold(key),
        }
    }

    /// The distinct keys: exact, or the reading's estimate.
    pub fn distinct(&self) -> f64 {
        match self {
            Keys::Few(v) => v.len() as f64,
            Keys::Filter(r) => r.distinct(),
        }
    }

    /// The distinct keys both hold:
    /// - between two of few values, exact;
    /// - of few values against a filter, those it passes (its false hits at most a few percent);
    /// - between two small filters, from their bits (`shared`), which tells a few shared keys best;
    /// - else the share of the fewer keys' sample the other's filter passes, less its false hits, of those keys.
    ///
    /// `None` where the filter tested is full.
    pub fn shared(&self, other: &Keys) -> Option<f64> {
        match (self, other) {
            (Keys::Few(a), Keys::Few(b)) => Some(a.iter().filter(|v| b.contains(v)).count() as f64),
            (Keys::Few(a), Keys::Filter(r)) | (Keys::Filter(r), Keys::Few(a)) => {
                (!r.full()).then(|| a.iter().filter(|&&v| r.filter.may_hold(v)).count() as f64)
            }
            (Keys::Filter(a), Keys::Filter(b)) => {
                let bits = (a.filter.len().max(b.filter.len()) <= COMPARED_WORDS).then(|| shared(&*a.filter, &*b.filter)).flatten();
                bits.or_else(|| {
                    let (fewer, other) = if a.distinct() <= b.distinct() { (a, b) } else { (b, a) };
                    if other.full() || fewer.sample.is_empty() {
                        return None;
                    }
                    let held = fewer.sample.iter().filter(|&&h| other.filter.may_hold_hashed(h)).count() as f64 / fewer.sample.len() as f64;
                    let fp = other.false_hits();
                    Some(((held - fp) / (1.0 - fp)).clamp(0.0, 1.0) * fewer.distinct())
                })
            }
        }
    }
}

fn keys_of(ones: u64, words: usize) -> f64 {
    let m = (words * 64) as f64;
    let x = (ones as f64).min(m - 1.0);
    -(m / f64::from(K)) * (1.0 - x / m).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key added is held; keys not added are mostly refused, near the rate the size gives.
    #[test]
    fn a_filter_holds_its_keys_and_refuses_most_others() {
        let mut f = Filter::new(words_for(1659));
        for k in 0..1659 {
            f.add(k * 7919);
        }
        assert!((0..1659).all(|k| f.may_hold(k * 7919)));
        let passed = (0..100_000).filter(|&k| f.may_hold(-1 - k)).count();
        assert!(passed < 3_000, "{passed} of 100000 passed");
        let keys = f.keys();
        assert!((keys / 1659.0 - 1.0).abs() < 0.1, "{keys}");
    }

    /// Two filters' shared keys: 626 of 7,650 and 7,650 (the SLF001 capture's shape), within a fifth.
    #[test]
    fn two_filters_estimate_the_keys_they_share() {
        let words = words_for(8000);
        let (mut a, mut b) = (Filter::new(words), Filter::new(words));
        for k in 0..7650 {
            a.add(k);
            b.add(k + 7650 - 626);
        }
        let shared = a.shared(&b).unwrap();
        assert!((shared / 626.0 - 1.0).abs() < 0.2, "{shared}");
        assert_eq!(Filter::new(words).shared(&Filter::new(words * 2)), Some(0.0));
    }

    /// Kept filters of different sizes compare folded: a column of 300 keys against one of 40,000 that holds 100 of
    /// them, and two disjoint ones. A filter full past telling gives `None`.
    #[test]
    fn kept_filters_of_two_sizes_estimate_the_keys_they_share() {
        let (small, big) = (Kept::new(kept_words_for(300)), Kept::new(kept_words_for(40_000)));
        let (mut ss, mut sb) = (Sample::default(), Sample::default());
        for k in 0..300 {
            if small.add_hashed(hash(k)) { ss.offer(hash(k)); }
        }
        for k in 200..40_200 {
            if big.add_hashed(hash(k)) { sb.offer(hash(k)); }
        }
        let keys = |f: &Kept, s: &Sample| Keys::Filter(std::sync::Arc::new(Reading::new(std::sync::Arc::new(f.clone()), s.hashes())));
        // Folded to 300 keys' words, the 40,000 fill them: the sample tells.
        assert!(shared(&small, &big).is_none());
        let s = keys(&small, &ss).shared(&keys(&big, &sb)).unwrap();
        assert!((s / 100.0 - 1.0).abs() < 0.35, "{s}");
        let d = Sample::distinct(sb.hashes());
        assert!((d / 40_000.0 - 1.0).abs() < 0.25, "{d}");
        let apart = Kept::new(kept_words_for(40_000));
        for k in 0..40_000 {
            apart.add_hashed(hash(-1 - k));
        }
        let none = shared(&big, &apart).unwrap();
        assert!(none < 0.02 * 40_000.0, "{none}");
        // A few values against a filter: those it passes.
        let few = Keys::Few((150..250).collect::<Vec<_>>().into());
        assert_eq!(few.shared(&keys(&small, &ss)), Some(100.0));
        let full = Kept::new(MIN_WORDS);
        for k in 0..100_000 {
            full.add_hashed(hash(k));
        }
        assert!(shared(&full, &big).is_none() && full.full() && full.outgrown());
    }
}
