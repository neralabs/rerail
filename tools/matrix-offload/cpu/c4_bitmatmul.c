// C4. Bit-matrix AND-popcount product C[i][j] = popcount(A_i & B_j), A: n x k bits, B: m x k bits.
//   (a) scalar popcount (2x2 register block, vectorisation disabled)
//   (b) NEON: vandq + vcntq + vpadalq, 4x4 register block, k chunked so the B chunk stays in L2
//   (c) SME: BMOPA (XNOR-popcount) over a 2x2 tile block (32x32 outputs), corrections seeded into
//       ZA with ADDVA/ADDHA so ZA ends holding 2*popcount(a&b); readout shifts by 1 and stores.
// Scalar and NEON compute a subset of output rows (whole width) to fit the run-time cap; SME
// computes the whole product. Every method is checked against a scalar reference on sampled cells.
#include "bench.h"
#include <arm_neon.h>
#include <arm_sme.h>

static long N, M, KB;          // rows of A, rows of B, k in bits
static uint64_t *A, *B;        // row-major, KB/64 words per row
static uint32_t *C;            // n x m

static uint32_t ref_cell(long i, long j) {
  const uint64_t *a = A + i * (KB / 64), *b = B + j * (KB / 64); uint32_t s = 0;
  for (long w = 0; w < KB / 64; w++) s += (uint32_t)__builtin_popcountll(a[w] & b[w]);
  return s;
}

// (a) scalar
static void scalar_rows(long i0, long i1) {
  long W = KB / 64;
  for (long i = i0; i < i1; i += 2) for (long j = 0; j < M; j += 2) {
    const uint64_t *a0 = A + i * W, *a1 = a0 + W, *b0 = B + j * W, *b1 = b0 + W;
    uint64_t s00 = 0, s01 = 0, s10 = 0, s11 = 0;
#pragma clang loop vectorize(disable) interleave(disable)
    for (long w = 0; w < W; w++) {
      uint64_t x0 = a0[w], x1 = a1[w], y0 = b0[w], y1 = b1[w];
      s00 += (uint64_t)__builtin_popcountll(x0 & y0); s01 += (uint64_t)__builtin_popcountll(x0 & y1);
      s10 += (uint64_t)__builtin_popcountll(x1 & y0); s11 += (uint64_t)__builtin_popcountll(x1 & y1);
    }
    C[i * M + j] = (uint32_t)s00; C[i * M + j + 1] = (uint32_t)s01; C[(i + 1) * M + j] = (uint32_t)s10; C[(i + 1) * M + j + 1] = (uint32_t)s11;
  }
}

// (a2) same, popcount as fmov + NEON cnt.8b + addv per u64 (what clang emits without +cssc)
static inline uint64_t pop_neon64(uint64_t x) { return vaddv_u8(vcnt_u8(vcreate_u8(x))); }
static void scalar_neoncnt_rows(long i0, long i1) {
  long W = KB / 64;
  for (long i = i0; i < i1; i += 2) for (long j = 0; j < M; j += 2) {
    const uint64_t *a0 = A + i * W, *a1 = a0 + W, *b0 = B + j * W, *b1 = b0 + W;
    uint64_t s00 = 0, s01 = 0, s10 = 0, s11 = 0;
#pragma clang loop vectorize(disable) interleave(disable)
    for (long w = 0; w < W; w++) {
      uint64_t x0 = a0[w], x1 = a1[w], y0 = b0[w], y1 = b1[w];
      s00 += pop_neon64(x0 & y0); s01 += pop_neon64(x0 & y1); s10 += pop_neon64(x1 & y0); s11 += pop_neon64(x1 & y1);
    }
    C[i * M + j] = (uint32_t)s00; C[i * M + j + 1] = (uint32_t)s01; C[(i + 1) * M + j] = (uint32_t)s10; C[(i + 1) * M + j + 1] = (uint32_t)s11;
  }
}

