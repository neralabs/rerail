// W5. Name co-occurrence over documents (MLDB's A^T A): 2048 names x 13,088 document bits
// (205 u64 words). Sum over the upper triangle of popcount(row_i & row_j); expected 29,397,646.
// Forms: bitmap AND+popcount (the baseline's form, C port, autovectorised), NEON 4x4 register
// block, SME BMOPA 2x2 tiles (upper-triangle blocks, corrections seeded with ADDVA/ADDHA), and on
// sorted u16 postings derived from the bitmaps: branch-free merge (baseline form) and the NEON
// 8x8 all-pairs intersection, plain and with two pairs interleaved.
#include "mo.h"

static long N = 2048, W = 205; static uint64_t *A;
static const uint64_t EXPECT = 29397646;

static uint64_t f_bitmap(void) {
  uint64_t s = 0;
  for (long i = 0; i < N; i++) { const uint64_t *x = A + i * W;
    for (long j = i + 1; j < N; j++) { const uint64_t *y = A + j * W; uint32_t c = 0; for (long w = 0; w < W; w++) c += (uint32_t)__builtin_popcountll(x[w] & y[w]); s += c; } }
  return s;
}

// NEON 4x4 over row blocks i0..i0+3 x j0..j0+3; 205 words = 102 q-vectors + 1 tail word.
static uint64_t f_neon(void) {
  uint64_t s = 0; long WV = W / 2;
  for (long i = 0; i < N; i += 4) for (long j = i; j < N; j += 4) {
    const uint64_t *a[4], *b[4]; for (int q = 0; q < 4; q++) { a[q] = A + (i + q) * W; b[q] = A + (j + q) * W; }
    uint16x8_t acc[16]; for (int q = 0; q < 16; q++) acc[q] = vdupq_n_u16(0);
    for (long v = 0; v < WV; v++) {
      uint8x16_t x0 = vld1q_u8((const uint8_t *)(a[0] + 2 * v)), x1 = vld1q_u8((const uint8_t *)(a[1] + 2 * v)), x2 = vld1q_u8((const uint8_t *)(a[2] + 2 * v)), x3 = vld1q_u8((const uint8_t *)(a[3] + 2 * v));
      uint8x16_t y0 = vld1q_u8((const uint8_t *)(b[0] + 2 * v)), y1 = vld1q_u8((const uint8_t *)(b[1] + 2 * v)), y2 = vld1q_u8((const uint8_t *)(b[2] + 2 * v)), y3 = vld1q_u8((const uint8_t *)(b[3] + 2 * v));
#define P(r, x, y) acc[r] = vpadalq_u8(acc[r], vcntq_u8(vandq_u8(x, y)))
      P(0, x0, y0); P(1, x0, y1); P(2, x0, y2); P(3, x0, y3); P(4, x1, y0); P(5, x1, y1); P(6, x1, y2); P(7, x1, y3);
      P(8, x2, y0); P(9, x2, y1); P(10, x2, y2); P(11, x2, y3); P(12, x3, y0); P(13, x3, y1); P(14, x3, y2); P(15, x3, y3);
#undef P
    }
    for (int q = 0; q < 16; q++) { long ii = i + q / 4, jj = j + q % 4; if (jj <= ii) continue;
      uint32_t c = vaddlvq_u16(acc[q]) + (uint32_t)__builtin_popcountll(a[q / 4][W - 1] & b[q % 4][W - 1]); s += c; }
  }
  return s;
}

