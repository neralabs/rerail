// C7. Memory bandwidth calibration (CPU time based).
// Read-and-reduce kernels over arrays of 64 KB (L1), 1 MB and 4 MB (L2), and 512 MB (DRAM):
//   scalar:  ldp + 4 independent u64 add chains
//   NEON:    ld1 {4 regs} x2 per iteration, 8 independent u64 add accumulators
//   SME-SVE: streaming ld1d {4 regs} (SME2) + 512-bit EOR accumulators
//   SME-MOPA: streaming ld1w {4 regs} feeding BMOPA (the operand path of C2/C4 kernels)
// Then all threads: per-thread rate by observed concurrency (bench.h run_chunked), DRAM and L2.
#include "bench.h"
#include <arm_neon.h>
#include <arm_sme.h>

static uint64_t rd_scalar(const uint64_t *p, long n) {
  uint64_t a = 0, b = 0, c = 0, d = 0;
#pragma clang loop vectorize(disable) interleave(disable)
  for (long i = 0; i < n; i += 4) { a += p[i]; b += p[i + 1]; c += p[i + 2]; d += p[i + 3]; }
  return a + b + c + d;
}
static uint64_t rd_neon(const uint64_t *p, long n) {
  uint64x2_t a0 = vdupq_n_u64(0), a1 = a0, a2 = a0, a3 = a0, a4 = a0, a5 = a0, a6 = a0, a7 = a0;
  for (long i = 0; i < n; i += 16) {
    uint64x2x4_t x = vld1q_u64_x4(p + i), y = vld1q_u64_x4(p + i + 8);
    a0 = vaddq_u64(a0, x.val[0]); a1 = vaddq_u64(a1, x.val[1]); a2 = vaddq_u64(a2, x.val[2]); a3 = vaddq_u64(a3, x.val[3]);
    a4 = vaddq_u64(a4, y.val[0]); a5 = vaddq_u64(a5, y.val[1]); a6 = vaddq_u64(a6, y.val[2]); a7 = vaddq_u64(a7, y.val[3]);
  }
  uint64x2_t s = vaddq_u64(vaddq_u64(vaddq_u64(a0, a1), vaddq_u64(a2, a3)), vaddq_u64(vaddq_u64(a4, a5), vaddq_u64(a6, a7)));
  return vgetq_lane_u64(s, 0) + vgetq_lane_u64(s, 1);
}
__arm_locally_streaming static uint64_t rd_ssve(const uint64_t *p, long n) {
  svbool_t pt = svptrue_b64(); svcount_t pc = svptrue_c64();
  svuint64_t a0 = svdup_n_u64(0), a1 = a0, a2 = a0, a3 = a0;
  for (long i = 0; i < n; i += 32) {
    svuint64x4_t x = svld1_u64_x4(pc, p + i);
    a0 = sveor_u64_x(pt, a0, svget4(x, 0)); a1 = sveor_u64_x(pt, a1, svget4(x, 1));
    a2 = sveor_u64_x(pt, a2, svget4(x, 2)); a3 = sveor_u64_x(pt, a3, svget4(x, 3));
  }
  return svaddv_u64(pt, sveor_u64_x(pt, sveor_u64_x(pt, a0, a1), sveor_u64_x(pt, a2, a3)));
}
__arm_locally_streaming __arm_new("za") static uint64_t rd_mopa(const uint64_t *p64, long n) {
  svbool_t pt = svptrue_b32(); svcount_t pc = svptrue_c32(); const uint32_t *p = (const uint32_t *)p64;
  svzero_za();
  for (long i = 0; i < 2 * n; i += 64) {
    svuint32x4_t v = svld1_u32_x4(pc, p + i);
    svbmopa_za32_u32_m(0, pt, pt, svget4(v, 0), svget4(v, 2)); svbmopa_za32_u32_m(1, pt, pt, svget4(v, 0), svget4(v, 3));
    svbmopa_za32_u32_m(2, pt, pt, svget4(v, 1), svget4(v, 2)); svbmopa_za32_u32_m(3, pt, pt, svget4(v, 1), svget4(v, 3));
  }
  uint32_t row[16]; svst1_hor_za32(0, 0, pt, row); return row[0];
}

typedef uint64_t (*rd_fn)(const uint64_t *, long);
static struct { const char *n; rd_fn f; } K[] = { { "scalar ldp, 4 chains", rd_scalar }, { "NEON ld1 x4, 8 accumulators", rd_neon },
                                                  { "SME streaming ld1d x4 + EOR", rd_ssve }, { "SME streaming ld1w x4 -> BMOPA", rd_mopa } };

static uint64_t *g_big; static long g_big_words; static int g_kern; static long g_chunk_words; static long g_slice_words;
static long g_pos[64];
static uint64_t *g_l2buf[64]; static long g_l2_words;
static void chunk_dram(void *ctx, int tid) {
  (void)ctx; long base = tid * g_slice_words; long off = g_pos[tid];
  SINK(K[g_kern].f(g_big + base + off, g_chunk_words));
  off += g_chunk_words; if (off + g_chunk_words > g_slice_words) off = 0; g_pos[tid] = off;
}
static void chunk_l2(void *ctx, int tid) { (void)ctx; SINK(K[g_kern].f(g_l2buf[tid], g_l2_words)); }

