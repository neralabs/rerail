// Shared timing helpers for the CPU matrix-offload probes.
//
// Primary measure is CPU time (thread CPU clock or getrusage user+sys), never wall time:
// the machine is heavily loaded, so wall time mostly measures the scheduler.
#pragma once
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <pthread.h>
#include <sys/resource.h>
#include <sys/qos.h>
#include <mach/mach_time.h>

// Thread CPU time in ns.
static inline uint64_t tcpu_ns(void) { return clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID); }
// Wall clock in ns (only for chunk overlap bookkeeping, never as the reported measure).
static inline uint64_t wall_ns(void) { return clock_gettime_nsec_np(CLOCK_UPTIME_RAW); }
// Process CPU time (user+sys, all threads) in ns.
static inline uint64_t pcpu_ns(void) {
  struct rusage r; getrusage(RUSAGE_SELF, &r);
  return (uint64_t)(r.ru_utime.tv_sec + r.ru_stime.tv_sec) * 1000000000ull +
         (uint64_t)(r.ru_utime.tv_usec + r.ru_stime.tv_usec) * 1000ull;
}
// Logical CPU number (XNU keeps it in the low bits of TPIDR_EL0 on arm64).
// On this M5 Max: 0-5 = M cluster 0, 6-11 = M cluster 1, 12-17 = P ("Super") cluster 2.
static inline unsigned cpu_id(void) { uint64_t p; __asm__ volatile("mrs %0, TPIDR_EL0" : "=r"(p)); return (unsigned)(p & 0xfff); }
static inline unsigned cluster_of(unsigned cpu) { return cpu / 6; }

static inline void prefer_p_cores(void) { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0); }

// Clock estimate: a dependent chain of 1-cycle adds, best of 15 short runs (on whatever core).
static double clock_ghz(unsigned *cpu_out) {
  double best = 1e30; unsigned bc = 0;
  for (int r = 0; r < 15; r++) {
    unsigned c0 = (unsigned)({ uint64_t p; __asm__ volatile("mrs %0, TPIDR_EL0" : "=r"(p)); p & 0xfff; });
    uint64_t t0 = clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID), x = 0;
    for (long i = 0; i < (1 << 20); i += 8)
      __asm__ volatile("add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1\n add %0,%0,#1" : "+r"(x));
    double ns = (double)(clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID) - t0);
    if (ns < best) { best = ns; bc = c0; }
  }
  if (cpu_out) *cpu_out = bc; return (1 << 20) / best;
}
static void print_load(const char *tag) {
  double l[3]; getloadavg(l, 3); unsigned c; double g = clock_ghz(&c);
  printf("# %s load average: %.1f %.1f %.1f; clock estimate %.2f GHz (dependent adds, cpu %u = %s core)\n", tag, l[0], l[1], l[2], g, c, c >= 12 ? "P" : "M");
}

static int cmp_u64(const void *a, const void *b) { uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b; return x < y ? -1 : x > y; }
static int cmp_dbl(const void *a, const void *b) { double x = *(const double *)a, y = *(const double *)b; return x < y ? -1 : x > y; }
typedef struct { double min, med; } stat2;
static stat2 stats_d(double *v, int n) { qsort(v, n, sizeof *v, cmp_dbl); stat2 s = { v[0], v[n / 2] }; return s; }

// Deterministic RNG.
static inline uint64_t rng_next(uint64_t *s) { uint64_t z = (*s += 0x9e3779b97f4a7c15ull); z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull; z = (z ^ (z >> 27)) * 0x94d049bb133111ebull; return z ^ (z >> 31); }

static void *xalloc(size_t n) { void *p = NULL; if (posix_memalign(&p, 128, n ? n : 1)) { perror("alloc"); exit(1); } return p; }

// Prevent the optimizer from discarding a value.
static volatile uint64_t g_sink;
#define SINK(x) (g_sink += (uint64_t)(x))

