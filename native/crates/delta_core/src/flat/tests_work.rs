//! Step 3 of *Flat Views and Lifecycles*: the working forms against std's (a `Vec`, a `HashMap`, lists), shared
//! claims from many threads, a shared map giving the same contents in any order, and each form's room running out
//! as a value.

extern crate std;

use super::arena::{Arena, Bucket};
use super::grow::Grow;
use super::map::{Map, Stamped};
use super::pool::{Chain, Pool};
use super::stack::Stack;
use super::Region;
use core::sync::atomic::AtomicU32;
use std::boxed::Box;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::vec;
use std::vec::Vec;

#[repr(C, align(64))]
struct Buf([u8; 4 << 20]);

fn buf() -> Box<Buf> {
    // Not zeroed by the arena: whatever bytes are there, every form writes before it reads.
    let mut b: Box<Buf> = Box::new(Buf([0; 4 << 20]));
    b.0.iter_mut().enumerate().for_each(|(i, x)| *x = (i * 131 % 251) as u8);
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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn a_grow_against_a_vec() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let g = Grow::<u64>::make(&a, 1, 0).unwrap();
    let mut model = Vec::new();
    let mut r = Rng(7);
    for _ in 0..20_000 {
        if r.below(3) == 0 {
            assert_eq!(g.pop(), model.pop());
        } else {
            let v = r.next();
            g.push(&a, v).unwrap();
            model.push(v);
        }
        assert_eq!(g.len(), model.len());
    }
    assert_eq!(g.as_slice(), &model[..]);
    assert!(g.cap() >= model.len());
    g.as_mut_slice().reverse();
    model.reverse();
    assert_eq!(g.as_slice(), &model[..]);
}

/// The room a vector leaves when it moves is its spare (`lp_spare`: the larger kept), and its owner gives it to
/// another vector, whose growth takes it instead of the arena's room.
#[test]
fn a_grow_keeps_the_room_it_leaves_as_a_spare_and_gives_it_on() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let g = Grow::<u32>::make(&a, 12, 8).unwrap();
    assert_eq!(g.spare(), 0);
    for i in 0..9 {
        g.push(&a, i).unwrap();
    }
    assert_eq!((g.cap(), g.spare()), (16, 32), "8 items' room left, kept");
    for i in 9..17 {
        g.push(&a, i).unwrap();
    }
    assert_eq!((g.cap(), g.spare()), (32, 64), "the larger room left, kept");
    let h = Grow::<u32>::make(&a, 13, 0).unwrap();
    g.give_spare(&a, h);
    assert_eq!((g.spare(), h.spare()), (0, 64));
    let used = a.used();
    for i in 0..16 {
        h.push(&a, 100 + i).unwrap();
    }
    assert_eq!(a.used(), used, "its growth took the spare, not the arena's room");
    assert_eq!((h.cap(), h.spare()), (16, 0));
    assert_eq!(h.as_slice(), &(100..116).collect::<Vec<u32>>()[..]);
    assert_eq!(g.as_slice(), &(0..17).collect::<Vec<u32>>()[..]);
    h.push(&a, 116).unwrap();
    assert!(a.used() > used && h.cap() == 32, "past the spare, the arena's room");
    assert_eq!(h.as_slice(), &(100..117).collect::<Vec<u32>>()[..]);
}

