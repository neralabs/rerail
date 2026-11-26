//! Each four-lane read is the compiled scalar read (`rerail_clif::jit::Reads`), lane for lane, on every row of a
//! relation of many chunks: a frozen base, and a frozen delta over it with some of its rows dead, an 8-byte column and
//! a 4-byte one, each row's key's index group, a frozen relation whose chunks are sealed (decision 0622: each lane's
//! sealed chunk read by `col_sealed`, the compiled read's through the compiled one), and a bloom filter's verdicts.
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

    let one = Compiled { rel_get: rel_get_1, rel_dead: rel_dead_1, rel_group: rel_group_1, group_row: group_row_1, mix: mix_1 };
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
    // engine's frozen relations are): a constant run, runs, a line and packed values, each lane's chunk read by
    // `col_sealed` where it is sealed.
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
                let got = rel_get(d, rv, c, far).to_array();
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

/// The layouts the scalar reads take (`ROWS_*`, `INDEX_MARKS`, `KEY_SEED`, the atoms directory's with its table) are
/// `rerail_blocks`' own.
#[test]
fn the_scalar_reads_layouts_are_the_blocks() {
    use crate::join_program::reads::atoms_at as a;
    use rerail_blocks::atoms::{AtomsDirRoot, DirEntry};
    use rerail_blocks::relation::Index;
    use rerail_blocks::rows::Rows;
    use std::mem::{offset_of, size_of};
    assert_eq!((ROWS_LEN, ROWS_RESERVED, ROWS_NEEDED, ROWS_NCOLS), (offset_of!(Rows, len) as u64, offset_of!(Rows, reserved) as u64, offset_of!(Rows, needed) as u64, offset_of!(Rows, ncols) as u64));
    assert_eq!((INDEX_MARKS, REL_INDEXES, INDEX_SIZE), (offset_of!(Index, marks) as u64, offset_of!(Relation, indexes) as u64, size_of::<Index>()));
    assert_eq!(KEY_SEED, rerail_blocks::SEED);
    assert_eq!((a::DIR_N, a::DIR_ENTRIES), (offset_of!(AtomsDirRoot, n), offset_of!(AtomsDirRoot, entries)));
    assert_eq!(
        (a::E_FIRST, a::E_END, a::E_FROM, a::E_ENDS, a::E_STRINGS, a::E_TABLE, a::E_SIZE),
        (offset_of!(DirEntry, first), offset_of!(DirEntry, end), offset_of!(DirEntry, from), offset_of!(DirEntry, ends), offset_of!(DirEntry, strings), offset_of!(DirEntry, table), size_of::<DirEntry>())
    );
}

