//! Step 4 of *Flat Views and Lifecycles*: each final form's need is exact; what it settles reads back equal to the
//! working form it came from; no reference in it points out of its room (checked by its spans' targets, and by
//! copying the room alone elsewhere and reading it there).

extern crate std;

use super::arena::{Arena, Bucket, HEAD};
use super::finals::{BloomFrom, Csr, CsrFrom, IntFrom, IntTable, Sorted, SortedFrom, Strings, StringsFrom};
use super::map::Map;
use super::pool::Chain;
use super::settle::Settle;
use super::Region;
use crate::bloom::{self, Bloom};
use crate::col::{CONST, DICT, GROUPS, LINE, PACK, RUNS};
use std::boxed::Box;
use std::collections::BTreeMap;
use std::vec;
use std::vec::Vec;

#[repr(C, align(64))]
struct Buf([u8; 4 << 20]);

fn buf() -> Box<Buf> {
    let mut b: Box<Buf> = Box::new(Buf([0; 4 << 20]));
    b.0.iter_mut().enumerate().for_each(|(i, x)| *x = (i * 89 % 241) as u8);
    b
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Settles `s` into a fresh out arena over `b`: the final form's offset in the region, and its room's bytes, which
/// must be its need exactly.
fn settle_alone<S: Settle>(b: &mut Buf, s: &S) -> (usize, usize) {
    let out = Arena::init(Region::of(&mut b.0), Bucket::Out).unwrap();
    let need = s.need().bytes;
    let f = out.settle(0, s).unwrap();
    assert_eq!(out.used(), need, "the room taken is the need, exactly");
    let at = core::ptr::from_ref(f).addr() - out.region().addr();
    assert_eq!(at, HEAD, "the descriptor first in its room");
    (at, need)
}

/// The room `at..at + len` of `from`, copied alone to a different place in `to` (at another 64-byte boundary).
fn moved(from: &Buf, to: &mut Buf, at: usize, len: usize) -> usize {
    let dst = 4096 + 64 * 3;
    to.0[dst..dst + len].copy_from_slice(&from.0[at..at + len]);
    dst
}

/// Whether bit address `b` lies in the room `at..at + len` of the buffer at `base`.
fn inside(base: usize, at: usize, len: usize, b: Option<u64>) -> bool {
    b.is_none_or(|b| b / 8 >= (base + at) as u64 && b / 8 < (base + at + len) as u64)
}

fn columns() -> Vec<(u32, Vec<i64>)> {
    let mut r = Rng(17);
    let n = 5000;
    vec![
        (CONST, vec![42; n]),
        (PACK, (0..n).map(|_| (r.next() % 1_000_000) as i64).collect()),
        (RUNS, (0..n).map(|i| (i / 1000) as i64 * 7).collect()),
        (LINE, (0..n).map(|i| i as i64 * 1000 + (r.next() % 4) as i64).collect()),
        (GROUPS, (0..n).map(|i| (i as i64 / 128) * 1_000_000 + (i % 128) as i64).collect()),
        (DICT, (0..n).map(|i| [-5_000_000_000i64, 3, 9_000_000_000_000, -1][i % 4]).collect()),
    ]
}

#[test]
fn an_int_table_settles_every_encoding_exactly() {
    for (tag, vals) in columns() {
        let mut wb = buf();
        let work = Arena::init(Region::of(&mut wb.0), Bucket::Phase).unwrap();
        let plan = IntFrom::plan(&vals[..], Some(&work), 1).unwrap();
        assert_eq!(plan.choice().tag, tag, "the column made for tag {tag} takes it");
        let (mut a, mut b) = (buf(), buf());
        let (at, len) = settle_alone(&mut a, &plan);
        let t: &IntTable = unsafe { &*a.0.as_ptr().add(at).cast() };
        for (i, &v) in vals.iter().enumerate() {
            assert_eq!(t.get(i), v, "tag {tag}, value {i}");
        }
        assert!(inside(a.0.as_ptr().addr(), at, len, t.body.target_bits()), "a body inside its room");
        let dst = moved(&a, &mut b, at, len);
        let m: &IntTable = unsafe { &*b.0.as_ptr().add(dst).cast() };
        assert_eq!(m.len(), vals.len());
        assert!((0..vals.len()).all(|i| m.get(i) == vals[i]), "tag {tag}: a moved table reads the same");
    }
}

#[test]
fn strings_settle_and_read_back() {
    let words = ["", "a", "flat", "views", "", "and", "lifecycles", "settled", "exactly", "in", "their", "rooms"];
    let (mut bytes, mut ends) = (Vec::new(), Vec::new());
    for w in words.iter().cycle().take(3000) {
        bytes.extend_from_slice(w.as_bytes());
        ends.push(bytes.len() as u32);
    }
    let plan = StringsFrom::plan(&bytes, &ends);
    let (mut a, mut b) = (buf(), buf());
    let (at, len) = settle_alone(&mut a, &plan);
    let base = a.0.as_ptr().addr();
    let s: &Strings = unsafe { &*a.0.as_ptr().add(at).cast() };
    assert_eq!(s.len(), 3000);
    for (i, w) in words.iter().cycle().take(3000).enumerate() {
        assert_eq!(s.get(i), w.as_bytes());
    }
    assert!(inside(base, at, len, s.ends.body.target_bits()) && inside(base, at, len, s.bytes.target_bits()));
    let dst = moved(&a, &mut b, at, len);
    let m: &Strings = unsafe { &*b.0.as_ptr().add(dst).cast() };
    assert!(words.iter().cycle().take(3000).enumerate().all(|(i, w)| m.get(i) == w.as_bytes()));
}

#[test]
fn a_map_settles_sorted() {
    let mut wb = buf();
    let work = Arena::init(Region::of(&mut wb.0), Bucket::Launch).unwrap();
    let m = Map::<u64, u32>::make(&work, 1, 3000, u64::MAX, 0).unwrap();
    let mut model = BTreeMap::new();
    let mut r = Rng(23);
    for i in 0..3000u32 {
        let k = r.next() % 1_000_000;
        m.set(k, i).unwrap();
        model.insert(k, i);
    }
    let plan = SortedFrom(&*m);
    let (mut a, mut b) = (buf(), buf());
    let (at, len) = settle_alone(&mut a, &plan);
    let base = a.0.as_ptr().addr();
    let s: &Sorted<u64, u32> = unsafe { &*a.0.as_ptr().add(at).cast() };
    assert_eq!(s.keys(), &model.keys().copied().collect::<Vec<_>>()[..]);
    assert_eq!(s.vals(), &model.values().copied().collect::<Vec<_>>()[..]);
    for (&k, &v) in &model {
        assert_eq!(s.get(k), Some(v));
    }
    assert_eq!(s.get(1_000_001), None);
    assert!(inside(base, at, len, Some(s.keys().as_ptr().addr() as u64 * 8)) && inside(base, at, len, Some(s.vals().as_ptr().addr() as u64 * 8)));
    let dst = moved(&a, &mut b, at, len);
    let t: &Sorted<u64, u32> = unsafe { &*b.0.as_ptr().add(dst).cast() };
    assert_eq!(t.keys(), s.keys());
    assert_eq!(t.vals(), s.vals());
}

#[test]
fn chains_settle_as_a_csr() {
    let mut wb = buf();
    let work = Arena::init(Region::of(&mut wb.0), Bucket::Launch).unwrap();
    let c = Chain::<u32>::make(&work, 1, 5000).unwrap();
    let mut heads = vec![0u32; 50];
    let mut model: Vec<Vec<u32>> = vec![Vec::new(); 50];
    let mut r = Rng(29);
    for i in 0..4000u32 {
        let k = (r.next() % 50) as usize;
        heads[k] = c.cons(heads[k], i).unwrap();
        model[k].insert(0, i);
    }
    let read = c.read();
    let plan = CsrFrom { lists: &read, heads: &heads };
    let (mut a, mut b) = (buf(), buf());
    let (at, len) = settle_alone(&mut a, &plan);
    let x: &Csr<u32> = unsafe { &*a.0.as_ptr().add(at).cast() };
    assert_eq!(x.groups(), 50);
    for (k, items) in model.iter().enumerate() {
        assert_eq!(x.group(k), &items[..]);
    }
    let base = a.0.as_ptr().addr();
    assert!(inside(base, at, len, Some(x.items().as_ptr().addr() as u64 * 8)));
    let dst = moved(&a, &mut b, at, len);
    let y: &Csr<u32> = unsafe { &*b.0.as_ptr().add(dst).cast() };
    assert!(model.iter().enumerate().all(|(k, items)| y.group(k) == &items[..]));
}

#[test]
fn keys_settle_as_a_bloom_filter() {
    let keys: Vec<i64> = (0..2000).map(|i| i * 7919 - 1_000_000).collect();
    let plan = BloomFrom { keys: &keys, bits_per_key: 10 };
    let (mut a, mut b) = (buf(), buf());
    let (at, len) = settle_alone(&mut a, &plan);
    let f: &Bloom = unsafe { &*a.0.as_ptr().add(at).cast() };
    assert_eq!(f.len(), (2000usize * 10).div_ceil(64).next_power_of_two());
    assert!(keys.iter().all(|&k| f.may_hold_hashed(bloom::hash(k))), "a filter holds every key it was made of");
    let others = (0..20_000).filter(|i| f.may_hold_hashed(bloom::hash(i * 7919 + 13))).count();
    assert!(others < 20_000 / 10, "a filter of 10 bits a key refuses most others: {others} of 20000 taken");
    let dst = moved(&a, &mut b, at, len);
    let g: &Bloom = unsafe { &*b.0.as_ptr().add(dst).cast() };
    assert!(keys.iter().all(|&k| g.may_hold_hashed(bloom::hash(k))));
    assert_eq!((0..f.len()).map(|i| f.word(i)).collect::<Vec<_>>(), (0..g.len()).map(|i| g.word(i)).collect::<Vec<_>>());
}

#[test]
fn an_out_arena_too_small_for_a_need_says_so() {
    let vals: Vec<i64> = (0..10_000).map(|i| i * i).collect();
    let plan = IntFrom::plan(&vals[..], None, 0).unwrap();
    let mut a = buf();
    let small = unsafe { Region::from_raw(a.0.as_mut_ptr(), HEAD + 256) };
    let out = Arena::init(small, Bucket::Out).unwrap();
    let f = out.settle(5, &plan).err().expect("a need past the room");
    assert_eq!(f.part, 5);
    assert!(f.wanted as usize >= plan.need().bytes);
}
