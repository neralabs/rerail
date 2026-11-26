//! A relation keyed by one node of one document, read by that node's local id (t57 §8, E7; a spike beside the
//! engine).
//!
//! The reference graph's rules pass each name read through a dozen per-node relations (`nameref`, `!bind_only`,
//! `!rf_deleted`, `!guest_of`, `wstart`, `rf_in`, …). Each is a materialized relation, and each test of a bound node
//! is a probe of its hash index: hash the key, find the group, read each row of it and compare. Here such a
//! relation is a column of the document's nodes instead, in its block, and a test is one load:
//!
//! - `SET`: a bit a node (`bind_only(i)`, `rr_ast_8(i)`): `has(l)`.
//! - `ONE`: a value a node, the key's one row (`wstart(n, s)`, `rf_in(n, g)`): `one(l)`. A second value for a node
//!   breaks the key; the writer is told (`Broken`), as 0361 checks a key where its rows are written.
//! - `FAN`: a node's values together, by offsets (`child`-like, a few a node): `each(l)`. Made once from its rows.
//!
//! Values are 32 bits (a node's local id, an atom). Every part is a `Span` of the block: the descriptor and its words
//! are read where they lie, by any reader of the block (decision 0353), with no structure besides them.

use crate::block::Block;
use crate::structure;
use crate::refs::Span;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};

/// No value: `ONE`'s empty slot.
pub const NONE: u32 = u32::MAX;

/// A node's set of the relation: a bit a node.
pub const SET: u32 = 1;
/// A value a node.
pub const ONE: u32 = 2;
/// A node's values, grouped.
pub const FAN: u32 = 3;

structure! {
    /// A relation keyed by one node column of one document (module docs).
    pub struct NodeKeyed {
        /// The document's numbered nodes: the local ids `0..nodes`.
        pub nodes: u32,
        /// `SET`, `ONE` or `FAN`.
        pub form: u32,
        /// Its rows: the nodes it holds (`SET`, `ONE`), or its pairs (`FAN`).
        pub rows: AtomicU32,
        /// A node a writer gave a second value (`ONE`'s key broken), plus one; 0 for none.
        pub broke: AtomicU32,
        /// A bit a node: held (`SET`), or has a value (`ONE`).
        pub bits: Span<u64>,
        /// `ONE`: each node's value, `NONE` for none.
        pub one: Span<u32>,
        /// `FAN`: node `l`'s values are `vals[starts[l]..starts[l + 1]]`.
        pub starts: Span<u32>,
        pub vals: Span<u32>,
    }
}

/// `span`'s items as atomics, for the writers of an open block (the same layout).
#[inline(always)]
fn atomic<T, A>(span: &Span<T>) -> &[A] {
    const { assert!(size_of::<T>() == size_of::<A>()) };
    let n = span.len();
    if n == 0 {
        return &[];
    }
    // SAFETY: `A` is `T`'s atomic, of its size and alignment; the items lie in the block for as long as `span`.
    unsafe { std::slice::from_raw_parts(span.as_ptr() as *const A, n) }
}

/// A node already holds another value: `ONE`'s key is broken there.
#[derive(Debug, PartialEq, Eq)]
pub struct Broken {
    pub node: u32,
    pub had: u32,
    pub value: u32,
}