// Measure a body: run `reps` repetitions, each timed on the thread CPU clock, iterating the body
// `iters` times per repetition. Returns min/median ns per iteration. g_min_cpu records the logical
// CPU the fastest repetition ran on (cluster 2 = P/"Super" cores, 0-1 = M cores).
static int g_min_cpu = -1;
static const char *cl_name(int cpu) { return cpu < 0 ? "?" : cpu >= 12 ? "P" : "M"; }
#define TIME_CPU(reps, iters, out_stat, ...) do { \
  double _v[64]; int _r = (reps) > 64 ? 64 : (reps); double _best = 1e300; \
  for (int _k = 0; _k < _r; _k++) { unsigned _c0 = cpu_id(); uint64_t _t0 = tcpu_ns(); \
    for (long _i = 0; _i < (long)(iters); _i++) { __VA_ARGS__; } \
    _v[_k] = (double)(tcpu_ns() - _t0) / (double)(iters); \
    if (_v[_k] < _best && cpu_id() == _c0) { _best = _v[_k]; g_min_cpu = (int)_c0; } } \
  (out_stat) = stats_d(_v, _r); } while (0)

// ---------------------------------------------------------------------------------------------
// Chunked multi-thread runner with concurrency attribution.
//
// Under heavy load, threads of one run are rarely all on-CPU at once, so "T threads" does not mean
// "T concurrent". Each worker repeats a fixed chunk of work and records (wall start, wall end, CPU
// start, CPU end, cpu id). A chunk is "clean" if it ran on-CPU for >=95% of its wall span. For each
// clean chunk we count the other threads whose clean chunks cover >=90% of its span: that is the
// observed concurrency. Chunk duration (CPU) is then grouped by concurrency, and by how many of the
// concurrent threads were on the same cluster.
typedef struct { uint64_t w0, w1, c0, c1; uint16_t cpu0, cpu1; } chunk_rec;
typedef struct {
  int tid, nthreads; long max_chunks; long n; chunk_rec *rec;
  uint64_t wall_budget_ns;
  void (*work)(void *ctx, int tid); void *ctx;
  volatile int *go;
} chunk_thread;

static void *chunk_worker(void *arg) {
  chunk_thread *t = arg; prefer_p_cores();
  while (!*t->go) { }
  uint64_t start = wall_ns(); long i = 0;
  while (i < t->max_chunks && wall_ns() - start < t->wall_budget_ns) {
    chunk_rec *r = &t->rec[i];
    r->cpu0 = (uint16_t)cpu_id(); r->w0 = wall_ns(); r->c0 = tcpu_ns();
    t->work(t->ctx, t->tid);
    r->c1 = tcpu_ns(); r->w1 = wall_ns(); r->cpu1 = (uint16_t)cpu_id();
    i++;
  }
  t->n = i; return NULL;
}

static int clean(const chunk_rec *r) { uint64_t w = r->w1 - r->w0, c = r->c1 - r->c0; return r->cpu0 == r->cpu1 && c * 100 >= w * 95; }

// Fraction of [a,b] covered by clean chunks of thread u (chunks sorted by time). Also returns the cpu.
static double covered(const chunk_thread *u, uint64_t a, uint64_t b, int *cpu_out) {
  long lo = 0, hi = u->n; // first chunk with w1 > a
  while (lo < hi) { long mid = (lo + hi) / 2; if (u->rec[mid].w1 <= a) lo = mid + 1; else hi = mid; }
  uint64_t cov = 0; *cpu_out = -1;
  for (long i = lo; i < u->n && u->rec[i].w0 < b; i++) {
    const chunk_rec *r = &u->rec[i]; if (!clean(r)) continue;
    uint64_t s = r->w0 > a ? r->w0 : a, e = r->w1 < b ? r->w1 : b;
    if (e > s) { cov += e - s; *cpu_out = r->cpu0; }
  }
  return (double)cov / (double)(b - a);
}

// same_*[t][q]: chunks on core type t (0 = M, 1 = P) with q threads (itself included) on its cluster.
typedef struct { long n_clean; double med_ns[33], min_ns[33]; long cnt[33]; double same_med[2][33]; long same_cnt[2][33]; uint64_t total_cpu_ns; long total_chunks; } chunk_summary;

