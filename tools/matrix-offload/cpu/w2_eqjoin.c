// W2. Within-document equality join count: per document, the number of (read, binding) pairs with
// equal local name id; expected total 83,066,745 (13,088 documents, 4.26 M reads, 1.53 M bindings,
// 4.14 G brute-force compares). Every form fills per-document counts, checked against the direct form.
// Forms: direct-indexed counts (baseline form, C port), brute-force all-pairs autovectorised
// (baseline form, C port), NEON brute-force tile (4 reads broadcast x 16 bindings per step), and SME
// BMOPA brute force: ids encoded as u32 0xFFFF0000|id so popcount(~(r^b)) == 32 iff r == b; one
// BMOPA scores 16 reads x 16 bindings; ZA is read out and thresholded after every 4 BMOPAs
// (equality is not linear in any outer product, so it cannot accumulate across BMOPAs).
#include "mo.h"

static long ND; static uint16_t *Rd, *Bd; static uint64_t *Ro, *Bo;
static uint64_t *ref, *got;
static const uint64_t EXPECT = 83066745;

static uint64_t f_direct(uint64_t *out) {
  static uint32_t cnt[1 << 16]; uint64_t tot = 0;
  for (long d = 0; d < ND; d++) {
    const uint16_t *b = Bd + Bo[d], *r = Rd + Ro[d]; long nb = Bo[d + 1] - Bo[d], nr = Ro[d + 1] - Ro[d]; uint64_t s = 0;
    for (long i = 0; i < nb; i++) cnt[b[i]]++;
    for (long i = 0; i < nr; i++) s += cnt[r[i]];
    for (long i = 0; i < nb; i++) cnt[b[i]] = 0;
    out[d] = s; tot += s;
  }
  return tot;
}
static uint64_t f_brute(uint64_t *out) {
  uint64_t tot = 0;
  for (long d = 0; d < ND; d++) {
    const uint16_t *b = Bd + Bo[d], *r = Rd + Ro[d]; long nb = Bo[d + 1] - Bo[d], nr = Ro[d + 1] - Ro[d]; uint64_t s = 0;
    for (long i = 0; i < nr; i++) { uint16_t x = r[i]; uint32_t c = 0; for (long j = 0; j < nb; j++) c += b[j] == x; s += c; }
    out[d] = s; tot += s;
  }
  return tot;
}

// Padded copies: reads to a multiple of 4 (pad 0xFFFE), bindings to a multiple of 16 (pad 0xFFFF);
// real ids are <= 2763, so pads never match.
static uint16_t *Rp, *Bp; static uint64_t *Rpo, *Bpo;
static uint64_t f_neon(uint64_t *out) {
  uint64_t tot = 0;
  for (long d = 0; d < ND; d++) {
    const uint16_t *b = Bp + Bpo[d], *r = Rp + Rpo[d]; long nb = Bpo[d + 1] - Bpo[d], nr = Rpo[d + 1] - Rpo[d]; uint64_t s = 0;
    for (long i = 0; i < nr; i += 4) {
      uint16x8_t x0 = vdupq_n_u16(r[i]), x1 = vdupq_n_u16(r[i + 1]), x2 = vdupq_n_u16(r[i + 2]), x3 = vdupq_n_u16(r[i + 3]);
      uint16x8_t a0 = vdupq_n_u16(0), a1 = a0, a2 = a0, a3 = a0; // lane counts <= nb/8 <= 1527: no overflow
      for (long j = 0; j < nb; j += 16) {
        uint16x8_t y0 = vld1q_u16(b + j), y1 = vld1q_u16(b + j + 8);
        a0 = vsubq_u16(a0, vceqq_u16(x0, y0)); a1 = vsubq_u16(a1, vceqq_u16(x1, y0)); a2 = vsubq_u16(a2, vceqq_u16(x2, y0)); a3 = vsubq_u16(a3, vceqq_u16(x3, y0));
        a0 = vsubq_u16(a0, vceqq_u16(x0, y1)); a1 = vsubq_u16(a1, vceqq_u16(x1, y1)); a2 = vsubq_u16(a2, vceqq_u16(x2, y1)); a3 = vsubq_u16(a3, vceqq_u16(x3, y1));
      }
      s += vaddlvq_u16(a0) + vaddlvq_u16(a1) + vaddlvq_u16(a2) + vaddlvq_u16(a3);
    }
    out[d] = s; tot += s;
  }
  return tot;
}

