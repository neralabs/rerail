// C6. Roaring-style bitmap containers (8 KB = 1024 u64 = 65536 bits): AND + popcount.
//   pair:      two bitmaps -> popcount(a & b)
//   one-many:  one query bitmap against R others -> R counts. R = 128 (1 MB), R = 1000 (8 MB, half
//              the P-cluster L2) and R = 32768 (256 MB, DRAM).
// Methods: scalar (CSSC CNT), NEON (and + cnt + pairwise accumulate), SME streaming-SVE vectors
// (512-bit and/cnt/add in streaming mode), SME BMOPA "diagonal" (zn = 16 query words, zm = the same
// 16 words of another bitmap; only the tile diagonal is useful, 1/16 of the outer product; the
// correction uses each bitmap's stored cardinality, which Roaring keeps per container).
#include "bench.h"
#include <arm_neon.h>
#include <arm_sme.h>

enum { W = 1024 }; // u64 words per bitmap

static uint32_t pair_scalar(const uint64_t *a, const uint64_t *b) {
  uint64_t s0 = 0, s1 = 0, s2 = 0, s3 = 0;
#pragma clang loop vectorize(disable) interleave(disable)
  for (int w = 0; w < W; w += 4) {
    s0 += (uint64_t)__builtin_popcountll(a[w] & b[w]); s1 += (uint64_t)__builtin_popcountll(a[w + 1] & b[w + 1]);
    s2 += (uint64_t)__builtin_popcountll(a[w + 2] & b[w + 2]); s3 += (uint64_t)__builtin_popcountll(a[w + 3] & b[w + 3]);
  }
  return (uint32_t)(s0 + s1 + s2 + s3);
}

static inline uint8x16_t and_cnt64(const uint8_t *a, const uint8_t *b) {
  uint8x16x4_t x = vld1q_u8_x4(a), y = vld1q_u8_x4(b);
  uint8x16_t c0 = vaddq_u8(vcntq_u8(vandq_u8(x.val[0], y.val[0])), vcntq_u8(vandq_u8(x.val[1], y.val[1])));
  uint8x16_t c1 = vaddq_u8(vcntq_u8(vandq_u8(x.val[2], y.val[2])), vcntq_u8(vandq_u8(x.val[3], y.val[3])));
  return vaddq_u8(c0, c1); // <= 32 per lane
}
static uint32_t pair_neon(const uint64_t *a, const uint64_t *b) {
  const uint8_t *p = (const uint8_t *)a, *q = (const uint8_t *)b;
  uint16x8_t acc0 = vdupq_n_u16(0), acc1 = vdupq_n_u16(0);
  for (int o = 0; o < W * 8; o += 128) { acc0 = vpadalq_u8(acc0, and_cnt64(p + o, q + o)); acc1 = vpadalq_u8(acc1, and_cnt64(p + o + 64, q + o + 64)); }
  return vaddlvq_u16(acc0) + vaddlvq_u16(acc1);
}

__arm_locally_streaming static uint32_t pair_ssve(const uint64_t *a, const uint64_t *b) {
  svbool_t p = svptrue_b64(); svuint64_t s0 = svdup_n_u64(0), s1 = s0, s2 = s0, s3 = s0;
  for (int w = 0; w < W; w += 32) {
    s0 = svadd_u64_x(p, s0, svcnt_u64_x(p, svand_u64_x(p, svld1_u64(p, a + w), svld1_u64(p, b + w))));
    s1 = svadd_u64_x(p, s1, svcnt_u64_x(p, svand_u64_x(p, svld1_u64(p, a + w + 8), svld1_u64(p, b + w + 8))));
    s2 = svadd_u64_x(p, s2, svcnt_u64_x(p, svand_u64_x(p, svld1_u64(p, a + w + 16), svld1_u64(p, b + w + 16))));
    s3 = svadd_u64_x(p, s3, svcnt_u64_x(p, svand_u64_x(p, svld1_u64(p, a + w + 24), svld1_u64(p, b + w + 24))));
  }
  return (uint32_t)svaddv_u64(p, svadd_u64_x(p, svadd_u64_x(p, s0, s1), svadd_u64_x(p, s2, s3)));
}

