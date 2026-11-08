// Block programs' Metal interpreter (metal_block_program.rs): a batch of files, each under its own
// program, in one launch (decision 0066). Each lane owns W consecutive bytes of a file (W = 16, 32
// or 64) and its W-bit word of every value, so a 32-lane SIMD group works on 32 words at once. A
// group's values are slots of lane words in its worker's scratch, in device memory (decision
// 0034). Each step runs once; a carried step resolves its carry by look-back (`look_back`,
// `halo_exchange`) before the next.

// An operation of the planes form (native IR, `class` written as formulas of bit-planes): op (0
// eq, 1 const, 2 not, 3 and, 4 or, 5 xor, 6 andnot, 7 plane, 11 the byte before's plane), operands
// a and b, and an immediate (eq's byte; const's pattern as b = low half, imm = high half; plane's
// bit), and the carried operations 8 prev (imm the distance), 9 toggle, 10 escaped, 12 fill
// (imm its ends), each with its index among the program's carried steps in b; 13 within (imm its delay).
struct BlockStep { uint op, a, b, imm; };
// A program of the batch: its steps, its outputs (value slots, at `outs[out0..]`) and its extracts
// (output indices, at `outs[ext0..]`).
struct ProgramRow { uint step0, steps, out0, outs, ext0, exts, pad0, pad1; };

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

// A lane's word of a byte test over its chunks, from chunk `first` of the file.
template <typename M> static inline M eq_file(device const uint4* bytes, FileRow f, uint first, uint c4) {
  M out = M(0);
  for (uint c = 0u; c < uint(sizeof(M)) / 2u; c++) out = M(out | M(ulong(eq16(load16(bytes, f, first + c), c4)) << (16u * c)));
  return out;
}
template <typename M> static inline M plane_file(device const uint4* bytes, FileRow f, uint first, uint k) {
  M out = M(0);
  for (uint c = 0u; c < uint(sizeof(M)) / 2u; c++) out = M(out | M(ulong(plane16(load16(bytes, f, first + c), k)) << (16u * c)));
  return out;
}

// The lane's W bits of a 64-bit pattern that repeats every 64 bytes.
template <typename M>
static inline M pattern(ulong c, uint lane) {
  const uint w = uint(sizeof(M)) * 8u;
  return M(c >> ((lane * w) & 63u));
}

// Group g of file f (lane l): every step of its program, then its outputs and extracts. False
// when a look-back has to wait for another launch.
template <typename M>
static bool interpret_file(device const uint4* bytes, FileRow f, uint g, uint l, uint spins, device const ProgramRow* programs,
                           device const BlockStep* steps, device const uint* outs, device M* slots, device M* out,
                           device atomic_uint* states, device atomic_uint* halos, device atomic_uint* ext_states,
                           device uint* positions, device uint* counts) {
  const ProgramRow pr = programs[f.program];
  const uint chunks = uint(sizeof(M)) / 2u;
  const uint W = uint(sizeof(M)) * 8u;
  const uint gl = g * 32u + l;
  const uint lanes = f.groups * 32u;
  for (uint at = 0u; at < pr.steps; at++) {
    const BlockStep s = steps[pr.step0 + at];
    // Operands are earlier values. Steps without one hold other things in a and b (const's
    // pattern), so the reads are clamped to slots already defined.
    const M a = slots[min(s.a, at) * 32u + l];
    const M b = slots[min(s.b, at) * 32u + l];
    device atomic_uint* state = states + f.state_base + s.b * f.groups;
    M v = M(0);
    switch (s.op) {
      case 0u: v = eq_file<M>(bytes, f, gl * chunks, s.imm * 0x01010101u); break;
      case 1u: v = pattern<M>(ulong(s.b) | (ulong(s.imm) << 32u), gl); break;
      case 2u: v = ~a; break;
      case 3u: v = a & b; break;
      case 4u: v = a | b; break;
      case 5u: v = a ^ b; break;
      case 6u: v = a & ~b; break;
      case 7u: v = plane_file<M>(bytes, f, gl * chunks, s.imm); break;
      case 11u: v = M((plane_file<M>(bytes, f, gl * chunks, s.imm) << 1u) | M((file_byte_before(bytes, f, gl * chunks) >> s.imm) & 1u)); break;
      case 8u: {
        ulong h = 0ul;
        if (!halo_exchange(halos + f.halo_base + s.b * f.groups * 3u, g, l, halo_last(a), spins, h)) return false;
        v = apply_prev(a, s.imm, halo_before(a, l, h));
        break;
      }
      case 9u: {
        const uint t = summary_toggle(a);
        uint cin = 0u;
        if (!look_back(state, g, l, t | ((1u ^ t) << 1u), spins, cin)) return false;
        v = apply_toggle(a, cin);
        break;
      }
      case 10u: {
        uint o0 = 0u, o1 = 0u, cin = 0u;
        const M f0 = escaped(a, M(0), o0), f1 = escaped(a, M(1), o1);
        if (!look_back(state, g, l, (o0 & 1u) | ((o1 & 1u) << 1u), spins, cin)) return false;
        v = select(f0, f1, cin != 0u);
        break;
      }
      case 12u: {
        // `fill`: its ends are in imm (b is the carry's index).
        const M e = slots[min(s.imm, at) * 32u + l];
        uint o0 = 0u, o1 = 0u, cin = 0u;
        const M f0 = fill_word(a, e, M(0), o0), f1 = fill_word(a, e, M(1), o1);
        if (!look_back(state, g, l, (o0 & 1u) | ((o1 & 1u) << 1u), spins, cin)) return false;
        v = select(f0, f1, cin != 0u);
        break;
      }
      case 13u: v = M(within_lane<M>(f.len + s.imm, gl * W) & ~within_lane<M>(s.imm, gl * W)); break;
      default: break;
    }
    slots[at * 32u + l] = v;
  }
  for (uint k = 0u; k < pr.outs; k++) out[f.out_base + k * lanes + gl] = slots[outs[pr.out0 + k] * 32u + l];
  const uint capacity = max(f.len, 1u);
  for (uint e = 0u; e < pr.exts; e++) {
    const M v = slots[outs[pr.out0 + outs[pr.ext0 + e]] * 32u + l];
    if (!extract_write(v, gl, g, l, f.len, ext_states + f.ext_base + e * f.groups, positions + f.pos_base + e * capacity,
                       capacity, counts + f.ext_base + e * f.groups, spins * 64u))
      return false;
  }
  return true;
}

