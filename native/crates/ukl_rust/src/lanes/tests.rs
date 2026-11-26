//! Each four-lane read is the compiled scalar read (`rerail_clif::jit::Reads`), lane for lane, on every row of a
//! relation of many chunks: a frozen base, and a frozen delta over it with some of its rows dead, an 8-byte column and
//! a 4-byte one, each row's key's index group, a frozen relation whose chunks are sealed (decision 0622: each lane's
//! sealed chunk read through the compiled `col_sealed`), and a bloom filter's verdicts.
use super::*;
use rerail_simd::{LaneCount, SupportedLaneCount};
use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{RelW, Relation, Spec, freeze};
use std::sync::Mutex;

#[test]
fn the_lane_reads_are_the_compiled_reads() {
    let reads = rerail_clif::jit::Reads::new();
    type Two = extern "C" fn(*const u8, u32, u32, u64) -> u64;
    type One = extern "C" fn(*const u8, u32) -> u32;
    type Group = extern "C" fn(*const u8, u32, u64) -> u64;
    type GroupRow = extern "C" fn(*const u8, u32, u32) -> u32;
    type Mix = extern "C" fn(*const u8, u64, u64) -> u64;
    type Hash = extern "C" fn(*const u8, u64) -> u64;
    type May = extern "C" fn(*const u8, u64) -> u32;
    // SAFETY: the read's signature: its chunk's address, then the place.
    let sealed: Sealed = unsafe { std::mem::transmute::<*const u8, Sealed>(reads.function("col_sealed")) };
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

    let one = Compiled { rel_get: rel_get_1, rel_dead: rel_dead_1, rel_group: rel_group_1, group_row: group_row_1, mix: mix_1, sealed };
    // A frozen relation, and a delta's own rows (`base_rows` on): its keys' groups, its dead rows.
    for rel in [base, top] {
        let d = std::ptr::from_ref(rel).cast::<u8>();
        let (lo, n, own) = (rel.base_rows, rel.base_rows + rel.rows.len(), rel.rows.len());
        relation_lanes::<4>(d, std::ptr::null(), lo, n, own, &one, true);
        relation_lanes::<8>(d, std::ptr::null(), lo, n, own, &one, true);
        relation_lanes::<16>(d, std::ptr::null(), lo, n, own, &one, true);
    }
    // A chain of continuations (`try_continue`: a fork of a fork): every id read down the chain, each lane walking
    // to its own row's relation.
    let c1 = Relation::try_continue(&base_block, &spec, base, 3_000).unwrap();
    RelW { block: &base_block, rel: c1, lock: &lock }.extend(&(0..900i64).flat_map(|i| [i * 3 + 1, i % 11]).collect::<Vec<_>>());
    let c2 = Relation::try_continue(&base_block, &spec, c1, 3_500).unwrap();
    RelW { block: &base_block, rel: c2, lock: &lock }.extend(&(0..1_300i64).flat_map(|i| [-(i << 20), 7 + i % 3]).collect::<Vec<_>>());
    let d = std::ptr::from_ref(c2).cast::<u8>();
    let n = c2.base_rows + c2.rows.len();
    relation_lanes::<4>(d, std::ptr::null(), 0, n, c2.rows.len(), &one, false);
    relation_lanes::<8>(d, std::ptr::null(), 0, n, c2.rows.len(), &one, false);
    // A link in a block of its own over a base in another (`try_link`, `FAR_BASE`: decision 0353 step 6): ids below
    // its `base_rows` read in that block, at its start (`far`), each lane's walk leaving its own block there.
    let link_block = Block::anon(kind::FACTS, 16 << 20);
    let own_rows = Relation::create(&link_block, &spec, None);
    RelW { block: &link_block, rel: own_rows, lock: &lock }.extend(&(0..1_100i64).flat_map(|i| [i << 9, 3 + i % 7]).collect::<Vec<_>>());
    freeze(&link_block, own_rows, &spec, None);
    let link = Relation::try_link(&link_block, &spec, Some((&base_block, base)), base.rows.len(), own_rows, own_rows.rows.len()).unwrap();
    assert!(link.far_base());
    let (d, far) = (std::ptr::from_ref(link).cast::<u8>(), base_block.ptr::<u8>(0).cast_const());
    let n = link.base_rows + link.rows.len();
    relation_lanes::<4>(d, far, 0, n, link.rows.len(), &one, false);
    relation_lanes::<8>(d, far, 0, n, link.rows.len(), &one, false);

    // A frozen relation in a block with a free list, its chunks sealed in their frozen encodings (decision 0622, as the
    // engine's frozen relations are): a constant run, runs, a line and packed values, each lane's chunk read through
    // the compiled read where it is sealed.
    let sealed_block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
    let frozen = Relation::create(&sealed_block, &spec, None);
    let col0 = |i: i64| match i {
        0..2_000 => -3,
        2_000..6_000 => i / 700 * 11,
        _ => i * 3 + (i * 7) % 5 - 1_000_000,
    };
    RelW { block: &sealed_block, rel: frozen, lock: &lock }.extend(&(0..10_000i64).flat_map(|i| [col0(i), i % 37]).collect::<Vec<_>>());
    freeze(&sealed_block, frozen, &spec, None);
    // SAFETY: the relation's rows lie in the block, which keeps a free list; no one writes them after, or holds an
    // address in them.
    let (raw, packed) = unsafe { frozen.rows.seal(&sealed_block) };
    assert!(packed < raw, "its chunks sealed: {packed} bytes of {raw}");
    let d = std::ptr::from_ref(frozen).cast::<u8>();
    relation_lanes::<4>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, true);
    relation_lanes::<8>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, true);
    relation_lanes::<16>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, true);
    // Rows written past its sealed ones: their chunks raw, so a step's lanes meet sealed and raw chunks together, each
    // read its own way.
    RelW { block: &sealed_block, rel: frozen, lock: &lock }.extend(&(10_000..14_000i64).flat_map(|i| [col0(i), i % 37]).collect::<Vec<_>>());
    relation_lanes::<4>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, false);
    relation_lanes::<8>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, false);
    relation_lanes::<16>(d, std::ptr::null(), 0, frozen.rows.len(), frozen.rows.len(), &one, false);

    // A bloom filter's hashes and verdicts, for keys it holds and keys it does not.
    use rerail_blocks::bloom::{self, Bloom};
    let keys: Vec<u64> = (0..3_000u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i << 7)).chain([1, 2, 3, u64::MAX]).collect();
    let block = Block::anon(kind::FACTS, 1 << 20);
    // SAFETY: the filter just made in the block.
    let f: &Bloom = unsafe { &*block.ptr::<Bloom>(Bloom::try_create(&block, 64).unwrap()) };
    for &k in keys.iter().step_by(3) {
        f.add_hashed(bloom::hash(k as i64));
    }
    let fd = std::ptr::from_ref(f).cast::<u8>();
    let probes: Vec<u64> = keys.iter().map(|x| x.wrapping_add(17)).collect();
    for ks in [&keys, &probes] {
        bloom_lanes::<4>(fd, ks, hash_1, may_1);
        bloom_lanes::<8>(fd, ks, hash_1, may_1);
    }
}

