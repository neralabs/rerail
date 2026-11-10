// W3. Import closure over the condensed DAG: 10,824 components x 170 u64 words; row c holds the
// successors of c, all with smaller index (sinks first), so the matrix is strictly lower triangular.
// Verify: sum_c size(c) * (sum_{d reachable from c, d != c} size(d) + (size(c) if size(c) > 1))
// = 29,722,613.
// Forms:
//   (a) bit-row DP in index order (baseline form, C port): reach[c] |= reach[s] | {s}
//   (a') the same, ORing only the words below s (reach[s] has no bits >= s), explicit NEON
//   (b) boolean squaring of M = A + I to a fixpoint (5 squarings):
//       row-OR, word-parallel (baseline form, C port); NEON row-OR restricted to the triangle;
//       SME BMOPA: C[i][j] = any(M_i & M^T_j) over lower-triangular 32x32 blocks with the k range cut
//       to the words that can be nonzero on both sides; seeds from prefix popcounts; ZA tested != 0
//       and packed straight into the next round's panel layout; 32x32 bit transposes give M^T.
#include "mo.h"

static long NC, W = 170; static uint64_t *Adj; static uint32_t *Sz;
static const uint64_t EXPECT = 29722613;

// reach rows given with or without the diagonal; the diagonal bit is ignored.
static uint64_t pairs_from_rows(const uint64_t *R, long stride64) {
  uint64_t pairs = 0;
  for (long c = 0; c < NC; c++) { uint64_t s = 0; const uint64_t *row = R + c * stride64;
    for (long k = 0; k < W; k++) { uint64_t x = row[k]; while (x) { long b = k * 64 + __builtin_ctzll(x); if (b != c) s += Sz[b]; x &= x - 1; } }
    pairs += (uint64_t)Sz[c] * (s + (Sz[c] > 1 ? Sz[c] : 0)); }
  return pairs;
}

// adjacency lists (setup)
static uint32_t *aoff, *aval; static uint64_t *Rch;
static void f_dp(void) {
  memset(Rch, 0, NC * W * 8);
  for (long c = 0; c < NC; c++) { uint64_t *rc = Rch + c * W;
    for (uint32_t e = aoff[c]; e < aoff[c + 1]; e++) { long s = aval[e]; rc[s / 64] |= 1ull << (s % 64); const uint64_t *rs = Rch + s * W; for (long k = 0; k < W; k++) rc[k] |= rs[k]; } }
}
static void f_dp_tri(void) {
  memset(Rch, 0, NC * W * 8);
  for (long c = 0; c < NC; c++) { uint64_t *rc = Rch + c * W;
    for (uint32_t e = aoff[c]; e < aoff[c + 1]; e++) { long s = aval[e]; rc[s / 64] |= 1ull << (s % 64); const uint64_t *rs = Rch + s * W;
      long nw = s / 64 + 1, k = 0;
      for (; k + 2 <= nw; k += 2) vst1q_u64(rc + k, vorrq_u64(vld1q_u64(rc + k), vld1q_u64(rs + k)));
      for (; k < nw; k++) rc[k] |= rs[k]; } }
}

