// Device-side enqueue experiments: an unbounded fixpoint (a recursive utility) run as frontier waves.
// Workload: R(n) := T(n) OR (P(n) AND (R(parent n) OR R(some child of n))), pushed from the seeds.
// The number of waves is the longest shortest path inside the P-component: data-dependent, unbounded.
#include <metal_stdlib>
using namespace metal;

constant uint NONE = 0xffffffffu;
struct Indirect { uint x, y, z; };

// counters: [0] count of F0, [1] count of F1, [2] waves that did work, [3] marked total, [4] mode (0 device waves, 1 tg loop handed off, 2 done)
// [5] waves run inside the threadgroup loop, [6] overflow
struct Tape {
    device const ushort* kind;
    device const uint* parent;
    device const uint* size;
    device const uchar* pass;
};

inline void visit(uint j, device const ushort* kind, device const uchar* pass, device atomic_uint* mark,
                  device uint* dst, device atomic_uint* dstCount, uint cap, device atomic_uint* counters) {
    if (pass[kind[j]] == 0) return;
    if (atomic_exchange_explicit(&mark[j], 1u, memory_order_relaxed) != 0u) return;
    uint w = atomic_fetch_add_explicit(dstCount, 1u, memory_order_relaxed);
    if (w < cap) dst[w] = j; else atomic_fetch_add_explicit(&counters[6], 1u, memory_order_relaxed);
}

// One wave over the source frontier. srcCount is read from counters[src].
kernel void expand(device const ushort* kind [[buffer(0)]],
                   device const uint* parent [[buffer(1)]],
                   device const uint* size [[buffer(2)]],
                   device const uchar* pass [[buffer(3)]],
                   device atomic_uint* mark [[buffer(4)]],
                   device const uint* src [[buffer(5)]],
                   device uint* dst [[buffer(6)]],
                   device atomic_uint* counters [[buffer(7)]],
                   constant uint2& sel [[buffer(8)]],   // x = src slot, y = cap
                   uint tid [[thread_position_in_grid]]) {
    uint count = atomic_load_explicit(&counters[sel.x], memory_order_relaxed);
    if (tid >= count) return;
    uint i = src[tid];
    device atomic_uint* dstCount = &counters[1u - sel.x];
    uint p = parent[i];
    if (p != NONE) visit(p, kind, pass, mark, dst, dstCount, sel.y, counters);
    uint end = i + size[i];
    for (uint c = i + 1; c < end; c += size[c]) visit(c, kind, pass, mark, dst, dstCount, sel.y, counters);
}

// Scheduler step after a wave: size the next dispatch from the new count, consume the old frontier.
// If handoff > 0 and the frontier is narrow, switch to the single-threadgroup loop (mode 1).
kernel void prep(device atomic_uint* counters [[buffer(0)]],
                 device Indirect* args [[buffer(1)]],      // [0] expand args, [1] tg-loop args
                 constant uint3& sel [[buffer(2)]],        // x = slot just produced, y = handoff threshold
                 uint tid [[thread_position_in_grid]]) {
    uint mode = atomic_load_explicit(&counters[4], memory_order_relaxed);
    if (mode != 0u) { args[0] = Indirect{0u, 1u, 1u}; return; }   // handed off or done: leave frontier and loop args alone
    uint produced = atomic_load_explicit(&counters[sel.x], memory_order_relaxed);
    atomic_store_explicit(&counters[1u - sel.x], 0u, memory_order_relaxed);
    if (produced == 0u) { atomic_store_explicit(&counters[4], 2u, memory_order_relaxed); args[0] = Indirect{0u, 1u, 1u}; return; }
    atomic_fetch_add_explicit(&counters[2], 1u, memory_order_relaxed);
    atomic_fetch_add_explicit(&counters[3], produced, memory_order_relaxed);
    if (sel.y > 0u && produced <= sel.y) {
        atomic_store_explicit(&counters[4], 1u, memory_order_relaxed);
        args[0] = Indirect{0u, 1u, 1u};
        args[1] = Indirect{1u, 1u, 1u};
        return;
    }
    args[0] = Indirect{(produced + 255u) / 256u, 1u, 1u};
}

// Single-threadgroup persistent loop: forward progress is guaranteed inside one threadgroup (barriers),
// unlike a grid-wide persistent kernel. Runs all remaining waves of a narrow frontier in one dispatch.
// Frontier lives in device buffers; the slot parity continues from the device waves.
kernel void tg_loop(device const ushort* kind [[buffer(0)]],
                    device const uint* parent [[buffer(1)]],
                    device const uint* size [[buffer(2)]],
                    device const uchar* pass [[buffer(3)]],
                    device atomic_uint* mark [[buffer(4)]],
                    device uint* f0 [[buffer(5)]],
                    device uint* f1 [[buffer(6)]],
                    device atomic_uint* counters [[buffer(7)]],
                    constant uint2& sel [[buffer(8)]],    // x = slot holding the live frontier, y = cap
                    constant uint& limit [[buffer(9)]],
                    uint lid [[thread_position_in_threadgroup]],
                    uint lsize [[threads_per_threadgroup]]) {
    threadgroup uint live;
    threadgroup uint slot;
    if (lid == 0) {
        // the live frontier is whichever slot is non-empty (prep zeroed the consumed one)
        uint c0 = atomic_load_explicit(&counters[0], memory_order_relaxed);
        slot = c0 > 0u ? 0u : 1u;
        live = atomic_load_explicit(&counters[slot], memory_order_relaxed);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint waves = 0;
    while (live > 0u && waves < limit) {
        device uint* src = slot == 0u ? f0 : f1;
        device uint* dst = slot == 0u ? f1 : f0;
        device atomic_uint* dstCount = &counters[1u - slot];
        for (uint t = lid; t < live; t += lsize) {
            uint i = src[t];
            uint p = parent[i];
            if (p != NONE) visit(p, kind, pass, mark, dst, dstCount, sel.y, counters);
            uint end = i + size[i];
            for (uint c = i + 1; c < end; c += size[c]) visit(c, kind, pass, mark, dst, dstCount, sel.y, counters);
        }
        threadgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup);
        if (lid == 0) {
            uint produced = atomic_load_explicit(dstCount, memory_order_relaxed);
            atomic_store_explicit(&counters[slot], 0u, memory_order_relaxed);
            if (produced > 0u) {
                atomic_fetch_add_explicit(&counters[2], 1u, memory_order_relaxed);
                atomic_fetch_add_explicit(&counters[3], produced, memory_order_relaxed);
                atomic_fetch_add_explicit(&counters[5], 1u, memory_order_relaxed);
            }
            live = produced;
            slot = 1u - slot;
        }
        threadgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup);
        waves++;
    }
    if (lid == 0 && live == 0u) atomic_store_explicit(&counters[4], 2u, memory_order_relaxed);
}