impl NodeKeyed {
    fn make<'b>(block: &'b Block, nodes: u32, form: u32) -> Option<&'b NodeKeyed> {
        let p: *mut NodeKeyed = block.try_alloc_t(1)?;
        let words = (nodes as usize).div_ceil(64);
        let bits: *mut u64 = block.try_alloc_t(words.max(1))?;
        // SAFETY: fresh, zeroed room of the block; one thread holds it until it is returned.
        unsafe {
            (*p).nodes = nodes;
            (*p).form = form;
            Span::set(&raw const (*p).bits, bits, words);
            Some(&*p)
        }
    }

    /// An empty `SET` of a document of `nodes` nodes; `None` where the block has no room.
    pub fn try_set(block: &Block, nodes: u32) -> Option<&NodeKeyed> {
        Self::make(block, nodes, SET)
    }

    /// An empty `ONE` of a document of `nodes` nodes.
    pub fn try_one(block: &Block, nodes: u32) -> Option<&NodeKeyed> {
        let p = Self::make(block, nodes, ONE)?;
        let one: *mut u32 = block.try_alloc_t(nodes as usize)?;
        // SAFETY: as `make`.
        unsafe {
            std::slice::from_raw_parts_mut(one, nodes as usize).fill(NONE);
            Span::set(&raw const p.one, one, nodes as usize);
        }
        Some(p)
    }

    /// A `FAN` of `pairs` `(node, value)`, in any order, repeats dropped: each node's values in the order given.
    pub fn try_fan<'b>(block: &'b Block, nodes: u32, pairs: &[(u32, u32)]) -> Option<&'b NodeKeyed> {
        let p = Self::make(block, nodes, FAN)?;
        let n = nodes as usize;
        let starts: *mut u32 = block.try_alloc_t(n + 1)?;
        let vals: *mut u32 = block.try_alloc_t(pairs.len().max(1))?;
        // SAFETY: as `make`; `starts` and `vals` are this structure's alone.
        unsafe {
            let starts = std::slice::from_raw_parts_mut(starts, n + 1);
            let vals = std::slice::from_raw_parts_mut(vals, pairs.len().max(1));
            // A counting sort by node: counts, then their prefix sums, then each pair at its node's next place.
            for &(l, _) in pairs {
                starts[l as usize + 1] += 1;
            }
            for l in 0..n {
                starts[l + 1] += starts[l];
            }
            let mut at: Vec<u32> = starts[..n].to_vec();
            for &(l, v) in pairs {
                let s = &mut at[l as usize];
                // A repeat of a value the node has already: dropped (set semantics).
                if !vals[starts[l as usize] as usize..*s as usize].contains(&v) {
                    vals[*s as usize] = v;
                    *s += 1;
                }
            }
            // Close the gaps repeats left: each node's values moved down to follow the last node's.
            let mut w = 0u32;
            for l in 0..n {
                let (from, to) = (starts[l], at[l]);
                starts[l] = w;
                for i in from..to {
                    vals[w as usize] = vals[i as usize];
                    w += 1;
                }
                if to > from {
                    atomic::<u64, AtomicU64>(&p.bits)[l / 64].fetch_or(1 << (l % 64), Relaxed);
                }
            }
            starts[n] = w;
            Span::set(&raw const p.starts, starts.as_ptr(), n + 1);
            Span::set(&raw const p.vals, vals.as_ptr(), w as usize);
            p.rows.store(w, Relaxed);
        }
        Some(p)
    }

    /// Adds node `l` to a `SET`: whether it is new. Any thread.
    #[inline]
    pub fn add(&self, l: u32) -> bool {
        debug_assert_eq!(self.form, SET);
        let new = atomic::<u64, AtomicU64>(&self.bits)[(l / 64) as usize].fetch_or(1 << (l % 64), Relaxed) & (1 << (l % 64)) == 0;
        if new {
            self.rows.fetch_add(1, Relaxed);
        }
        new
    }

    /// Gives node `l` value `v` in a `ONE`: `Ok(true)` when new, `Ok(false)` when it had `v`, `Broken` when it had
    /// another (the key is broken: the relation needs another form). Any thread.
    #[inline]
    pub fn put(&self, l: u32, v: u32) -> Result<bool, Broken> {
        debug_assert_eq!(self.form, ONE);
        debug_assert_ne!(v, NONE);
        let slot = &atomic::<u32, AtomicU32>(&self.one)[l as usize];
        match slot.compare_exchange(NONE, v, Relaxed, Relaxed) {
            Ok(_) => {
                atomic::<u64, AtomicU64>(&self.bits)[(l / 64) as usize].fetch_or(1 << (l % 64), Relaxed);
                self.rows.fetch_add(1, Relaxed);
                Ok(true)
            }
            Err(had) if had == v => Ok(false),
            Err(had) => {
                let _ = self.broke.compare_exchange(0, l + 1, Relaxed, Relaxed);
                Err(Broken { node: l, had, value: v })
            }
        }
    }

    /// Whether node `l` is held (`SET`), or has a value (`ONE`, `FAN`): one load.
    #[inline(always)]
    pub fn has(&self, l: u32) -> bool {
        self.bits.as_slice()[(l / 64) as usize] >> (l % 64) & 1 != 0
    }

    /// Node `l`'s value (`ONE`): one load.
    #[inline(always)]
    pub fn one(&self, l: u32) -> Option<u32> {
        let v = self.one.as_slice()[l as usize];
        (v != NONE).then_some(v)
    }

    /// Node `l`'s values (`FAN`): two loads and the run.
    #[inline(always)]
    pub fn each(&self, l: u32) -> &[u32] {
        let s = self.starts.as_slice();
        &self.vals.as_slice()[s[l as usize] as usize..s[l as usize + 1] as usize]
    }

    /// Its rows: nodes held (`SET`, `ONE`) or pairs (`FAN`).
    pub fn len(&self) -> u32 {
        self.rows.load(Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The node whose key a writer broke (`ONE`), if one did.
    pub fn broken(&self) -> Option<u32> {
        self.broke.load(Relaxed).checked_sub(1)
    }

    /// The bytes it takes in its block, the descriptor's included.
    pub fn bytes(&self) -> usize {
        size_of::<NodeKeyed>() + self.bits.len() * 8 + self.one.len() * 4 + self.starts.len() * 4 + self.vals.len() * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;
    use crate::relation::{RelW, Relation, Spec};
    use std::sync::Mutex;

    /// A small generator: the same rows each run.
    fn lcg(s: &mut u64) -> u32 {
        *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*s >> 33) as u32
    }

    /// The rows of a frozen relation of `arity` columns with an index on column 0, as the engine holds a derived
    /// relation its readers probe by node.
    fn relation<'b>(block: &'b Block, arity: usize, rows: &[i64]) -> (&'b Relation, usize) {
        let lock = Mutex::new(());
        let spec = Spec { widths: vec![8; arity], order: Vec::new(), indexes: Vec::new(), min_log: 4 };
        let rel = Relation::create(block, &spec, None);
        RelW { block, rel, lock: &lock }.extend(rows);
        let n = rel.rows.len();
        // SAFETY: one thread; the rows are written.
        let room = unsafe { rel.try_reserve_index(block, &[0], n) }.expect("room");
        unsafe {
            rel.build_index(&room);
            rel.index_built(&room);
        }
        (rel, rel.find_index(&[0], n).expect("built"))
    }

    /// The relation's rows for node `l` by its index: the probe a step makes today.
    fn probe(rel: &Relation, ix: usize, l: u32) -> impl Iterator<Item = u32> + '_ {
        let k = [i64::from(l)];
        rel.group(ix, crate::key_hash(&k)).iter().copied().filter(move |&r| rel.rows.get(r, 0) == k[0])
    }

    /// A `SET` holds exactly the nodes added, each counted once, and answers as the relation's index does.
    #[test]
    fn a_set_answers_as_the_relations_probe() {
        let block = Block::anon(kind::FACTS, 64 << 20);
        let nodes = 10_000u32;
        let set = NodeKeyed::try_set(&block, nodes).expect("room");
        let mut s = 7u64;
        let held: Vec<u32> = (0..3_000).map(|_| lcg(&mut s) % nodes).collect();
        let fresh = held.iter().filter(|&&l| set.add(l)).count();
        let (rel, ix) = relation(&block, 1, &held.iter().map(|&l| i64::from(l)).collect::<Vec<_>>());
        assert_eq!(set.len() as usize, fresh);
        for l in 0..nodes {
            assert_eq!(set.has(l), probe(rel, ix, l).next().is_some(), "node {l}");
        }
    }

    /// The relation's own node-keyed index (`Relation::try_add_node_bits`, decision 0694 step 1), over nodes of several
    /// documents, answers every test as its hashed index does, values of no document, slot or word included, in
    /// either of a document's forms (bits, or its sorted lows); it is found by its column and rows, not by
    /// `find_index`.
    #[test]
    fn a_relations_node_bits_answer_as_its_hashed_index() {
        let block = Block::anon(kind::FACTS, 64 << 20);
        let mut s = 11u64;
        let node = |doc: u32, l: u32| (i64::from(doc) << 32) | i64::from(l);
        // Ids as the engine's are (`packed::ID_SPLIT`): a granule's slot, then an index in it.
        let id = |slot: u32, x: u32| slot << crate::packed::ID_SPLIT | x;
        let held: Vec<i64> = (0..4_000).map(|_| { let d = lcg(&mut s) % 7 * 3; node(d, id(3 + lcg(&mut s) % 40 * 2, 5 + lcg(&mut s) % 300)) }).collect();
        let (rel, ix) = relation(&block, 1, &held);
        let n = rel.rows.len();
        // SAFETY: one thread; the rows are written.
        let bits = unsafe { rel.try_add_node_bits(&block, 0, n) }.expect("dense enough");
        assert_eq!(rel.find_node_bits(0, n), Some(bits));
        assert_eq!(rel.find_node_bits(0, n + 1), None);
        assert_eq!(rel.find_index(&[0], n), Some(ix), "the hashed index still found as before");
        let mut hint = 0;
        for doc in 0..22 {
            for l in (0..90).flat_map(|slot| (0..330).map(move |x| id(slot, x))).chain([u32::MAX - 1, u32::MAX]) {
                let v = node(doc, l);
                let k = [v];
                let hashed = rel.group(ix, crate::key_hash(&k)).iter().any(|&r| rel.rows.get(r, 0) == v);
                assert_eq!(rel.node_bits_has(bits, v, &mut hint), hashed, "document {doc} node {l}");
            }
        }
        // A column of a few values a document, far apart: each document holds its lows, and answers as the hashed
        // index does.
        let sparse: Vec<i64> = (0..128).flat_map(|d| [node(d, id(d % 50, d * 997 % 200_000)), node(d, id(1_000 + d, 7))]).collect();
        let (rel, ix) = relation(&block, 1, &sparse);
        // SAFETY: as above.
        let bits = unsafe { rel.try_add_node_bits(&block, 0, rel.rows.len()) }.expect("room");
        for doc in 0..130 {
            for l in [id(doc % 50, doc * 997 % 200_000), id(1_000 + doc, 7), id(1_000 + doc, 8), id(doc % 50, 0), 0] {
                let v = node(doc, l);
                let hashed = rel.group(ix, crate::key_hash(&[v])).iter().any(|&r| rel.rows.get(r, 0) == v);
                assert_eq!(rel.node_bits_has(bits, v, &mut hint), hashed, "document {doc} id {l:#x}");
            }
        }
    }

    /// A `ONE` gives each node its value; the same value again is no new row, another breaks the key and says where.
    #[test]
    fn a_one_keeps_a_value_a_node_and_reports_a_broken_key() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let one = NodeKeyed::try_one(&block, 100).expect("room");
        assert_eq!(one.put(5, 40), Ok(true));
        assert_eq!(one.put(5, 40), Ok(false));
        assert_eq!(one.put(9, 0), Ok(true));
        assert_eq!((one.one(5), one.one(9), one.one(6)), (Some(40), Some(0), None));
        assert!(one.has(9) && !one.has(6));
        assert_eq!(one.broken(), None);
        assert_eq!(one.put(5, 41), Err(Broken { node: 5, had: 40, value: 41 }));
        assert_eq!(one.broken(), Some(5));
        assert_eq!((one.len(), one.one(5)), (2, Some(40)));
    }

    /// A `FAN` holds each node's values, repeats dropped, in the order given, and answers as the relation's index.
    #[test]
    fn a_fan_groups_each_nodes_values() {
        let block = Block::anon(kind::FACTS, 64 << 20);
        let nodes = 2_000u32;
        let mut s = 3u64;
        let pairs: Vec<(u32, u32)> = (0..6_000).map(|_| (lcg(&mut s) % nodes, lcg(&mut s) % 8)).collect();
        let fan = NodeKeyed::try_fan(&block, nodes, &pairs).expect("room");
        let (rel, ix) = relation(&block, 2, &pairs.iter().flat_map(|&(l, v)| [i64::from(l), i64::from(v)]).collect::<Vec<_>>());
        let mut total = 0;
        for l in 0..nodes {
            let mut want: Vec<u32> = Vec::new();
            for &(k, v) in &pairs {
                if k == l && !want.contains(&v) {
                    want.push(v);
                }
            }
            let mut by_rel: Vec<u32> = probe(rel, ix, l).map(|r| rel.rows.get(r, 1) as u32).collect();
            by_rel.sort_unstable();
            by_rel.dedup();
            let mut got = fan.each(l).to_vec();
            assert_eq!(got, want, "node {l}: in the order given");
            got.sort_unstable();
            assert_eq!(got, by_rel, "node {l}: the relation's");
            assert_eq!(fan.has(l), !want.is_empty());
            total += want.len();
        }
        assert_eq!(fan.len() as usize, total);
    }

    /// Many threads add to one `SET` and put to one `ONE` at once: every node counted once.
    #[test]
    fn writers_on_many_threads_count_each_node_once() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (set, one) = (NodeKeyed::try_set(&block, 4_096).expect("room"), NodeKeyed::try_one(&block, 4_096).expect("room"));
        std::thread::scope(|sc| {
            for t in 0..4u32 {
                sc.spawn(move || {
                    for l in (0..4_096u32).filter(|l| l % 4 == t || l % 3 == 0) {
                        set.add(l);
                        let _ = one.put(l, l * 2);
                    }
                });
            }
        });
        assert_eq!((set.len(), one.len()), (4_096, 4_096));
        assert!((0..4_096u32).all(|l| set.has(l) && one.one(l) == Some(l * 2)));
    }

    /// What a test of a bound node costs, by form: the relation's index probe against the column's load, over the same
    /// rows, read in a random order of nodes (a name read's nodes, which a step reads as its rows come). Prints the
    /// ns a test and the bytes; checks that both answer the same. A measurement: `--ignored`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn what_a_test_of_a_bound_node_costs() {
        let block = Block::anon(kind::FACTS, 512 << 20);
        let nodes = 200_000u32;
        let mut s = 11u64;
        let probes: Vec<u32> = (0..2_000_000).map(|_| lcg(&mut s) % nodes).collect();
        let time = |f: &dyn Fn() -> u64| {
            let t = std::time::Instant::now();
            let n = std::hint::black_box(f());
            (t.elapsed().as_nanos() as f64 / probes.len() as f64, n)
        };
        // SET, a quarter of the nodes (`bind_only`, `rr_ast_8`).
        let set = NodeKeyed::try_set(&block, nodes).expect("room");
        let held: Vec<i64> = (0..nodes).filter(|_| lcg(&mut s) % 4 == 0).map(|l| { set.add(l); i64::from(l) }).collect();
        let (rel, ix) = relation(&block, 1, &held);
        let (rel_ns, a) = time(&|| probes.iter().filter(|&&l| probe(rel, ix, l).next().is_some()).count() as u64);
        let (col_ns, b) = time(&|| probes.iter().filter(|&&l| set.has(l)).count() as u64);
        assert_eq!(a, b);
        println!("SET  relation {rel_ns:6.1} ns  column {col_ns:6.1} ns  ({:.1}x)  bytes {} against {}", rel_ns / col_ns, rel.bytes(), set.bytes());
        // ONE, two thirds of the nodes (`wstart(n, s)`): the value read too.
        let one = NodeKeyed::try_one(&block, nodes).expect("room");
        let mut rows: Vec<i64> = Vec::new();
        for l in 0..nodes {
            let (keep, v) = (lcg(&mut s) % 3 != 0, lcg(&mut s) % nodes);
            if keep {
                one.put(l, v).unwrap();
                rows.extend([i64::from(l), i64::from(v)]);
            }
        }
        let (rel, ix) = relation(&block, 2, &rows);
        let (rel_ns, a) = time(&|| probes.iter().filter_map(|&l| probe(rel, ix, l).next().map(|r| rel.rows.get(r, 1) as u64)).sum());
        let (col_ns, b) = time(&|| probes.iter().filter_map(|&l| one.one(l).map(u64::from)).sum());
        assert_eq!(a, b);
        println!("ONE  relation {rel_ns:6.1} ns  column {col_ns:6.1} ns  ({:.1}x)  bytes {} against {}", rel_ns / col_ns, rel.bytes(), one.bytes());
    }
}
