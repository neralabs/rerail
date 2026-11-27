//! *Flat Views and Lifecycles* step 7: the flat layer's working forms are one set of bytes on the host and the device.
//! `ukl/flat.h` claims, grows and reads them as `rerail_delta_core::flat` does, so a region made in Rust is worked
//! on by a kernel, and what the kernel wrote reads back through the Rust views. On the tests' device (the CPU's
//! vector path, `tests::device`).

use rerail_blocks::flat::{Arena, Bucket, Chain, Grow, Map, Pool, Region, Stamped, hash_word};
use rerail_gpu::device::ComputeQueue;
use rerail_gpu::kernels::{buffer_of, launch, program, read};

/// The region's bytes, 64-byte aligned (the arena's base).
#[repr(C, align(64))]
struct Bytes([u8; BYTES]);

const BYTES: usize = 1 << 17;

/// Where each structure's header lies in the region, and the script's steps or the lanes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct FlatArgs {
    g: u32,
    p: u32,
    c: u32,
    head: u32,
    m: u32,
    n: u32,
    /// A map claimed by row index over the 64-bit values at `vals`, and a stamped table.
    m2: u32,
    vals: u32,
    t: u32,
    pad: u32,
}

// SAFETY: ten u32s, no padding.
unsafe impl rerail_gpu::Pod for FlatArgs {}

const KERNELS: &str = r#"
#include <ukl/flat.h>

typedef struct FlatArgs { uint32_t g; uint32_t p; uint32_t c; uint32_t head; uint32_t m; uint32_t n; uint32_t m2; uint32_t vals; uint32_t t; uint32_t pad; } FlatArgs;
UKL_REFLECT_STRUCT(FlatArgs, g, p, c, head, m, n, m2, vals, t, pad)

// The script `script` runs in Rust, in one lane: each step's claims, in order.
DEFINE_KERNEL2(flat, flat_script,
               ARGS,     FlatArgs, args, ,
               RWBUFFER, uint8_t,  r,    "[131072]",
               GID0,     uint32_t, lane, )
{
    if (lane != 0u)
        return;
    uint32_t last = UKL_NONE, tail = 0u;
    for (uint32_t i = 0u; i < args.n; i++) {
        uint32_t o = ukl_flat_grow_claim(r, args.g, 1u, 4u);
        if (o != UKL_NONE)
            *(UKL_GLOBAL uint32_t*)(r + o) = i * 3u;
        if (i % 3u == 0u) {
            uint32_t t = ukl_flat_pool_take(r, args.p);
            if (t != UKL_NONE) {
                *(UKL_GLOBAL uint64_t*)(r + ukl_flat_pool_item(r, args.p, t, 8u)) = ((uint64_t)i << 20) | t;
                last = t;
            }
        }
        if (i % 5u == 4u && last != UKL_NONE) {
            ukl_flat_pool_give(r, args.p, last);
            last = UKL_NONE;
        }
        uint32_t l = ukl_flat_chain_take(r, args.c);
        if (l != UKL_NONE) {
            *(UKL_GLOBAL uint32_t*)(r + ukl_flat_pool_item(r, args.c, l, 4u)) = i;
            if (i % 2u == 0u) {
                ukl_flat_chain_cons(r, args.c, l, tail);
                tail = l;
            } else {
                ukl_flat_chain_push(r, args.c, args.head, l);
            }
        }
        ukl_flat_map_claim_min(r, args.m, (i * 7u) % 23u, 1000u - i);
        ukl_flat_map_set(r, args.m, 100u + i % 4u, i);
        ukl_flat_map_claim_i64(r, args.m2, i, (UKL_GLOBAL const int64_t*)(r + args.vals));
        uint32_t st = ukl_flat_stamped_claim(r, args.t, ukl_flat_round(r, 0u), (i * 5u) % 17u);
        if (st != UKL_NONE && (st & UKL_FLAT_FRESH) != 0u)
            ukl_atomic_store(ukl_flat_stamped_val(r, args.t, st), i);
        if (i % 7u == 0u) {
            uint32_t a = ukl_flat_claim(r, 0u, 6u, 13u + i, 16u);
            if (a != UKL_NONE)
                *(UKL_GLOBAL uint32_t*)(r + a) = 0xA5A50000u | i;
        }
    }
    ukl_flat_claim(r, 0u, 7u, 1u << 20, 8u);
}