// Persistent workers (decision 0027): each takes group numbers in launch order and interprets
// its file's program over the group, reading the file in its slab (decision 0067). `p`: the batch's groups, the polls after the first, the
// most steps of any program.
#define BATCH_KERNEL(W, M)                                                                                       \
  kernel void block_batch_##W(                                                                                   \
      device const uint4* s0 [[buffer(0)]], device M* out [[buffer(1)]], device const FileRow* files [[buffer(2)]], \
      device const uint* group_file [[buffer(3)]], device atomic_uint* states [[buffer(4)]],                     \
      device atomic_uint* halos [[buffer(5)]], device atomic_uint* ext_states [[buffer(6)]],                     \
      device uint* positions [[buffer(7)]], device uint* counts [[buffer(8)]], device atomic_uint* tickets [[buffer(9)]], \
      device uint* done [[buffer(10)]], constant uint4& p [[buffer(11)]], device const ProgramRow* programs [[buffer(12)]], \
      device const BlockStep* steps [[buffer(13)]], device const uint* outs [[buffer(14)]], device M* scratch [[buffer(15)]], \
      device const uint4* s1 [[buffer(16)]], device const uint4* s2 [[buffer(17)]], device const uint4* s3 [[buffer(18)]], \
      uint w [[threadgroup_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {                           \
    device M* slots = scratch + w * p.z * 32u;                                                                   \
    for (uint round = 0u; round < p.x; round++) {                                                               \
      uint ticket = 0u;                                                                                          \
      if (l == 0u) ticket = atomic_fetch_add_explicit(&tickets[0], 1u, memory_order_relaxed);                    \
      const uint t = simd_broadcast(ticket, ushort(0));                                                          \
      if (t >= p.x) return;                                                                                      \
      if (done[t] != 0u) continue;                                                                               \
      const FileRow f = files[group_file[t]];                                                                    \
      device const uint4* bytes = f.slab == 0u ? s0 : f.slab == 1u ? s1 : f.slab == 2u ? s2 : s3;                \
      const bool ok = interpret_file<M>(bytes, f, t - f.group0, l, p.y, programs, steps, outs, slots, out, states, halos, \
                                        ext_states, positions, counts);                                          \
      if (l == 0u && ok) done[t] = 1u;                                                                           \
      if (l == 0u && !ok) atomic_fetch_add_explicit(&tickets[1], 1u, memory_order_relaxed);                     \
    }                                                                                                            \
  }

BATCH_KERNEL(16, ushort)
BATCH_KERNEL(32, uint)
BATCH_KERNEL(64, ulong)
