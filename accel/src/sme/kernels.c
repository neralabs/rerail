// The SME lane's kernels: the bit product on BMOPA (decision 0098 §4), from the matrix-offload spike
// (tools/matrix-offload/cpu/c4_bitmatmul.c, w5_cooc.c).
//
// BMOPA accumulates XNOR popcounts: ZA[i][j] += popcount(~(a_i ^ b_j)) per 32-bit word. Over K words,
// with xnor the accumulated value,
//
//     popcount(a_i & b_j) = (pop(a_i) + pop(b_j) - 32K + xnor) / 2
//
// so each 32 x 32 output block is seeded with pop(a_i) - 32K down its rows (ADDVA) and pop(b_j) along
// its columns (ADDHA), accumulates K BMOPAs per tile, and is read out with a rounding shift right by
// one (exact: the sum is even). All arithmetic is mod 2^32 and the true value fits, so the wrap of
// the negative seed is harmless.
//
// Layout: rows are packed into panels of 32 rows, `P[(panel * K + w) * 32 + r]` = 32-bit word `w` of
// row `panel * 32 + r`; rows past the matrix are zero. Four ZA tiles hold a 32 x 32 block: tile 0
// rows 0-15 x cols 0-15, tile 1 rows 0-15 x cols 16-31, tile 2 rows 16-31 x cols 0-15, tile 3 rows
// 16-31 x cols 16-31.
//
// A worker enters streaming mode once per submission (`rr_sme_run`) and takes tiles of work from a
// shared counter until none are left, so a streaming block is as long as the worker's whole share.
//
// Built with -mcpu=apple-m4: -march=armv9.2-a makes the compiler emit SVE outside streaming mode,
// which faults on Apple silicon. No MATCH/HISTCNT/COMPACT/BDEP here: they fault in streaming mode
// (no FEAT_SME_FA64).
#include <arm_sme.h>
#include <pthread.h>
#include <stdint.h>
#include <sys/qos.h>

// What a tile of work writes.
enum { RR_COUNTS = 0, RR_MASK = 1 };

// A tile of work: A panels [ib0, ib1) against B panels [jb0, jb1) of one launch.
typedef struct {
  const uint32_t *ap;   // A panels
  const uint32_t *apop; // per padded A row: pop - 32 * k32 (mod 2^32)
  const uint32_t *bp;   // B panels
  const uint32_t *bpop; // per padded B row: pop
  const uint32_t *min;  // RR_MASK: per padded B row, the threshold (UINT32_MAX past the rows)
  uint32_t *out;        // RR_COUNTS: nrows x ldc counts; RR_MASK: padded rows x ldc 32-bit masks
  int64_t k32;          // 32-bit words per row
  int64_t ib0, ib1, jb0, jb1;
  int64_t nrows, ncols; // RR_COUNTS: the output's real size (stores past it are masked off)
  int64_t ldc;          // RR_COUNTS: ncols; RR_MASK: B panels
  int32_t mode;
  int32_t sym; // RR_COUNTS with A == B: only blocks jb >= ib are run, each also written transposed
} rr_sme_tile;

// One SME worker per cluster asks to run on the fastest cores it can (macOS has no affinity: the
// scheduler places it).
void rr_sme_worker_init(void) { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0); }

// Packs `nrows` rows of `words` u64 into panels (`panels`: ceil(nrows / 32) * 32 * 2 * words u32,
// zeroed by the caller), and each row's popcount into `pop` (the caller's padded rows stay 0).
// Rows [r0, r1) only, so callers can split the work.
void rr_sme_pack(const uint64_t *rows, int64_t r0, int64_t r1, int64_t words, uint32_t *panels, uint32_t *pop) {
  int64_t k32 = words * 2;
  for (int64_t i = r0; i < r1; i++) {
    const uint32_t *src = (const uint32_t *)(rows + i * words);
    uint32_t *dst = panels + (i / 32) * k32 * 32 + (i % 32);
    uint32_t s = 0;
    for (int64_t w = 0; w < k32; w++) {
      dst[w * 32] = src[w];
      s += (uint32_t)__builtin_popcount(src[w]);
    }
    pop[i] = s;
  }
}

