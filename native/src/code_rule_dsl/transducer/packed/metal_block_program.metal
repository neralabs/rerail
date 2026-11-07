#include <metal_stdlib>
using namespace metal;

// Block programs' Metal interpreter (metal_block_program.rs): the set program's shape, every
// operation in order, one dispatch per round: a carried operation ends a round, its groups'
// aggregates are scanned, and the next round starts with it (metal_block_carry.metal). Each lane owns W consecutive source bytes (W = 16,
// 32 or 64) and its W-bit word of every value, so the SIMD group's full ALU width works on
// words: a 32-lane group covers 32 * W bytes. Values are slots of lane words.
// The operations a dispatch runs, [from, to), and the lanes.
struct BlockArgs { uint from, to, lanes, pad; };  // pad: the stream's length (`within`)
// An operation of the planes form (native IR, `class` written as formulas of bit-planes): op (0
// eq, 1 const, 2 not, 3 and, 4 or, 5 xor, 6 andnot, 7 plane, 11 the byte before's plane), operands a and b, and an immediate
// (eq's byte; const's pattern as b = low half, imm = high half; plane's bit), and the carried
// operations 8 prev (imm the distance), 9 toggle, 10 after_odd_run, 12 fill (imm its ends), each
// with its round in b; 13 within.
struct BlockStep { uint op, a, b, imm; };

