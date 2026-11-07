// The extract pass of block programs on Metal (`docs/parser-generators/10-lowering-to-bit-operations.md`
// §4.1): a plane's set bits as byte positions. A chunk is one SIMD group's span of the stream (32
// lanes of W bytes).
//
// 1. Each chunk's count: generated MSL's `tempo_block` writes it as it writes the plane;
//    the interpreter runs `block_extract_count_W` over the plane's slot.
// 2. `block_extract_scan` turns the counts into each chunk's offset, and the total after the last.
//    The host reads the total and allocates the positions once, at their final size.
// 3. `block_extract_scatter_W` writes each lane's positions from its chunk's offset and the
//    lane's prefix count in the group: the only work that depends on the data, at most W writes
//    per lane.

// A lane's word with the bits at and past byte `len` cleared (lane `gl` holds bytes
// gl*W .. gl*W + W - 1, its word one bit per byte).
template <typename M> static inline M extract_word(M w, uint gl, uint len) {
  const uint bits = uint(sizeof(M)) * 8u;
  const uint have = uint(clamp(int(len) - int(gl * bits), 0, int(bits)));
  const ulong low = (1ul << min(have, 63u)) - 1ul;
  return M(w & M(select(low, ~0ul, have == bits)));
}

// The group's count. Every lane writes the same total: no branch in the mask pass.
static inline void extract_count(uint n, uint g, device uint* counts) {
  counts[g] = simd_sum(n);
}

// One SIMD group: each chunk's exclusive offset, then the total at `offsets[chunks]`.
kernel void block_extract_scan(device const uint* counts [[buffer(0)]], device uint* offsets [[buffer(1)]],
                               constant uint& chunks [[buffer(2)]], uint l [[thread_index_in_simdgroup]]) {
  uint running = 0u;
  for (uint base = 0u; base < chunks; base += 32u) {
    const uint i = base + l;
    const uint n = select(0u, counts[min(i, chunks - 1u)], i < chunks);
    const uint before = simd_prefix_exclusive_sum(n);
    if (i < chunks) offsets[i] = running + before;
    running += simd_sum(n);
  }
  if (l == 0u) offsets[chunks] = running;
}

// The common case in one dispatch (generated MSL's `tempo_block`): each group publishes its
// count, finds its offset by a bounded look-back over the groups before (the carries' flagged
// states), and writes its positions directly into a buffer of `capacity` (an upper bound: for
// positions, the stream's length). A group whose look-back runs out of polls writes nothing and
// sets `deferred`; the host then runs the scan and the scatter above, which write every group.
// Data-dependent, so not lane code: plain `inline`, outside the mask pass's straight-line code.
template <typename M>
inline void extract_write(M v, uint gl, uint g, uint l, uint len, device atomic_uint* state, device uint* positions,
                          uint capacity, device atomic_uint* deferred, device uint* counts, uint spins) {
  const uint W = uint(sizeof(M)) * 8u;
  M w = extract_word(v, gl, len);
  const uint n = uint(popcount(w));
  const uint agg = simd_sum(n);
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_AGGREGATE | agg, memory_order_relaxed);
  counts[g] = agg;
  uint prefix = 0u;
  bool known = true;
  for (uint back = 1u; back <= g; back++) {
    uint st = 0u;
    for (uint spin = 0u; spin < spins && (st >> 30u) == 0u; spin++) {
      st = simd_broadcast(atomic_load_explicit(&state[g - back], memory_order_relaxed), ushort(0));
    }
    if ((st >> 30u) == 0u) {
      known = false;
      break;
    }
    prefix += st & CARRY_VALUE;
    if ((st >> 30u) == 2u) break;
  }
  if (!known) {
    if (l == 0u) atomic_store_explicit(deferred, 1u, memory_order_relaxed);
    return;
  }
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_INCLUSIVE | (prefix + agg), memory_order_relaxed);
  const uint at = prefix + simd_prefix_exclusive_sum(n);
  for (uint j = 0u; j < W && w != M(0); j++) {
    if (at + j < capacity) positions[at + j] = gl * W + uint(ctz(w));
    w = M(w & M(w - M(1)));
  }
}

#define EXTRACT_KERNELS(W, M)                                                                         \
  kernel void block_extract_count_##W(device const M* plane [[buffer(0)]], device uint* counts [[buffer(1)]], \
                                      constant uint& len [[buffer(2)]], uint gl [[thread_position_in_grid]], \
                                      uint g [[threadgroup_position_in_grid]]) {                      \
    extract_count(uint(popcount(extract_word(plane[gl], gl, len))), g, counts);                   \
  }                                                                                                   \
  kernel void block_extract_scatter_##W(device const M* plane [[buffer(0)]], device const uint* offsets [[buffer(1)]], \
                                        constant uint& len [[buffer(2)]], device uint* positions [[buffer(3)]], \
                                        uint gl [[thread_position_in_grid]],                          \
                                        uint g [[threadgroup_position_in_grid]]) {                    \
    M w = extract_word(plane[gl], gl, len);                                                           \
    const uint at = offsets[g] + simd_prefix_exclusive_sum(uint(popcount(w)));                        \
    for (uint j = 0u; j < W##u && w != M(0); j++) {                                                   \
      positions[at + j] = gl * W##u + uint(ctz(w));                                                   \
      w = M(w & M(w - M(1)));                                                                         \
    }                                                                                                 \
  }

EXTRACT_KERNELS(16, ushort)
EXTRACT_KERNELS(32, uint)
EXTRACT_KERNELS(64, ulong)