// One-vs-many.
static void many_scalar(const uint64_t *q, const uint64_t *X, long R, uint32_t *out) { for (long r = 0; r < R; r++) out[r] = pair_scalar(q, X + r * W); }
static void many_neon1(const uint64_t *q, const uint64_t *X, long R, uint32_t *out) { for (long r = 0; r < R; r++) out[r] = pair_neon(q, X + r * W); }
// 4 others per pass: each 64-byte query chunk is loaded once for 4 bitmaps.
static void many_neon4(const uint64_t *q, const uint64_t *X, long R, uint32_t *out) {
  const uint8_t *qp = (const uint8_t *)q;
  for (long r = 0; r < R; r += 4) {
    const uint8_t *x0 = (const uint8_t *)(X + r * W), *x1 = x0 + W * 8, *x2 = x1 + W * 8, *x3 = x2 + W * 8;
    uint16x8_t a0 = vdupq_n_u16(0), a1 = a0, a2 = a0, a3 = a0;
    for (int o = 0; o < W * 8; o += 64) {
      uint8x16x4_t y = vld1q_u8_x4(qp + o);
#define ONE(acc, xp) { uint8x16x4_t x = vld1q_u8_x4(xp + o); \
        uint8x16_t c = vaddq_u8(vaddq_u8(vcntq_u8(vandq_u8(x.val[0], y.val[0])), vcntq_u8(vandq_u8(x.val[1], y.val[1]))), \
                                vaddq_u8(vcntq_u8(vandq_u8(x.val[2], y.val[2])), vcntq_u8(vandq_u8(x.val[3], y.val[3])))); \
        acc = vpadalq_u8(acc, c); }
      ONE(a0, x0) ONE(a1, x1) ONE(a2, x2) ONE(a3, x3)
#undef ONE
    }
    out[r] = vaddlvq_u16(a0); out[r + 1] = vaddlvq_u16(a1); out[r + 2] = vaddlvq_u16(a2); out[r + 3] = vaddlvq_u16(a3);
  }
}
__arm_locally_streaming static void many_ssve(const uint64_t *q, const uint64_t *X, long R, uint32_t *out) {
  svbool_t p = svptrue_b64();
  for (long r = 0; r < R; r += 2) {
    const uint64_t *x0 = X + r * W, *x1 = x0 + W; svuint64_t s0 = svdup_n_u64(0), s1 = s0, s2 = s0, s3 = s0;
    for (int w = 0; w < W; w += 16) {
      svuint64_t qa = svld1_u64(p, q + w), qb = svld1_u64(p, q + w + 8);
      s0 = svadd_u64_x(p, s0, svcnt_u64_x(p, svand_u64_x(p, qa, svld1_u64(p, x0 + w))));
      s1 = svadd_u64_x(p, s1, svcnt_u64_x(p, svand_u64_x(p, qb, svld1_u64(p, x0 + w + 8))));
      s2 = svadd_u64_x(p, s2, svcnt_u64_x(p, svand_u64_x(p, qa, svld1_u64(p, x1 + w))));
      s3 = svadd_u64_x(p, s3, svcnt_u64_x(p, svand_u64_x(p, qb, svld1_u64(p, x1 + w + 8))));
    }
    out[r] = (uint32_t)svaddv_u64(p, svadd_u64_x(p, s0, s1)); out[r + 1] = (uint32_t)svaddv_u64(p, svadd_u64_x(p, s2, s3));
  }
}
// BMOPA diagonal: tile t accumulates xnor(q[w+i], X_t[w+j]); diagonal i==j sums every word pair.
// count = (pop(q) + pop(X) - 65536 + sum(diag)) / 2. popX = stored per-bitmap cardinality.
#define DIAG(t, dst) { for (int i = 0; i < 16; i++) svst1_hor_za32(t, i, p, buf + 16 * i); uint32_t d = 0; for (int i = 0; i < 16; i++) d += buf[17 * i]; dst = d; }
__arm_locally_streaming __arm_new("za")
static void many_bmopa(const uint64_t *q64, const uint64_t *X64, long R, const uint32_t *popX, uint32_t popq, uint32_t *out) {
  svbool_t p = svptrue_b32(); const uint32_t *q = (const uint32_t *)q64; uint32_t buf[256];
  for (long r = 0; r < R; r += 4) {
    const uint32_t *x0 = (const uint32_t *)(X64 + r * W), *x1 = x0 + 2 * W, *x2 = x1 + 2 * W, *x3 = x2 + 2 * W;
    svzero_za();
    for (int w = 0; w < 2 * W; w += 16) {
      svuint32_t qv = svld1_u32(p, q + w);
      svbmopa_za32_u32_m(0, p, p, qv, svld1_u32(p, x0 + w)); svbmopa_za32_u32_m(1, p, p, qv, svld1_u32(p, x1 + w));
      svbmopa_za32_u32_m(2, p, p, qv, svld1_u32(p, x2 + w)); svbmopa_za32_u32_m(3, p, p, qv, svld1_u32(p, x3 + w));
    }
    uint32_t d0, d1, d2, d3; DIAG(0, d0) DIAG(1, d1) DIAG(2, d2) DIAG(3, d3)
    out[r] = (popq + popX[r] - 65536u + d0) / 2; out[r + 1] = (popq + popX[r + 1] - 65536u + d1) / 2;
    out[r + 2] = (popq + popX[r + 2] - 65536u + d2) / 2; out[r + 3] = (popq + popX[r + 3] - 65536u + d3) / 2;
  }
}

