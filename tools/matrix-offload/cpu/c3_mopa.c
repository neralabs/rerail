// C3. Outer-product throughput by type (all tiles, operands in registers), single thread split by
// core type, then thread scaling with concurrency attribution (see run_chunked in bench.h).
#include "bench.h"
#include <arm_sme.h>
#include <arm_neon.h>
#include <sys/sysctl.h>

// Kernel generator: K outer products round-robin over the 4 za32 tiles (2x2 operand reuse).
#define MOPA4(name, VT, LD, OP, ST)                                                              \
  __attribute__((noinline)) __arm_locally_streaming __arm_new("za")                              \
  static void name(long K, const void *in, void *out) {                                          \
    svbool_t p = svptrue_b8();                                                                    \
    VT a0 = LD(p, (const void *)((const char *)in)), a1 = LD(p, (const void *)((const char *)in + 64)); \
    VT b0 = LD(p, (const void *)((const char *)in + 128)), b1 = LD(p, (const void *)((const char *)in + 192)); \
    svzero_za();                                                                                  \
    for (long i = 0; i < K; i += 4) { OP(0, p, p, a0, b0); OP(1, p, p, a0, b1); OP(2, p, p, a1, b0); OP(3, p, p, a1, b1); } \
    svst1_hor_za32(0, 0, svptrue_b32(), out);                                                     \
  }
#define LD_F32(p, q) svld1_f32(p, (const float *)(q))
#define LD_F16(p, q) svld1_f16(p, (const float16_t *)(q))
#define LD_BF16(p, q) svld1_bf16(p, (const bfloat16_t *)(q))
#define LD_S8(p, q) svld1_s8(p, (const int8_t *)(q))
#define LD_U8(p, q) svld1_u8(p, (const uint8_t *)(q))
#define LD_S16(p, q) svld1_s16(p, (const int16_t *)(q))
#define LD_U32(p, q) svld1_u32(p, (const uint32_t *)(q))
MOPA4(k_fmopa_f32, svfloat32_t, LD_F32, svmopa_za32_f32_m, 0)
MOPA4(k_fmopa_f16, svfloat16_t, LD_F16, svmopa_za32_f16_m, 0)
MOPA4(k_bfmopa, svbfloat16_t, LD_BF16, svmopa_za32_bf16_m, 0)
MOPA4(k_smopa_s8, svint8_t, LD_S8, svmopa_za32_s8_m, 0)
MOPA4(k_umopa_u8, svuint8_t, LD_U8, svmopa_za32_u8_m, 0)
MOPA4(k_smopa_s16, svint16_t, LD_S16, svmopa_za32_s16_m, 0)
MOPA4(k_bmopa, svuint32_t, LD_U32, svbmopa_za32_u32_m, 0)

// za64 has 8 tiles; operands 4 A x 2 B.
#define MOPA8(name, VT, LD, OP)                                                                  \
  __attribute__((noinline)) __arm_locally_streaming __arm_new("za")                              \
  static void name(long K, const void *in, void *out) {                                          \
    svbool_t p = svptrue_b8(); const char *c = in;                                               \
    VT a0 = LD(p, c), a1 = LD(p, c + 64), a2 = LD(p, c + 128), a3 = LD(p, c + 192), b0 = LD(p, c + 256), b1 = LD(p, c + 320); \
    svzero_za();                                                                                  \
    for (long i = 0; i < K; i += 8) { OP(0, p, p, a0, b0); OP(1, p, p, a0, b1); OP(2, p, p, a1, b0); OP(3, p, p, a1, b1); \
                                      OP(4, p, p, a2, b0); OP(5, p, p, a2, b1); OP(6, p, p, a3, b0); OP(7, p, p, a3, b1); } \
    svst1_hor_za64(0, 0, svptrue_b64(), out);                                                     \
  }
#define LD_F64(p, q) svld1_f64(p, (const double *)(q))
MOPA8(k_fmopa_f64, svfloat64_t, LD_F64, svmopa_za64_f64_m)
MOPA8(k_smopa_s16_64, svint16_t, LD_S16, svmopa_za64_s16_m)

// NEON control: 12 independent fp32 FMLA chains (4 lanes x 2 flops each).
__attribute__((noinline)) static void k_neon_fmla(long K, const void *in, void *out) {
  const float *f = in; float32x4_t x = vld1q_f32(f), y = vld1q_f32(f + 4);
  float32x4_t a[12]; for (int i = 0; i < 12; i++) a[i] = vdupq_n_f32((float)i);
  for (long i = 0; i < K; i += 12) {
#pragma clang loop unroll(full)
    for (int j = 0; j < 12; j++) a[j] = vfmaq_f32(a[j], x, y);
  }
  float32x4_t s = a[0]; for (int i = 1; i < 12; i++) s = vaddq_f32(s, a[i]); vst1q_f32(out, s);
}