// SME: reads padded to 16 (enc 0xFFFFFFFE), bindings to 64 (enc 0xFFFFFFFF).
static uint32_t *Re, *Be; static uint64_t *Reo, *Beo;
__arm_locally_streaming __arm_new("za") static uint64_t f_sme(uint64_t *out) {
  svbool_t p = svptrue_b32(); uint64_t tot = 0;
  for (long d = 0; d < ND; d++) {
    const uint32_t *b = Be + Beo[d], *r = Re + Reo[d]; long nb = Beo[d + 1] - Beo[d], nr = Reo[d + 1] - Reo[d]; uint64_t s = 0;
    for (long i = 0; i < nr; i += 16) {
      svuint32_t x = svld1_u32(p, r + i);
      for (long j = 0; j < nb; j += 64) {
        svzero_za();
        svbmopa_za32_u32_m(0, p, p, x, svld1_u32(p, b + j)); svbmopa_za32_u32_m(1, p, p, x, svld1_u32(p, b + j + 16));
        svbmopa_za32_u32_m(2, p, p, x, svld1_u32(p, b + j + 32)); svbmopa_za32_u32_m(3, p, p, x, svld1_u32(p, b + j + 48));
#define EQ(t) for (int q = 0; q < 16; q += 4) { svuint32x4_t v = svread_hor_za32_u32_vg4(t, q); \
          s += svcntp_b32(p, svcmpeq_n_u32(p, svget4(v, 0), 32)) + svcntp_b32(p, svcmpeq_n_u32(p, svget4(v, 1), 32)) \
             + svcntp_b32(p, svcmpeq_n_u32(p, svget4(v, 2), 32)) + svcntp_b32(p, svcmpeq_n_u32(p, svget4(v, 3), 32)); }
        EQ(0) EQ(1) EQ(2) EQ(3)
#undef EQ
      }
    }
    out[d] = s; tot += s;
  }
  return tot;
}

static void pad16(const uint16_t *v, const uint64_t *o, long mult, uint16_t padv, uint16_t **pv, uint64_t **po) {
  *po = xalloc((ND + 1) * 8); (*po)[0] = 0;
  for (long d = 0; d < ND; d++) { long n = o[d + 1] - o[d]; (*po)[d + 1] = (*po)[d] + (n + mult - 1) / mult * mult; }
  *pv = xalloc((*po)[ND] * 2 + 64);
  for (long d = 0; d < ND; d++) { long n = o[d + 1] - o[d], m = (*po)[d + 1] - (*po)[d]; for (long i = 0; i < m; i++) (*pv)[(*po)[d] + i] = i < n ? v[o[d] + i] : padv; }
}
static void enc32(const uint16_t *v, const uint64_t *o, long mult, uint32_t padv, uint32_t **pv, uint64_t **po) {
  *po = xalloc((ND + 1) * 8); (*po)[0] = 0;
  for (long d = 0; d < ND; d++) { long n = o[d + 1] - o[d]; (*po)[d + 1] = (*po)[d] + (n + mult - 1) / mult * mult; }
  *pv = xalloc((*po)[ND] * 4 + 64);
  for (long d = 0; d < ND; d++) { long n = o[d + 1] - o[d], m = (*po)[d + 1] - (*po)[d]; for (long i = 0; i < m; i++) (*pv)[(*po)[d] + i] = i < n ? 0xFFFF0000u | v[o[d] + i] : padv; }
}