// BMOPA over a 16-bitmap interleaved ("transposed") layout: Xt[g][w][16] = u32 word w of bitmap
// 16g + lane. zn = query word w replicated in all 16 lanes, zm = word w of 16 bitmaps, so every
// tile row holds the 16 bitmaps' xnor sums (1/16 of the outer product is useful, but no diagonal
// extraction: one row store per 16 bitmaps). 4 tiles = 64 bitmaps per pass.
__arm_locally_streaming __arm_new("za")
static void many_bmopa_t(const uint64_t *q64, const uint32_t *Xt, long R, const uint32_t *popX, uint32_t popq, uint32_t *out) {
  svbool_t p = svptrue_b32(); const uint32_t *q = (const uint32_t *)q64; uint32_t row[64];
  for (long g = 0; g < R / 16; g += 4) {
    const uint32_t *x0 = Xt + g * 2 * W * 16, *x1 = x0 + 2 * W * 16, *x2 = x1 + 2 * W * 16, *x3 = x2 + 2 * W * 16;
    svzero_za();
    for (int w = 0; w < 2 * W; w++) {
      svuint32_t qv = svdup_n_u32(q[w]);
      svbmopa_za32_u32_m(0, p, p, qv, svld1_u32(p, x0 + 16 * w)); svbmopa_za32_u32_m(1, p, p, qv, svld1_u32(p, x1 + 16 * w));
      svbmopa_za32_u32_m(2, p, p, qv, svld1_u32(p, x2 + 16 * w)); svbmopa_za32_u32_m(3, p, p, qv, svld1_u32(p, x3 + 16 * w));
    }
    svst1_hor_za32(0, 0, p, row); svst1_hor_za32(1, 0, p, row + 16); svst1_hor_za32(2, 0, p, row + 32); svst1_hor_za32(3, 0, p, row + 48);
    for (int j = 0; j < 64; j++) { long r = g * 16 + j; out[r] = (popq + popX[r] - 65536u + row[j]) / 2; }
  }
}

