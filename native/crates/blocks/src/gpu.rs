//! Kernels that read and write block structures in place, in UKL (decision 0440: every backend's). Blocks are the
//! device's buffers where they lie (`Block::device_buffer`): nothing is uploaded. The structs come from `structure!`
//! (`ukl_of`), with offset assertions, so this file holds no copy of any layout. The accessors below (`rows_get`,
//! `v_live`, `rel_group`, …) are the layer's kernel half, written once, and named as the host's are.
//!
//! The join: `out(l_out, r_out) :- left(.., l_key, ..), right(.., r_key, ..), l_out != r_out`, with `left` and
//! `right` each a base + delta view and `right` probed by its index. It runs in two passes, as 0266 §5 says:
//!
//! 1. A count pass gives each left row's matches.
//! 2. The host reserves the total (chunks made between passes, never during one).
//! 3. A write pass claims rows with one atomic per thread, and writes them.
//!
//! A claim past what was reserved writes nothing and adds to `needed`; the launcher reruns the write pass at the
//! exact size. The launcher is the device layer's caller's (`examples/paces.rs`): this crate is below it.

use crate::relation::Relation;
use crate::{structure, ukl_of};

pub const NONE: u64 = u64::MAX;

structure! {
    /// A join's arguments: each relation's offset in its block (`NONE`: no base), and the columns.
    pub struct JoinArgs {
        pub lbase: u64,
        pub ltop: u64,
        pub rbase: u64,
        pub rtop: u64,
        pub out: u64,
        pub n: u32,
        pub idx: u32,
        pub lkey: u32,
        pub lout: u32,
        pub rkey: u32,
        pub rout: u32,
    }
}

const LIB: &str = r#"
#define NONE 0xFFFFFFFFFFFFFFFFull
#define SEED 0x243F6A8885A308D3ull
UKL_INLINE uint64_t mix(uint64_t h, int64_t v) { return clif_rel_key_mix((UKL_GLOBAL const uint8_t*)0, 0ull, h, (uint64_t)v); }

// A relation's reads are written once in CLIF (`<ukl/reads/relation.h>`, generated: decision 0325); these name them as
// the host's are. A read takes a base and the descriptor's offset from it: here the descriptor itself, at 0.
UKL_INLINE UKL_GLOBAL const uint8_t* desc(UKL_GLOBAL const void* p) { return (UKL_GLOBAL const uint8_t*)p; }
UKL_INLINE int64_t rows_get(UKL_GLOBAL const Rows* r, uint32_t i, uint32_t c) { return (int64_t)clif_rows_get(desc(r), 0ull, i, c); }
UKL_INLINE void rows_put(UKL_GLOBAL const Rows* r, uint32_t i, uint32_t c, int64_t v) {
    UKL_GLOBAL uint8_t* p = (UKL_GLOBAL uint8_t*)desc(r) + clif_rows_at(desc(r), 0ull, i, c);
    if (clif_rows_width(desc(r), 0ull, c) == 4u) { *(UKL_GLOBAL uint32_t*)p = (uint32_t)v; } else { *(UKL_GLOBAL int64_t*)p = v; }
}
UKL_INLINE bool dead_bit(UKL_GLOBAL const Relation* r, uint32_t id) { return clif_rel_dead(desc(r), 0ull, id) != 0u; }

// A relation as readers see it: a base and its delta, or a full relation (base null).
typedef struct View { UKL_GLOBAL const Relation* base; UKL_GLOBAL const Relation* top; } View;
UKL_INLINE int64_t v_get(View v, uint32_t id, uint32_t c) {
    return id < v.top->base_rows ? rows_get(&v.base->rows, id, c) : rows_get(&v.top->rows, id - v.top->base_rows, c);
}
UKL_INLINE bool v_live(View v, uint32_t id) {
    if (dead_bit(v.top, id)) return false;
    return id >= v.top->base_rows || !dead_bit(v.base, id);
}
UKL_INLINE View view_of(UKL_GLOBAL uint8_t* b1, UKL_GLOBAL uint8_t* b2, uint64_t base, uint64_t top) {
    View v;
    v.base = base == NONE ? (UKL_GLOBAL const Relation*)0 : (UKL_GLOBAL const Relation*)(b1 + base);
    v.top = (UKL_GLOBAL const Relation*)(b2 + top);
    return v;
}

UKL_INLINE uint32_t join_row(View L, View R, JoinArgs a, uint32_t id, UKL_GLOBAL const Relation* out, uint32_t at, bool write) {
    if (!v_live(L, id)) return 0u;
    int64_t k = v_get(L, id, a.lkey);
    int64_t lo = v_get(L, id, a.lout);
    uint64_t h = mix(SEED, k);
    uint32_t n = 0u;
    for (uint32_t side = 0u; side < 2u; side++) {
        UKL_GLOBAL const Relation* r = side == 0u ? R.base : R.top;
        if (r == 0) continue;
        // A key's group in one relation's index: (start, len) into its rows; bounded by the table's size.
        uint64_t g = clif_rel_group(desc(r), 0ull, a.idx, h);
        uint32_t start = (uint32_t)(g >> 32), len = (uint32_t)g;
        for (uint32_t j = 0u; j < len; j++) {
            uint32_t rid = clif_rel_group_row(desc(r), 0ull, a.idx, start + j);
            if (!v_live(R, rid) || v_get(R, rid, a.rkey) != k) continue;
            int64_t ro = v_get(R, rid, a.rout);
            if (ro == lo) continue;
            if (write) { rows_put(&out->rows, at + n, 0u, lo); rows_put(&out->rows, at + n, 1u, ro); }
            n++;
        }
    }
    return n;
}

UKL_REFLECT_STRUCT(JoinArgs, lbase, ltop, rbase, rtop, out, n, idx, lkey, lout, rkey, rout)