struct Compiled {
    rel_get: extern "C" fn(*const u8, u32, u32, u64) -> u64,
    rel_dead: extern "C" fn(*const u8, u32) -> u32,
    rel_group: extern "C" fn(*const u8, u32, u64) -> u64,
    group_row: extern "C" fn(*const u8, u32, u32) -> u32,
    mix: extern "C" fn(*const u8, u64, u64) -> u64,
    sealed: Sealed,
}

/// Every id `lo..n` of relation `d` (`own` rows of its own), `N` a step from far apart (a chain's lanes in different
/// relations), each read at `N` lanes against the compiled read on each lane; its keys' groups where it has its
/// index (`group`).
fn relation_lanes<const N: usize>(d: *const u8, far: *const u8, lo: u32, n: u32, own: u32, one: &Compiled, group: bool)
where
    LaneCount<N>: SupportedLaneCount,
{
    let m = n - lo;
    for i in 0..m {
        let r: [u32; N] = core::array::from_fn(|l| lo + (i + (l as u32) * (m / N as u32 + 7)) % m);
        let rv = Simd::from_array(r);
        unsafe {
            for c in 0..2u32 {
                let got = rel_get(d, rv, c, far, one.sealed).to_array();
                for l in 0..N {
                    assert_eq!(got[l], (one.rel_get)(d, r[l], c, far as u64), "rel_get row {} column {c} at {N} lanes", r[l]);
                }
            }
            let o = r.map(|x| x % own);
            let dead = rel_dead(d, Simd::from_array(o)).to_array();
            for l in 0..N {
                assert_eq!(dead[l], (one.rel_dead)(d, o[l]) != 0, "rel_dead row {}", o[l]);
            }
            if !group {
                continue;
            }
            let h = r.map(|x| rerail_blocks::key_hash(&[(one.rel_get)(d, x, 1, far as u64) as i64]));
            let (start, len) = rel_group(d, 0, Simd::from_array(h));
            let (start, len) = (start.to_array(), len.to_array());
            for l in 0..N {
                assert_eq!(((start[l] as u64) << 32) | len[l] as u64, (one.rel_group)(d, 0, h[l]), "rel_group of row {}'s key", r[l]);
            }
            let rows = rel_group_row(d, 0, Simd::from_array(start)).to_array();
            for l in 0..N {
                assert_eq!(rows[l], (one.group_row)(d, 0, start[l]), "rel_group_row {}", start[l]);
            }
            let v = r.map(|x| (one.rel_get)(d, x, 0, far as u64));
            let mixed = key_mix(Simd::from_array(h), Simd::from_array(v)).to_array();
            for l in 0..N {
                assert_eq!(mixed[l], (one.mix)(std::ptr::null(), h[l], v[l]), "key_mix");
            }
        }
    }
}