int main(void) {
  prefer_p_cores();
  print_load("C6 start");
  uint64_t seed = 5;
  long Rs[] = { 128, 1000, 32768 }; // 1 MB (L2), 8 MB (the task size; half the P L2, shared under load), 256 MB (DRAM)
  long Rmax = 32768;
  uint64_t *X = xalloc((size_t)Rmax * W * 8), *q = xalloc(W * 8);
  uint32_t *popX = xalloc(Rmax * 4 + 256), *out = xalloc(Rmax * 4 + 256), *ref = xalloc(Rmax * 4);
  for (long i = 0; i < Rmax * W; i++) X[i] = rng_next(&seed) & rng_next(&seed);
  for (int i = 0; i < W; i++) q[i] = rng_next(&seed);
  for (long r = 0; r < Rmax; r++) { uint32_t s = 0; for (int w = 0; w < W; w++) s += (uint32_t)__builtin_popcountll(X[r * W + w]); popX[r] = s; }
  uint32_t popq = 0; for (int w = 0; w < W; w++) popq += (uint32_t)__builtin_popcountll(q[w]);
  for (long r = 0; r < Rmax; r++) { uint32_t s = 0; for (int w = 0; w < W; w++) s += (uint32_t)__builtin_popcountll(q[w] & X[r * W + w]); ref[r] = s; }

  // Interleaved layout for many_bmopa_t (setup, timed separately); R padded to a multiple of 64.
  uint64_t t0 = tcpu_ns();
  uint32_t *Xt = xalloc((size_t)Rmax * W * 8);
  for (long r = 0; r < Rmax; r++) { const uint32_t *x = (const uint32_t *)(X + r * W); long g = r / 16, l = r % 16;
    for (int w = 0; w < 2 * W; w++) Xt[(g * 2 * W + w) * 16 + l] = x[w]; }
  double pack_ms = (tcpu_ns() - t0) / 1e6;
  printf("interleaved-layout pack of %ld bitmaps: %.1f ms CPU (setup, not in the tables)\n\n", Rmax, pack_ms);

  // Pair.
  printf("## Pair: popcount(a & b) for two 8 KB bitmaps (both L1-resident); 16 KB read per pair\n\n| method | ns/pair min | median | GB/CPU-s read | core |\n|---|---|---|---|---|\n");
  struct { const char *n; uint32_t (*f)(const uint64_t *, const uint64_t *); } pm[] = {
    { "scalar (CSSC CNT)", pair_scalar }, { "NEON and+cnt+padal", pair_neon }, { "SME streaming SVE (512-bit and/cnt/add)", pair_ssve } };
  for (int m = 0; m < 3; m++) {
    int bad = 0; for (int r = 0; r < 64; r++) if (pm[m].f(q, X + r * W) != ref[r]) bad++;
    if (bad) { printf("| %s | VERIFY FAIL | | | |\n", pm[m].n); continue; }
    stat2 st; uint32_t acc = 0; TIME_CPU(11, 20000, st, acc += pm[m].f(q, X + 3 * W)); SINK(acc);
    printf("| %s | %.1f | %.1f | %.1f | %s |\n", pm[m].n, st.min, st.med, 16384.0 / st.min, cl_name(g_min_cpu));
  }

  printf("\n## One query against R others (bytes counted: R x 8 KB of others)\n\n| R | others MB | method | ns/bitmap min | median | GB/CPU-s read | Gbit-op/CPU-s | core |\n|---|---|---|---|---|---|---|---|\n");
  struct { const char *n; int kind; } mm[] = { { "scalar (CSSC CNT)", 0 }, { "NEON, 1 bitmap per pass", 1 }, { "NEON, 4 bitmaps per pass", 2 }, { "SME streaming SVE, 2 per pass", 3 }, { "SME BMOPA diagonal, 4 tiles", 4 }, { "SME BMOPA, 16-bitmap interleaved layout, 4 tiles", 5 } };
  for (int ri = 0; ri < 3; ri++) {
    long R = Rs[ri];
    for (int m = 0; m < 6; m++) {
#define RUN(Rn) do { switch (mm[m].kind) { case 0: many_scalar(q, X, Rn, out); break; case 1: many_neon1(q, X, Rn, out); break; \
        case 2: many_neon4(q, X, Rn, out); break; case 3: many_ssve(q, X, Rn, out); break; case 4: many_bmopa(q, X, Rn, popX, popq, out); break; \
        default: if (Rn % 64 == 0) many_bmopa_t(q, Xt, Rn, popX, popq, out); else { many_bmopa_t(q, Xt, Rn - Rn % 64, popX, popq, out); many_neon1(q, X + (Rn - Rn % 64) * W, Rn % 64, out + Rn - Rn % 64); } } } while (0)
      memset(out, 0, R * 4); RUN(R);
      int bad = 0; for (long r = 0; r < R; r++) if (out[r] != ref[r]) bad++;
      if (bad) { printf("| %ld | %.0f | %s | VERIFY FAIL (%d) | | | | |\n", R, R * 8192.0 / 1e6, mm[m].n, bad); continue; }
      stat2 st; int reps = R > 5000 ? 5 : 11; long it = R > 5000 ? 1 : 20;
      TIME_CPU(reps, it, st, RUN(R));
      double per = st.min / R;
      printf("| %ld | %.0f | %s | %.1f | %.1f | %.1f | %.0f | %s |\n", R, R * 8192.0 / 1e6, mm[m].n, per, st.med / R, 8192.0 / per, 65536.0 / per, cl_name(g_min_cpu));
      fflush(stdout);
    }
  }
  print_load("C6 end");
  return 0;
}