// Dependent scalar add chain: 1 cycle per add -> clock estimate.
__attribute__((noinline)) static uint64_t k_addchain(long n) {
  uint64_t x = 0; for (long i = 0; i < n; i += 8)
    __asm__ volatile("add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1" : "+r"(x));
  return x;
}

static uint8_t g_in[512] __attribute__((aligned(64)));
static float g_f32[128] __attribute__((aligned(64)));
static __fp16 g_f16[256] __attribute__((aligned(64)));
static __bf16 g_bf16[256] __attribute__((aligned(64)));
static double g_f64[64] __attribute__((aligned(64)));
typedef struct { const char *name; void (*fn)(long, const void *, void *); double ops; const char *unit; const void *in; } kern;
static kern kerns[] = {
  { "FMOPA fp32 (za32, 4 tiles)", k_fmopa_f32, 16 * 16 * 2, "GFLOP" , g_f32 },
  { "FMOPA fp16->fp32 widening (4 tiles)", k_fmopa_f16, 16 * 16 * 2 * 2, "GFLOP" , g_f16 },
  { "BFMOPA bf16->fp32 (4 tiles)", k_bfmopa, 16 * 16 * 2 * 2, "GFLOP" , g_bf16 },
  { "SMOPA int8->int32 (4 tiles)", k_smopa_s8, 16 * 16 * 4 * 2, "GOP" , g_in },
  { "UMOPA uint8->int32 (4 tiles)", k_umopa_u8, 16 * 16 * 4 * 2, "GOP" , g_in },
  { "SMOPA int16->int32 (SME2, 4 tiles)", k_smopa_s16, 16 * 16 * 2 * 2, "GOP" , g_in },
  { "BMOPA u32 xnor-popcount (4 tiles)", k_bmopa, 16 * 16 * 32, "Gbit-op" , g_in },
  { "FMOPA fp64 (za64, 8 tiles)", k_fmopa_f64, 8 * 8 * 2, "GFLOP" , g_f64 },
  { "SMOPA int16->int64 (za64, 8 tiles)", k_smopa_s16_64, 8 * 8 * 4 * 2, "GOP" , g_in },
  { "NEON FMLA fp32 control (12 chains)", k_neon_fmla, 4 * 2, "GFLOP" , g_f32 },
};

typedef struct { kern *k; long K; } mt_ctx;
static void mt_work(void *ctx, int tid) { mt_ctx *c = ctx; uint8_t out[128]; c->k->fn(c->K, c->k->in, out); SINK(out[0]); (void)tid; }

static int sysint(const char *n) { int v = 0; size_t l = sizeof v; sysctlbyname(n, &v, &l, NULL, 0); return v; }