fn bloom_lanes<const N: usize>(fd: *const u8, keys: &[u64], hash_1: extern "C" fn(*const u8, u64) -> u64, may_1: extern "C" fn(*const u8, u64) -> u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    for k in keys.as_chunks::<N>().0 {
        let (lo, hi) = bloom_hash(Simd::<u64, N>::from_array(*k));
        let h = joined(lo, hi).to_array();
        // SAFETY: the filter.
        let may = unsafe { bloom_may(fd, lo, hi) }.to_array();
        for l in 0..N {
            assert_eq!(h[l], hash_1(std::ptr::null(), k[l]), "bloom_hash of {:#x}", k[l]);
            assert_eq!(may[l], may_1(fd, h[l]) != 0, "bloom_may of {:#x}", k[l]);
        }
    }
}

/// `chunk` is `clif_col_at`'s arithmetic, lane for lane.
#[test]
fn a_rows_chunk_is_col_ats() {
    for shift in [0u32, 1, 4, 6] {
        for i in (0..70_000u32).step_by(7) {
            let k = ((i as u64) >> shift) + 1;
            let s = 63 - k.leading_zeros() as u64;
            let off = i as u64 - (((1u64 << s) - 1) << shift);
            let e = (shift as u64 + s).wrapping_sub(2);
            let seg = (s << 2) + (off >> (e & 63));
            let within = off & ((1u64 << (e & 63)) - 1);
            let (sv, wv) = chunk::<4>(Simd::splat(i), Simd::splat(shift));
            assert_eq!((sv[0] as u64, wv[0] as u64), (seg, within), "row {i} from {shift}");
        }
    }
}