// 16 bytes equal to c (repeated in c4) as 16 bits, by SWAR on four words: an exact zero-byte
// test leaves each zero byte's high bit, and a multiply gathers the four bits of a word.
static inline uint eq16(uint4 v, uint c4) {
  const uint4 x = v ^ c4;
  const uint4 z = ~(((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}

// Bit k of 16 bytes as 16 bits: each byte's bit k moved to its high bit, then eq16's gather.
static inline uint plane16(uint4 v, uint k) {
  const uint4 z = (v << (7u - k)) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}

static inline ushort plane_lane(device const uint4* p, uint k, ushort) { return ushort(plane16(p[0], k)); }
static inline uint plane_lane(device const uint4* p, uint k, uint) { return plane16(p[0], k) | (plane16(p[1], k) << 16u); }
static inline ulong plane_lane(device const uint4* p, uint k, ulong) {
  return ulong(plane16(p[0], k)) | (ulong(plane16(p[1], k)) << 16u) | (ulong(plane16(p[2], k)) << 32u) |
         (ulong(plane16(p[3], k)) << 48u);
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
                       device const BlockStep* steps, device atomic_uint* states, device const ulong* halos, uint lane,
                       uint g, uint l) {
  const uint chunks = uint(sizeof(M)) / 2u;
  const uint groups = p.lanes / 32u;
  device const uint4* mine = bytes + lane * chunks;
  for (uint at = p.from; at < p.to; at++) {
    const BlockStep s = steps[at];
    // Operands are earlier values. Steps without one hold other things in a and b (const's
    // pattern), so the reads are clamped to slots already defined.
    const M a = values[min(s.a, at) * p.lanes + lane];
    const M b = values[min(s.b, at) * p.lanes + lane];
    // A carried step's round is in b; its state array holds each group's inclusive prefix.
    device atomic_uint* state = states + s.b * groups;
    M out;
    switch (s.op) {
      case 0u: out = eq_lane(mine, s.imm * 0x01010101u, M(0)); break;
      case 1u: out = pattern<M>(ulong(s.b) | (ulong(s.imm) << 32u), lane); break;
      case 2u: out = ~a; break;
      case 3u: out = a & b; break;
      case 4u: out = a | b; break;
      case 5u: out = a ^ b; break;
      case 6u: out = a & ~b; break;
      case 7u: out = plane_lane(mine, s.imm, M(0)); break;
      case 11u: out = M((plane_lane(mine, s.imm, M(0)) << 1u) | M((byte_before(bytes, lane * chunks) >> s.imm) & 1u)); break;
      case 8u: out = apply_prev(a, s.imm, halo_before(a, l, halo_of_group_before(halos + s.b * groups, g))); break;
      case 9u: out = apply_toggle(a, lane_in_parity(summary_toggle(a), carry_before(state, 0u, g), l)); break;
      case 10u: out = apply_after_odd_run(a, lane_in_borrow(summary_after_odd_run(a), carry_before(state, 1u, g), l)); break;
      case 12u: {
        // `fill`: its ends are in imm (b is the round).
        const M e = values[min(s.imm, at) * p.lanes + lane];
        out = apply_fill(a, e, lane_in_borrow(summary_fill(a, e), carry_before(state, 1u, g), l));
        break;
      }
      // `within`: the stream's length is the dispatch's `pad`.
      case 13u: out = within_lane<M>(p.pad, lane * uint(sizeof(M)) * 8u); break;
      default: out = M(0); break;
    }
    values[at * p.lanes + lane] = out;
  }
}

// A carried step's group aggregate: the SIMD group's combined lane summaries of its operand
// (`toggle`, `after_odd_run`), or its last 64 bits (`prev`, into `halos`).
template <typename M>
static inline uint aggregate(device const M* values, constant BlockArgs& p, device const BlockStep* steps, uint lane) {
  const BlockStep s = steps[p.to];
  const M a = values[s.a * p.lanes + lane];
  const M e = values[min(s.imm, p.to) * p.lanes + lane];
  const uint borrow = select(summary_after_odd_run(a), summary_fill(a, e), s.op == 12u);
  return select(reduce_borrow(borrow), reduce_parity(summary_toggle(a)), s.op == 9u);
}

template <typename M>
static inline ulong group_halo(device const M* values, constant BlockArgs& p, device const BlockStep* steps, uint lane) {
  const BlockStep s = steps[p.to];
  return halo_last(values[s.a * p.lanes + lane]);
}

#define BLOCK_KERNELS(W, M)                                                                                     \
  kernel void block_program_##W(device const uint4* bytes [[buffer(0)]], device M* values [[buffer(1)]],       \
                                constant BlockArgs& p [[buffer(2)]], device const BlockStep* steps [[buffer(3)]], \
                                device atomic_uint* states [[buffer(4)]], device const ulong* halos [[buffer(6)]], \
                                uint lane [[thread_position_in_grid]], uint g [[threadgroup_position_in_grid]],   \
                                uint l [[thread_index_in_simdgroup]]) {                                          \
    run<M>(bytes, values, p, steps, states, halos, lane, g, l);                                                  \
  }                                                                                                              \
  kernel void block_summary_##W(device const M* values [[buffer(1)]], constant BlockArgs& p [[buffer(2)]],      \
                                device const BlockStep* steps [[buffer(3)]], device uint* aggregates [[buffer(5)]], \
                                device ulong* halos [[buffer(6)]], uint lane [[thread_position_in_grid]],         \
                                uint g [[threadgroup_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {  \
    const uint groups = p.lanes / 32u;                                                                           \
    const BlockStep s = steps[p.to];                                                                             \
    if (s.op == 8u) {                                                                                            \
      const ulong h = group_halo<M>(values, p, steps, lane);                                                     \
      if (l == 0u) halos[s.b * groups + g] = h;                                                                  \
    } else {                                                                                                     \
      const uint agg = aggregate<M>(values, p, steps, lane);                                                     \
      if (l == 0u) aggregates[g] = agg;                                                                          \
    }                                                                                                            \
  }

BLOCK_KERNELS(16, ushort)
BLOCK_KERNELS(32, uint)
BLOCK_KERNELS(64, ulong)

// The groups' aggregates scanned into inclusive prefixes (the interpreter's threadgroup level):
// one threadgroup of 256 threads, each folding a stretch of groups, a scan of the stretches in
// threadgroup memory, then each stretch written. Loops over sizes, bounded.
kernel void block_carry_scan(device const uint* aggregates [[buffer(0)]], device uint* states [[buffer(1)]],
                             constant uint2& p [[buffer(2)]], uint t [[thread_position_in_threadgroup]]) {
  threadgroup uint totals[256];
  const uint groups = p.x, kind = p.y;
  const uint per = (groups + 255u) / 256u;
  const uint from = min(t * per, groups), to = min(from + per, groups);
  uint acc = carry_identity(kind);
  for (uint g = from; g < to; g++) acc = combine_carry(kind, acc, aggregates[g]);
  totals[t] = acc;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  for (uint d = 1u; d < 256u; d <<= 1u) {
    const uint before = t >= d ? totals[t - d] : carry_identity(kind);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    totals[t] = combine_carry(kind, before, totals[t]);
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
  acc = t > 0u ? totals[t - 1u] : carry_identity(kind);
  for (uint g = from; g < to; g++) {
    acc = combine_carry(kind, acc, aggregates[g]);
    states[g] = CARRY_INCLUSIVE | acc;
  }
}
