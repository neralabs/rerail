// C5. Sorted unique u16 array intersection COUNT (Roaring array containers, <= 4096 values).
// NEON replacement for the x86 SSE4.2 PCMPESTRM kernel MLDB uses
// (mldb/builtin/intersection_utils.cc, intersectionCountOptimized): an 8x8 all-pairs u16 equality
// per step, popcount of the mask, advance whichever block has the smaller (or equal) last element.
//   (a) branch-free scalar merge (MLDB's non-x86 path)
//   (b) NEON 8x8 all-pairs: vceqq against the 8 rotations of the B block (vextq), OR, count
//   (b2) NEON 8x8 all-pairs: vceqq against the 8 lane broadcasts of the B block
//   (c) scalar galloping (exponential + binary search of each small element in the large array)
//   (c2) NEON block skip: per small element, skip 8-blocks of the large array by their last
//        element, then one vceqq against the block
#include "bench.h"
#include <arm_neon.h>

static int merge_scalar(const uint16_t *a, int na, const uint16_t *b, int nb) {
  int i = 0, j = 0, r = 0;
  while (i < na && j < nb) { int x = a[i], y = b[j]; r += x == y; i += x <= y; j += x >= y; }
  return r;
}

static inline int tail(const uint16_t *a, int i, int na, const uint16_t *b, int j, int nb, int r) {
  while (i < na && j < nb) { int x = a[i], y = b[j]; r += x == y; i += x <= y; j += x >= y; }
  return r;
}

static int neon_rot(const uint16_t *a, int na, const uint16_t *b, int nb) {
  int i = 0, j = 0; uint16x8_t acc = vdupq_n_u16(0);
  while (i + 8 <= na && j + 8 <= nb) {
    uint16x8_t va = vld1q_u16(a + i), vb = vld1q_u16(b + j);
    uint16x8_t m = vceqq_u16(va, vb);
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 1))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 2)));
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 3))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 4)));
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 5))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 6)));
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 7)));
    acc = vsubq_u16(acc, m); // each match lane is 0xffff = -1
    uint16_t la = a[i + 7], lb = b[j + 7];
    i += 8 * (la <= lb); j += 8 * (lb <= la);
  }
  return tail(a, i, na, b, j, nb, vaddvq_u16(acc));
}

// Two independent intersections interleaved in one loop: the 8x8 step is latency-bound (the next
// load address depends on the last-element compare), so a second chain fills the idle issue slots.
#define ROT_STEP(a, b, i, j, acc) do {                                                            \
    uint16x8_t va = vld1q_u16(a + i), vb = vld1q_u16(b + j);                                      \
    uint16x8_t m = vceqq_u16(va, vb);                                                             \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 1))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 2))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 3))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 4))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 5))); m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 6))); \
    m = vorrq_u16(m, vceqq_u16(va, vextq_u16(vb, vb, 7)));                                        \
    acc = vsubq_u16(acc, m);                                                                      \
    uint16_t la = a[i + 7], lb = b[j + 7]; i += 8 * (la <= lb); j += 8 * (lb <= la); } while (0)
static void neon_rot_x2(const uint16_t *a0, int na0, const uint16_t *b0, int nb0, const uint16_t *a1, int na1, const uint16_t *b1, int nb1, int *r0, int *r1) {
  int i0 = 0, j0 = 0, i1 = 0, j1 = 0; uint16x8_t c0 = vdupq_n_u16(0), c1 = vdupq_n_u16(0);
  while (i0 + 8 <= na0 && j0 + 8 <= nb0 && i1 + 8 <= na1 && j1 + 8 <= nb1) { ROT_STEP(a0, b0, i0, j0, c0); ROT_STEP(a1, b1, i1, j1, c1); }
  while (i0 + 8 <= na0 && j0 + 8 <= nb0) ROT_STEP(a0, b0, i0, j0, c0);
  while (i1 + 8 <= na1 && j1 + 8 <= nb1) ROT_STEP(a1, b1, i1, j1, c1);
  *r0 = tail(a0, i0, na0, b0, j0, nb0, vaddvq_u16(c0)); *r1 = tail(a1, i1, na1, b1, j1, nb1, vaddvq_u16(c1));
}