// (b) row-OR squaring
static uint64_t *M0, *M1;
static int f_rowor(int tri) {
  memset(M0, 0, NC * W * 8);
  for (long c = 0; c < NC; c++) { memcpy(M0 + c * W, Adj + c * W, W * 8); M0[c * W + c / 64] |= 1ull << (c % 64); }
  int steps = 0; uint64_t *m = M0, *m2 = M1;
  for (;;) {
    steps++; memset(m2, 0, NC * W * 8);
    for (long i = 0; i < NC; i++) { const uint64_t *mi = m + i * W; uint64_t *oi = m2 + i * W;
      if (!tri) { for (long k = 0; k < NC; k++) if (mi[k / 64] >> (k % 64) & 1) { const uint64_t *mk = m + k * W; for (long x = 0; x < W; x++) oi[x] |= mk[x]; } }
      else { for (long kw = 0; kw <= i / 64; kw++) { uint64_t bits = mi[kw]; while (bits) { long k = kw * 64 + __builtin_ctzll(bits); bits &= bits - 1;
            const uint64_t *mk = m + k * W; long nw = k / 64 + 1, x = 0;
            for (; x + 2 <= nw; x += 2) vst1q_u64(oi + x, vorrq_u64(vld1q_u64(oi + x), vld1q_u64(mk + x)));
            for (; x < nw; x++) oi[x] |= mk[x]; } } }
    }
    int done = !memcmp(m, m2, NC * W * 8);
    uint64_t *t = m; m = m2; m2 = t;
    if (done || steps > 20) break;
  }
  if (m != M0) memcpy(M0, m, NC * W * 8); // result in M0
  return steps;
}