DEFINE_KERNEL2(blocks, join_count, ARGS, JoinArgs, a, , RWBUFFER, uint8_t, b1, "[block]", RWBUFFER, uint8_t, b2, "[block]",
               RWBUFFER, uint32_t, counts, "[n]", GID0, uint32_t, id, )
{
    if (id >= a.n) return;
    View L = view_of(b1, b2, a.lbase, a.ltop);
    View R = view_of(b1, b2, a.rbase, a.rtop);
    counts[id] = join_row(L, R, a, id, (UKL_GLOBAL const Relation*)0, 0u, false);
}

DEFINE_KERNEL2(blocks, join_write, ARGS, JoinArgs, a, , RWBUFFER, uint8_t, b1, "[block]", RWBUFFER, uint8_t, b2, "[block]",
               RWBUFFER, uint32_t, counts, "[n]", RWBUFFER, uint8_t, b3, "[block]", GID0, uint32_t, id, )
{
    if (id >= a.n) return;
    uint32_t c = counts[id];
    if (c == 0u) return;
    UKL_GLOBAL Relation* out = (UKL_GLOBAL Relation*)(b3 + a.out);
    uint32_t at = ukl_atomic_add((UKL_GLOBAL uint32_t*)&out->rows.len, c);
    if (at + c > out->rows.reserved) {
        ukl_atomic_add((UKL_GLOBAL uint32_t*)&out->rows.needed, c);
        return;
    }
    View L = view_of(b1, b2, a.lbase, a.ltop);
    View R = view_of(b1, b2, a.rbase, a.rtop);
    join_row(L, R, a, id, out, at, true);
}
"#;

/// The kernels' whole source (UKL): the generated structs (with their assertions), the relation's reads, then the
/// accessors and kernels.
pub fn source() -> String {
    let mut s = String::from("#include <ukl/backend.h>\n");
    s += &ukl_of::<Relation>();
    s += &ukl_of::<JoinArgs>();
    s += crate::RELATION_READS;
    s += LIB;
    s
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux", windows)))]
mod tests {
    use super::*;
    use crate::block::{Block, kind};
    use crate::relation::{RelW, Spec, freeze};
    use std::sync::Mutex;

    /// The kernels read a relation as the host does, through the reads written once (decision 0325): every row's
    /// value in each column (a 4-byte and an 8-byte one, over many chunks), its dead bit, and its key's index group,
    /// for a frozen base and a frozen delta of it in another block. On the process's device, whatever its backend.
    #[test]
    fn the_device_reads_a_relation_as_the_host_does() {
        use rerail_gpu::device::ComputeQueue;
        use rerail_gpu::kernels::{alloc, launch, read};
        let Some(shared) = rerail_gpu::shared::get() else { return };
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

        /// The probe's parameters: the relation's offset in its block, its rows.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct ProbeP {
            rel: u64,
            n: u32,
            pad: u32,
        }
        unsafe impl rerail_gpu::Pod for ProbeP {}
        const KERNEL: &str = r#"
typedef struct ProbeP { uint64_t rel; uint32_t n; uint32_t pad; } ProbeP;
UKL_REFLECT_STRUCT(ProbeP, rel, n, pad)
DEFINE_KERNEL2(blocks_tests, reads_probe, ROBUFFER, uint8_t, b, "[block]", ARGS, ProbeP, p, , RWBUFFER, uint64_t, out, "[n * 4]",
               GID0, uint32_t, id, )
{
    if (id >= p.n) return;
    uint64_t rel = p.rel;
    UKL_GLOBAL uint64_t* o = out + id * 4u;
    UKL_GLOBAL const Relation* r = (UKL_GLOBAL const Relation*)(b + rel);
    o[0] = clif_rel_get(b, rel, r->base_rows + id, 0u, 0ul);
    o[1] = clif_rel_get(b, rel, r->base_rows + id, 1u, 0ul);
    o[2] = clif_rel_dead(b, rel, id);
    o[3] = clif_rel_group(b, rel, 0u, mix(SEED, (int64_t)o[1]));
}
"#;
        let program = rerail_gpu::kernels::program("reads_probe.uglykl", Box::leak((source() + KERNEL).into_boxed_str())).unwrap_or_else(|e| panic!("the kernels compile: {e}"));
        let probe = program.function("reads_probe").unwrap();
        for (block, rel) in [(&base_block, base), (&top_block, top)] {
            let n = rel.rows.len();
            let at = block.offset_of(std::ptr::from_ref(rel)) as u64;
            let out = alloc::<u64>(n as usize * 4).unwrap();
            let mut q = ComputeQueue::new(&shared.queue);
            launch(&q, probe)
                // The relation's block, read where it lies.
                .buffer("b", &block.device_buffer(&shared.device).unwrap())
                .value("p", &ProbeP { rel: at, n, pad: 0 })
                .buffer("out", &out)
                .run(&[u64::from(n).div_ceil(64)], &[64])
                .unwrap_or_else(|e| panic!("{e}"));
            q.flush().await_().unwrap();
            let got: Vec<u64> = read(&out, n as usize * 4);
            for i in 0..n {
                let id = rel.base_rows + i;
                let key = rel.own(id, 1);
                let group = rel.group(0, crate::key_hash(&[key]));
                let start = if group.is_empty() { 0 } else { (group.as_ptr() as usize - rel.index_at(0).rows.as_slice().as_ptr() as usize) / 4 };
                let want = [rel.own(id, 0) as u64, key as u64, u64::from(rel.dead_bit(i)), ((start as u64) << 32) | group.len() as u64];
                assert_eq!(&got[i as usize * 4..][..4], &want, "row {id} of a relation of {n}");
            }
        }
    }
}
