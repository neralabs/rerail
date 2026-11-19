// W4. Kind histogram per document: kinds.bin is CSR (u32 n, u64 offsets[n+1], u16 kind per named
// node); 22,295,712 nodes, 115 kinds, 13,088 documents -> counts[13,088 x 115]. Every form is
// checked cell by cell against the direct form, and the sum must be 22,295,712.
// Forms: direct scalar increments (baseline form, C port); S sub-histograms (S = 2, 4, 8) that
// break the store-to-load dependency of repeated kinds, folded with NEON adds at document end.
#include "mo.h"

enum { NK = 115, NKP = 116 }; // padded to a multiple of 4
static long ND, NN; static uint64_t *Off; static uint16_t *Kd;
static uint32_t *H, *Href;

static void f_direct(void) {
  memset(H, 0, ND * NK * 4);
  for (long d = 0; d < ND; d++) { uint32_t *h = H + d * NK; for (uint64_t i = Off[d]; i < Off[d + 1]; i++) h[Kd[i]]++; }
}
static uint32_t sub[8][NKP] __attribute__((aligned(64)));
static void fold(int S, uint32_t *h) { // h[k] = sum_s sub[s][k]; clear sub
  for (int k = 0; k < NKP; k += 4) {
    uint32x4_t a = vld1q_u32(&sub[0][k]); vst1q_u32(&sub[0][k], vdupq_n_u32(0));
    for (int s = 1; s < S; s++) { a = vaddq_u32(a, vld1q_u32(&sub[s][k])); vst1q_u32(&sub[s][k], vdupq_n_u32(0)); }
    if (k + 4 <= NK) vst1q_u32(h + k, a); else { uint32_t t[4]; vst1q_u32(t, a); for (int q = 0; k + q < NK; q++) h[k + q] = t[q]; }
  }
}
#define SUBH(S) static void f_sub##S(void) { \
  for (long d = 0; d < ND; d++) { const uint16_t *k = Kd + Off[d]; long n = (long)(Off[d + 1] - Off[d]), i = 0; \
    for (; i + S <= n; i += S) { _Pragma("clang loop unroll(full)") for (int s = 0; s < S; s++) sub[s][k[i + s]]++; } \
    for (; i < n; i++) sub[0][k[i]]++; \
    fold(S, H + d * NK); } }
SUBH(2) SUBH(4) SUBH(8)

int main(int argc, char **argv) {
  init_data(argc, argv); prefer_p_cores();
  print_load("W4 start");
  size_t sz; uint8_t *b = load_file("kinds.bin", &sz);
  ND = *(uint32_t *)b; Off = xalloc((ND + 1) * 8); memcpy(Off, b + 4, (ND + 1) * 8); NN = (long)Off[ND];
  Kd = xalloc(NN * 2 + 64); memcpy(Kd, b + 4 + 8 * (ND + 1), NN * 2);
  uint16_t mx = 0; for (long i = 0; i < NN; i++) if (Kd[i] > mx) mx = Kd[i];
  printf("documents %ld, nodes %ld, max kind %u, input %.1f MB of u16 codes\n\n", ND, NN, mx, NN * 2 / 1e6);
  if (mx >= NK) { printf("kind out of range\n"); return 1; }
  H = xalloc(ND * NK * 4); Href = xalloc(ND * NK * 4);
  f_direct(); memcpy(Href, H, ND * NK * 4);
  uint64_t tot = 0; for (long i = 0; i < ND * NK; i++) tot += Href[i];
  if (tot != 22295712) { printf("sum %llu != expected\n", (unsigned long long)tot); return 1; }
  printf("| form | CPU s min | median | ns/node | GB/CPU-s of codes | sum + per-document check | core |\n|---|---|---|---|---|---|---|\n");
  stat2 st;
#define ROW(name, reps, fn) do { memset(H, 0xff, ND * NK * 4); fn(); int ok = !memcmp(H, Href, ND * NK * 4); TIME_CPU(reps, 1, st, fn()); ok &= !memcmp(H, Href, ND * NK * 4); \
    printf("| %s | %.4f | %.4f | %.3f | %.2f | %s | %s |\n", name, st.min / 1e9, st.med / 1e9, st.min / NN, NN * 2.0 / st.min, ok ? "OK" : "MISMATCH", cl_name(g_min_cpu)); fflush(stdout); } while (0)
  ROW("direct increments (baseline form, C port)", 15, f_direct);
  ROW("2 sub-histograms + NEON fold", 15, f_sub2);
  ROW("4 sub-histograms + NEON fold", 15, f_sub4);
  ROW("8 sub-histograms + NEON fold", 15, f_sub8);
  print_load("W4 end");
  return 0;
}