/// The scalar reads the join program makes outside its lanes (`one`, `rows_len`, `rows_put`, `rel_group_mark`, and
/// the atoms' `join_program::reads::atoms_len`, `atoms_str`, `atoms_find`) answer as the compiled reads: every row of a relation with
/// an index and a delta over it, every place of its index's rows and marks, keys in a bloom filter and not; a write
/// of every row and column and some past them into two relations alike, one by each, leaving the same rows and the
/// same overflow count; every id of an atoms directory of two runs with a gap, id 0 and ids past them.
#[test]
fn the_scalar_reads_are_the_compiled_reads() {
    use rerail_blocks::atoms::{AtomsDir, AtomsW};
    use rerail_blocks::bloom::{self, Bloom};
    let reads = rerail_clif::jit::Reads::new();
    let f = |name: &str| reads.function(name);
    // SAFETY: each read's signature: its descriptor's address, then its parameters.
    let (get, dead, group, row, mark): (extern "C" fn(*const u8, u32, u32, u64) -> u64, extern "C" fn(*const u8, u32) -> u32, extern "C" fn(*const u8, u32, u64) -> u64, extern "C" fn(*const u8, u32, u32) -> u32, extern "C" fn(*const u8, u32, u32) -> u32) =
        unsafe { (std::mem::transmute(f("rel_get")), std::mem::transmute(f("rel_dead")), std::mem::transmute(f("rel_group")), std::mem::transmute(f("rel_group_row")), std::mem::transmute(f("rel_group_mark"))) };
    let (len, put, rows_get): (extern "C" fn(*const u8) -> u32, extern "C" fn(*const u8, u32, u32, u64) -> u32, extern "C" fn(*const u8, u32, u32) -> u64) =
        unsafe { (std::mem::transmute(f("rows_len")), std::mem::transmute(f("rows_put")), std::mem::transmute(f("rows_get"))) };
    let (mix, hash, may): (extern "C" fn(*const u8, u64, u64) -> u64, extern "C" fn(*const u8, u64) -> u64, extern "C" fn(*const u8, u64) -> u32) =
        unsafe { (std::mem::transmute(f("rel_key_mix")), std::mem::transmute(f("bloom_hash")), std::mem::transmute(f("bloom_may"))) };
    let (atoms_at, atoms_len): (extern "C" fn(*const u8, u32) -> u64, extern "C" fn(*const u8, u32) -> u32) = unsafe { (std::mem::transmute(f("atoms_at")), std::mem::transmute(f("atoms_len"))) };
    let atoms_find: extern "C" fn(*const u8, u64, u32, u32) -> u32 = unsafe { std::mem::transmute(f("atoms_find")) };

    let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![vec![1]], min_log: 4 };
    let lock = Mutex::new(());
    let base_block = Block::anon(kind::FACTS, 16 << 20);
    let base = Relation::create(&base_block, &spec, None);
    RelW { block: &base_block, rel: base, lock: &lock }.extend(&(0..3_000i64).flat_map(|i| [(i * 7919) << 33 | i, i % 37]).collect::<Vec<_>>());
    freeze(&base_block, base, &spec, None);
    let top_block = Block::anon(kind::FACTS, 16 << 20);
    let top = Relation::create(&top_block, &spec, Some(base));
    let w = RelW { block: &top_block, rel: top, lock: &lock };
    for id in (0..base.rows.len()).step_by(11) {
        w.kill(id);
    }
    w.extend(&(0..500i64).flat_map(|i| [-i - 1, 40 + i % 5]).collect::<Vec<_>>());
    freeze(&top_block, top, &spec, Some(base));
    let mut tried = 0;
    for rel in [base, top] {
        let d = std::ptr::from_ref(rel).cast::<u8>();
        let rows = std::ptr::from_ref(&rel.rows).cast::<u8>();
        // SAFETY: the relation's descriptor, its rows', in their blocks, held here.
        unsafe {
            assert_eq!(rows_len(rows), len(rows), "rows_len");
            for id in rel.base_rows..rel.base_rows + rel.rows.len() {
                for c in 0..2 {
                    assert_eq!(one::rel_get(d, id, c, std::ptr::null()), get(d, id, c, 0), "rel_get {id} {c}");
                }
                let own = id - rel.base_rows;
                assert_eq!(one::rel_dead(d, own), dead(d, own) != 0, "rel_dead {own}");
                let h = one::key_hash(&[get(d, id, 1, 0) as i64]);
                assert_eq!(h, rerail_blocks::key_hash(&[get(d, id, 1, 0) as i64]), "key_hash");
                assert_eq!(one::key_mix(h, id.into()), mix(std::ptr::null(), h, id.into()), "key_mix");
                let (start, n) = one::rel_group(d, 0, h);
                assert_eq!(u64::from(start) << 32 | u64::from(n), group(d, 0, h), "rel_group of {id}'s key");
                tried += 1;
            }
            let ix = rel.index_at(0);
            for q in 0..ix.rows.len() as u32 {
                assert_eq!(one::rel_group_row(d, 0, q), row(d, 0, q), "rel_group_row {q}");
            }
            for q in 0..ix.marks.len() as u32 {
                assert_eq!(rel_group_mark(d, 0, q), mark(d, 0, q), "rel_group_mark {q}");
            }
        }
    }
    // Writes: two relations alike, one written by each, every row and column and some past them.
    let (ba, bb) = (Block::anon(kind::FACTS, 16 << 20), Block::anon(kind::FACTS, 16 << 20));
    let (ra, rb) = (Relation::create(&ba, &spec, None), Relation::create(&bb, &spec, None));
    for (b, r) in [(&ba, ra), (&bb, rb)] {
        RelW { block: b, rel: r, lock: &lock }.extend(&(0..700i64).flat_map(|i| [i, i]).collect::<Vec<_>>());
    }
    let (pa, pb) = (std::ptr::from_ref(&ra.rows).cast::<u8>(), std::ptr::from_ref(&rb.rows).cast::<u8>());
    let reserved = ra.rows.reserved.load(std::sync::atomic::Ordering::Relaxed);
    for i in 0..reserved + 5 {
        for c in 0..3u32 {
            let v = (u64::from(i) << 40) ^ u64::from(c).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            // SAFETY: the relations' rows, in their blocks, held here.
            unsafe { assert_eq!(rows_put(pa, i, c, v), put(pb, i, c, v), "rows_put {i} {c}") };
        }
    }
    for i in 0..reserved {
        for c in 0..2 {
            assert_eq!(rows_get(pa, i, c), rows_get(pb, i, c), "row {i} {c} as written");
        }
    }
    assert_eq!(ra.rows.needed.load(std::sync::atomic::Ordering::Relaxed), rb.rows.needed.load(std::sync::atomic::Ordering::Relaxed), "the overflow counts");
    // A bloom filter's hashes and verdicts.
    let block = Block::anon(kind::FACTS, 1 << 20);
    // SAFETY: the filter just made in the block.
    let fl: &Bloom = unsafe { &*block.ptr::<Bloom>(Bloom::try_create(&block, 64).unwrap()) };
    let keys: Vec<u64> = (0..2_000u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i << 7)).collect();
    for &k in keys.iter().step_by(3) {
        fl.add_hashed(bloom::hash(k as i64));
    }
    for &k in &keys {
        let h = one::bloom_hash(k);
        assert_eq!(h, hash(std::ptr::null(), k), "bloom_hash {k}");
        // SAFETY: the filter, held here.
        assert_eq!(unsafe { one::bloom_may(std::ptr::from_ref(fl).cast(), h) }, may(std::ptr::from_ref(fl).cast(), h) != 0, "bloom_may {k}");
    }
    // An atoms directory of two runs, a gap between them.
    let k_hash = |s: &str| s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3));
    let words: Vec<String> = (0..20).map(|k| format!("w{k}{}", "z".repeat(k))).collect();
    let mut a = AtomsW::anon(1, 8, 256);
    for w in &words[..8] {
        a.append(w, k_hash(w)).unwrap();
    }
    let mut b = AtomsW::anon(9, 16, 1024);
    for w in &words[8..] {
        b.append(w, k_hash(w)).unwrap();
    }
    let runs = [a.run_from(1), b.run_from(12)];
    let dir = AtomsDir::host(&runs);
    let root = dir.root();
    for id in 0..=22u32 {
        // SAFETY: a host directory over blocks that live here.
        unsafe {
            assert_eq!(crate::join_program::reads::atoms_len(root, id), atoms_len(root, id), "atoms_len {id}");
            assert_eq!(crate::join_program::reads::atoms_str(root, id) as u64, atoms_at(root, id), "atoms_at {id}");
        }
    }
    // Each word's atom by its hash and length (decision 0666), the `k`th of a hash's slots of its length, and words of
    // none: the CLIF's answers.
    for w in words.iter().map(String::as_str).chain(["absent", "w3", "w3zzz", ""]) {
        let (h, len) = (k_hash(w), w.len() as u32);
        for k in 0..3 {
            // SAFETY: the host directory, as above.
            let got = unsafe { crate::join_program::reads::atoms_find(root, h, len, k) };
            assert_eq!(got, atoms_find(root, h, len, k), "atoms_find {w:?} {k}");
        }
        tried += 1;
    }
    assert!(tried > 3_000);
}

