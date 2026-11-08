// The sinks of block programs on Metal, each lowered from the IR (`native_ir::BlockNative`): the
// output masks at their delays, the extracts' positions and the totals. Written in the same
// launch as the masks. A chunk is one SIMD group's span of a file (32 lanes of W bytes).

// A lane's word cut to the stream at delay `d`: the bits `d <= gl * W + i < len + d`.
template <typename M> static inline M stream_cut(M v, uint gl, uint len, uint d) {
  const uint W = uint(sizeof(M)) * 8u;
  return M(v & within_lane<M>(len + d, gl * W) & M(~within_lane<M>(d, gl * W)));
}

// Bits ORed into word `j` of an output of 16-, 32- or 64-bit words, by 32-bit atomics.
static inline void or_word(device atomic_uint* o, uint j, ushort x) {
  atomic_fetch_or_explicit(&o[j >> 1u], uint(x) << ((j & 1u) * 16u), memory_order_relaxed);
}
static inline void or_word(device atomic_uint* o, uint j, uint x) { atomic_fetch_or_explicit(&o[j], x, memory_order_relaxed); }
static inline void or_word(device atomic_uint* o, uint j, ulong x) {
  atomic_fetch_or_explicit(&o[2u * j], uint(x), memory_order_relaxed);
  atomic_fetch_or_explicit(&o[2u * j + 1u], uint(x >> 32u), memory_order_relaxed);
}

// An output at delay `d` (`native_ir::BlockNative::outputs`): lane gl's bit i is the stream's byte
// gl * W + i - d. With q = d / W and r = d % W, its bits land in the stream's words gl - q
// (`v >> r`) and gl - q - 1 (`v << (W - r)`). Both are ORed in, since the lanes of one stream word
// can be in two groups; the buffer starts zeroed. A word before the stream takes nothing: its
// bits are cut.
template <typename M> static inline void write_output(device M* out, uint base, uint gl, uint len, uint d, M v) {
  const uint W = uint(sizeof(M)) * 8u;
  const M w = stream_cut(v, gl, len, d);
  const uint r = d % W;
  const int j1 = int(gl) - int(d / W), j0 = j1 - 1;
  const M lo = M(lane_bits(w) >> r);
  const M hi = M(select(0ul, lane_bits(M(ulong(w) << ((W - r) % W))), r != 0u));
  device atomic_uint* o = (device atomic_uint*)out;
  or_word(o, base + uint(max(j1, 0)), M(select(M(0), lo, j1 >= 0)));
  or_word(o, base + uint(max(j0, 0)), M(select(M(0), hi, j0 >= 0)));
}

// A counted output (`native_ir::BlockNative::extracts` and `totals`): the stream's set bits,
// counted from the file's start. The count is a carry like the others, combined by addition: the
// lanes' counts summed in the SIMD group, the group's aggregate published, and the predecessors'
// states read back to an inclusive count, by `look_back`'s protocol. The file's last group writes
// the total. With `write`, each lane's bits are written as positions at the count before them,
// one per set bit (at most W). Data-dependent, so not lane code: plain `inline`. False when a
// predecessor has not published its count yet.
template <typename M>
inline bool count_sink(M v, uint gl, uint g, uint l, uint groups, uint len, uint delay, device atomic_uint* state,
                       device uint* positions, uint capacity, bool write, device uint* total, uint spins) {
  const uint W = uint(sizeof(M)) * 8u;
  M w = stream_cut(v, gl, len, delay);
  const uint n = uint(popcount(lane_bits(w)));
  const uint agg = simd_sum(n);
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_AGGREGATE | agg, memory_order_relaxed);
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
  if (l == 0u && g + 1u == groups) *total = prefix + agg;
  if (write) {
    const uint at = prefix + simd_prefix_exclusive_sum(n);
    for (uint j = 0u; j < W && w != M(0); j++) {
      if (at + j < capacity) positions[at + j] = gl * W + uint(ctz(w)) - delay;
      w = M(w & M(w - M(1)));
    }
  }
  return true;
}

// A counted output once its count before the group is known (`prefix`, from generated MSL's
// exchange of every count at once): the file's last group writes the total; with `write`, each
// lane's bits `w` (`n` of them, `agg` in the group) are written as positions at the count before
// them, one per set bit (at most W).
template <typename M>
inline void count_finish(M w, uint n, uint agg, uint prefix, uint gl, uint g, uint l, uint groups, uint delay,
                         device uint* positions, uint capacity, bool write, device uint* total) {
  const uint W = uint(sizeof(M)) * 8u;
  if (l == 0u && g + 1u == groups) *total = prefix + agg;
  if (write) {
    const uint at = prefix + simd_prefix_exclusive_sum(n);
    for (uint j = 0u; j < W && w != M(0); j++) {
      if (at + j < capacity) positions[at + j] = gl * W + uint(ctz(w)) - delay;
      w = M(w & M(w - M(1)));
    }
  }
}
