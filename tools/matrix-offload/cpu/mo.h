// Shared pieces for the real-workload probes (w1..w5): data loading and the SME/NEON bit-matrix
// building blocks (panel packing, popcount seeds).
#pragma once
#include "bench.h"
#include <arm_neon.h>
#include <arm_sme.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>

static const char *g_data = NULL; // data dir (argv[1] or $MO_DATA)
static void *load_file(const char *rel, size_t *size) {
  char p[2048]; snprintf(p, sizeof p, "%s/%s", g_data, rel);
  int fd = open(p, O_RDONLY); if (fd < 0) { perror(p); exit(1); }
  struct stat st; fstat(fd, &st); void *b = xalloc((size_t)st.st_size + 64);
  size_t got = 0; while (got < (size_t)st.st_size) { ssize_t r = read(fd, (char *)b + got, (size_t)st.st_size - got); if (r <= 0) { perror("read"); exit(1); } got += (size_t)r; }
  close(fd); if (size) *size = (size_t)st.st_size; return b;
}
static void init_data(int argc, char **argv) {
  g_data = argc > 1 ? argv[1] : getenv("MO_DATA");
  if (!g_data) { fprintf(stderr, "usage: %s DATA_DIR (or set MO_DATA)\n", argv[0]); exit(2); }
}

// Panels: rows padded to a multiple of 32; P[(pb*K32 + w)*32 + r] = u32 word w of row pb*32+r.
static uint32_t *pack_panels(const uint64_t *rows, long nrows, long words64, long *npanels) {
  long np = (nrows + 31) / 32, K32 = words64 * 2;
  uint32_t *P = xalloc((size_t)np * K32 * 32 * 4); memset(P, 0, (size_t)np * K32 * 32 * 4);
  for (long i = 0; i < nrows; i++) { const uint32_t *src = (const uint32_t *)(rows + i * words64); long pb = i / 32, r = i % 32;
    for (long w = 0; w < K32; w++) P[(pb * K32 + w) * 32 + r] = src[w]; }
  *npanels = np; return P;
}
static uint32_t *row_pops(const uint64_t *rows, long nrows, long words64, long padded, int64_t minus) {
  uint32_t *p = xalloc((size_t)padded * 4); memset(p, 0, (size_t)padded * 4);
  for (long i = 0; i < nrows; i++) { uint32_t s = 0; for (long w = 0; w < words64; w++) s += (uint32_t)__builtin_popcountll(rows[i * words64 + w]); p[i] = s; }
  for (long i = 0; i < padded; i++) p[i] = (uint32_t)((int64_t)p[i] - minus);
  return p;
}