static int neon_bcast(const uint16_t *a, int na, const uint16_t *b, int nb) {
  int i = 0, j = 0; uint16x8_t acc = vdupq_n_u16(0);
  while (i + 8 <= na && j + 8 <= nb) {
    uint16x8_t va = vld1q_u16(a + i), vb = vld1q_u16(b + j);
    uint16x8_t m0 = vorrq_u16(vceqq_u16(va, vdupq_laneq_u16(vb, 0)), vceqq_u16(va, vdupq_laneq_u16(vb, 1)));
    uint16x8_t m1 = vorrq_u16(vceqq_u16(va, vdupq_laneq_u16(vb, 2)), vceqq_u16(va, vdupq_laneq_u16(vb, 3)));
    uint16x8_t m2 = vorrq_u16(vceqq_u16(va, vdupq_laneq_u16(vb, 4)), vceqq_u16(va, vdupq_laneq_u16(vb, 5)));
    uint16x8_t m3 = vorrq_u16(vceqq_u16(va, vdupq_laneq_u16(vb, 6)), vceqq_u16(va, vdupq_laneq_u16(vb, 7)));
    acc = vsubq_u16(acc, vorrq_u16(vorrq_u16(m0, m1), vorrq_u16(m2, m3)));
    uint16_t la = a[i + 7], lb = b[j + 7];
    i += 8 * (la <= lb); j += 8 * (lb <= la);
  }
  return tail(a, i, na, b, j, nb, vaddvq_u16(acc));
}

// small s (ns), large l (nl)
static int gallop(const uint16_t *s, int ns, const uint16_t *l, int nl) {
  int j = 0, r = 0;
  for (int i = 0; i < ns && j < nl; i++) {
    uint16_t x = s[i];
    if (l[j] < x) {
      int lo = j, step = 1; // invariant: l[lo] < x
      while (lo + step < nl && l[lo + step] < x) { lo += step; step <<= 1; }
      int hi = lo + step < nl ? lo + step : nl; // l[hi] >= x or hi == nl
      while (hi - lo > 1) { int mid = (lo + hi) >> 1; if (l[mid] < x) lo = mid; else hi = mid; }
      j = hi;
    }
    if (j < nl && l[j] == x) r++;
  }
  return r;
}

static int neon_skip(const uint16_t *s, int ns, const uint16_t *l, int nl) {
  int j = 0, r = 0, i = 0;
  for (; i < ns; i++) {
    uint16_t x = s[i];
    while (j + 8 <= nl && l[j + 7] < x) j += 8;
    if (j + 8 > nl) break;
    r -= (int16_t)vmaxvq_u16(vceqq_u16(vld1q_u16(l + j), vdupq_n_u16(x))) ; // 0xffff -> -1
  }
  return tail(s, i, ns, l, j, nl, r);
}

typedef int (*isect_fn)(const uint16_t *, int, const uint16_t *, int);

static void gen(uint16_t *out, int n, int U, uint64_t *seed, uint8_t *mark) {
  memset(mark, 0, 65536); int got = 0;
  while (got < n) { int v = (int)(rng_next(seed) % (uint64_t)U); if (!mark[v]) { mark[v] = 1; got++; } }
  int k = 0; for (int v = 0; v < U; v++) if (mark[v]) out[k++] = (uint16_t)v;
}

