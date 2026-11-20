// C1. Streaming-mode entry/exit cost (smstart/smstop, with and without ZA) and how many outer
// products per streaming block amortise it.
#include "bench.h"
#include <arm_sme.h>

__attribute__((noinline)) int plain_fn(int x) { __asm__ volatile("" ::: "memory"); return x + 1; }

// Compiler-generated entry: smstart sm ... smstop sm, plus spills of d8-d15 it must preserve.
__attribute__((noinline)) __arm_locally_streaming int sm_fn(int x) { __asm__ volatile("" ::: "memory"); return x + 1; }

// Streaming + fresh ZA: smstart (sm+za), zero za, smstop.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za") int smza_fn(int x) { svzero_za(); return x + 1; }

// ZA only (not streaming): smstart za, zero, smstop za.
__attribute__((noinline)) __arm_new("za") int za_fn(int x) { svzero_za(); return x + 1; }

// Raw instruction pairs, no compiler spills.
__attribute__((noinline)) void raw_sm(void) { __asm__ volatile("smstart sm\n smstop sm" ::: "memory", "z0", "z1", "z2", "z3", "z4", "z5", "z6", "z7", "z8", "z9", "z10", "z11", "z12", "z13", "z14", "z15", "z16", "z17", "z18", "z19", "z20", "z21", "z22", "z23", "z24", "z25", "z26", "z27", "z28", "z29", "z30", "z31"); }
__attribute__((noinline)) void raw_smza(void) { __asm__ volatile("smstart\n smstop" ::: "memory", "z0", "z1", "z2", "z3", "z4", "z5", "z6", "z7", "z8", "z9", "z10", "z11", "z12", "z13", "z14", "z15", "z16", "z17", "z18", "z19", "z20", "z21", "z22", "z23", "z24", "z25", "z26", "z27", "z28", "z29", "z30", "z31"); }

// A streaming block doing K fp32 outer products (round-robin over the 4 ZA tiles, operands in
// registers), then storing one row of tile 0 so the work is observable.
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
void fmopa_block(long K, const float *in, float *out) {
  svbool_t p = svptrue_b32();
  svfloat32_t a0 = svld1_f32(p, in), a1 = svld1_f32(p, in + 16), b0 = svld1_f32(p, in + 32), b1 = svld1_f32(p, in + 48);
  svzero_za();
  for (long i = 0; i < K; i += 4) {
    svmopa_za32_f32_m(0, p, p, a0, b0); svmopa_za32_f32_m(1, p, p, a0, b1);
    svmopa_za32_f32_m(2, p, p, a1, b0); svmopa_za32_f32_m(3, p, p, a1, b1);
  }
  svst1_hor_za32(0, 0, p, out);
}
// Same, BMOPA (u32 XNOR-popcount outer product).
__attribute__((noinline)) __arm_locally_streaming __arm_new("za")
void bmopa_block(long K, const uint32_t *in, uint32_t *out) {
  svbool_t p = svptrue_b32();
  svuint32_t a0 = svld1_u32(p, in), a1 = svld1_u32(p, in + 16), b0 = svld1_u32(p, in + 32), b1 = svld1_u32(p, in + 48);
  svzero_za();
  for (long i = 0; i < K; i += 4) {
    svbmopa_za32_u32_m(0, p, p, a0, b0); svbmopa_za32_u32_m(1, p, p, a0, b1);
    svbmopa_za32_u32_m(2, p, p, a1, b0); svbmopa_za32_u32_m(3, p, p, a1, b1);
  }
  svst1_hor_za32(0, 0, p, out);
}

int main(void) {
  prefer_p_cores();
  print_load("C1 start");
  stat2 s; long N = 200000; int R = 11; int acc = 0;
  printf("| entry/exit variant | ns/call min | ns/call median |\n|---|---|---|\n");
  TIME_CPU(R, N, s, acc = plain_fn(acc)); printf("| plain noinline call | %.2f | %.2f |\n", s.min, s.med);
  TIME_CPU(R, N, s, acc = sm_fn(acc)); printf("| __arm_locally_streaming call (smstart sm/smstop sm + spills) | %.2f | %.2f |\n", s.min, s.med);
  TIME_CPU(R, N, s, acc = smza_fn(acc)); printf("| locally_streaming + __arm_new(za) + zero za | %.2f | %.2f |\n", s.min, s.med);
  TIME_CPU(R, N, s, acc = za_fn(acc)); printf("| __arm_new(za) only (smstart za, not streaming) + zero za | %.2f | %.2f |\n", s.min, s.med);
  TIME_CPU(R, N, s, raw_sm()); printf("| raw asm smstart sm; smstop sm | %.2f | %.2f |\n", s.min, s.med);
  TIME_CPU(R, N, s, raw_smza()); printf("| raw asm smstart; smstop (sm+za) | %.2f | %.2f |\n", s.min, s.med);
  SINK(acc);

  float fin[64], fout[16]; uint32_t uin[64], uout[16];
  for (int i = 0; i < 64; i++) { fin[i] = 1.0f; uin[i] = 0x0f0f0f0fu * (unsigned)i; }
  // Verify block results: tile 0 row 0 = K/4 * a0[0]*b0[j] = K/4 for all-ones.
  fmopa_block(64, fin, fout);
  for (int j = 0; j < 16; j++) if (fout[j] != 16.0f) { printf("FMOPA verify FAIL %d %f\n", j, fout[j]); return 1; }
  bmopa_block(64, uin, uout);
  for (int j = 0; j < 16; j++) { uint32_t want = 16u * (uint32_t)__builtin_popcount(~(uin[0] ^ uin[32 + j])); if (uout[j] != want) { printf("BMOPA verify FAIL %d %u %u\n", j, uout[j], want); return 1; } }
  printf("\nverify: OK (FMOPA and BMOPA block outputs match scalar)\n\n");

  printf("| K outer products per block | FMOPA ns/block min | ns/MOPA | BMOPA ns/block min | ns/MOPA | BMOPA median ns/block |\n|---|---|---|---|---|---|\n");
  long Ks[] = { 4, 8, 16, 32, 64, 128, 256, 512, 1024, 4096, 16384, 65536 };
  for (int ki = 0; ki < (int)(sizeof Ks / sizeof *Ks); ki++) {
    long K = Ks[ki]; long it = 20000000 / (K + 64); if (it < 50) it = 50;
    stat2 sf, sb;
    TIME_CPU(R, it, sf, fmopa_block(K, fin, fout));
    TIME_CPU(R, it, sb, bmopa_block(K, uin, uout));
    printf("| %ld | %.1f | %.3f | %.1f | %.3f | %.1f |\n", K, sf.min, sf.min / K, sb.min, sb.min / K, sb.med);
  }
  SINK(fout[0]); SINK(uout[0]);
  print_load("C1 end");
  return 0;
}
