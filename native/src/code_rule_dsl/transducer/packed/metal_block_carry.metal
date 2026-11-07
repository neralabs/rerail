#include <metal_stdlib>
using namespace metal;

// Carries of block programs on Metal (`docs/parser-generators/06-block-parallel.md` §2, §5):
// each carried operation has a lane summary, an associative combine and an apply. A SIMD group
// (one threadgroup of 32 lanes) combines its lanes' summaries with SIMD-group operations; between
// threadgroups the combined summaries pass through look-back states. Shared by the interpreter
// and generated MSL. No loops and no branches here.
//
// Kinds: 0 parity (`toggle`), 1 borrow (`after_odd_run`, `fill`: bit 0 generate, bit 1 propagate).
// `prev` needs no scan: its carry is a halo, the 64 bits before a lane, from the lanes before it
// in the group and the group before's last 64 bits (published in its round, read in later ones).

// A look-back state: a flag in the top two bits (1: the group's aggregate, 2: its inclusive
// prefix), the value below.
constant uint CARRY_AGGREGATE = 1u << 30;
constant uint CARRY_INCLUSIVE = 2u << 30;
constant uint CARRY_VALUE = (1u << 30) - 1u;

static inline uint carry_identity(uint kind) { return select(0u, 2u, kind == 1u); }

// `a` then `b`.
static inline uint combine_parity(uint a, uint b) { return (a ^ b) & 1u; }
static inline uint combine_borrow(uint a, uint b) {
  const uint g = (b & 1u) | ((b >> 1u) & a & 1u);
  const uint p = (a >> 1u) & (b >> 1u) & 1u;
  return g | (p << 1u);
}
static inline uint combine_carry(uint kind, uint a, uint b) {
  return select(combine_parity(a, b), combine_borrow(a, b), kind == 1u);
}

static inline uint ballot(bool x) { return uint(static_cast<simd_vote::vote_t>(simd_ballot(x))); }

// The SIMD group's aggregate of its lanes' summaries.
static inline uint reduce_parity(uint s) { return simd_xor(s) & 1u; }
static inline uint reduce_borrow(uint s) {
  const uint G = ballot((s & 1u) != 0u), P = ballot((s & 2u) != 0u);
  const uint g = uint((ulong(G | P) + ulong(G)) >> 32u);
  return g | (uint(P == 0xffffffffu) << 1u);
}

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

// The lane's word of each carried operation, from its operand `a` and its carry in.
// The 64 bits before lane l (the newest at bit 63): the words of the lanes before it, then the
// group before's last 64 bits `P` where the group runs out.
// Lane l - d's word: `w` (shuffled up by d) inside the group, else the word `d - l` back in `P`.
static inline ulong halo_word(ulong w, uint d, uint l, ulong P, uint bits) {
  const uint back = uint(max(int(d) - int(l), 1));
  return select(w, P >> (64u - back * bits), l < d);
}
static inline ulong halo_before(ushort a, uint l, ulong P) {
  const ulong m = 0xfffful;
  return ((halo_word(simd_shuffle_up(a, 1u), 1u, l, P, 16u) & m) << 48u) |
         ((halo_word(simd_shuffle_up(a, 2u), 2u, l, P, 16u) & m) << 32u) |
         ((halo_word(simd_shuffle_up(a, 3u), 3u, l, P, 16u) & m) << 16u) |
         (halo_word(simd_shuffle_up(a, 4u), 4u, l, P, 16u) & m);
}
static inline ulong halo_before(uint a, uint l, ulong P) {
  const ulong m = 0xfffffffful;
  return ((halo_word(simd_shuffle_up(a, 1u), 1u, l, P, 32u) & m) << 32u) |
         (halo_word(simd_shuffle_up(a, 2u), 2u, l, P, 32u) & m);
}
// SIMD-group shuffles have no 64-bit form: the halves move separately.
static inline ulong shuffle_up64(ulong a, ushort d) {
  return ulong(simd_shuffle_up(uint(a), d)) | (ulong(simd_shuffle_up(uint(a >> 32u), d)) << 32u);
}
static inline ulong broadcast64(ulong a, ushort lane) {
  return ulong(simd_broadcast(uint(a), lane)) | (ulong(simd_broadcast(uint(a >> 32u), lane)) << 32u);
}
static inline ulong halo_before(ulong a, uint l, ulong P) {
  return select(shuffle_up64(a, 1), P, l == 0u);
}
// The group's last 64 bits, for the group after.
static inline ulong halo_last(ushort a) {
  return (ulong(simd_broadcast(a, 31u)) << 48u) | (ulong(simd_broadcast(a, 30u)) << 32u) |
         (ulong(simd_broadcast(a, 29u)) << 16u) | ulong(simd_broadcast(a, 28u));
}
static inline ulong halo_last(uint a) { return (ulong(simd_broadcast(a, 31u)) << 32u) | ulong(simd_broadcast(a, 30u)); }
static inline ulong halo_last(ulong a) { return broadcast64(a, 31); }
// `prev` by k (1..63): the word shifted up, the newest k bits of the halo below.
template <typename M> static inline M apply_prev(M a, uint k, ulong before) {
  return M((ulong(a) << k) | (before >> (64u - k)));
}
// The halo of group g's lanes: the group before's last 64 bits (zero for the first group).
static inline ulong halo_of_group_before(device const ulong* halos, uint g) {
  return select(0ul, halos[max(g, 1u) - 1u], g > 0u);
}