// SME
static uint32_t *Pn, *pa_k, *pb; static long NP, K32; static uint32_t *Cbuf; // Cbuf: N x N counts
__arm_locally_streaming __arm_new("za") static void sme_rows(long ib) {
  svbool_t p = svptrue_b32(); svcount_t pc = svptrue_c32(); svuint32_t m1 = svdup_n_u32(0xffffffffu);
  const uint32_t *ap = Pn + ib * K32 * 32;
  svuint32_t ra0 = svld1_u32(p, pa_k + ib * 32), ra1 = svld1_u32(p, pa_k + ib * 32 + 16);
  for (long jb = ib; jb < NP; jb++) {
    const uint32_t *bp = Pn + jb * K32 * 32;
    svuint32_t cb0 = svld1_u32(p, pb + jb * 32), cb1 = svld1_u32(p, pb + jb * 32 + 16);
    svzero_za();
    svaddva_za32_u32_m(0, p, p, ra0); svaddva_za32_u32_m(1, p, p, ra0); svaddva_za32_u32_m(2, p, p, ra1); svaddva_za32_u32_m(3, p, p, ra1);
    svaddha_za32_u32_m(0, p, p, cb0); svaddha_za32_u32_m(1, p, p, cb1); svaddha_za32_u32_m(2, p, p, cb0); svaddha_za32_u32_m(3, p, p, cb1);
    for (long w = 0; w < K32; w++) {
      svuint32x2_t x = svld1_u32_x2(pc, ap + w * 32), y = svld1_u32_x2(pc, bp + w * 32);
      svbmopa_za32_u32_m(0, p, p, svget2(x, 0), svget2(y, 0)); svbmopa_za32_u32_m(1, p, p, svget2(x, 0), svget2(y, 1));
      svbmopa_za32_u32_m(2, p, p, svget2(x, 1), svget2(y, 0)); svbmopa_za32_u32_m(3, p, p, svget2(x, 1), svget2(y, 1));
    }
    uint32_t *Cb = Cbuf + ib * 32 * N + jb * 32;
#define OUT(t, ro, co) for (int r = 0; r < 16; r += 4) { svuint32x4_t v = svrshl_single_u32_x4(svread_hor_za32_u32_vg4(t, r), m1); \
      svst1_u32(p, Cb + (ro + r) * N + co, svget4(v, 0)); svst1_u32(p, Cb + (ro + r + 1) * N + co, svget4(v, 1)); \
      svst1_u32(p, Cb + (ro + r + 2) * N + co, svget4(v, 2)); svst1_u32(p, Cb + (ro + r + 3) * N + co, svget4(v, 3)); }
    OUT(0, 0, 0) OUT(1, 0, 16) OUT(2, 16, 0) OUT(3, 16, 16)
#undef OUT
  }
}
static uint64_t f_sme(void) {
  for (long ib = 0; ib < NP; ib++) sme_rows(ib);
  uint64_t s = 0; // reduce the strict upper triangle of the written block-upper product
  for (long i = 0; i < N; i++) { const uint32_t *c = Cbuf + i * N; uint32x4_t acc = vdupq_n_u32(0); long j = i + 1;
    for (; j < N && (j & 3); j++) s += c[j];
    for (; j + 4 <= N; j += 4) acc = vaddq_u32(acc, vld1q_u32(c + j));
    for (; j < N; j++) s += c[j];
    s += vaddvq_u32(acc); }
  return s;
}

// Postings (u16 doc ids, 13,088 < 65536)
static uint16_t *post[2048]; static int plen[2048];
static uint64_t f_merge(void) {
  uint64_t s = 0;
  for (long i = 0; i < N; i++) for (long j = i + 1; j < N; j++) {
    const uint16_t *a = post[i], *b = post[j]; int na = plen[i], nb = plen[j], x = 0, y = 0, r = 0;
    while (x < na && y < nb) { int u = a[x], v = b[y]; r += u == v; x += u <= v; y += u >= v; }
    s += (uint64_t)r; }
  return s;
}
static inline int tail(const uint16_t *a, int i, int na, const uint16_t *b, int j, int nb, int r) {
  while (i < na && j < nb) { int x = a[i], y = b[j]; r += x == y; i += x <= y; j += x >= y; } return r;
}
#define ROT_STEP(a, b, i, j, acc) do { uint16x8_t va = vld1q_u16(a + i), vb = vld1q_u16(b + j); uint16x8_t m = vceqq_u16(va, vb); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 1))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 2))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 3))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 4))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 5))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 6))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 7))); acc = vsubq_u16(acc, m); \
    uint16_t la = a[i + 7], lb = b[j + 7]; i += 8 * (la <= lb); j += 8 * (lb <= la); } while (0)
// u16 lane accumulators can overflow on very long postings (max 12,558): flush every 4096 steps.
static int rot1(const uint16_t *a, int na, const uint16_t *b, int nb) {
  int i = 0, j = 0, r = 0; uint16x8_t acc = vdupq_n_u16(0); int steps = 0;
  while (i + 8 <= na && j + 8 <= nb) { ROT_STEP(a, b, i, j, acc); if (++steps == 4096) { r += vaddlvq_u16(acc); acc = vdupq_n_u16(0); steps = 0; } }
  return tail(a, i, na, b, j, nb, r + (int)vaddlvq_u16(acc));
}
static uint64_t f_rot(void) { uint64_t s = 0; for (long i = 0; i < N; i++) for (long j = i + 1; j < N; j++) s += (uint64_t)rot1(post[i], plen[i], post[j], plen[j]); return s; }
static uint64_t f_rot2(void) {
  uint64_t s = 0;
  for (long i = 0; i < N; i++) {
    long j = i + 1;
    for (; j + 1 < N; j += 2) {
      const uint16_t *a0 = post[i], *b0 = post[j], *a1 = post[i], *b1 = post[j + 1];
      int na0 = plen[i], nb0 = plen[j], na1 = plen[i], nb1 = plen[j + 1];
      int i0 = 0, j0 = 0, i1 = 0, j1 = 0; uint16x8_t c0 = vdupq_n_u16(0), c1 = c0; int r0 = 0, r1 = 0, st = 0;
      while (i0 + 8 <= na0 && j0 + 8 <= nb0 && i1 + 8 <= na1 && j1 + 8 <= nb1) {
        ROT_STEP(a0, b0, i0, j0, c0); ROT_STEP(a1, b1, i1, j1, c1);
        if (++st == 4096) { r0 += vaddlvq_u16(c0); r1 += vaddlvq_u16(c1); c0 = c1 = vdupq_n_u16(0); st = 0; } }
      st = 0; while (i0 + 8 <= na0 && j0 + 8 <= nb0) { ROT_STEP(a0, b0, i0, j0, c0); if (++st == 4096) { r0 += vaddlvq_u16(c0); c0 = vdupq_n_u16(0); st = 0; } }
      st = 0; while (i1 + 8 <= na1 && j1 + 8 <= nb1) { ROT_STEP(a1, b1, i1, j1, c1); if (++st == 4096) { r1 += vaddlvq_u16(c1); c1 = vdupq_n_u16(0); st = 0; } }
      s += (uint64_t)tail(a0, i0, na0, b0, j0, nb0, r0 + (int)vaddlvq_u16(c0)) + (uint64_t)tail(a1, i1, na1, b1, j1, nb1, r1 + (int)vaddlvq_u16(c1));
    }
    for (; j < N; j++) s += (uint64_t)rot1(post[i], plen[i], post[j], plen[j]);
  }
  return s;
}