int main(int argc, char **argv) {
  prefer_p_cores();
  print_load("C7 start");
  g_big_words = (512l << 20) / 8; g_big = xalloc(g_big_words * 8);
  for (long i = 0; i < g_big_words; i++) g_big[i] = (uint64_t)i * 0x9e3779b97f4a7c15ull;
  // verify kernels against scalar on 64 KB (MOPA kernel: check against a scalar xnor-popcount model)
  { long n = 8192; uint64_t r = 0; for (long i = 0; i < n; i++) r += g_big[i];
    int bad = rd_neon(g_big, n) != r || rd_scalar(g_big, n) != r;
    uint64_t x = 0; for (long i = 0; i < n; i++) x ^= g_big[i];
    uint64_t xs = rd_ssve(g_big, n); uint64_t xfold = 0; { // rd_ssve returns sum of lanes of the xor-fold; recompute
      uint64_t lanes[8] = {0}; for (long i = 0; i < n; i++) lanes[i % 8] ^= g_big[i]; for (int l = 0; l < 8; l++) xfold += lanes[l]; }
    bad |= xs != xfold; (void)x;
    const uint32_t *u = (const uint32_t *)g_big; uint32_t want = 0;
    for (long i = 0; i < 2 * n; i += 64) want += (uint32_t)__builtin_popcount(~(u[i] ^ u[i + 32]));
    bad |= (uint32_t)rd_mopa(g_big, n) != want;
    printf("verify read kernels: %s\n\n", bad ? "FAIL" : "OK"); if (bad) return 1; }

  int mt_only = argc > 1 && !strcmp(argv[1], "mt");
  if (!mt_only) {
    printf("## Single thread, read-and-reduce, GB per CPU-second (min over reps; median in parentheses)\n\n| kernel | 64 KB (L1) | 1 MB (L2) | 4 MB (L2) | 512 MB (DRAM) | core |\n|---|---|---|---|---|---|\n");
    long sizes[] = { 64l << 10, 1l << 20, 4l << 20, 512l << 20 };
    for (int k = 0; k < 4; k++) {
      char cell[4][48]; const char *core = "?";
      for (int s = 0; s < 4; s++) {
        long words = sizes[s] / 8; long iters = (long)((256l << 20) / sizes[s]); if (iters < 1) iters = 1;
        int reps = s == 3 ? 5 : 11; stat2 st;
        const uint64_t *p = s == 3 ? g_big : g_big + (64l << 20) / 8; // distinct region for the cache-sized cases
        TIME_CPU(reps, iters, st, SINK(K[k].f(p, words)));
        snprintf(cell[s], 48, "%.1f (%.1f)", sizes[s] / st.min, sizes[s] / st.med); core = cl_name(g_min_cpu);
      }
      printf("| %s | %s | %s | %s | %s | %s |\n", K[k].n, cell[0], cell[1], cell[2], cell[3], core); fflush(stdout);
    }
    printf("\n");
  }
  if (argc > 1 && !strcmp(argv[1], "single")) { print_load("C7 end"); return 0; }

  // Multi-thread.
  int Ts[] = { 1, 6, 12, 18 };
  int kerns[] = { 1, 3 };
  for (int kk = 0; kk < 2; kk++) {
    g_kern = kerns[kk];
    for (int where = 0; where < 2; where++) {
      printf("### %s, %s, chunk %s per thread\n\n| threads | total CPU s | GB per CPU-s (all chunks) | per-thread GB/CPU-s by concurrency k=1,2,4,6,12,18 (k x rate = aggregate) | P core, by threads on its cluster 1..6 | M core, by threads on its cluster 1..6 |\n|---|---|---|---|---|---|\n",
             K[g_kern].n, where == 0 ? "DRAM: each thread streams its own slice of a 512 MB array" : "L2: each thread re-reads its own 1 MB buffer", where == 0 ? "2 MB" : "1 MB");
      for (int ti = 0; ti < 4; ti++) {
        int T = Ts[ti]; g_slice_words = g_big_words / T; g_slice_words -= g_slice_words % 1024;
        g_chunk_words = (2l << 20) / 8; g_l2_words = (1l << 20) / 8; memset(g_pos, 0, sizeof g_pos);
        if (where == 1) for (int t = 0; t < T; t++) { g_l2buf[t] = g_big + (long)t * (2l << 20) / 8; }
        chunk_summary cs = run_chunked(T, 1000000000ull, 100000, where == 0 ? chunk_dram : chunk_l2, NULL);
        double bytes = where == 0 ? g_chunk_words * 8.0 : g_l2_words * 8.0;
        char cells[256] = ""; int l = 0; int ks[] = { 1, 2, 4, 6, 12, 18 };
        for (int q = 0; q < 6; q++) { int k = ks[q]; if (k > T) break;
          l += snprintf(cells + l, sizeof cells - l, "%s", q ? " / " : "");
          l += cs.cnt[k] >= 10 ? snprintf(cells + l, sizeof cells - l, "%.1f", bytes / cs.med_ns[k]) : snprintf(cells + l, sizeof cells - l, "-"); }
        char sc[2][256] = { "", "" };
        for (int y = 0; y < 2; y++) { int m = 0; for (int q = 1; q <= 6 && q <= T; q++) {
          m += snprintf(sc[y] + m, sizeof sc[y] - m, "%s", q > 1 ? " / " : "");
          m += cs.same_cnt[y][q] >= 10 ? snprintf(sc[y] + m, sizeof sc[y] - m, "%.1f", bytes / cs.same_med[y][q]) : snprintf(sc[y] + m, sizeof sc[y] - m, "-"); } }
        printf("| %d | %.2f | %.1f | %s | %s | %s |\n", T, cs.total_cpu_ns / 1e9, bytes * cs.total_chunks / (double)cs.total_cpu_ns, cells, sc[1], sc[0]); fflush(stdout);
      }
      printf("\n");
    }
  }
  print_load("C7 end");
  return 0;
}