// Lane `lane` of `n`, all at once: a map's least value a key, a vector's item, a pool's item, a chain's push.
DEFINE_KERNEL2(flat, flat_lanes,
               ARGS,     FlatArgs, args, ,
               RWBUFFER, uint8_t,  r,    "[131072]",
               GID0,     uint32_t, lane, )
{
    if (lane >= args.n)
        return;
    ukl_flat_map_claim_min(r, args.m, lane % 300u, lane);
    uint32_t o = ukl_flat_grow_claim(r, args.g, 1u, 4u);
    if (o != UKL_NONE)
        *(UKL_GLOBAL uint32_t*)(r + o) = lane;
    uint32_t t = ukl_flat_pool_take(r, args.p);
    if (t != UKL_NONE)
        *(UKL_GLOBAL uint64_t*)(r + ukl_flat_pool_item(r, args.p, t, 8u)) = lane;
    uint32_t l = ukl_flat_chain_take(r, args.c);
    if (l != UKL_NONE) {
        *(UKL_GLOBAL uint32_t*)(r + ukl_flat_pool_item(r, args.c, l, 4u)) = lane;
        ukl_flat_chain_push(r, args.c, args.head, l);
    }
    ukl_flat_map_claim_i64(r, args.m2, lane, (UKL_GLOBAL const int64_t*)(r + args.vals));
}
"#;

fn bytes() -> Box<Bytes> {
    // Bytes no structure reads before it writes them: whatever they held.
    let mut b = Box::new(Bytes([0; BYTES]));
    b.0.iter_mut().enumerate().for_each(|(i, x)| *x = (i * 131 % 251) as u8);
    b
}

/// The values the row-index map's keys stand for: `n` 64-bit values of few distinct ones, spread wide.
fn values(n: usize) -> Vec<i64> {
    let spread = [-(1i64 << 40), 7, 1 << 50, 3, i64::MAX - 5, i64::MIN];
    (0..n).map(|i| spread[i * 7 % 6] + (i % 4) as i64).collect()
}

/// The structures made in `b`'s arena, each header's offset in it; the stamped table filled in round 1, then the
/// round moved on (every slot stale), the structures kept.
fn made(b: &mut Bytes, grow: u32, pool: u32, chain: u32, keys: usize, n: usize) -> FlatArgs {
    let base = b.0.as_ptr().addr();
    let mut a = Arena::init(Region::of(&mut b.0), Bucket::Launch).unwrap();
    let off = |p: usize| (p - base) as u32;
    let mut args = {
        let g = off(std::ptr::from_ref(Grow::<u32>::make(&a, 1, grow).unwrap()).addr());
        let p = off(std::ptr::from_ref(Pool::<u64>::make(&a, 2, pool).unwrap()).addr());
        let c = off(std::ptr::from_ref(Chain::<u32>::make(&a, 3, chain).unwrap()).addr());
        let head = off(a.claim_with::<u32>(4, 1, 0).unwrap().as_ptr().addr());
        let m = off(std::ptr::from_ref(Map::<u32, u32>::make(&a, 5, keys, u32::MAX, u32::MAX).unwrap()).addr());
        let m2 = off(std::ptr::from_ref(Map::<u32, u32>::make(&a, 8, 64, u32::MAX, 0).unwrap()).addr());
        let vals = a.claim::<i64>(9, n).unwrap();
        for (x, v) in vals.iter_mut().zip(values(n)) {
            x.write(v);
        }
        let vals = off(vals.as_ptr().addr());
        let t = Stamped::<u32, u32>::make(&a, 10, 20).unwrap();
        for k in 0..12 {
            let c = t.claim(&a, k * 3).unwrap();
            t.set_at(c.slot, 500 + k);
        }
        let t = off(std::ptr::from_ref(t).addr());
        FlatArgs { g, p, c, head, m, n: n as u32, m2, vals, t, pad: 0 }
    };
    let keep = a.mark();
    assert_eq!(a.next_round(keep), 2);
    args.n = n as u32;
    args
}