// SME squaring. Panel layout, K32 = 340 u32 words per row, NP = 339 panels (10,848 rows).
static long NP, K32; static int g_readout = 0; // 0: vertical column readout, 1: per-row OR-reduction, 2: none (timing split only)
static uint32_t *PA, *PB, *PA2, *PB2, *prefA, *prefB; // pref*: [(pb*(K32+1) + w)*32 + r]
static void transpose32(uint32_t *a) { // bit c of a[r]  ->  bit r of a[c]
  uint32_t m = 0x0000FFFFu;
  for (int j = 16; j; j >>= 1, m ^= m << j)
    for (int k = 0; k < 32; k = (k + j + 1) & ~j) { uint32_t t = ((a[k] >> j) ^ a[k + j]) & m; a[k + j] ^= t; a[k] ^= t << j; }
}
static void transpose_into(const uint32_t *src, uint32_t *dst) { // dst panel (jb) word ib = transpose of src panel ib word jb
  for (long ib = 0; ib < NP; ib++) for (long jb = 0; jb <= ib; jb++) {
    uint32_t t[32]; memcpy(t, src + (ib * K32 + jb) * 32, 128); transpose32(t); memcpy(dst + (jb * K32 + ib) * 32, t, 128); }
}
static void prefix(const uint32_t *P, uint32_t *pref) {
  for (long pb = 0; pb < NP; pb++) {
    uint32x4_t acc[8]; for (int q = 0; q < 8; q++) acc[q] = vdupq_n_u32(0);
    for (long w = 0; w <= K32; w++) {
      uint32_t *o = pref + (pb * (K32 + 1) + w) * 32; for (int q = 0; q < 8; q++) vst1q_u32(o + 4 * q, acc[q]);
      if (w == K32) break;
      const uint32_t *x = P + (pb * K32 + w) * 32;
      for (int q = 0; q < 8; q++) acc[q] = vaddq_u32(acc[q], vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(vld1q_u32(x + 4 * q))))));
    }
  }
}
__arm_locally_streaming __arm_new("za") static void sme_square(void) {
  svbool_t p = svptrue_b32(); svcount_t pc = svptrue_c32();
  svuint32_t pow2 = svlsl_u32_x(p, svdup_n_u32(1), svindex_u32(0, 1));
  for (long ib = 0; ib < NP; ib++) {
    const uint32_t *ap = PA + ib * K32 * 32;
    for (long jb = 0; jb <= ib; jb++) {
      const uint32_t *bp = PB + jb * K32 * 32; long k0 = jb, k1 = ib + 1; uint32_t L32 = (uint32_t)(32 * (k1 - k0));
      const uint32_t *a0p = prefA + (ib * (K32 + 1) + k0) * 32, *a1p = prefA + (ib * (K32 + 1) + k1) * 32;
      const uint32_t *b0p = prefB + (jb * (K32 + 1) + k0) * 32, *b1p = prefB + (jb * (K32 + 1) + k1) * 32;
      svuint32_t ra0 = svsub_n_u32_x(p, svsub_u32_x(p, svld1_u32(p, a1p), svld1_u32(p, a0p)), L32);
      svuint32_t ra1 = svsub_n_u32_x(p, svsub_u32_x(p, svld1_u32(p, a1p + 16), svld1_u32(p, a0p + 16)), L32);
      svuint32_t cb0 = svsub_u32_x(p, svld1_u32(p, b1p), svld1_u32(p, b0p));
      svuint32_t cb1 = svsub_u32_x(p, svld1_u32(p, b1p + 16), svld1_u32(p, b0p + 16));
      svzero_za();
      svaddva_za32_u32_m(0, p, p, ra0); svaddva_za32_u32_m(1, p, p, ra0); svaddva_za32_u32_m(2, p, p, ra1); svaddva_za32_u32_m(3, p, p, ra1);
      svaddha_za32_u32_m(0, p, p, cb0); svaddha_za32_u32_m(1, p, p, cb1); svaddha_za32_u32_m(2, p, p, cb0); svaddha_za32_u32_m(3, p, p, cb1);
      for (long w = k0; w < k1; w++) {
        svuint32x2_t x = svld1_u32_x2(pc, ap + w * 32), y = svld1_u32_x2(pc, bp + w * 32);
        svbmopa_za32_u32_m(0, p, p, svget2(x, 0), svget2(y, 0)); svbmopa_za32_u32_m(1, p, p, svget2(x, 0), svget2(y, 1));
        svbmopa_za32_u32_m(2, p, p, svget2(x, 1), svget2(y, 0)); svbmopa_za32_u32_m(3, p, p, svget2(x, 1), svget2(y, 1));
      }
      uint32_t *o = PA2 + (ib * K32 + jb) * 32;
      if (g_readout == 2) { /* timing only: no readout */ }
      else if (g_readout == 0) {
        // Vertical readout: column c of a tile is a vector over rows; OR (1 << c) into the rows
        // whose cell is nonzero. No horizontal reductions; the 16 row masks land in one vector.
#define COLS(t, sh, acc) for (int c = 0; c < 16; c += 4) { svuint32x4_t v = svread_ver_za32_u32_vg4(t, c); \
          acc = svorr_n_u32_m(svcmpne_n_u32(p, svget4(v, 0), 0), acc, 1u << (sh + c)); acc = svorr_n_u32_m(svcmpne_n_u32(p, svget4(v, 1), 0), acc, 1u << (sh + c + 1)); \
          acc = svorr_n_u32_m(svcmpne_n_u32(p, svget4(v, 2), 0), acc, 1u << (sh + c + 2)); acc = svorr_n_u32_m(svcmpne_n_u32(p, svget4(v, 3), 0), acc, 1u << (sh + c + 3)); }
        svuint32_t m0 = svdup_n_u32(0), m1 = svdup_n_u32(0);
        COLS(0, 0, m0) COLS(1, 16, m0) COLS(2, 0, m1) COLS(3, 16, m1)
#undef COLS
        svst1_u32(p, o, m0); svst1_u32(p, o + 16, m1);
      } else {
#define BITS(tl, th, ro) for (int r = 0; r < 16; r += 4) { svuint32x4_t lo = svread_hor_za32_u32_vg4(tl, r), hi = svread_hor_za32_u32_vg4(th, r); \
        for (int q = 0; q < 4; q++) { svuint32_t vl = q == 0 ? svget4(lo, 0) : q == 1 ? svget4(lo, 1) : q == 2 ? svget4(lo, 2) : svget4(lo, 3); \
          svuint32_t vh = q == 0 ? svget4(hi, 0) : q == 1 ? svget4(hi, 1) : q == 2 ? svget4(hi, 2) : svget4(hi, 3); \
          o[ro + r + q] = svorv_u32(svcmpne_n_u32(p, vl, 0), pow2) | (svorv_u32(svcmpne_n_u32(p, vh, 0), pow2) << 16); } }
        BITS(0, 1, 0) BITS(2, 3, 16)
#undef BITS
      }
    }
  }
}
static int f_sme(void) {
  int steps = 0;
  for (;;) {
    steps++;
    prefix(PA, prefA); prefix(PB, prefB);
    sme_square();
    transpose_into(PA2, PB2);
    int done = !memcmp(PA, PA2, (size_t)NP * K32 * 32 * 4);
    uint32_t *t = PA; PA = PA2; PA2 = t; t = PB; PB = PB2; PB2 = t;
    if (done || steps > 20) break;
  }
  return steps;
}
static uint32_t *PA_init, *PB_init;
static void sme_reset(void) { memcpy(PA, PA_init, (size_t)NP * K32 * 32 * 4); memcpy(PB, PB_init, (size_t)NP * K32 * 32 * 4); }
static uint64_t pairs_from_panels(const uint32_t *P) {
  uint64_t *rows = xalloc(NC * W * 8);
  for (long i = 0; i < NC; i++) { uint32_t *r = (uint32_t *)(rows + i * W); for (long w = 0; w < K32; w++) r[w] = P[((i / 32) * K32 + w) * 32 + i % 32]; }
  uint64_t v = pairs_from_rows(rows, W); free(rows); return v;
}

