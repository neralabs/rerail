//! Each four-lane read is the compiled scalar read (`rerail_clif::jit::Reads`), lane for lane, on every row of a
//! relation of many chunks: a frozen base, and a frozen delta over it with some of its rows dead, an 8-byte column and
//! a 4-byte one, each row's key's index group, and a bloom filter's verdicts.
use super::*;
use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{RelW, Relation, Spec, freeze};
use std::sync::Mutex;

unsafe fn u(a: [u32; 4]) -> U {
    vld1q_u32(a.as_ptr())
}
unsafe fn lanes32(v: U) -> [u32; 4] {
    let mut a = [0u32; 4];
    vst1q_u32(a.as_mut_ptr(), v);
    a
}
unsafe fn lanes64(v: W) -> [u64; 4] {
    let mut a = [0u64; 4];
    v.store(a.as_mut_ptr());
    a
}

#[test]
fn the_four_lane_reads_are_the_compiled_reads() {
    let reads = rerail_clif::jit::Reads::new();
    type Two = extern "C" fn(*const u8, u32, u32) -> u64;
    type One = extern "C" fn(*const u8, u32) -> u32;
    type Group = extern "C" fn(*const u8, u32, u64) -> u64;
    type GroupRow = extern "C" fn(*const u8, u32, u32) -> u32;
    type Mix = extern "C" fn(*const u8, u64, u64) -> u64;
    type Hash = extern "C" fn(*const u8, u64) -> u64;
    type May = extern "C" fn(*const u8, u64) -> u32;
    // SAFETY: each read's signature: its descriptor's address, then its parameters.
    let (rel_get_1, rel_dead_1, rel_group_1, group_row_1): (Two, One, Group, GroupRow) = unsafe {
        (
            std::mem::transmute::<*const u8, Two>(reads.function("rel_get")),
            std::mem::transmute::<*const u8, One>(reads.function("rel_dead")),
            std::mem::transmute::<*const u8, Group>(reads.function("rel_group")),
            std::mem::transmute::<*const u8, GroupRow>(reads.function("rel_group_row")),
        )
    };
    let (mix_1, hash_1, may_1): (Mix, Hash, May) = unsafe {
        (
            std::mem::transmute::<*const u8, Mix>(reads.function("rel_key_mix")),
            std::mem::transmute::<*const u8, Hash>(reads.function("bloom_hash")),
            std::mem::transmute::<*const u8, May>(reads.function("bloom_may")),
        )
    };

    let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![vec![1]], min_log: 4 };
    let lock = Mutex::new(());
    let base_block = Block::anon(kind::FACTS, 16 << 20);
    let base = Relation::create(&base_block, &spec, None);
    let rows: Vec<i64> = (0..5_000i64).flat_map(|i| [(i * 7919) << 33 | i, i % 37]).collect();
    RelW { block: &base_block, rel: base, lock: &lock }.extend(&rows);
    freeze(&base_block, base, &spec, None);
    let top_block = Block::anon(kind::FACTS, 16 << 20);
    let top = Relation::create(&top_block, &spec, Some(base));
    let w = RelW { block: &top_block, rel: top, lock: &lock };
    for id in (0..base.rows.len()).step_by(13) {
        w.kill(id);
    }
    w.extend(&(0..700i64).flat_map(|i| [-i - 1, 40 + i % 5]).collect::<Vec<_>>());
    freeze(&top_block, top, &spec, Some(base));

    for rel in [base, top] {
        let d = std::ptr::from_ref(rel).cast::<u8>();
        let n = rel.base_rows + rel.rows.len();
        // Four rows a step, from far apart (a delta's lanes in different blocks).
        for i in 0..n {
            let r = [i, (i * 7 + 3) % n, n - 1 - i, (i * 131) % n];
            unsafe {
                let rv = u(r);
                for c in 0..2u32 {
                    let got = lanes64(rel_get(d, rv, c));
                    for l in 0..4 {
                        assert_eq!(got[l], rel_get_1(d, r[l], c), "rel_get row {} column {c}", r[l]);
                    }
                }
                let own = rel.rows.len();
                let o = r.map(|x| x % own);
                let dead = lanes32(rel_dead(d, u(o)));
                for l in 0..4 {
                    assert_eq!(dead[l] != 0, rel_dead_1(d, o[l]) != 0, "rel_dead row {}", o[l]);
                }
                let h = r.map(|x| rerail_blocks::key_hash(&[rel_get_1(d, x, 1) as i64]));
                let hv = W::load(h.as_ptr());
                let (start, len) = rel_group(d, 0, hv);
                let (start, len) = (lanes32(start), lanes32(len));
                for l in 0..4 {
                    let g = rel_group_1(d, 0, h[l]);
                    assert_eq!(((start[l] as u64) << 32) | len[l] as u64, g, "rel_group of row {}'s key", r[l]);
                }
                let rows = lanes32(rel_group_row(d, 0, u(start)));
                for l in 0..4 {
                    assert_eq!(rows[l], group_row_1(d, 0, start[l]), "rel_group_row {}", start[l]);
                }
                let v = r.map(|x| rel_get_1(d, x, 0));
                let mixed = lanes64(key_mix(W::load(h.as_ptr()), W::load(v.as_ptr())));
                for l in 0..4 {
                    assert_eq!(mixed[l], mix_1(std::ptr::null(), h[l], v[l]), "key_mix");
                }
            }
        }
    }

    // A bloom filter's hashes and verdicts, for keys it holds and keys it does not.
    use rerail_blocks::bloom::{self, Bloom};
    let keys: Vec<u64> = (0..3_000u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i << 7)).collect();
    let block = Block::anon(kind::FACTS, 1 << 20);
    // SAFETY: the filter just made in the block.
    let f: &Bloom = unsafe { &*block.ptr::<Bloom>(Bloom::try_create(&block, 64).unwrap()) };
    for &k in keys.iter().step_by(3) {
        f.add_hashed(bloom::hash(k as i64));
    }
    let fd = std::ptr::from_ref(f).cast::<u8>();
    for k in keys.chunks_exact(4).chain(std::iter::once(&[1u64, 2, 3, u64::MAX][..])) {
        let k: [u64; 4] = k.try_into().unwrap();
        let probes: [u64; 4] = k.map(|x| x.wrapping_add(17));
        for keys4 in [k, probes] {
            unsafe {
                let (lo, hi) = bloom_hash(W::load(keys4.as_ptr()));
                let h = lanes64(W::join(lo, hi));
                let may = lanes32(bloom_may(fd, lo, hi));
                for l in 0..4 {
                    assert_eq!(h[l], hash_1(std::ptr::null(), keys4[l]), "bloom_hash of {:#x}", keys4[l]);
                    assert_eq!(may[l] != 0, may_1(fd, h[l]) != 0, "bloom_may of {:#x}", keys4[l]);
                }
            }
        }
    }
}