// (b) NEON
static void neon_rows(long i0, long i1) {
  long W = KB / 64; long kc = KB < 8192 ? KB / 64 : 8192 / 64; // words per chunk
  for (long i = i0; i < i1; i++) memset(C + i * M, 0, M * 4);
  for (long w0 = 0; w0 < W; w0 += kc)
    for (long i = i0; i < i1; i += 4) for (long j = 0; j < M; j += 4) {
      const uint8_t *a[4], *b[4];
      for (int q = 0; q < 4; q++) { a[q] = (const uint8_t *)(A + (i + q) * W + w0); b[q] = (const uint8_t *)(B + (j + q) * W + w0); }
      uint16x8_t acc[16]; for (int q = 0; q < 16; q++) acc[q] = vdupq_n_u16(0);
      for (long w = 0; w < kc * 8; w += 16) {
        uint8x16_t x0 = vld1q_u8(a[0] + w), x1 = vld1q_u8(a[1] + w), x2 = vld1q_u8(a[2] + w), x3 = vld1q_u8(a[3] + w);
        uint8x16_t y0 = vld1q_u8(b[0] + w), y1 = vld1q_u8(b[1] + w), y2 = vld1q_u8(b[2] + w), y3 = vld1q_u8(b[3] + w);
#define P(r, x, y) acc[r] = vpadalq_u8(acc[r], vcntq_u8(vandq_u8(x, y)))
        P(0, x0, y0); P(1, x0, y1); P(2, x0, y2); P(3, x0, y3);
        P(4, x1, y0); P(5, x1, y1); P(6, x1, y2); P(7, x1, y3);
        P(8, x2, y0); P(9, x2, y1); P(10, x2, y2); P(11, x2, y3);
        P(12, x3, y0); P(13, x3, y1); P(14, x3, y2); P(15, x3, y3);
#undef P
      }
      for (int q = 0; q < 16; q++) C[(i + q / 4) * M + j + q % 4] += vaddlvq_u16(acc[q]);
    }
}

// (c) SME. Packed panels: Ap[ib][w][32] = u32 word w of rows ib*32 .. ib*32+31; same for Bp.
static uint32_t *Ap, *Bp, *pa_k, *pb; // pa_k[i] = popcount(A_i) - k ; pb[j] = popcount(B_j)

#define TILE_OUT(t, roff, coff)                                                                   \
  for (int r = 0; r < 16; r++) {                                                                  \
    svuint32_t v = svread_hor_za32_u32_m(svundef_u32(), p, t, r);                                 \
    svst1_u32(p, Cb + (roff + r) * M + coff, svlsr_n_u32_x(p, v, 1));                             \
  }

// Readout 1: store ZA slices straight to C (holding 2*count); a NEON pass halves C afterwards.
#define TILE_OUT_RAW(t, roff, coff) for (int r = 0; r < 16; r++) svst1_hor_za32(t, r, p, Cb + (roff + r) * M + coff);
// Readout 2: SME2 4-row MOVA, multi-vector rounding shift right by 1 (exact: values are even).
#define TILE_OUT_VG4(t, roff, coff)                                                               \
  for (int r = 0; r < 16; r += 4) {                                                               \
    svuint32x4_t v = svrshl_single_u32_x4(svread_hor_za32_u32_vg4(t, r), m1);                     \
    svst1_u32(p, Cb + (roff + r) * M + coff, svget4(v, 0)); svst1_u32(p, Cb + (roff + r + 1) * M + coff, svget4(v, 1)); \
    svst1_u32(p, Cb + (roff + r + 2) * M + coff, svget4(v, 2)); svst1_u32(p, Cb + (roff + r + 3) * M + coff, svget4(v, 3)); \
  }
static int g_readout = 0;
static const char *readout_name[] = { "MOVA+LSR+ST1 per row", "ST1W ZA slice + NEON halving pass", "SME2 MOVA vg4 + URSHL x4 + ST1" };