int main(int argc, char **argv) {
  prefer_p_cores();
  print_load("C3 start");
  printf("cores: perflevel0 (%s-type 'Super') physical=%d, perflevel1 ('Performance') physical=%d, ncpu=%d; cpus per L2: %d / %d\n",
         "P", sysint("hw.perflevel0.physicalcpu"), sysint("hw.perflevel1.physicalcpu"), sysint("hw.ncpu"), sysint("hw.perflevel0.cpusperl2"), sysint("hw.perflevel1.cpusperl2"));
  printf("logical CPUs 0-5 = M cluster 0, 6-11 = M cluster 1, 12-17 = P cluster 2 (IORegistry cluster-type)\n\n");
  // Inputs: small values so integer/float accumulations stay finite; verify each kernel's tile row
  // against scalar for a short K.
  for (int i = 0; i < 512; i++) g_in[i] = (uint8_t)(i * 7 + 1) & 0x3f;
  for (int i = 0; i < 128; i++) g_f32[i] = 1.0f + i * 1e-3f;
  for (int i = 0; i < 256; i++) { g_f16[i] = (__fp16)(1.0f + i * 1e-3f); g_bf16[i] = (__bf16)(1.0f + i * 1e-3f); }
  for (int i = 0; i < 64; i++) g_f64[i] = 1.0 + i * 1e-3;
  stat_cl s; char b1[128];
  double tc;
  { TIME_CPU_CL(31, 1, s, SINK(k_addchain(1 << 22)));
    printf("clock estimate (dependent adds): P %.2f GHz, M %.2f GHz (reps on P/M: %d/%d)\n\n",
           s.np ? (1 << 22) / s.p.min : 0, s.nm ? (1 << 22) / s.m.min : 0, s.np, s.nm); tc = s.np ? (1 << 22) / s.p.min : 0; }
  (void)tc;

  // Verification: int8 SMOPA and BMOPA row 0 against scalar.
  { int32_t out[16]; k_smopa_s8(8, g_in, out); const int8_t *a0 = (const int8_t *)g_in, *b0 = (const int8_t *)g_in + 128;
    int bad = 0; for (int j = 0; j < 16; j++) { int32_t w = 0; for (int q = 0; q < 4; q++) w += a0[q] * b0[4 * j + q]; if (out[j] != 2 * w) bad++; }
    uint32_t uo[16]; k_bmopa(8, g_in, uo); const uint32_t *ua = (const uint32_t *)g_in, *ub = (const uint32_t *)(g_in + 128);
    for (int j = 0; j < 16; j++) if (uo[j] != 2u * (uint32_t)__builtin_popcount(~(ua[0] ^ ub[j]))) bad++;
    float fo[16]; k_fmopa_f32(8, g_f32, fo); const float *fa = g_f32, *fb = g_f32 + 32;
    for (int j = 0; j < 16; j++) if (fo[j] != 2.0f * fa[0] * fb[j]) bad++;
    printf("verify SMOPA int8 / BMOPA / FMOPA fp32 tile 0 row 0 vs scalar: %s\n\n", bad ? "FAIL" : "OK"); if (bad) return 1; }

  int only_single = argc > 1 && !strcmp(argv[1], "single");
  int only_mt = argc > 1 && !strcmp(argv[1], "mt");
  if (!only_mt) {
    printf("## Single thread, operands in registers (K = 65536 per streaming block)\n\n");
    printf("| instruction | ops/instr | ns/instr P | ns/instr M | G-ops per CPU-s P | M |\n|---|---|---|---|---|---|\n");
    for (int i = 0; i < (int)(sizeof kerns / sizeof *kerns); i++) {
      long K = 65536 - 65536 % 24; uint8_t out[128];
      TIME_CPU_CL(41, 4, s, kerns[i].fn(K, kerns[i].in, out)); SINK(out[0]);
      char p[32] = "-", m[32] = "-";
      if (s.np) snprintf(p, sizeof p, "%.3f", s.p.min / K); if (s.nm) snprintf(m, sizeof m, "%.3f", s.m.min / K);
      printf("| %s | %.0f | %s | %s | %s %s |\n", kerns[i].name, kerns[i].ops, p, m, fmt_cl(b1, s, kerns[i].ops * K, "%.0f"), kerns[i].unit);
    }
    printf("\n");
  }
  if (only_single) { print_load("C3 end"); return 0; }

  // Thread scaling. Chunk = one streaming block of K instructions.
  int sel[] = { 0, 1, 3, 6, 9 }; int threads[] = { 1, 2, 4, 6, 12, 18 };
  printf("## Thread scaling (chunk = one streaming block; wall budget 1.2 s per point)\n\n");
  printf("Per-thread rate when k threads were observed on-CPU at once (median chunk CPU time), and the\n");
  printf("rate split by how many of those concurrent threads shared the chunk's cluster.\n\n");
  for (int si = 0; si < (int)(sizeof sel / sizeof *sel); si++) {
    kern *k = &kerns[sel[si]]; mt_ctx c = { k, k->fn == k_neon_fmla ? 24 * 2000 : 32768 };
    printf("### %s (G-ops per CPU-s per thread)\n\n| threads | total CPU s | aggregate G-ops/CPU-s | per-thread by concurrency k=1..6 | P core, by threads on its cluster 1..6 | M core, by threads on its cluster 1..6 |\n|---|---|---|---|---|---|\n", k->name);
    for (int ti = 0; ti < (int)(sizeof threads / sizeof *threads); ti++) {
      int T = threads[ti];
      chunk_summary cs = run_chunked(T, 1200000000ull, 200000, mt_work, &c);
      double opc = k->ops * c.K;
      char conc[256] = "", same[2][256] = { "", "" }; int lc = 0, ls[2] = { 0, 0 };
      for (int q = 1; q <= 6 && q <= T; q++) {
        lc += snprintf(conc + lc, sizeof conc - lc, "%s", q > 1 ? " / " : "");
        lc += cs.cnt[q] >= 20 ? snprintf(conc + lc, sizeof conc - lc, "%.0f", opc / cs.med_ns[q]) : snprintf(conc + lc, sizeof conc - lc, "-");
        for (int y = 0; y < 2; y++) {
          ls[y] += snprintf(same[y] + ls[y], sizeof same[y] - ls[y], "%s", q > 1 ? " / " : "");
          ls[y] += cs.same_cnt[y][q] >= 20 ? snprintf(same[y] + ls[y], sizeof same[y] - ls[y], "%.0f", opc / cs.same_med[y][q]) : snprintf(same[y] + ls[y], sizeof same[y] - ls[y], "-");
        }
      }
      printf("| %d | %.2f | %.0f | %s | %s | %s |\n", T, cs.total_cpu_ns / 1e9, opc * cs.total_chunks / (double)cs.total_cpu_ns, conc, same[1], same[0]);
      fflush(stdout);
    }
    printf("\n");
  }
  print_load("C3 end");
  return 0;
}
