// W1. Prefilter overlap: documents 13,088 x 767 name bits (12 u64 words), rules 444 x 767 bits.
// Count (document, rule) cells with a nonzero overlap; expected 1,473,222.
// Forms: postings per rule (the engine's form, C port: postings derived from the bits), dense
// AND-popcount != 0 (baseline dense form, C port), NEON AND-any 4x4, SME BMOPA 2x2 with a
// nonzero-count readout (small k = 24 u32 words: readout-heavy by construction).
#include "mo.h"

static long ND = 13088, NR = 444, W = 12; static uint64_t *D, *Rb;
static const long EXPECT = 1473222;

static long f_dense(void) {
  long nz = 0;
  for (long d = 0; d < ND; d++) { const uint64_t *x = D + d * W;
    for (long r = 0; r < NR; r++) { const uint64_t *y = Rb + r * W; uint32_t c = 0; for (long w = 0; w < W; w++) c += (uint32_t)__builtin_popcountll(x[w] & y[w]); nz += c != 0; } }
  return nz;
}

// postings: column -> docs; rules: list of columns
static uint32_t *poff, *pval; static uint32_t *roff, *rval; static uint32_t *cnt;
static long f_postings(void) {
  long nz = 0;
  for (long r = 0; r < NR; r++) {
    for (uint32_t t = roff[r]; t < roff[r + 1]; t++) { uint32_t c = rval[t]; for (uint32_t q = poff[c]; q < poff[c + 1]; q++) cnt[pval[q]]++; }
    for (long d = 0; d < ND; d++) { nz += cnt[d] != 0; cnt[d] = 0; }
  }
  return nz;
}

// NEON: per (4 docs x 4 rules) block, OR-accumulate the ANDs over 6 q-vectors, then test nonzero.
static long f_neon(void) {
  long nzl = 0; uint32x4_t nzv = vdupq_n_u32(0);
  for (long d = 0; d < ND; d += 4) {
    uint64x2_t x[4][6];
    for (int q = 0; q < 4; q++) for (int v = 0; v < 6; v++) x[q][v] = vld1q_u64(D + (d + q) * W + 2 * v);
    for (long r = 0; r < NR; r += 4) {
      uint64x2_t acc[16];
      for (int b = 0; b < 4; b++) { const uint64_t *y = Rb + (r + b) * W;
        uint64x2_t y0 = vld1q_u64(y), y1 = vld1q_u64(y + 2), y2 = vld1q_u64(y + 4), y3 = vld1q_u64(y + 6), y4 = vld1q_u64(y + 8), y5 = vld1q_u64(y + 10);
        for (int q = 0; q < 4; q++)
          acc[q * 4 + b] = vorrq_u64(vorrq_u64(vorrq_u64(vandq_u64(x[q][0], y0), vandq_u64(x[q][1], y1)), vorrq_u64(vandq_u64(x[q][2], y2), vandq_u64(x[q][3], y3))), vorrq_u64(vandq_u64(x[q][4], y4), vandq_u64(x[q][5], y5)));
      }
      for (int g = 0; g < 16; g += 4) {
        uint32x4_t a = vpmaxq_u32(vreinterpretq_u32_u64(acc[g]), vreinterpretq_u32_u64(acc[g + 1]));
        uint32x4_t b = vpmaxq_u32(vreinterpretq_u32_u64(acc[g + 2]), vreinterpretq_u32_u64(acc[g + 3]));
        uint32x4_t z = vpmaxq_u32(a, b);                // lane k: max over the 4 u32 of acc[g+k]
        nzv = vsubq_u32(nzv, vtstq_u32(z, z));
      }
    }
  }
  return nzl + vaddvq_u32(nzv);
}

// SME
static uint32_t *Pd, *Pr, *pd_k, *pr; static long NPD, NPR, K32;
__arm_locally_streaming __arm_new("za") static long sme_all(void) {
  svbool_t p = svptrue_b32(); svcount_t pc = svptrue_c32(); uint64_t nz = 0;
  for (long ib = 0; ib < NPD; ib++) {
    const uint32_t *ap = Pd + ib * K32 * 32;
    svuint32_t ra0 = svld1_u32(p, pd_k + ib * 32), ra1 = svld1_u32(p, pd_k + ib * 32 + 16);
    for (long jb = 0; jb < NPR; jb++) {
      const uint32_t *bp = Pr + jb * K32 * 32;
      svuint32_t cb0 = svld1_u32(p, pr + jb * 32), cb1 = svld1_u32(p, pr + jb * 32 + 16);
      svzero_za();
      svaddva_za32_u32_m(0, p, p, ra0); svaddva_za32_u32_m(1, p, p, ra0); svaddva_za32_u32_m(2, p, p, ra1); svaddva_za32_u32_m(3, p, p, ra1);
      svaddha_za32_u32_m(0, p, p, cb0); svaddha_za32_u32_m(1, p, p, cb1); svaddha_za32_u32_m(2, p, p, cb0); svaddha_za32_u32_m(3, p, p, cb1);
      for (long w = 0; w < K32; w++) {
        svuint32x2_t x = svld1_u32_x2(pc, ap + w * 32), y = svld1_u32_x2(pc, bp + w * 32);
        svbmopa_za32_u32_m(0, p, p, svget2(x, 0), svget2(y, 0)); svbmopa_za32_u32_m(1, p, p, svget2(x, 0), svget2(y, 1));
        svbmopa_za32_u32_m(2, p, p, svget2(x, 1), svget2(y, 0)); svbmopa_za32_u32_m(3, p, p, svget2(x, 1), svget2(y, 1));
      }
#define NZ(t) for (int r = 0; r < 16; r += 4) { svuint32x4_t v = svread_hor_za32_u32_vg4(t, r); \
        nz += svcntp_b32(p, svcmpne_n_u32(p, svget4(v, 0), 0)) + svcntp_b32(p, svcmpne_n_u32(p, svget4(v, 1), 0)) \
            + svcntp_b32(p, svcmpne_n_u32(p, svget4(v, 2), 0)) + svcntp_b32(p, svcmpne_n_u32(p, svget4(v, 3), 0)); }
      NZ(0) NZ(1) NZ(2) NZ(3)
#undef NZ
    }
  }
  return (long)nz;
}