// One streaming block: panel ib against panels jb0..jb1-1.
__arm_locally_streaming __arm_new("za")
static void sme_panel(long ib, long jb0, long jb1, long K32) {
  svbool_t p = svptrue_b32(); svcount_t pc = svptrue_c32();
  const uint32_t *ap = Ap + ib * K32 * 32;
  svuint32_t ra0 = svld1_u32(p, pa_k + ib * 32), ra1 = svld1_u32(p, pa_k + ib * 32 + 16);
  for (long jb = jb0; jb < jb1; jb++) {
    const uint32_t *bp = Bp + jb * K32 * 32;
    svuint32_t cb0 = svld1_u32(p, pb + jb * 32), cb1 = svld1_u32(p, pb + jb * 32 + 16);
    svzero_za();
    // seed ZA[i][j] = (popA_i - k) + popB_j
    svaddva_za32_u32_m(0, p, p, ra0); svaddva_za32_u32_m(1, p, p, ra0); svaddva_za32_u32_m(2, p, p, ra1); svaddva_za32_u32_m(3, p, p, ra1);
    svaddha_za32_u32_m(0, p, p, cb0); svaddha_za32_u32_m(1, p, p, cb1); svaddha_za32_u32_m(2, p, p, cb0); svaddha_za32_u32_m(3, p, p, cb1);
    for (long w = 0; w < K32; w++) {
      svuint32x2_t x = svld1_u32_x2(pc, ap + w * 32), y = svld1_u32_x2(pc, bp + w * 32);
      svuint32_t a0 = svget2(x, 0), a1 = svget2(x, 1), b0 = svget2(y, 0), b1 = svget2(y, 1);
      svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1);
      svbmopa_za32_u32_m(2, p, p, a1, b0); svbmopa_za32_u32_m(3, p, p, a1, b1);
    }
    uint32_t *Cb = C + ib * 32 * M + jb * 32;
    if (g_readout == 0) { TILE_OUT(0, 0, 0) TILE_OUT(1, 0, 16) TILE_OUT(2, 16, 0) TILE_OUT(3, 16, 16) }
    else if (g_readout == 1) { TILE_OUT_RAW(0, 0, 0) TILE_OUT_RAW(1, 0, 16) TILE_OUT_RAW(2, 16, 0) TILE_OUT_RAW(3, 16, 16) }
    else { svuint32_t m1 = svdup_n_u32(0xffffffffu); /* shift count -1 (the ACLE prototype here takes an unsigned vector) */ TILE_OUT_VG4(0, 0, 0) TILE_OUT_VG4(1, 0, 16) TILE_OUT_VG4(2, 16, 0) TILE_OUT_VG4(3, 16, 16) }
  }
}

static void sme_pack(void) {
  long K32 = KB / 32;
  for (long ib = 0; ib < N / 32; ib++) for (long w = 0; w < K32; w++) for (int r = 0; r < 32; r++)
    Ap[(ib * K32 + w) * 32 + r] = ((const uint32_t *)(A + (ib * 32 + r) * (KB / 64)))[w];
  for (long jb = 0; jb < M / 32; jb++) for (long w = 0; w < K32; w++) for (int r = 0; r < 32; r++)
    Bp[(jb * K32 + w) * 32 + r] = ((const uint32_t *)(B + (jb * 32 + r) * (KB / 64)))[w];
  for (long i = 0; i < N; i++) { uint32_t s = 0; for (long w = 0; w < KB / 64; w++) s += (uint32_t)__builtin_popcountll(A[i * (KB / 64) + w]); pa_k[i] = s - (uint32_t)KB; }
  for (long j = 0; j < M; j++) { uint32_t s = 0; for (long w = 0; w < KB / 64; w++) s += (uint32_t)__builtin_popcountll(B[j * (KB / 64) + w]); pb[j] = s; }
}

static void sme_all(void) {
  long K32 = KB / 32;
  // Group B panels so a group (32 rows x k bits each) is ~4 MB: stays in L2 while all A panels stream.
  long panel_bytes = 32 * KB / 8; long grp = (4l << 20) / panel_bytes; if (grp < 1) grp = 1;
  for (long jg = 0; jg < M / 32; jg += grp) {
    long je = jg + grp < M / 32 ? jg + grp : M / 32;
    for (long ib = 0; ib < N / 32; ib++) sme_panel(ib, jg, je, K32);
  }
  if (g_readout == 1) { uint32x4_t *c = (uint32x4_t *)C; for (long q = 0; q < N * M / 4; q++) c[q] = vshrq_n_u32(c[q], 1); }
}

static int verify(long i0, long i1, uint64_t *seed, const char *what) {
  int bad = 0;
  for (int s = 0; s < 256; s++) {
    long i = i0 + (long)(rng_next(seed) % (uint64_t)(i1 - i0)), j = (long)(rng_next(seed) % (uint64_t)M);
    if (C[i * M + j] != ref_cell(i, j)) { if (bad < 3) printf("  %s mismatch at (%ld,%ld): %u vs %u\n", what, i, j, C[i * M + j], ref_cell(i, j)); bad++; }
  }
  return bad;
}