// The byte before 16-byte chunk `first` of the source (0x00 before the stream): for `pair` and
// the byte before's bit-planes.
static inline uint byte_before(device const uint4* bytes, uint first) {
  return select(0u, bytes[max(first, 1u) - 1u].w >> 24u, first > 0u);
}
// 16 bytes one back: each byte replaced by the byte before it, `before` the byte before the first.
static inline uint4 bytes_before(uint4 v, uint before) {
  return uint4((v.x << 8u) | before, (v.y << 8u) | (v.x >> 24u), (v.z << 8u) | (v.y >> 24u), (v.w << 8u) | (v.z >> 24u));
}
// An XOR prefix scan of a word across the SIMD group (Hillis-Steele, five shuffles): for scans of
// more than one bit; one bit's parity is a ballot's popcount (`lane_in_parity`).
static inline uint simd_prefix_inclusive_xor32(uint v, uint l) {
  v ^= select(0u, simd_shuffle_up(v, ushort(1)), l >= 1u);
  v ^= select(0u, simd_shuffle_up(v, ushort(2)), l >= 2u);
  v ^= select(0u, simd_shuffle_up(v, ushort(4)), l >= 4u);
  v ^= select(0u, simd_shuffle_up(v, ushort(8)), l >= 8u);
  v ^= select(0u, simd_shuffle_up(v, ushort(16)), l >= 16u);
  return v;
}

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

// Bitlex's `fill(s, e)` on a lane word (`block::carry::fill`): the carry into each bit of
// s + (s | ~e) + cin is the fill coming into that byte; `out` is the carry out, the last bit.
// Computed on 64 bits with the lane's mask: on 16-bit words the arithmetic in M gave a correct
// result but a wrong ballot of the summary (lane_in_borrow saw no generate) on Apple's compiler.
template <typename M> static inline M fill_word(M s, M e, M cin, thread uint& out) {
  const uint bits = uint(sizeof(M)) * 8u;
  const ulong mask = ~0ul >> (64u - bits);
  const ulong S = ulong(s), b = S | (~ulong(e) & mask);
  const ulong into = (S + b + ulong(cin)) ^ S ^ b;
  const ulong f = (S | (~ulong(e) & into)) & mask;
  out = uint(f >> (bits - 1u)) & 1u;
  return M(f);
}
template <typename M> static inline uint summary_fill(M s, M e) {
  uint c0 = 0u, c1 = 0u;
  fill_word(s, e, M(0), c0);
  fill_word(s, e, M(1), c1);
  return c0 | ((c1 & ~c0) << 1u);
}
template <typename M> static inline M apply_fill(M s, M e, uint cin) {
  uint unused = 0u;
  return fill_word(s, e, M(cin), unused);
}

// The group's exclusive prefix of a finished round, from its state array: the inclusive prefix
// of the group before (the identity for the first group).
static inline uint carry_before(device atomic_uint* state, uint kind, uint g) {
  const uint v = atomic_load_explicit(&state[max(g, 1u) - 1u], memory_order_relaxed) & CARRY_VALUE;
  return select(carry_identity(kind), v, g > 0u);
}