int main(int argc, char **argv) {
  init_data(argc, argv); prefer_p_cores();
  print_load("W1 start");
  size_t s1, s2; D = load_file("w1/docs_bits.u64", &s1); Rb = load_file("w1/rules_bits.u64", &s2);
  if (s1 != (size_t)ND * W * 8 || s2 != (size_t)NR * W * 8) { printf("size mismatch\n"); return 1; }
  double inb = (double)(s1 + s2);
  // postings setup
  uint64_t t0 = tcpu_ns(); long ncol = 767;
  poff = xalloc((ncol + 1) * 4); memset(poff, 0, (ncol + 1) * 4);
  for (long d = 0; d < ND; d++) for (long c = 0; c < ncol; c++) if (D[d * W + c / 64] >> (c % 64) & 1) poff[c + 1]++;
  for (long c = 0; c < ncol; c++) poff[c + 1] += poff[c];
  pval = xalloc(poff[ncol] * 4 + 4); uint32_t *fill = xalloc(ncol * 4); memcpy(fill, poff, ncol * 4);
  for (long d = 0; d < ND; d++) for (long c = 0; c < ncol; c++) if (D[d * W + c / 64] >> (c % 64) & 1) pval[fill[c]++] = (uint32_t)d;
  roff = xalloc((NR + 1) * 4); roff[0] = 0; rval = xalloc(NR * ncol * 4);
  for (long r = 0; r < NR; r++) { uint32_t k = roff[r]; for (long c = 0; c < ncol; c++) if (Rb[r * W + c / 64] >> (c % 64) & 1) rval[k++] = (uint32_t)c; roff[r + 1] = k; }
  cnt = xalloc(ND * 4); memset(cnt, 0, ND * 4);
  printf("postings derived: %u (document, name) entries, %u rule names; setup %.2f ms CPU\n", poff[ncol], roff[NR], (tcpu_ns() - t0) / 1e6);
  double ops = (double)ND * NR * W * 64;
  printf("input: docs %.2f MB + rules %.3f MB; expected nonzero %ld of %ld\n\n| form | CPU s min | median | Gbit-op/CPU-s | input GB/CPU-s | result | core |\n|---|---|---|---|---|---|---|\n", s1 / 1e6, s2 / 1e6, EXPECT, ND * NR);
  stat2 st; long r;
#define ROW(name, reps, call, opsv) do { r = call; int ok = r == EXPECT; TIME_CPU(reps, 1, st, r = call); ok &= r == EXPECT; \
    printf("| %s | %.5f | %.5f | %s | %.1f | %ld %s | %s |\n", name, st.min / 1e9, st.med / 1e9, opsv > 0 ? ({ static char b[32]; snprintf(b, 32, "%.0f", opsv / st.min); b; }) : "-", inb / st.min, r, ok ? "OK" : "MISMATCH", cl_name(g_min_cpu)); fflush(stdout); } while (0)
  ROW("postings per rule (engine form, C port)", 9, f_postings(), 0.0);
  ROW("dense AND-popcount != 0 (baseline dense form, C port)", 9, f_dense(), ops);
  ROW("NEON AND-any 4x4 (OR-accumulate, no popcount)", 9, f_neon(), ops);
  t0 = tcpu_ns();
  Pd = pack_panels(D, ND, W, &NPD); Pr = pack_panels(Rb, NR, W, &NPR); K32 = 2 * W;
  pd_k = row_pops(D, ND, W, NPD * 32, 32 * K32); pr = row_pops(Rb, NR, W, NPR * 32, 0);
  double setup = (tcpu_ns() - t0) / 1e6;
  ROW("SME BMOPA 2x2, nonzero-count readout", 9, sme_all(), ops);
  printf("\nSME setup (pack %ld + %ld panels, row popcounts): %.2f ms CPU; rules padded to %ld rows\n", NPD, NPR, setup, NPR * 32);
  print_load("W1 end");
  return 0;
}