int main(int argc, char **argv) {
  init_data(argc, argv); prefer_p_cores();
  print_load("W5 start");
  size_t sz; A = load_file("w5/names_bits.u64", &sz);
  if (sz != (size_t)N * W * 8) { printf("size mismatch %zu\n", sz); return 1; }
  double in_mb = sz / 1e6; double ops = (double)N * (N - 1) / 2 * W * 64;
  printf("2048 names x %ld words; pairs %ld; input %.2f MB; expected sum %llu\n\n", W, N * (N - 1) / 2, in_mb, (unsigned long long)EXPECT);
  printf("| form | CPU s min | median | Gbit-op/CPU-s | result | core |\n|---|---|---|---|---|---|\n");
  stat2 st; uint64_t r = 0;
#define ROW(name, reps, call, opsv) do { r = call; int ok = r == EXPECT; TIME_CPU(reps, 1, st, r = call); ok &= r == EXPECT; \
    printf("| %s | %.4f | %.4f | %s | %llu %s | %s |\n", name, st.min / 1e9, st.med / 1e9, opsv > 0 ? ({ static char b[32]; snprintf(b, 32, "%.0f", opsv / st.min); b; }) : "-", (unsigned long long)r, ok ? "OK" : "MISMATCH", cl_name(g_min_cpu)); fflush(stdout); } while (0)
  ROW("bitmap AND+popcount, autovectorised (baseline form, C port)", 5, f_bitmap(), ops);
  ROW("NEON 4x4 register block", 5, f_neon(), ops);
  // SME setup
  uint64_t t0 = tcpu_ns();
  Pn = pack_panels(A, N, W, &NP); K32 = W * 2;
  pa_k = row_pops(A, N, W, NP * 32, 32 * K32); pb = row_pops(A, N, W, NP * 32, 0);
  Cbuf = xalloc((size_t)N * N * 4); memset(Cbuf, 0, (size_t)N * N * 4);
  double setup = (tcpu_ns() - t0) / 1e6;
  double sme_ops = (double)(NP * (NP + 1) / 2) * 32 * 32 * K32 * 32; // block-upper work actually done
  ROW("SME BMOPA 2x2, block-upper product written + reduced", 7, f_sme(), ops);
  printf("\nSME setup (panel pack + row popcounts + 16 MB output alloc): %.2f ms CPU; work done incl. diagonal blocks %.1f Gbit-op, issue-rate view %.0f Gbit-op/CPU-s\n\n", setup, sme_ops / 1e9, sme_ops / st.min);
  // postings
  t0 = tcpu_ns(); long tot = 0;
  for (long i = 0; i < N; i++) { int c = 0; for (long w = 0; w < W; w++) c += __builtin_popcountll(A[i * W + w]);
    post[i] = xalloc((size_t)(c + 16) * 2); int k = 0;
    for (long w = 0; w < W; w++) { uint64_t x = A[i * W + w]; while (x) { post[i][k++] = (uint16_t)(w * 64 + __builtin_ctzll(x)); x &= x - 1; } }
    plen[i] = k; tot += k; }
  printf("postings derived from bitmaps: %ld entries (%.2f MB as u16), %.2f ms CPU\n\n| form | CPU s min | median | Gbit-op/CPU-s | result | core |\n|---|---|---|---|---|---|\n", tot, tot * 2 / 1e6, (tcpu_ns() - t0) / 1e6);
  ROW("sorted u16 postings, branch-free merge (baseline form, C port)", 1, f_merge(), 0.0);
  ROW("sorted u16 postings, NEON 8x8 all-pairs", 3, f_rot(), 0.0);
  ROW("sorted u16 postings, NEON 8x8, two pairs interleaved", 3, f_rot2(), 0.0);
  print_load("W5 end");
  return 0;
}