int main(void) {
  prefer_p_cores();
  print_load("C5 start");
  int Ls[] = { 16, 64, 256, 1024, 4096 }; int ratios[] = { 1, 10, 100 };
  enum { PAIRS = 32 };
  uint16_t *A[PAIRS], *Bv[PAIRS]; uint8_t *mark = xalloc(65536); uint64_t seed = 99;
  for (int p = 0; p < PAIRS; p++) { A[p] = xalloc(2 * 4096 + 64); Bv[p] = xalloc(2 * 4096 + 64); }
  struct { const char *name; isect_fn f; int skewed_only; } meth[] = {
    { "scalar merge (branch-free)", merge_scalar, 0 }, { "NEON 8x8 rotations", neon_rot, 0 }, { "NEON 8x8 broadcasts", neon_bcast, 0 },
    { "scalar galloping", gallop, 1 }, { "NEON block skip", neon_skip, 1 } };
  int NM = 5;
  printf("ns per element = CPU time / (|small| + |large|), min (median) over 9 repetitions; Melem/CPU-s from the min.\n\n");
  printf("| large | small | ratio | density | hits/small | merge | NEON rot | NEON bcast | NEON rot, 2 pairs interleaved | gallop | NEON skip | best Melem per CPU-s | core |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
  for (int li = 0; li < 5; li++) for (int ri = 0; ri < 3; ri++) for (int dense = 0; dense < 2; dense++) {
    int nl = Ls[li], nsml = nl / ratios[ri]; if (nsml < 4) continue;
    int U = dense ? 2 * nl : 65536; if (U > 65536) U = 65536;
    for (int p = 0; p < PAIRS; p++) { gen(Bv[p], nl, U, &seed, mark); gen(A[p], nsml, U, &seed, mark); }
    // reference via bitmap
    long refsum = 0; int refs[PAIRS];
    for (int p = 0; p < PAIRS; p++) { memset(mark, 0, 65536); for (int q = 0; q < nl; q++) mark[Bv[p][q]] = 1; int r = 0; for (int q = 0; q < nsml; q++) r += mark[A[p][q]]; refs[p] = r; refsum += r; }
    double elems = (double)PAIRS * (nl + nsml);
    long iters = (long)(4e6 / elems); if (iters < 1) iters = 1;
    char cells[6][48]; double best = 1e30;
    { // interleaved pairs
      int bad = 0; for (int p = 0; p < PAIRS; p += 2) { int r0, r1; neon_rot_x2(A[p], nsml, Bv[p], nl, Bv[p + 1], nl, A[p + 1], nsml, &r0, &r1); bad += (r0 != refs[p]) + (r1 != refs[p + 1]); }
      if (bad) snprintf(cells[5], 48, "VERIFY FAIL %d", bad);
      else { stat2 st; long acc = 0;
        TIME_CPU(9, iters, st, for (int p = 0; p < PAIRS; p += 2) { int r0, r1; neon_rot_x2(A[p], nsml, Bv[p], nl, Bv[p + 1], nl, A[p + 1], nsml, &r0, &r1); acc += r0 + r1; });
        SINK(acc); double nse = st.min / elems; if (nse < best) best = nse; snprintf(cells[5], 48, "%.3f (%.3f)", nse, st.med / elems); }
    }
    for (int mi = 0; mi < NM; mi++) {
      if (meth[mi].skewed_only && ratios[ri] == 1 && 0) { strcpy(cells[mi], "-"); continue; }
      // verify (both argument orders for the symmetric merges)
      int bad = 0;
      for (int p = 0; p < PAIRS; p++) {
        if (meth[mi].f(A[p], nsml, Bv[p], nl) != refs[p]) bad++;
        if (!meth[mi].skewed_only && meth[mi].f(Bv[p], nl, A[p], nsml) != refs[p]) bad++;
      }
      if (bad) { snprintf(cells[mi], 48, "VERIFY FAIL %d", bad); continue; }
      stat2 st; long acc = 0;
      TIME_CPU(9, iters, st, for (int p = 0; p < PAIRS; p++) acc += meth[mi].f(A[p], nsml, Bv[p], nl));
      SINK(acc);
      double nse = st.min / elems; if (nse < best) best = nse;
      snprintf(cells[mi], 48, "%.3f (%.3f)", nse, st.med / elems);
    }
    printf("| %d | %d | 1:%d | %s | %.2f | %s | %s | %s | %s | %s | %s | %.0f | %s |\n", nl, nsml, ratios[ri], dense ? "dense" : "sparse", (double)refsum / PAIRS / nsml,
           cells[0], cells[1], cells[2], cells[5], cells[3], cells[4], 1e3 / best, cl_name(g_min_cpu));
    fflush(stdout);
  }
  print_load("C5 end");
  return 0;
}
