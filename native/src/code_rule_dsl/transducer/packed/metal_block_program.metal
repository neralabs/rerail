#include <metal_stdlib>
using namespace metal;

// Block programs' Metal interpreter (metal_block_program.rs): the set program's shape, one
// dispatch running every operation in order. Each lane owns W consecutive source bytes (W = 16,
// 32 or 64) and its W-bit word of every value, so the SIMD group's full ALU width works on
// words: a 32-lane group covers 32 * W bytes. Values are slots of lane words.
struct BlockArgs { uint ops, lanes, pad0, pad1; };
// An operation: op (0 eq, 1 const, 2 not, 3 and, 4 or, 5 xor, 6 andnot), operands a and b, and
// an immediate (eq's byte; const's pattern as b = low half, imm = high half).
struct BlockStep { uint op, a, b, imm; };

// 16 bytes equal to c (repeated in c4) as 16 bits, by SWAR on four words: an exact zero-byte
// test leaves each zero byte's high bit, and a multiply gathers the four bits of a word.
static inline uint eq16(uint4 v, uint c4) {
  const uint4 x = v ^ c4;
  const uint4 z = ~(((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}

static inline ushort eq_lane(device const uint4* p, uint c4, ushort) { return ushort(eq16(p[0], c4)); }
static inline uint eq_lane(device const uint4* p, uint c4, uint) { return eq16(p[0], c4) | (eq16(p[1], c4) << 16u); }
static inline ulong eq_lane(device const uint4* p, uint c4, ulong) {
  return ulong(eq16(p[0], c4)) | (ulong(eq16(p[1], c4)) << 16u) | (ulong(eq16(p[2], c4)) << 32u) |
         (ulong(eq16(p[3], c4)) << 48u);
}

// The lane's W bits of a 64-bit pattern that repeats every 64 bytes.
template <typename M>
static inline M pattern(ulong c, uint lane) {
  const uint w = uint(sizeof(M)) * 8u;
  return M(c >> ((lane * w) & 63u));
}

template <typename M>
static inline void run(device const uint4* bytes, device M* values, constant BlockArgs& p,
                       device const BlockStep* steps, uint lane) {
  const uint chunks = uint(sizeof(M)) / 2u;
  device const uint4* mine = bytes + lane * chunks;
  for (uint at = 0u; at < p.ops; at++) {
    const BlockStep s = steps[at];
    const M a = values[s.a * p.lanes + lane];
    const M b = values[s.b * p.lanes + lane];
    M out;
    switch (s.op) {
      case 0u: out = eq_lane(mine, s.imm * 0x01010101u, M(0)); break;
      case 1u: out = pattern<M>(ulong(s.b) | (ulong(s.imm) << 32u), lane); break;
      case 2u: out = ~a; break;
      case 3u: out = a & b; break;
      case 4u: out = a | b; break;
      case 5u: out = a ^ b; break;
      case 6u: out = a & ~b; break;
      default: out = M(0); break;
    }
    values[at * p.lanes + lane] = out;
  }
}

kernel void block_program_16(device const uint4* bytes [[buffer(0)]], device ushort* values [[buffer(1)]],
                             constant BlockArgs& p [[buffer(2)]], device const BlockStep* steps [[buffer(3)]],
                             uint lane [[thread_position_in_grid]]) {
  run<ushort>(bytes, values, p, steps, lane);
}

kernel void block_program_32(device const uint4* bytes [[buffer(0)]], device uint* values [[buffer(1)]],
                             constant BlockArgs& p [[buffer(2)]], device const BlockStep* steps [[buffer(3)]],
                             uint lane [[thread_position_in_grid]]) {
  run<uint>(bytes, values, p, steps, lane);
}

kernel void block_program_64(device const uint4* bytes [[buffer(0)]], device ulong* values [[buffer(1)]],
                             constant BlockArgs& p [[buffer(2)]], device const BlockStep* steps [[buffer(3)]],
                             uint lane [[thread_position_in_grid]]) {
  run<ulong>(bytes, values, p, steps, lane);
}
