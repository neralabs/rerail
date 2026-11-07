// The extract pass of block programs on Metal (`docs/parser-generators/10-bit-programs.md`
// §4.1): a plane's set bits as byte positions, written in the same launch as the masks. A chunk
// is one SIMD group's span of a file (32 lanes of W bytes).
//
// Each group publishes its count, finds its offset by a bounded look-back over the file's groups
// before it (the carries' flagged states), and writes its positions directly into a buffer of
// `capacity` (an upper bound: the file's length). A group whose look-back runs out of polls writes
// nothing and leaves unfinished, as a carry's does; the host launches again (decision 0044).

// A lane's word with the bits at and past byte `len` cleared (lane `gl` holds bytes
// gl*W .. gl*W + W - 1, its word one bit per byte).
template <typename M> static inline M extract_word(M w, uint gl, uint len) {
  const uint bits = uint(sizeof(M)) * 8u;
  const uint have = uint(clamp(int(len) - int(gl * bits), 0, int(bits)));
  const ulong low = (1ul << min(have, 63u)) - 1ul;
  return M(w & M(select(low, ~0ul, have == bits)));
}

// Data-dependent (at most W writes per lane), so not lane code: plain `inline`, outside the mask
// pass's straight-line code. False when a predecessor has not published its count yet.
template <typename M>
inline bool extract_write(M v, uint gl, uint g, uint l, uint len, device atomic_uint* state, device uint* positions,
                          uint capacity, device uint* counts, uint spins) {
  const uint W = uint(sizeof(M)) * 8u;
  M w = extract_word(v, gl, len);
  const uint n = uint(popcount(w));
  const uint agg = simd_sum(n);
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_AGGREGATE | agg, memory_order_relaxed);
  counts[g] = agg;
  uint prefix = 0u;
  for (uint back = 1u; back <= g; back++) {
    uint st = 0u;
    for (uint spin = 0u; spin <= spins && (st >> 30u) == 0u; spin++) {
      st = simd_broadcast(atomic_load_explicit(&state[g - back], memory_order_relaxed), ushort(0));
    }
    if ((st >> 30u) == 0u) return false;
    prefix += st & CARRY_VALUE;
    if ((st >> 30u) == 2u) break;
  }
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_INCLUSIVE | (prefix + agg), memory_order_relaxed);
  const uint at = prefix + simd_prefix_exclusive_sum(n);
  for (uint j = 0u; j < W && w != M(0); j++) {
    if (at + j < capacity) positions[at + j] = gl * W + uint(ctz(w));
    w = M(w & M(w - M(1)));
  }
  return true;
}