int main(int argc, char **argv) {
  init_data(argc, argv); prefer_p_cores();
  print_load("W3 start");
  size_t sa, ss; Adj = load_file("w3/cadj_bits.u64", &sa); Sz = load_file("w3/comp_sizes.u32", &ss);
  NC = (long)(ss / 4); if (sa != (size_t)NC * W * 8) { printf("size mismatch\n"); return 1; }
  // transpose self-test
  { uint32_t a[32], b[32]; uint64_t sd = 3; for (int i = 0; i < 32; i++) a[i] = (uint32_t)rng_next(&sd); memcpy(b, a, 128); transpose32(b);
    for (int r = 0; r < 32; r++) for (int c = 0; c < 32; c++) if (((a[r] >> c) & 1) != ((b[c] >> r) & 1)) { printf("transpose32 FAIL\n"); return 1; } }
  uint64_t t0 = tcpu_ns(); long edges = 0;
  aoff = xalloc((NC + 1) * 4); aval = xalloc(NC * 64 * 4 + 4096 * 64); aoff[0] = 0;
  for (long c = 0; c < NC; c++) { uint32_t k = aoff[c]; for (long w = 0; w < W; w++) { uint64_t x = Adj[c * W + w]; while (x) { long s = w * 64 + __builtin_ctzll(x); if (s >= c) { printf("not lower triangular\n"); return 1; } aval[k++] = (uint32_t)s; x &= x - 1; } } aoff[c + 1] = k; edges = k; }
  printf("components %ld, condensed edges %ld, input %.1f MB; adjacency lists %.2f ms CPU (setup)\n\n", NC, edges, sa / 1e6, (tcpu_ns() - t0) / 1e6);
  Rch = xalloc(NC * W * 8); M0 = xalloc(NC * W * 8); M1 = xalloc(NC * W * 8);
  printf("| form | CPU s min | median | squarings | result | core |\n|---|---|---|---|---|---|\n");
  stat2 st; uint64_t r; int steps = 0;
#define ROW(name, reps, call, check) do { call; r = check; int ok = r == EXPECT; TIME_CPU(reps, 1, st, call); ok &= (check) == EXPECT; \
    printf("| %s | %.4f | %.4f | %s | %llu %s | %s |\n", name, st.min / 1e9, st.med / 1e9, steps ? ({ static char b[8]; snprintf(b, 8, "%d", steps); b; }) : "-", (unsigned long long)r, ok ? "OK" : "MISMATCH", cl_name(g_min_cpu)); fflush(stdout); } while (0)
  ROW("(a) bit-row DP (baseline form, C port)", 15, f_dp(), pairs_from_rows(Rch, W));
  ROW("(a') bit-row DP, triangle-restricted ORs, NEON", 15, f_dp_tri(), pairs_from_rows(Rch, W));
  ROW("(b) row-OR squaring (baseline form, C port)", 1, steps = f_rowor(0), pairs_from_rows(M0, W));
  ROW("(b) row-OR squaring, triangle-restricted, NEON", 3, steps = f_rowor(1), pairs_from_rows(M0, W));
  // SME setup
  t0 = tcpu_ns(); NP = (NC + 31) / 32; K32 = 2 * W;
  size_t pbytes = (size_t)NP * K32 * 32 * 4, fbytes = (size_t)NP * (K32 + 1) * 32 * 4;
  PA = xalloc(pbytes); PB = xalloc(pbytes); PA2 = xalloc(pbytes); PB2 = xalloc(pbytes); PA_init = xalloc(pbytes); PB_init = xalloc(pbytes);
  prefA = xalloc(fbytes); prefB = xalloc(fbytes);
  memset(PA2, 0, pbytes); memset(PB2, 0, pbytes); memset(PB_init, 0, pbytes);
  { uint64_t *mi = xalloc(NC * W * 8); memcpy(mi, Adj, NC * W * 8); for (long c = 0; c < NC; c++) mi[c * W + c / 64] |= 1ull << (c % 64);
    long np; uint32_t *P = pack_panels(mi, NC, W, &np); memcpy(PA_init, P, pbytes); free(P); free(mi); }
  transpose_into(PA_init, PB_init);
  double setup = (tcpu_ns() - t0) / 1e6;
  // one-round cost breakdown
  g_readout = 0; sme_reset(); uint64_t a0 = tcpu_ns(); prefix(PA, prefA); prefix(PB, prefB); uint64_t a1 = tcpu_ns(); sme_square(); uint64_t a2 = tcpu_ns(); transpose_into(PA2, PB2); uint64_t a3 = tcpu_ns();
  g_readout = 1;
  ROW("(b) SME BMOPA squaring, per-row OR-reduction readout", 3, (sme_reset(), steps = f_sme()), pairs_from_panels(PA));
  g_readout = 0;
  ROW("(b) SME BMOPA squaring, triangular blocks + k range, vertical readout", 5, (sme_reset(), steps = f_sme()), pairs_from_panels(PA));
  { stat2 sr; TIME_CPU(5, 1, sr, sme_reset()); printf("\n(timed SME totals include a reset copy of the initial panels: %.2f ms)\n", sr.min / 1e6); }
  printf("\nSME setup (A+I panel pack, initial transpose): %.2f ms CPU. One squaring (first round): prefix popcounts %.2f ms, BMOPA + bit readout %.2f ms, 32x32 transposes %.2f ms (+ fixpoint compare and reset copy in the timed total)\n",
         setup, (a1 - a0) / 1e6, (a2 - a1) / 1e6, (a3 - a2) / 1e6);
  long blocks = 0, bm = 0; for (long ib = 0; ib < NP; ib++) for (long jb = 0; jb <= ib; jb++) { blocks++; bm += 4 * (ib - jb + 1); }
  { g_readout = 2; sme_reset(); prefix(PA, prefA); prefix(PB, prefB); stat2 sn; TIME_CPU(5, 1, sn, sme_square()); g_readout = 0;
    printf("first-round kernel with the readout removed (timing split only): %.2f ms\n", sn.min / 1e6); }
  printf("per squaring: %ld blocks, %ld BMOPAs (%.1f Gbit-op), vs %.1f Gbit-op for the full square product\n", blocks, bm, bm * 8192.0 / 1e9, (double)NP * NP * 4 * K32 * 8192.0 / 1e9);
  print_load("W3 end");
  return 0;
}