// ---- GPU-encoded indirect command buffer ----
// The scheduler kernel encodes the next wave's expand command into an ICB slot: pipeline, buffers, grid.
struct ICBArgs {
    command_buffer cmds [[id(0)]];
    compute_pipeline_state expandPSO [[id(1)]];
};
struct ExpandBinds {
    device const ushort* kind;
    device const uint* parent;
    device const uint* size;
    device const uchar* pass;
    device atomic_uint* mark;
    device uint* f0;
    device uint* f1;
    device atomic_uint* counters;
    device uint2* sel;     // sel[0] for src slot 0, sel[1] for src slot 1
};

kernel void prep_icb(device ICBArgs& icb [[buffer(0)]],
                     device ExpandBinds& b [[buffer(1)]],
                     constant uint2& w [[buffer(2)]],      // x = slot just produced, y = ICB slot to encode
                     uint tid [[thread_position_in_grid]]) {
    device atomic_uint* counters = b.counters;
    uint mode = atomic_load_explicit(&counters[4], memory_order_relaxed);
    uint produced = atomic_load_explicit(&counters[w.x], memory_order_relaxed);
    atomic_store_explicit(&counters[1u - w.x], 0u, memory_order_relaxed);
    compute_command cmd(icb.cmds, w.y);
    if (mode != 0u || produced == 0u) {
        if (produced == 0u) atomic_store_explicit(&counters[4], 2u, memory_order_relaxed);
        cmd.reset();
        return;
    }
    atomic_fetch_add_explicit(&counters[2], 1u, memory_order_relaxed);
    atomic_fetch_add_explicit(&counters[3], produced, memory_order_relaxed);
    cmd.set_compute_pipeline_state(icb.expandPSO);
    cmd.set_kernel_buffer(b.kind, 0);
    cmd.set_kernel_buffer(b.parent, 1);
    cmd.set_kernel_buffer(b.size, 2);
    cmd.set_kernel_buffer(b.pass, 3);
    cmd.set_kernel_buffer(b.mark, 4);
    cmd.set_kernel_buffer(w.x == 0u ? b.f0 : b.f1, 5);
    cmd.set_kernel_buffer(w.x == 0u ? b.f1 : b.f0, 6);
    cmd.set_kernel_buffer(b.counters, 7);
    cmd.set_kernel_buffer(&b.sel[w.x], 8);
    cmd.concurrent_dispatch_threadgroups(uint3((produced + 255u) / 256u, 1, 1), uint3(256, 1, 1));
    cmd.set_barrier();
}

// Variant for "self-encoding": a prep command living inside the same ICB encodes a later slot of the same ICB.
kernel void prep_icb_self(device ICBArgs& icb [[buffer(0)]],
                          device ExpandBinds& b [[buffer(1)]],
                          device const uint2* wtab [[buffer(2)]],
                          constant uint& k [[buffer(3)]],
                          uint tid [[thread_position_in_grid]]) {
    uint2 w = wtab[k];
    device atomic_uint* counters = b.counters;
    uint produced = atomic_load_explicit(&counters[w.x], memory_order_relaxed);
    atomic_store_explicit(&counters[1u - w.x], 0u, memory_order_relaxed);
    compute_command cmd(icb.cmds, w.y);
    if (produced == 0u) { atomic_store_explicit(&counters[4], 2u, memory_order_relaxed); cmd.reset(); return; }
    atomic_fetch_add_explicit(&counters[2], 1u, memory_order_relaxed);
    atomic_fetch_add_explicit(&counters[3], produced, memory_order_relaxed);
    cmd.set_compute_pipeline_state(icb.expandPSO);
    cmd.set_kernel_buffer(b.kind, 0);
    cmd.set_kernel_buffer(b.parent, 1);
    cmd.set_kernel_buffer(b.size, 2);
    cmd.set_kernel_buffer(b.pass, 3);
    cmd.set_kernel_buffer(b.mark, 4);
    cmd.set_kernel_buffer(w.x == 0u ? b.f0 : b.f1, 5);
    cmd.set_kernel_buffer(w.x == 0u ? b.f1 : b.f0, 6);
    cmd.set_kernel_buffer(b.counters, 7);
    cmd.set_kernel_buffer(&b.sel[w.x], 8);
    cmd.concurrent_dispatch_threadgroups(uint3((produced + 255u) / 256u, 1, 1), uint3(256, 1, 1));
    cmd.set_barrier();
}