int main(int argc, char **argv) {
  init_data(argc, argv); prefer_p_cores();
  print_load("W2 start");
  size_t sr, sb, so1, so2;
  Rd = load_file("w2/reads.u16", &sr); Bd = load_file("w2/binds.u16", &sb); Ro = load_file("w2/reads_off.u64", &so1); Bo = load_file("w2/binds_off.u64", &so2);
  ND = (long)(so1 / 8) - 1;
  uint64_t work = 0; uint16_t mx = 0;
  for (long d = 0; d < ND; d++) work += (Ro[d + 1] - Ro[d]) * (Bo[d + 1] - Bo[d]);
  for (size_t i = 0; i < sr / 2; i++) if (Rd[i] > mx) mx = Rd[i];
  for (size_t i = 0; i < sb / 2; i++) if (Bd[i] > mx) mx = Bd[i];
  printf("documents %ld, reads %zu, bindings %zu, max local id %u, brute-force compares %llu; input %.1f MB\n", ND, sr / 2, sb / 2, mx, (unsigned long long)work, (sr + sb + so1 + so2) / 1e6);
  if (mx >= 0xFFFE) { printf("pad values collide\n"); return 1; }
  ref = xalloc(ND * 8); got = xalloc(ND * 8);
  uint64_t t0 = tcpu_ns(); pad16(Rd, Ro, 4, 0xFFFE, &Rp, &Rpo); pad16(Bd, Bo, 16, 0xFFFF, &Bp, &Bpo); double pad_ms = (tcpu_ns() - t0) / 1e6;
  t0 = tcpu_ns(); enc32(Rd, Ro, 16, 0xFFFFFFFEu, &Re, &Reo); enc32(Bd, Bo, 64, 0xFFFFFFFFu, &Be, &Beo); double enc_ms = (tcpu_ns() - t0) / 1e6;
  uint64_t padwork = 0, encwork = 0;
  for (long d = 0; d < ND; d++) { padwork += (Rpo[d + 1] - Rpo[d]) * (Bpo[d + 1] - Bpo[d]); encwork += (Reo[d + 1] - Reo[d]) * (Beo[d + 1] - Beo[d]); }
  printf("setup: NEON padding %.2f ms CPU (padded compares %llu); SME encoding %.2f ms CPU (padded compares %llu)\n\n", pad_ms, (unsigned long long)padwork, enc_ms, (unsigned long long)encwork);
  if (f_direct(ref) != EXPECT) { printf("direct form disagrees with expected\n"); return 1; }
  double inb = (double)(sr + sb);
  printf("| form | CPU s min | median | G compares/CPU-s (real pairs) | input GB/CPU-s | total | per-document check | core |\n|---|---|---|---|---|---|---|---|\n");
  stat2 st; uint64_t r;
#define ROW(name, reps, fn) do { memset(got, 0, ND * 8); r = fn(got); int ok = r == EXPECT && !memcmp(got, ref, ND * 8); \
    TIME_CPU(reps, 1, st, r = fn(got)); ok &= r == EXPECT; \
    printf("| %s | %.4f | %.4f | %.2f | %.2f | %llu | %s | %s |\n", name, st.min / 1e9, st.med / 1e9, work / st.min, inb / st.min, (unsigned long long)r, ok ? "OK" : "MISMATCH", cl_name(g_min_cpu)); fflush(stdout); } while (0)
  ROW("direct-indexed counts (baseline form, C port)", 15, f_direct);
  ROW("brute force, autovectorised (baseline form, C port)", 3, f_brute);
  ROW("NEON brute-force tile, 4 reads x 16 bindings", 3, f_neon);
  ROW("SME BMOPA brute force, threshold readout per 4 BMOPAs", 3, f_sme);
  print_load("W2 end");
  return 0;
}