/// `T` at byte `at` of the region at `base`, the Rust form over what lies there.
///
/// # Safety
/// A `T` lies at `at`, made there, and the region outlives `'a`; nothing else holds it meanwhile.
unsafe fn at<'a, T>(base: *mut u8, at: u32) -> &'a mut T {
    unsafe { &mut *base.add(at as usize).cast::<T>() }
}

/// The kernel `name` run over a copy of `b` on the tests' device, in `lanes` lanes: the bytes it leaves.
fn on_device(b: &Bytes, name: &str, args: FlatArgs, lanes: u32) -> Box<Bytes> {
    crate::tests::device();
    let shared = rerail_gpu::shared::get().expect("the tests' device");
    let p = program("flat_tests.uglykl", KERNELS).unwrap_or_else(|e| panic!("{e}"));
    let buf = buffer_of::<u8>(&b.0).unwrap();
    let mut q = ComputeQueue::new(&shared.queue);
    launch(&q, p.function(name).unwrap()).value("args", &args).buffer("r", &buf).run(&[u64::from(lanes.div_ceil(64))], &[64]).unwrap();
    q.flush().await_().unwrap();
    let mut out = Box::new(Bytes([0; BYTES]));
    out.0.copy_from_slice(&read::<u8>(&buf, BYTES));
    out
}

/// One script of claims, run in Rust over a region and by `flat_script` over a copy of it: the same bytes after,
/// every byte (each form's header, its items and its failures alike).
#[test]
fn a_script_of_claims_leaves_the_same_bytes_in_rust_and_ukl() {
    let mut a = bytes();
    let args = made(&mut a, 40, 16, 24, 50, 60);
    let vals = values(60);
    let before = Box::new(Bytes(a.0));
    {
        let base = a.0.as_mut_ptr();
        // SAFETY: the arena lies at the region's start, made by `made`; each structure at its offset.
        let ar = unsafe { Arena::open(Region::from_raw(base, BYTES)) }.unwrap();
        let (g, p, c, m) = unsafe { (at::<Grow<u32>>(base, args.g), at::<Pool<u64>>(base, args.p), at::<Chain<u32>>(base, args.c), at::<Map<u32, u32>>(base, args.m)) };
        let (m2, t) = unsafe { (at::<Map<u32, u32>>(base, args.m2), at::<Stamped<u32, u32>>(base, args.t)) };
        let head = unsafe { at::<std::sync::atomic::AtomicU32>(base, args.head) };
        let (mut last, mut tail) = (None, 0u32);
        for i in 0..args.n {
            if let Ok(o) = g.claim(1) {
                o[0].write(i * 3);
            }
            if i % 3 == 0
                && let Ok((t, room)) = p.take()
            {
                room.write(u64::from(i) << 20 | u64::from(t));
                last = Some(t);
            }
            if i % 5 == 4
                && let Some(t) = last.take()
            {
                p.give(t);
            }
            if i % 2 == 0 {
                if let Ok(l) = c.cons(tail, i) {
                    tail = l;
                }
            } else {
                let _ = c.push(head, i);
            }
            let _ = m.claim_min((i * 7) % 23, 1000 - i);
            let _ = m.set(100 + i % 4, i);
            let x = vals[i as usize];
            let _ = m2.claim_by(i, hash_word(x), |j| vals[j as usize] == x);
            if let Ok(c) = t.claim(&ar, (i * 5) % 17)
                && c.fresh
            {
                t.set_at(c.slot, i);
            }
            if i % 7 == 0
                && let Ok(o) = ar.claim_at(6, 13 + i as usize, 16)
            {
                // SAFETY: the claim just made, 13 bytes or more.
                unsafe { base.add(o).cast::<u32>().write(0xA5A5_0000 | i) };
            }
        }
        assert!(ar.claim_at(7, 1 << 20, 8).is_err(), "a claim past the arena's room fails");
    }
    let mut b = on_device(&before, "flat_script", args, 1);
    let differ: Vec<usize> = (0..BYTES).filter(|&i| a.0[i] != b.0[i]).take(16).collect();
    assert!(differ.is_empty(), "Rust and UKL differ at bytes {differ:?}");
    // And the script did what it says: the vector past its room, the pool's items given back, the arena's failure.
    let bb = b.0.as_mut_ptr();
    let (g, ar) = (unsafe { at::<Grow<u32>>(bb, args.g) }, unsafe { Arena::open(Region::from_raw(bb, BYTES)) }.unwrap());
    assert_eq!((g.len(), g.wanted()), (40, 60));
    assert_eq!(g.as_slice(), &(0..40).map(|i| i * 3).collect::<Vec<u32>>()[..]);
    assert_eq!(ar.failure().map(|f| f.part), Some(7));
    // The row-index map holds each distinct value's first row; the stamped table this round's keys alone.
    let m2 = unsafe { at::<Map<u32, u32>>(bb, args.m2) };
    let mut distinct: Vec<i64> = m2.iter().map(|(j, _)| vals[j as usize]).collect();
    distinct.sort_unstable();
    let mut want = vals.clone();
    want.sort_unstable();
    want.dedup();
    assert_eq!(distinct, want);
    assert!(m2.iter().all(|(j, _)| vals.iter().position(|&v| v == vals[j as usize]) == Some(j as usize)), "a value's first row");
    let t = unsafe { at::<Stamped<u32, u32>>(bb, args.t) };
    assert_eq!(t.live(&ar), 17, "keys 0..17 this round, the last round's stale");
    assert!((0..17).all(|k| t.get(&ar, k) == Some((0..60).find(|i| i * 5 % 17 == k).unwrap())), "each key's first claim");
}