// Runs `nthreads` workers for wall_budget_ns, then summarises. Concurrency index k = 1 + others.
static chunk_summary run_chunked(int nthreads, uint64_t wall_budget_ns, long max_chunks,
                                 void (*work)(void *, int), void *ctx) {
  pthread_t th[64]; chunk_thread ct[64]; volatile int go = 0;
  for (int i = 0; i < nthreads; i++) {
    ct[i] = (chunk_thread){ i, nthreads, max_chunks, 0, xalloc(sizeof(chunk_rec) * max_chunks), wall_budget_ns, work, ctx, &go };
    pthread_create(&th[i], NULL, chunk_worker, &ct[i]);
  }
  go = 1;
  for (int i = 0; i < nthreads; i++) pthread_join(th[i], NULL);
  chunk_summary s; memset(&s, 0, sizeof s);
  // Per concurrency level: collect durations.
  long cap = 0; for (int i = 0; i < nthreads; i++) cap += ct[i].n;
  double *buf[33]; long nb[33] = {0}; double *sbuf[2][33]; long ns[2][33] = {{0}};
  for (int k = 0; k < 33; k++) { buf[k] = xalloc(sizeof(double) * (cap + 1)); for (int y = 0; y < 2; y++) sbuf[y][k] = xalloc(sizeof(double) * (cap + 1)); }
  for (int t = 0; t < nthreads; t++) for (long i = 0; i < ct[t].n; i++) {
    const chunk_rec *r = &ct[t].rec[i]; s.total_cpu_ns += r->c1 - r->c0; s.total_chunks++;
    if (!clean(r)) continue; s.n_clean++;
    int k = 1, same = 1;
    for (int u = 0; u < nthreads; u++) { if (u == t) continue; int cpu;
      if (covered(&ct[u], r->w0, r->w1, &cpu) >= 0.9) { k++; if (cpu >= 0 && cluster_of((unsigned)cpu) == cluster_of(r->cpu0)) same++; } }
    if (k > 32) k = 32;
    double d = (double)(r->c1 - r->c0);
    buf[k][nb[k]++] = d;
    if (same > 32) same = 32;
    int ty = r->cpu0 >= 12; sbuf[ty][same][ns[ty][same]++] = d;
  }
  for (int k = 0; k < 33; k++) {
    s.cnt[k] = nb[k]; if (nb[k]) { stat2 st = stats_d(buf[k], (int)nb[k]); s.med_ns[k] = st.med; s.min_ns[k] = st.min; }
    for (int y = 0; y < 2; y++) { s.same_cnt[y][k] = ns[y][k]; if (ns[y][k]) { stat2 st = stats_d(sbuf[y][k], (int)ns[y][k]); s.same_med[y][k] = st.med; } free(sbuf[y][k]); }
    free(buf[k]);
  }
  for (int i = 0; i < nthreads; i++) free(ct[i].rec);
  return s;
}

// As TIME_CPU, but split repetitions by the core type they ran on (a repetition that migrated is
// dropped). P = "Super" cores (cluster 2), M = the two 6-core "Performance" clusters.
typedef struct { stat2 p, m; int np, nm; } stat_cl;
static stat_cl stats_cl(double *vp, int np, double *vm, int nm) {
  stat_cl s; memset(&s, 0, sizeof s); s.np = np; s.nm = nm;
  if (np) s.p = stats_d(vp, np); if (nm) s.m = stats_d(vm, nm); return s;
}
#define TIME_CPU_CL(reps, iters, out_stat, ...) do { \
  double _vp[256], _vm[256]; int _np = 0, _nm = 0; int _r = (reps) > 256 ? 256 : (reps); \
  for (int _k = 0; _k < _r; _k++) { unsigned _c0 = cpu_id(); uint64_t _t0 = tcpu_ns(); \
    for (long _i = 0; _i < (long)(iters); _i++) { __VA_ARGS__; } \
    double _d = (double)(tcpu_ns() - _t0) / (double)(iters); unsigned _c1 = cpu_id(); \
    if (_c0 != _c1) continue; if (_c0 >= 12) _vp[_np++] = _d; else _vm[_nm++] = _d; } \
  (out_stat) = stats_cl(_vp, _np, _vm, _nm); } while (0)
// Format "min/med" per core type into buf.
static const char *fmt_cl(char *buf, stat_cl s, double scale, const char *f) {
  char a[64] = "-", b[64] = "-";
  if (s.np) snprintf(a, sizeof a, f, scale / s.p.min); if (s.nm) snprintf(b, sizeof b, f, scale / s.m.min);
  sprintf(buf, "%s | %s", a, b); return buf;
}