/// Decision 0622: `col_sealed`, the read of a sealed chunk's frozen column, reads every value of each encoding as
/// `col::IntCol::get` does (const, pack, runs, line, groups, dict), and its layout and tags are the blocks'.
#[test]
fn col_sealed_reads_each_encoding_as_the_blocks() {
    use int_col::*;
    use rerail_blocks::col::{self, IntCol};
    use std::mem::offset_of;
    let at = [offset_of!(IntCol, tag), offset_of!(IntCol, bits), offset_of!(IntCol, rbits), offset_of!(IntCol, nruns), offset_of!(IntCol, base), offset_of!(IntCol, slope), offset_of!(IntCol, rmin), offset_of!(IntCol, data), offset_of!(IntCol, runs)];
    assert_eq!([AT_TAG, AT_BITS, AT_RBITS, AT_NRUNS, AT_BASE, AT_SLOPE, AT_RMIN, AT_DATA, AT_RUNS], at.map(|o| o as u64));
    assert_eq!([CONST, PACK, RUNS, LINE, GROUPS, DICT], [col::CONST, col::PACK, col::RUNS, col::LINE, col::GROUPS, col::DICT]);
    // Ids of 40 documents, a few hundred each with gaps: `GROUPS`.
    let ids: Vec<i64> = (0..40i64).flat_map(|d| (0..300i64).map(move |k| (d + 17) << 32 | (k * 2 + k % 3))).collect();
    let packed: Vec<Vec<i64>> = vec![
        vec![5; 100],
        (0..1000).map(|i| (i * 7919) % 1000 - 300).collect(),
        (0..1000).map(|i| i / 37).collect(),
        (0..1000).map(|i| 1_000_000 + i * 3 + i % 5).collect(),
        ids,
    ];
    let spread = [-(1i64 << 40), 7, 1 << 50, 3, i64::MAX - 5];
    let few: Vec<i64> = (0..1000i64).map(|i| spread[(i * 2_654_435_761 % 5) as usize]).collect();
    let block = Block::anon(kind::FACTS, 16 << 20);
    let mut cols: Vec<(*const IntCol, &[i64])> = packed
        .iter()
        .map(|v| {
            let p: *mut IntCol = block.alloc_t(1);
            // SAFETY: zeroed memory in the open block.
            unsafe { IntCol::place(p, &block, &col::encode(v)) };
            (p.cast_const(), v.as_slice())
        })
        .collect();
    // A column of few values spread wide, for reads at random: `DICT`.
    let c = col::choose_for_reads(&few);
    let p: *mut IntCol = block.alloc_t(1);
    let words: *mut u64 = block.alloc_t(c.data_words + c.runs_words);
    // SAFETY: zeroed memory in the open block, written by this writer alone.
    let (data, runs) = unsafe { IntCol::lay_out_at(p, words, &c) };
    col::write(&c, &few, data, runs);
    cols.push((p.cast_const(), few.as_slice()));
    let mut tags = Vec::new();
    for (p, v) in cols {
        // SAFETY: the column just placed in the block, held here.
        let ic = unsafe { &*p };
        tags.push(ic.tag);
        for (i, &x) in v.iter().enumerate() {
            assert_eq!(ic.get(i), x, "the blocks' read of {} at {i}", ic.tag_name());
            // SAFETY: the column's descriptor and words, in the block; `i` within its values.
            assert_eq!(unsafe { col_sealed(p.cast(), i as u32) } as i64, x, "col_sealed of {} at {i}", ic.tag_name());
        }
    }
    assert_eq!(tags, [CONST, PACK, RUNS, LINE, GROUPS, DICT], "every encoding read");
}