#[test]
fn shared_claims_on_a_grow_fit_or_say_what_was_wanted() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let g = Grow::<u32>::make(&a, 2, 1000).unwrap();
    // Eight lanes claim 3 items 100 times each: 2400 wanted, 1000 fit.
    let fails = AtomicU32::new(0);
    std::thread::scope(|s| {
        for t in 0..8u32 {
            let (g, fails) = (&*g, &fails);
            s.spawn(move || {
                for k in 0..100u32 {
                    match g.claim(3) {
                        Ok(room) => {
                            for (j, x) in room.iter_mut().enumerate() {
                                x.write(t << 24 | k << 8 | j as u32);
                            }
                        }
                        Err(f) => {
                            assert_eq!(f.part, 2);
                            fails.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    assert_eq!(g.wanted(), 2400);
    let fit = g.len();
    assert!(fit <= 1000 && fit.is_multiple_of(3), "the claims that fit are whole");
    assert_eq!(fit / 3 + fails.load(core::sync::atomic::Ordering::Relaxed) as usize, 800);
    // Every item that fit was written by exactly one claim.
    let items: BTreeSet<u32> = g.as_slice().iter().copied().collect();
    assert_eq!(items.len(), fit);
    // The owner grows it to what was wanted; the failed claims run again and fit.
    let again = 800 - fit / 3;
    g.reserve(&a, again * 3).unwrap();
    for _ in 0..again {
        g.claim(3).unwrap();
    }
    assert_eq!(g.len(), 2400);
}

#[test]
fn a_pool_against_a_set_of_live_items() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let p = Pool::<u64>::make(&a, 3, 500).unwrap();
    let mut live: BTreeMap<u32, u64> = BTreeMap::new();
    let mut r = Rng(11);
    for _ in 0..20_000 {
        if live.len() < 500 && (live.is_empty() || r.below(2) == 0) {
            let v = r.next();
            let (i, room) = p.take().unwrap();
            room.write(v);
            assert!(live.insert(i, v).is_none(), "item {i} taken twice");
        } else {
            let k = *live.keys().nth(r.below(live.len() as u64) as usize).unwrap();
            let v = live.remove(&k).unwrap();
            assert_eq!(*p.get(k), v);
            p.give(k);
        }
        assert_eq!(p.live(), live.len());
    }
    let read = p.read();
    for (&i, &v) in &live {
        assert_eq!(*read.get(i), v);
    }
}

#[test]
fn takes_from_many_threads_are_distinct_and_run_out_as_a_value() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let p = Pool::<u32>::make(&a, 4, 1000).unwrap();
    let got: Vec<Vec<u32>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let p = &*p;
                s.spawn(move || (0..200).filter_map(|_| p.take().ok().map(|(i, room)| { room.write(i); i })).collect::<Vec<_>>())
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let all: BTreeSet<u32> = got.iter().flatten().copied().collect();
    assert_eq!(all.len(), 1000, "every item taken once");
    assert_eq!(all, (1..=1000).collect());
    assert_eq!(p.wanted(), 1600);
    let f = p.take().err().unwrap();
    assert_eq!(f.part, 4);
    // Given back, an item is taken again.
    p.settle();
    p.give(17);
    assert_eq!(p.take().unwrap().0, 17);
}

#[test]
fn chains_with_shared_tails_against_lists() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let c = Chain::<u32>::make(&a, 5, 4000).unwrap();
    let mut lists: Vec<(u32, Vec<u32>)> = vec![(0, Vec::new())];
    let mut r = Rng(3);
    for n in 0..3000u32 {
        // Cons onto any list made so far: tails are shared.
        let (tail, items) = lists[r.below(lists.len() as u64) as usize].clone();
        let l = c.cons(tail, n).unwrap();
        let mut m = vec![n];
        m.extend(items);
        lists.push((l, m));
    }
    let read = c.read();
    for (l, items) in &lists {
        assert_eq!(read.walk(*l).copied().collect::<Vec<_>>(), *items);
    }
}

#[test]
fn pushes_onto_one_list_from_many_threads_keep_every_item() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let c = Chain::<u32>::make(&a, 6, 4000).unwrap();
    let head = AtomicU32::new(0);
    std::thread::scope(|s| {
        for t in 0..8u32 {
            let (c, head) = (&*c, &head);
            s.spawn(move || {
                for k in 0..500 {
                    c.push(head, t * 1000 + k).unwrap();
                }
            });
        }
    });
    let read = c.read();
    let items: Vec<u32> = read.walk(head.load(core::sync::atomic::Ordering::Relaxed)).copied().collect();
    let set: BTreeSet<u32> = items.iter().copied().collect();
    assert_eq!(items.len(), 4000);
    assert_eq!(set, (0..8).flat_map(|t| (0..500).map(move |k| t * 1000 + k)).collect());
    // Each thread's items in its order, last pushed first.
    for t in 0..8 {
        let mine: Vec<u32> = items.iter().copied().filter(|v| v / 1000 == t).collect();
        assert!(mine.windows(2).all(|w| w[0] > w[1]));
    }
}

#[test]
fn a_map_against_a_hashmap() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let m = Map::<u32, u32>::make(&a, 7, 3000, u32::MAX, 0).unwrap();
    let mut model: HashMap<u32, u32> = HashMap::new();
    let mut r = Rng(5);
    for _ in 0..30_000 {
        let k = r.below(4000) as u32;
        match r.below(3) {
            0 => assert_eq!(m.remove(k), model.remove(&k)),
            1 => assert_eq!(m.get(k), model.get(&k).copied()),
            _ => {
                if model.len() < 3000 || model.contains_key(&k) {
                    let v = (r.next() >> 33) as u32;
                    let c = m.set(k, v).unwrap();
                    assert_eq!(c.fresh, model.insert(k, v).is_none());
                }
            }
        }
        assert_eq!(m.len(), model.len());
    }
    let mut got: Vec<(u32, u32)> = m.iter().collect();
    let mut want: Vec<(u32, u32)> = model.into_iter().collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

/// The design's dedup: a row's key is its two columns, wider than a word, so the map's keys are row indexes, hashed and
/// compared by the rows' columns (`claim_min_by`), and each key keeps its least row. Eight lanes in an order the seed
/// decides; the contents read by the rows' keys.
fn shared_least(seed: u64) -> Vec<([u32; 2], u32)> {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let m = Map::<u32, u32>::make(&a, 8, 1000, u32::MAX, u32::MAX).unwrap();
    let keys: Vec<[u32; 2]> = (0..8000u32).map(|i| [i % 997, i % 997 % 13]).collect();
    let hash = |k: [u32; 2]| super::map::hash_word(((k[1] as i64) << 32) | k[0] as i64);
    let mut order: Vec<u32> = (0..8000).collect();
    let mut r = Rng(seed);
    for i in (1..order.len()).rev() {
        order.swap(i, r.below(i as u64 + 1) as usize);
    }
    std::thread::scope(|s| {
        for part in order.chunks(1000) {
            let (m, keys) = (&*m, &keys);
            s.spawn(move || {
                for &i in part {
                    let k = keys[i as usize];
                    m.claim_min_by(i, hash(k), |j| keys[j as usize] == k, i).unwrap();
                }
            });
        }
    });
    let mut got: Vec<_> = m.iter().map(|(j, v)| (keys[j as usize], v)).collect();
    got.sort();
    got
}

#[test]
fn a_shared_map_gives_the_same_contents_in_any_order() {
    let one = shared_least(1);
    for seed in [2, 3, 99] {
        assert_eq!(shared_least(seed), one);
    }
    // And they are the least row of each key.
    let mut least: BTreeMap<[u32; 2], u32> = BTreeMap::new();
    for i in 0..8000u32 {
        least.entry([i % 997, i % 997 % 13]).and_modify(|v| *v = (*v).min(i)).or_insert(i);
    }
    assert_eq!(one, least.into_iter().collect::<Vec<_>>());
}

#[test]
fn a_full_map_says_so_and_a_rehash_moves_it() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let m = Map::<u32, u32>::make(&a, 9, 4, u32::MAX, 0).unwrap();
    assert_eq!(m.slots(), 8);
    let mut placed = Vec::new();
    let mut full = None;
    for k in 0..20u32 {
        match m.set(k, k * 10) {
            Ok(_) => placed.push(k),
            Err(f) => {
                full = Some(f);
                break;
            }
        }
    }
    let f = full.expect("a map of 8 slots refuses a ninth key");
    assert_eq!(f.part, 9);
    assert_eq!(placed.len(), 8);
    let big = Map::<u32, u32>::make(&a, 9, 100, u32::MAX, 0).unwrap();
    m.rehash_into(big, 0, 4).unwrap();
    m.rehash_into(big, 4, 8).unwrap();
    for &k in &placed {
        assert_eq!(big.get(k), Some(k * 10));
    }
    assert_eq!(big.len(), 8);
}

#[test]
fn a_stamped_table_is_emptied_by_its_rounds() {
    let mut b = buf();
    let mut a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let at = {
        let t = Stamped::<u32, u32>::make(&a, 10, 100).unwrap();
        for k in 0..100 {
            let c = t.claim(&a, k).unwrap();
            assert!(c.fresh);
            t.set_at(c.slot, k + 1);
        }
        assert!(!t.claim(&a, 5).unwrap().fresh);
        assert_eq!(t.get(&a, 42), Some(43));
        assert_eq!(t.live(&a), 100);
        core::ptr::from_ref(t).addr() - a.region().addr()
    };
    // Everything claimed after the table is given back; the table, kept, reads empty in the new round.
    let keep = a.mark();
    a.next_round(keep);
    let t: &Stamped<u32, u32> = a.region().view(at).unwrap();
    assert_eq!(t.live(&a), 0);
    assert_eq!(t.get(&a, 42), None);
    let c = t.claim(&a, 42).unwrap();
    assert!(c.fresh, "a stale slot is claimed again");
    t.set_at(c.slot, 7);
    assert_eq!(t.get(&a, 42), Some(7));
    assert_eq!(t.live(&a), 1);
}

#[test]
fn a_stack_against_a_vec() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let spill = Grow::<u64>::make(&a, 11, 0).unwrap();
    let mut s: Stack<'_, '_, u64, 4> = Stack::new(spill, &a);
    let mut model = Vec::new();
    let mut r = Rng(13);
    let mut most = 0;
    for _ in 0..20_000 {
        if r.below(5) < 2 {
            assert_eq!(s.pop(), model.pop());
        } else {
            let v = r.next();
            s.push(v).unwrap();
            model.push(v);
        }
        assert_eq!(s.top(), model.last());
        assert_eq!(s.len(), model.len());
        most = most.max(s.spilled());
    }
    assert!(most > 100, "the stack spilled");
    while let Some(v) = model.pop() {
        assert_eq!(s.pop(), Some(v));
    }
    assert_eq!(s.pop(), None);
}
