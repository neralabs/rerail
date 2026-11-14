#include <metal_stdlib>
using namespace metal;

// A wave's node sets on the device (decision 0024 §1): slots over the wave's node space in one
// buffer, each file's nodes from a word boundary (`FileWords`), so a file's words are its own.
// The whole program is one dispatch (list step 13): a threadgroup per file runs every step over
// the file's words, a barrier between steps, so a step sees the last (metal_set_steps.rs).
struct ProgramArgs { uint steps, words, files, pad; };
// A step: op (0 clear, 1 every node, 2 copy, 3 or, 4 and, 5 and-not, 6 within), out, a, b.
struct Step { uint op, out, a, b; };
// A file's first word in a slot, its words, and its nodes.
struct FileWords { uint first, words, nodes, pad; };

kernel void set_program(
    device ulong* values [[buffer(0)]],
    constant ProgramArgs& p [[buffer(1)]],
    device const Step* steps [[buffer(2)]],
    device const FileWords* files [[buffer(3)]],
    uint file [[threadgroup_position_in_grid]],
    uint lane [[thread_position_in_threadgroup]],
    uint width [[threads_per_threadgroup]]) {
  if (file >= p.files) return;
  FileWords f = files[file];
  threadgroup atomic_uint any;
  for (uint at = 0u; at < p.steps; at++) {
    Step s = steps[at];
    device ulong* out = values + ulong(s.out) * p.words + f.first;
    device const ulong* a = values + ulong(s.a) * p.words + f.first;
    device const ulong* b = values + ulong(s.b) * p.words + f.first;
    if (s.op == 6u) {
      // `out`'s nodes kept where `a` has a node in the file.
      if (lane == 0u) atomic_store_explicit(&any, 0u, memory_order_relaxed);
      threadgroup_barrier(mem_flags::mem_threadgroup);
      for (uint w = lane; w < f.words; w += width) {
        if (a[w] != 0ul) { atomic_store_explicit(&any, 1u, memory_order_relaxed); break; }
      }
      threadgroup_barrier(mem_flags::mem_threadgroup);
      if (atomic_load_explicit(&any, memory_order_relaxed) == 0u) {
        for (uint w = lane; w < f.words; w += width) out[w] = 0ul;
      }
    } else {
      for (uint w = lane; w < f.words; w += width) {
        ulong value;
        switch (s.op) {
          case 0u: value = 0ul; break;
          case 1u: { uint left = f.nodes - w * 64u; value = left >= 64u ? ~0ul : ((1ul << left) - 1ul); } break;
          case 2u: value = a[w]; break;
          case 3u: value = a[w] | b[w]; break;
          case 4u: value = a[w] & b[w]; break;
          case 5u: value = a[w] & ~b[w]; break;
          default: value = out[w]; break;
        }
        out[w] = value;
      }
    }
    threadgroup_barrier(mem_flags::mem_device | mem_flags::mem_threadgroup);
  }
}
