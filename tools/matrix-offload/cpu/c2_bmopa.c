// C2. BMOPA semantics and throughput.
//
// 1. Establish what BMOPA (svbmopa_za32_u32_m) accumulates, on known and random inputs, and what
//    the predicates do.
// 2. Derive AND-popcount from it:  pop(a&b) = (pop(a) + pop(b) - pop(a^b)) / 2
//    and pop(a^b) = 32*K - xnor, where xnor is the BMOPA accumulation over K words, so
//    pop(a&b) summed over K words = (popA_i + popB_j - 32K + xnor_ij) / 2.
// 3. Throughput (bit-ops/s) with 1, 2, 4 tiles, operands in registers and streamed from L1/L2.
#include "bench.h"
#include <arm_sme.h>

// Single BMOPA into tile 0 with given row/column predicates; return the whole tile (16x16).
__arm_locally_streaming __arm_new("za")
static void bmopa_once(const uint32_t *zn, const uint32_t *zm, uint32_t pn_mask, uint32_t pm_mask, int reps, uint32_t *tile, int subtract) {
  svuint32_t idx = svindex_u32(0, 1);
  svbool_t all = svptrue_b32();
  svbool_t pn = svcmpne_n_u32(all, svand_u32_x(all, svlsr_u32_x(all, svdup_n_u32(pn_mask), idx), svdup_n_u32(1)), 0);
  svbool_t pm = svcmpne_n_u32(all, svand_u32_x(all, svlsr_u32_x(all, svdup_n_u32(pm_mask), idx), svdup_n_u32(1)), 0);
  svuint32_t a = svld1_u32(all, zn), b = svld1_u32(all, zm);
  svzero_za();
  for (int r = 0; r < reps; r++) {
    if (subtract) svbmops_za32_u32_m(0, pn, pm, a, b); else svbmopa_za32_u32_m(0, pn, pm, a, b);
  }
  for (int i = 0; i < 16; i++) svst1_hor_za32(0, i, all, tile + 16 * i);
}

// AND-popcount of a 16-row x 16-col block over K 32-bit words via BMOPA.
// An: K x 16 (word-major: An[w*16+i] = word w of row i), Bm likewise.
__arm_locally_streaming __arm_new("za")
static void and_pop_block(const uint32_t *An, const uint32_t *Bm, long K, uint32_t *tile) {
  svbool_t p = svptrue_b32();
  svzero_za();
  for (long w = 0; w < K; w++) svbmopa_za32_u32_m(0, p, p, svld1_u32(p, An + 16 * w), svld1_u32(p, Bm + 16 * w));
  for (int i = 0; i < 16; i++) svst1_hor_za32(0, i, p, tile + 16 * i);
}

// Throughput kernels: K BMOPAs, operands in registers, over T tiles.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
static void tp_reg(long K, int tiles, const uint32_t *in, uint32_t *out) {
  svbool_t p = svptrue_b32();
  svuint32_t a0 = svld1_u32(p, in), a1 = svld1_u32(p, in + 16), b0 = svld1_u32(p, in + 32), b1 = svld1_u32(p, in + 48);
  svzero_za();
  if (tiles == 1) for (long i = 0; i < K; i += 4) { svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(0, p, p, a0, b1); svbmopa_za32_u32_m(0, p, p, a1, b0); svbmopa_za32_u32_m(0, p, p, a1, b1); }
  else if (tiles == 2) for (long i = 0; i < K; i += 4) { svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1); svbmopa_za32_u32_m(0, p, p, a1, b0); svbmopa_za32_u32_m(1, p, p, a1, b1); }
  else for (long i = 0; i < K; i += 4) { svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1); svbmopa_za32_u32_m(2, p, p, a1, b0); svbmopa_za32_u32_m(3, p, p, a1, b1); }
  svst1_hor_za32(0, 0, p, out);
}
// K BMOPAs over 4 tiles in a 2x2 block, operands streamed from memory: per 4 BMOPAs, 2 A-vectors
// and 2 B-vectors (256 bytes). buf holds `words` u32, cycled.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
static void tp_mem(long K, const uint32_t *buf, long words, uint32_t *out) {
  svbool_t p = svptrue_b32();
  svzero_za();
  long off = 0;
  for (long i = 0; i < K; i += 4) {
    const uint32_t *q = buf + off;
    svuint32_t a0 = svld1_u32(p, q), a1 = svld1_u32(p, q + 16), b0 = svld1_u32(p, q + 32), b1 = svld1_u32(p, q + 48);
    svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1);
    svbmopa_za32_u32_m(2, p, p, a1, b0); svbmopa_za32_u32_m(3, p, p, a1, b1);
    off += 64; if (off >= words) off = 0;
  }
  svst1_hor_za32(0, 0, p, out);
}
// Same with 1 A-vector against 4 B-vectors (5 loads per 4 BMOPAs) -- the shape of a 16 x 64 block.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
static void tp_mem14(long K, const uint32_t *buf, long words, uint32_t *out) {
  svbool_t p = svptrue_b32();
  svzero_za();
  long off = 0;
  for (long i = 0; i < K; i += 4) {
    const uint32_t *q = buf + off;
    svuint32_t a0 = svld1_u32(p, q), b0 = svld1_u32(p, q + 16), b1 = svld1_u32(p, q + 32), b2 = svld1_u32(p, q + 48), b3 = svld1_u32(p, q + 64);
    svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1);
    svbmopa_za32_u32_m(2, p, p, a0, b2); svbmopa_za32_u32_m(3, p, p, a0, b3);
    off += 80; if (off + 80 > words) off = 0;
  }
  svst1_hor_za32(0, 0, p, out);
}
// SME2 multi-vector loads (ld1w {z0-z3}) feeding the 2x2 block: one 256-byte load per 4 BMOPAs.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
static void tp_mem_x4(long K, const uint32_t *buf, long words, uint32_t *out) {
  svbool_t p = svptrue_b32(); svcount_t pc = svptrue_c32();
  svzero_za();
  long off = 0;
  for (long i = 0; i < K; i += 4) {
    svuint32x4_t v = svld1_u32_x4(pc, buf + off);
    svuint32_t a0 = svget4(v, 0), a1 = svget4(v, 1), b0 = svget4(v, 2), b1 = svget4(v, 3);
    svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1);
    svbmopa_za32_u32_m(2, p, p, a1, b0); svbmopa_za32_u32_m(3, p, p, a1, b1);
    off += 64; if (off >= words) off = 0;
  }
  svst1_hor_za32(0, 0, p, out);
}

