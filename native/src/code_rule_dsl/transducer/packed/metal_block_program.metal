#include <metal_stdlib>
using namespace metal;

// Block programs' Metal interpreter (metal_block_program.rs): the set program's shape, one
// dispatch running every operation in order, with a thread per source byte and a threadgroup per
// 64-byte block (two SIMD groups of 32). A mask word is built by a SIMD-group ballot: each lane's
// bit, gathered in one instruction. Values are slots of 32-bit halves of the mask words.
struct BlockArgs { uint ops, halves, pad0, pad1; };
// An operation: op (0 eq), operands a and b, and an immediate.
struct BlockStep { uint op, a, b, imm; };

kernel void block_program(
    device const uchar* bytes [[buffer(0)]],
    device uint* values [[buffer(1)]],
    constant BlockArgs& p [[buffer(2)]],
    device const BlockStep* steps [[buffer(3)]],
    uint byte [[thread_position_in_grid]],
    uint lane [[thread_index_in_simdgroup]]) {
  const uchar b = bytes[byte];
  for (uint at = 0u; at < p.ops; at++) {
    const BlockStep s = steps[at];
    uint bits = 0u;
    switch (s.op) {
      case 0u: bits = static_cast<uint>(static_cast<simd_vote::vote_t>(simd_ballot(b == uchar(s.imm)))); break;
      default: break;
    }
    // One lane writes the SIMD group's 32 bits.
    if (lane == 0u) values[at * p.halves + byte / 32u] = bits;
    threadgroup_barrier(mem_flags::mem_device);
  }
}
