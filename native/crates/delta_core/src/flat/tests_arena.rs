//! Step 2 of *Flat Views and Lifecycles*: claims from many threads never overlap; a full arena reports its first
//! failure and everything wanted; room comes back whole, to a mark, and a round moves on without clearing.

extern crate std;

use super::arena::{Arena, Bucket, Full, GRAIN, HEAD};
use super::{Refusal, Region};
use std::boxed::Box;
use std::vec::Vec;

#[repr(C, align(64))]
struct Buf([u8; 1 << 20]);

fn buf() -> Box<Buf> {
    Box::new(Buf([0; 1 << 20]))
}

#[test]
fn claims_from_many_threads_never_overlap() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let threads = 8u32;
    let claims: Vec<Vec<(usize, usize, u8)>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let a = &a;
                s.spawn(move || {
                    let mut mine = Vec::new();
                    let mut x = 0x2545_f491_4f6c_dd1du64 ^ t as u64;
                    for k in 0..400u32 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let n = 1 + (x % 200) as usize;
                        let align = [1usize, 4, 8, 16, 64][(x >> 20) as usize % 5];
                        let at = a.claim_at(t, n, align).unwrap();
                        assert!((a.region().addr() + at).is_multiple_of(align), "aligned to {align}");
                        let tag = (t * 31 + k) as u8 | 1;
                        unsafe { core::ptr::write_bytes(a.region().ptr(at), tag, n) };
                        mine.push((at, n, tag));
                    }
                    mine
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut all: Vec<(usize, usize, u8)> = claims.into_iter().flatten().collect();
    all.sort();
    for w in all.windows(2) {
        assert!(w[0].0 + w[0].1 <= w[1].0, "claims {:?} and {:?} overlap", w[0], w[1]);
    }
    for &(at, n, tag) in &all {
        let got = unsafe { core::slice::from_raw_parts(a.region().ptr(at), n) };
        assert!(got.iter().all(|&v| v == tag), "a claim's bytes written over");
        assert!(at >= HEAD && at + n <= HEAD + a.cap());
    }
    assert!(a.failure().is_none());
}

#[test]
fn a_full_arena_reports_its_first_failure_and_what_was_wanted() {
    let mut b = buf();
    let region = unsafe { Region::from_raw(b.0.as_mut_ptr(), HEAD + 1024) };
    let a = Arena::init(region, Bucket::Phase).unwrap();
    assert_eq!(a.cap(), 1024);
    let first = a.claim::<u64>(1, 100).unwrap();
    assert_eq!(first.len(), 100);
    let Err(f) = a.claim::<u64>(2, 100) else { panic!("a claim past the room") };
    assert_eq!(f, Full { part: 2, need: 800, wanted: 1600 });
    let Err(g) = a.claim::<u32>(3, 10) else { panic!("a claim after the room ran out") };
    assert_eq!((g.part, g.need, g.wanted), (3, 40, 1640));
    // The first failure is kept, and everything asked for is counted.
    assert_eq!(a.failure(), Some(Full { part: 2, need: 800, wanted: 1640 }));
    assert_eq!(a.wanted(), 1640);
    assert_eq!(a.used(), 1024);
    // A claim too large to count at all.
    assert!(a.claim_at(4, usize::MAX / 2, 8).is_err());
}

#[test]
fn room_comes_back_whole_and_rounds_move_on() {
    let mut b = buf();
    let mut a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let r1 = a.round();
    let kept = a.claim_at(0, 64, 8).unwrap();
    let m = a.mark();
    let past = a.claim_at(1, 64, 8).unwrap();
    assert_eq!(past, kept + 64);
    let r2 = a.next_round(m);
    assert_eq!(r2, r1 + 1);
    assert_eq!(a.round(), r2);
    // What lay below the mark stays; the next claim takes the room past it again.
    assert_eq!(a.claim_at(1, 64, 8).unwrap(), past);
    a.reset();
    assert_eq!(a.used(), 0);
    assert_eq!(a.claim_at(0, 64, 8).unwrap(), kept);
    // A failure is forgotten by a reset.
    let _ = a.claim_at(9, 1 << 30, 8);
    assert!(a.failure().is_some());
    a.reset();
    assert!(a.failure().is_none());
}

#[test]
fn values_claimed_are_written_as_asked() {
    let mut b = buf();
    let a = Arena::init(Region::of(&mut b.0), Bucket::Phase).unwrap();
    let w = a.claim_with::<u32>(0, 5, 7).unwrap();
    assert_eq!(w, &[7; 5]);
    let c = a.claim_copy::<u16>(0, &[1, 2, 3]).unwrap();
    assert_eq!(c, &[1, 2, 3]);
    let d: &core::sync::atomic::AtomicU32 = a.put(0, core::sync::atomic::AtomicU32::new(11)).unwrap();
    d.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    assert_eq!(d.load(core::sync::atomic::Ordering::Relaxed), 12);
    // Every claim starts at a grain.
    assert!((a.region().addr() + a.claim_at(0, 1, 1).unwrap()).is_multiple_of(GRAIN));
}

#[test]
fn an_arena_is_made_only_where_it_fits_and_opened_as_it_lies() {
    let mut b = buf();
    let small = unsafe { Region::from_raw(b.0.as_mut_ptr(), HEAD - 1) };
    assert_eq!(Arena::init(small, Bucket::Phase).err(), Some(Refusal::Outside));
    let odd = unsafe { Region::from_raw(b.0.as_mut_ptr().add(4), 4096) };
    assert_eq!(Arena::init(odd, Bucket::Phase).err(), Some(Refusal::Misaligned));
    {
        let a = Arena::init(unsafe { Region::from_raw(b.0.as_mut_ptr(), 4096) }, Bucket::Out).unwrap();
        a.claim_at(0, 100, 8).unwrap();
    }
    let again = unsafe { Arena::open(Region::from_raw(b.0.as_mut_ptr(), 4096)) }.unwrap();
    assert_eq!(again.bucket(), Bucket::Out);
    assert_eq!(again.used(), 104);
    b.0[0] = 0;
    assert!(unsafe { Arena::open(Region::from_raw(b.0.as_mut_ptr(), 4096)) }.is_err());
}