int main(void) {
  prefer_p_cores();
  print_load("C2 start");
  uint64_t seed = 42; int fails = 0;
  uint32_t zn[16], zm[16], tile[256];

  // --- 1. Semantics on known inputs.
  struct { uint32_t a, b; const char *what; } known[] = {
    { 0u, 0u, "a=0, b=0" }, { ~0u, 0u, "a=~0, b=0" }, { ~0u, ~0u, "a=~0, b=~0" },
    { 0xF0F0F0F0u, 0xFF00FF00u, "a=F0F0F0F0, b=FF00FF00" }, { 1u, 3u, "a=1, b=3" } };
  printf("## BMOPA on known inputs (all lanes equal)\n\n| inputs | ZA[0][0] | popcount(~(a^b)) | popcount(a&b) |\n|---|---|---|---|\n");
  for (int t = 0; t < 5; t++) {
    for (int i = 0; i < 16; i++) { zn[i] = known[t].a; zm[i] = known[t].b; }
    bmopa_once(zn, zm, 0xffff, 0xffff, 1, tile, 0);
    printf("| %s | %u | %d | %d |\n", known[t].what, tile[0], __builtin_popcount(~(known[t].a ^ known[t].b)), __builtin_popcount(known[t].a & known[t].b));
  }
  // Random, all lanes, outer product layout: ZA[i][j] = sum xnor(zn[i], zm[j]).
  for (int i = 0; i < 16; i++) { zn[i] = (uint32_t)rng_next(&seed); zm[i] = (uint32_t)rng_next(&seed); }
  bmopa_once(zn, zm, 0xffff, 0xffff, 3, tile, 0);
  int bad = 0;
  for (int i = 0; i < 16; i++) for (int j = 0; j < 16; j++) if (tile[16 * i + j] != 3u * (uint32_t)__builtin_popcount(~(zn[i] ^ zm[j]))) bad++;
  printf("\nrandom 16x16, 3 accumulations: ZA[i][j] == 3*popcount(~(zn[i]^zm[j])) for %d/256 cells\n", 256 - bad); fails += bad;
  // Predicates: rows 0..7 active, columns odd active -> inactive cells untouched (stay 0).
  bmopa_once(zn, zm, 0x00ff, 0xaaaa, 1, tile, 0);
  bad = 0;
  for (int i = 0; i < 16; i++) for (int j = 0; j < 16; j++) {
    uint32_t want = (i < 8 && (j & 1)) ? (uint32_t)__builtin_popcount(~(zn[i] ^ zm[j])) : 0u;
    if (tile[16 * i + j] != want) bad++; }
  printf("predicated (rows 0-7, odd columns): inactive cells unchanged for %d/256 cells\n", 256 - bad); fails += bad;
  // BMOPS subtracts (wraps mod 2^32).
  bmopa_once(zn, zm, 0xffff, 0xffff, 1, tile, 1);
  printf("BMOPS from zero: ZA[0][0] = %u = 2^32 - %u (expected 2^32 - %d)\n", tile[0], (uint32_t)(0u - tile[0]), __builtin_popcount(~(zn[0] ^ zm[0])));
  if ((uint32_t)(0u - tile[0]) != (uint32_t)__builtin_popcount(~(zn[0] ^ zm[0]))) fails++;

  // --- 2. AND-popcount from BMOPA + corrections, K = 64 words (2048 bits).
  long K = 64; uint32_t *An = xalloc(4 * 16 * K), *Bm = xalloc(4 * 16 * K);
  for (long w = 0; w < 16 * K; w++) { An[w] = (uint32_t)rng_next(&seed) & (uint32_t)rng_next(&seed); Bm[w] = (uint32_t)rng_next(&seed); }
  and_pop_block(An, Bm, K, tile);
  bad = 0;
  for (int i = 0; i < 16; i++) for (int j = 0; j < 16; j++) {
    long pa = 0, pb = 0, want = 0;
    for (long w = 0; w < K; w++) { pa += __builtin_popcount(An[16 * w + i]); pb += __builtin_popcount(Bm[16 * w + j]); want += __builtin_popcount(An[16 * w + i] & Bm[16 * w + j]); }
    long got = (pa + pb - 32 * K + (long)tile[16 * i + j]) / 2;
    if (got != want) bad++;
  }
  printf("AND-popcount derived as (popA_i + popB_j - 32K + xnor_ij)/2, K=%ld words: %d/256 cells match scalar\n\n", K, 256 - bad); fails += bad;
  if (fails) { printf("VERIFY FAILED (%d)\n", fails); return 1; }

  // --- 3. Throughput.
  uint32_t in[128], out[16]; for (int i = 0; i < 128; i++) in[i] = (uint32_t)rng_next(&seed);
  int R = 9; long Kt = 1 << 16; stat2 s;
  printf("## BMOPA throughput (one thread; 1 BMOPA = 16x16 cells x 32 bits = 8192 bit-ops)\n\n");
  printf("| variant | ns/BMOPA min | median | Tbit-ops per CPU-s (min time) | operand bytes per CPU-s |\n|---|---|---|---|---|\n");
  for (int tiles = 1; tiles <= 4; tiles *= 2) {
    TIME_CPU(R, 20, s, tp_reg(Kt, tiles, in, out));
    printf("| registers, %d tile(s) | %.3f | %.3f | %.1f | - |\n", tiles, s.min / Kt, s.med / Kt, 8192.0 / (s.min / Kt) / 1000.0);
  }
  // Data dependence: same 4-tile register kernel with different operand bit patterns.
  { struct { const char *n; uint32_t v; int rnd; } pats[] = { { "all zero", 0u, 0 }, { "all ones", ~0u, 0 }, { "0x0f0f0f0f*i (C1's input)", 0, 2 }, { "random", 0, 1 } };
    uint32_t pin[128];
    for (int t = 0; t < 4; t++) {
      for (int i = 0; i < 128; i++) pin[i] = pats[t].rnd == 1 ? (uint32_t)rng_next(&seed) : pats[t].rnd == 2 ? 0x0f0f0f0fu * (unsigned)i : pats[t].v;
      TIME_CPU(R, 20, s, tp_reg(Kt, 4, pin, out));
      printf("| registers, 4 tiles, operands %s | %.3f | %.3f | %.1f | - |\n", pats[t].n, s.min / Kt, s.med / Kt, 8192.0 / (s.min / Kt) / 1000.0);
    } }
  long sizes[] = { 4096, 65536, 1 << 20, 4 << 20 }; // bytes: L1, L1, L2, L2
  const char *where[] = { "4 KB (L1)", "64 KB (L1)", "1 MB (L2)", "4 MB (L2)" };
  for (int si = 0; si < 4; si++) {
    long words = sizes[si] / 4; uint32_t *buf = xalloc(sizes[si] + 512);
    for (long w = 0; w < words + 128; w++) buf[w] = (uint32_t)rng_next(&seed);
    TIME_CPU(R, 20, s, tp_mem(Kt, buf, words, out));
    printf("| 2x2 tiles, ld1w x4 per 4 BMOPA, %s | %.3f | %.3f | %.1f | %.0f GB/s |\n", where[si], s.min / Kt, s.med / Kt, 8192.0 / (s.min / Kt) / 1000.0, 64.0 / (s.min / Kt));
    TIME_CPU(R, 20, s, tp_mem_x4(Kt, buf, words, out));
    printf("| 2x2 tiles, one ld1w {4 regs} per 4 BMOPA, %s | %.3f | %.3f | %.1f | %.0f GB/s |\n", where[si], s.min / Kt, s.med / Kt, 8192.0 / (s.min / Kt) / 1000.0, 64.0 / (s.min / Kt));
    TIME_CPU(R, 20, s, tp_mem14(Kt, buf, words, out));
    printf("| 1x4 tiles, 5 loads per 4 BMOPA, %s | %.3f | %.3f | %.1f | %.0f GB/s |\n", where[si], s.min / Kt, s.med / Kt, 8192.0 / (s.min / Kt) / 1000.0, 80.0 / (s.min / Kt));
    free(buf);
  }
  SINK(out[0]);
  print_load("C2 end");
  return 0;
}
