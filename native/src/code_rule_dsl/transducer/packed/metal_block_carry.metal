#include <metal_stdlib>
using namespace metal;

// Carries of block programs on Metal (`docs/parser-generators/06-block-parallel.md` §2, §5):
// each carried operation has a lane summary, an associative combine and an apply. A SIMD group
// (one threadgroup of 32 lanes) combines its lanes' summaries with SIMD-group operations; between
// threadgroups the combined summaries pass through look-back states. Shared by the interpreter
// and generated MSL. No loops and no branches here.
//
// Kinds: 0 parity (`toggle`), 1 borrow (`after_odd_run`: bit 0 generate, bit 1 propagate),
// 2 halo (`prev`: the top k bits, with bit 16 marking a value present).

// A look-back state: a flag in the top two bits (1: the group's aggregate, 2: its inclusive
// prefix), the value below.
constant uint CARRY_AGGREGATE = 1u << 30;
constant uint CARRY_INCLUSIVE = 2u << 30;
constant uint CARRY_VALUE = (1u << 30) - 1u;
constant uint HALO_PRESENT = 1u << 16;

static inline uint carry_identity(uint kind) { return select(select(0u, 2u, kind == 1u), 0u, kind == 2u); }

// `a` then `b`.
static inline uint combine_parity(uint a, uint b) { return (a ^ b) & 1u; }
static inline uint combine_borrow(uint a, uint b) {
  const uint g = (b & 1u) | ((b >> 1u) & a & 1u);
  const uint p = (a >> 1u) & (b >> 1u) & 1u;
  return g | (p << 1u);
}
static inline uint combine_halo(uint a, uint b) { return select(a, b, (b & HALO_PRESENT) != 0u); }
static inline uint combine_carry(uint kind, uint a, uint b) {
  return select(select(combine_parity(a, b), combine_borrow(a, b), kind == 1u), combine_halo(a, b), kind == 2u);
}

static inline uint ballot(bool x) { return uint(static_cast<simd_vote::vote_t>(simd_ballot(x))); }

// The SIMD group's aggregate of its lanes' summaries.
static inline uint reduce_parity(uint s) { return simd_xor(s) & 1u; }
static inline uint reduce_borrow(uint s) {
  const uint G = ballot((s & 1u) != 0u), P = ballot((s & 2u) != 0u);
  const uint g = uint((ulong(G | P) + ulong(G)) >> 32u);
  return g | (uint(P == 0xffffffffu) << 1u);
}
static inline uint reduce_halo(uint s) { return simd_broadcast(s, 31u) | HALO_PRESENT; }

// A lane's carry in: the group's exclusive prefix `e` combined with the lanes before it.
// Metal has no prefix XOR across a SIMD group; for one bit, the parity of the ballot below the lane.
static inline uint lane_in_parity(uint s, uint e, uint l) {
  const uint below = ballot((s & 1u) != 0u) & ((1u << l) - 1u);
  return (e ^ popcount(below)) & 1u;
}
static inline uint lane_in_borrow(uint s, uint e, uint l) {
  // Carry-lookahead as a 32-bit add: lane l generates when G, propagates when P; the carry into
  // bit l of (G | P) + G + carry-in is lane l's carry in.
  const uint G = ballot((s & 1u) != 0u), P = ballot((s & 2u) != 0u);
  const uint A = G | P;
  const uint sum = uint(ulong(A) + ulong(G) + ulong(e & 1u));
  return ((sum ^ A ^ G) >> l) & 1u;
}
static inline uint lane_in_halo(uint s, uint e, uint l) { return select(simd_shuffle_up(s, 1u), e & 0xffffu, l == 0u); }

// The lane's word of each carried operation, from its operand `a` and its carry in.
template <typename M> static inline uint summary_prev(M a, uint k) { return uint(a >> (uint(sizeof(M)) * 8u - k)); }
template <typename M> static inline M apply_prev(M a, uint k, uint cin) { return M(M(a << k) | M(cin)); }

static inline ushort prefix_xor(ushort x) {
  x ^= ushort(x << 1u); x ^= ushort(x << 2u); x ^= ushort(x << 4u); x ^= ushort(x << 8u);
  return x;
}
static inline uint prefix_xor(uint x) {
  x ^= x << 1u; x ^= x << 2u; x ^= x << 4u; x ^= x << 8u; x ^= x << 16u;
  return x;
}
static inline ulong prefix_xor(ulong x) {
  x ^= x << 1u; x ^= x << 2u; x ^= x << 4u; x ^= x << 8u; x ^= x << 16u; x ^= x << 32u;
  return x;
}
template <typename M> static inline uint summary_toggle(M a) { return uint(popcount(a)) & 1u; }
template <typename M> static inline M apply_toggle(M a, uint cin) { return M(prefix_xor(a) ^ M(M(0) - M(cin))); }

// simdjson's find_escaped_branchless on a lane word, with the carry in; `overflow` is the carry out.
template <typename M> static inline M after_odd_run(M run, M cin, thread uint& overflow) {
  const M even = M(0x5555555555555555ul);
  const M b = M(run & M(~cin));
  const M follows = M(M(b << 1u) | cin);
  const M odd_starts = M(b & M(~even) & M(~follows));
  const M sequences = M(odd_starts + b);
  overflow = uint(sequences < odd_starts);
  const M escaped = M(M(even ^ M(sequences << 1u)) & follows);
  return M(escaped & M(~run));
}
template <typename M> static inline uint summary_after_odd_run(M a) {
  uint c0 = 0u, c1 = 0u;
  after_odd_run(a, M(0), c0);
  after_odd_run(a, M(1), c1);
  return c0 | ((c1 & ~c0) << 1u);
}
template <typename M> static inline M apply_after_odd_run(M a, uint cin) {
  uint unused = 0u;
  return after_odd_run(a, M(cin), unused);
}

// The group's exclusive prefix of a finished round, from its state array: the inclusive prefix
// of the group before (the identity for the first group).
static inline uint carry_before(device atomic_uint* state, uint kind, uint g) {
  const uint v = atomic_load_explicit(&state[max(g, 1u) - 1u], memory_order_relaxed) & CARRY_VALUE;
  return select(carry_identity(kind), v, g > 0u);
}