#define AI static inline __attribute__((always_inline))

// One 32 x 32 block: seed, accumulate, leave it in ZA.
AI void block(const rr_sme_tile *t, int64_t ib, int64_t jb) __arm_streaming __arm_inout("za") {
  svbool_t p = svptrue_b32();
  svcount_t pc = svptrue_c32();
  const uint32_t *ap = t->ap + ib * t->k32 * 32, *bp = t->bp + jb * t->k32 * 32;
  svuint32_t ra0 = svld1_u32(p, t->apop + ib * 32), ra1 = svld1_u32(p, t->apop + ib * 32 + 16);
  svuint32_t cb0 = svld1_u32(p, t->bpop + jb * 32), cb1 = svld1_u32(p, t->bpop + jb * 32 + 16);
  svzero_za();
  svaddva_za32_u32_m(0, p, p, ra0);
  svaddva_za32_u32_m(1, p, p, ra0);
  svaddva_za32_u32_m(2, p, p, ra1);
  svaddva_za32_u32_m(3, p, p, ra1);
  svaddha_za32_u32_m(0, p, p, cb0);
  svaddha_za32_u32_m(1, p, p, cb1);
  svaddha_za32_u32_m(2, p, p, cb0);
  svaddha_za32_u32_m(3, p, p, cb1);
  for (int64_t w = 0; w < t->k32; w++) {
    svuint32x2_t x = svld1_u32_x2(pc, ap + w * 32), y = svld1_u32_x2(pc, bp + w * 32);
    svbmopa_za32_u32_m(0, p, p, svget2(x, 0), svget2(y, 0));
    svbmopa_za32_u32_m(1, p, p, svget2(x, 0), svget2(y, 1));
    svbmopa_za32_u32_m(2, p, p, svget2(x, 1), svget2(y, 0));
    svbmopa_za32_u32_m(3, p, p, svget2(x, 1), svget2(y, 1));
  }
}

// Stores four rows of a readout at output rows row..row+3, columns col..col+15, inside the limits.
#define STORE4(v, row, col)                                                                    \
  do {                                                                                         \
    svbool_t pcol = svwhilelt_b32_s64((col), t->ncols);                                        \
    if ((row) + 0 < t->nrows) svst1_u32(pcol, t->out + ((row) + 0) * t->ldc + (col), svget4(v, 0)); \
    if ((row) + 1 < t->nrows) svst1_u32(pcol, t->out + ((row) + 1) * t->ldc + (col), svget4(v, 1)); \
    if ((row) + 2 < t->nrows) svst1_u32(pcol, t->out + ((row) + 2) * t->ldc + (col), svget4(v, 2)); \
    if ((row) + 3 < t->nrows) svst1_u32(pcol, t->out + ((row) + 3) * t->ldc + (col), svget4(v, 3)); \
  } while (0)

// Counts out: tile T's rows (horizontal slices) at (i + RO, j + CO).
#define HOR(T, RO, CO)                                                              \
  for (int r = 0; r < 16; r += 4) {                                                 \
    svuint32x4_t v = svrshl_single_u32_x4(svread_hor_za32_u32_vg4(T, r), m1);       \
    STORE4(v, i + (RO) + r, j + (CO));                                              \
  }
// The transpose: tile T's columns (vertical slices) as rows, at (j + CO, i + RO).
#define VER(T, RO, CO)                                                              \
  for (int c = 0; c < 16; c += 4) {                                                 \
    svuint32x4_t v = svrshl_single_u32_x4(svread_ver_za32_u32_vg4(T, c), m1);       \
    STORE4(v, j + (CO) + c, i + (RO));                                              \
  }

AI void counts_out(const rr_sme_tile *t, int64_t ib, int64_t jb) __arm_streaming __arm_in("za") {
  svuint32_t m1 = svdup_n_u32(0xffffffffu); // shift by -1: a rounding shift right by one
  int64_t i = ib * 32, j = jb * 32;
  HOR(0, 0, 0) HOR(1, 0, 16) HOR(2, 16, 0) HOR(3, 16, 16)
  if (t->sym && jb != ib) {
    VER(0, 0, 0) VER(1, 0, 16) VER(2, 16, 0) VER(3, 16, 16)
  }
}