/// Many lanes at once: what a kernel wrote reads back through the Rust views, as the Rust forms would have it in
/// any order of lanes (each key's least value; every claim that fit, and the ones that did not counted).
#[test]
fn many_lanes_claim_and_rust_reads_what_they_wrote() {
    let mut a = bytes();
    let args = made(&mut a, 1000, 500, 700, 300, 4096);
    let mut b = on_device(&a, "flat_lanes", args, args.n);
    let bb = b.0.as_mut_ptr();
    let m = unsafe { at::<Map<u32, u32>>(bb, args.m) };
    assert_eq!(m.len(), 300);
    assert!((0..300).all(|k| m.get(k) == Some(k)), "each key's least lane");
    let g = unsafe { at::<Grow<u32>>(bb, args.g) };
    assert_eq!((g.len(), g.wanted()), (1000, 4096));
    let mut got = g.as_slice().to_vec();
    got.sort_unstable();
    got.dedup();
    assert!(got.len() == 1000 && got.iter().all(|&l| l < 4096), "a lane an item");
    let p = unsafe { at::<Pool<u64>>(bb, args.p) };
    assert_eq!((p.live(), p.wanted()), (500, 4096));
    let mut items: Vec<u64> = (1..=500).map(|i| *p.get(i)).collect();
    items.sort_unstable();
    items.dedup();
    assert_eq!(items.len(), 500, "every item a lane's");
    let c = unsafe { at::<Chain<u32>>(bb, args.c) };
    let head = unsafe { at::<std::sync::atomic::AtomicU32>(bb, args.head) }.load(std::sync::atomic::Ordering::Relaxed);
    let mut walked: Vec<u32> = c.read().walk(head).copied().collect();
    assert_eq!(walked.len(), 700, "every push on the one list");
    walked.sort_unstable();
    walked.dedup();
    assert_eq!(walked.len(), 700);
    // Every lane's value claimed by its row's index: the distinct values, each once, whichever lane's row holds it.
    let vals = values(4096);
    let m2 = unsafe { at::<Map<u32, u32>>(bb, args.m2) };
    let mut got: Vec<i64> = m2.iter().map(|(j, _)| vals[j as usize]).collect();
    got.sort_unstable();
    let mut want = vals.clone();
    want.sort_unstable();
    want.dedup();
    assert_eq!(got, want);
}