int main(int argc, char **argv) {
  prefer_p_cores();
  print_load("C4 start");
  long ns[] = { 1024, 2048, 4096, 8192 }, ks[] = { 1024, 4096, 16384, 65536 };
  int only = argc > 1 ? atoi(argv[1]) : -1; // optional: run only n index
  printf("| n=m | k bits | method | rows computed | setup ms (CPU) | CPU s (min) | median | Gbit-op per CPU-s | input MB/CPU-s | loaded operand GB/CPU-s | output GB/CPU-s | core |\n");
  printf("|---|---|---|---|---|---|---|---|---|---|---|---|\n");
  uint64_t seed = 7;
  for (int ni = 0; ni < 4; ni++) {
    if (only >= 0 && ni != only) continue;
    for (int ki = 0; ki < 4; ki++) {
      N = M = ns[ni]; KB = ks[ki];
      A = xalloc(N * KB / 8); B = xalloc(M * KB / 8); C = xalloc(N * M * 4);
      for (long w = 0; w < N * KB / 64; w++) A[w] = rng_next(&seed) & rng_next(&seed);   // density 1/4
      for (long w = 0; w < M * KB / 64; w++) B[w] = rng_next(&seed);                     // density 1/2
      double rowbytes = KB / 8.0;
      // (a) scalar subset
      long rs = (long)(1.5e9 / ((double)M * KB)); rs = rs < 2 ? 2 : rs > N ? N : rs; rs -= rs % 2;
      stat2 st; TIME_CPU(3, 1, st, scalar_rows(0, rs));
      int bad = verify(0, rs, &seed, "scalar");
      double ops = (double)rs * M * KB;
      printf("| %ld | %ld | scalar, CSSC CNT popcount | %ld | - | %.4f | %.4f | %.1f | %.0f | - | %.2f | %s |%s\n", N, KB, rs, st.min / 1e9, st.med / 1e9, ops / st.min,
             (rs + M) * rowbytes / st.min * 1e3, rs * M * 4.0 / st.min, cl_name(g_min_cpu), bad ? " VERIFY FAIL" : "");
      TIME_CPU(3, 1, st, scalar_neoncnt_rows(0, rs));
      bad = verify(0, rs, &seed, "scalar-neoncnt");
      printf("| %ld | %ld | scalar, fmov+cnt.8b+addv popcount | %ld | - | %.4f | %.4f | %.1f | %.0f | - | %.2f | %s |%s\n", N, KB, rs, st.min / 1e9, st.med / 1e9, ops / st.min,
             (rs + M) * rowbytes / st.min * 1e3, rs * M * 4.0 / st.min, cl_name(g_min_cpu), bad ? " VERIFY FAIL" : "");
      // (b) NEON subset
      long rn = (long)(2.5e10 / ((double)M * KB)); rn = rn < 16 ? 16 : rn > N ? N : rn; rn -= rn % 4;
      TIME_CPU(3, 1, st, neon_rows(0, rn));
      bad = verify(0, rn, &seed, "neon");
      ops = (double)rn * M * KB;
      printf("| %ld | %ld | NEON 4x4 | %ld | - | %.4f | %.4f | %.1f | %.0f | %.1f | %.2f | %s |%s\n", N, KB, rn, st.min / 1e9, st.med / 1e9, ops / st.min,
             (rn + M) * rowbytes / st.min * 1e3, (double)(rn / 4) * (M / 4) * 8 * rowbytes / st.min, rn * M * 4.0 / st.min, cl_name(g_min_cpu), bad ? " VERIFY FAIL" : "");
      // (c) SME, whole product
      Ap = xalloc(N * KB / 8); Bp = xalloc(M * KB / 8); pa_k = xalloc(N * 4); pb = xalloc(M * 4);
      uint64_t t0 = tcpu_ns(); sme_pack(); double setup_ms = (tcpu_ns() - t0) / 1e6;
      memset(C, 0, N * M * 4);
      int reps = (double)N * M * KB > 1e12 ? 3 : 5;
      for (g_readout = 0; g_readout < 3; g_readout++) {
        if (g_readout < 2 && N > 1024) continue; // the readout comparison is made at n=1024
        memset(C, 0, N * M * 4);
        TIME_CPU(reps, 1, st, sme_all());
        bad = verify(0, N, &seed, "sme");
        ops = (double)N * M * KB;
        double nbmopa = (double)(N / 32) * (M / 32) * (KB / 32) * 4;
        printf("| %ld | %ld | SME BMOPA 2x2, %s | %ld (all) | %.1f | %.4f | %.4f | %.1f | %.0f | %.1f | %.2f | %s |%s\n", N, KB, readout_name[g_readout], N, setup_ms, st.min / 1e9, st.med / 1e9, ops / st.min,
               (N + M) * rowbytes / st.min * 1e3, nbmopa * 64 / st.min, N * M * 4.0 / st.min, cl_name(g_min_cpu), bad ? " VERIFY FAIL" : "");
      }
      fflush(stdout);
      free(Ap); free(Bp); free(pa_k); free(pb); free(A); free(B); free(C);
    }
  }
  print_load("C4 end");
  return 0;
}