// Threshold out: per row of the block, a 32-bit mask of the columns whose count reaches min.
AI void mask_out(const rr_sme_tile *t, int64_t ib, int64_t jb) __arm_streaming __arm_in("za") {
  svbool_t p = svptrue_b32();
  svuint32_t m1 = svdup_n_u32(0xffffffffu);
  svuint32_t bit = svlsl_u32_x(p, svdup_n_u32(1), svindex_u32(0, 1));
  svuint32_t mn0 = svld1_u32(p, t->min + jb * 32), mn1 = svld1_u32(p, t->min + jb * 32 + 16);
  uint32_t *o = t->out + ib * 32 * t->ldc + jb;
#define MASKS(TL, TR, RO)                                                                        \
  for (int r = 0; r < 16; r += 4) {                                                              \
    svuint32x4_t l = svrshl_single_u32_x4(svread_hor_za32_u32_vg4(TL, r), m1);                   \
    svuint32x4_t h = svrshl_single_u32_x4(svread_hor_za32_u32_vg4(TR, r), m1);                   \
    o[((RO) + r + 0) * t->ldc] = svorv_u32(svcmpge_u32(p, svget4(l, 0), mn0), bit) |             \
                                 svorv_u32(svcmpge_u32(p, svget4(h, 0), mn1), bit) << 16;        \
    o[((RO) + r + 1) * t->ldc] = svorv_u32(svcmpge_u32(p, svget4(l, 1), mn0), bit) |             \
                                 svorv_u32(svcmpge_u32(p, svget4(h, 1), mn1), bit) << 16;        \
    o[((RO) + r + 2) * t->ldc] = svorv_u32(svcmpge_u32(p, svget4(l, 2), mn0), bit) |             \
                                 svorv_u32(svcmpge_u32(p, svget4(h, 2), mn1), bit) << 16;        \
    o[((RO) + r + 3) * t->ldc] = svorv_u32(svcmpge_u32(p, svget4(l, 3), mn0), bit) |             \
                                 svorv_u32(svcmpge_u32(p, svget4(h, 3), mn1), bit) << 16;        \
  }
  MASKS(0, 1, 0) MASKS(2, 3, 16)
#undef MASKS
}

// The pairs of rows [r0, r1) of a mask buffer (npb masks a row), in (i, j) order, as (i, j) u32 pairs
// at `out`; returns how many. `rr_sme_mask_count` gives the number first.
int64_t rr_sme_mask_count(const uint32_t *masks, int64_t n) {
  int64_t c = 0;
  for (int64_t k = 0; k < n; k++) c += __builtin_popcount(masks[k]);
  return c;
}
int64_t rr_sme_mask_pairs(const uint32_t *masks, int64_t r0, int64_t r1, int64_t npb, uint32_t *out) {
  int64_t c = 0;
  for (int64_t i = r0; i < r1; i++)
    for (int64_t jb = 0; jb < npb; jb++)
      for (uint32_t m = masks[i * npb + jb]; m; m &= m - 1) {
        out[2 * c] = (uint32_t)i;
        out[2 * c + 1] = (uint32_t)(jb * 32 + __builtin_ctz(m));
        c++;
      }
  return c;
}

// A worker's whole share of a submission, in one streaming block: tiles from `*next` until none
// are left. Returns the BMOPAs it issued.
__arm_locally_streaming __arm_new("za") int64_t rr_sme_run(const rr_sme_tile *tiles, int64_t ntiles, int64_t *next) {
  int64_t ops = 0;
  for (;;) {
    int64_t k = __atomic_fetch_add(next, 1, __ATOMIC_RELAXED);
    if (k >= ntiles) break;
    const rr_sme_tile *t = &tiles[k];
    for (int64_t jb = t->jb0; jb < t->jb1; jb++) {
      for (int64_t ib = t->ib0; ib < t->ib1; ib++) {
        if (t->sym && jb < ib) continue;
        block(t, ib, jb);
        if (t->mode == RR_MASK) mask_out(t, ib, jb);
        else counts_out(t, ib, jb);
        ops += 4 * t->k32;
      }
    }
  }
  return ops;
}
