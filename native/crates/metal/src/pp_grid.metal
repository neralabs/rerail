// The preprocessor's evaluation as a persistent grid (decision 0460 §4): G groups of 256 threads,
// resident together, run the passes as phases, each group a share of each pass's items; a
// device-wide barrier (`ukl_grid_barrier`) separates a phase from the next, and the scans,
// segmented scans and sorts are the grid's own (`ukl/grid.h`). Every count a phase needs is read
// from the arena, so the host sizes nothing but the arena.
//
// Launches, the same for every unit:
// - `pp_setup`: the front passes, the definitions, the visibility and the top level, to the
//   queue's seed;
// - `px_expand`, twice: the queue's first phase, the top level's rollback and places, the
//   queue's second phase. A stage word lets the second launch take up where the first ended (a
//   queue whose workers all gave up waiting while work remained);
// - `pp_check`: the refusals.
//
// The kernels bind the arena (`arena`: every store at its offset, `lay` the offsets by slot,
// `S_*`), and the slab the unit's documents are in (`data`).

#define AT(T, s) ((device T*)(arena + lay[S_##s]))
#define C32(s) ((UKL_COHERENT uint32_t*)(arena + lay[S_##s]))
#define C64(s) ((UKL_COHERENT uint64_t*)(arena + lay[S_##s]))
#define W(s) lay[S_##s]
// Each of `n` items, a thread each, over the grid.
#define EACH(t, n) for (uint t = g.group * g.lsz + g.lid, t##_n = (n); t < t##_n; t += g.groups * g.lsz)
// One thread of the grid.
#define ONE(t) for (uint t = 0u; g.group == 0u && g.lid == 0u && t < 1u; t++)
// A measurement's stop (`STOP`): the kernel ends before its phase `stop` (0: never).
#define TICK() if (++phase == stop) return
#define BARRIER() TICK(); if (!ukl_grid_barrier(g, &tg.flag)) return
#define SCAN(n, in, out, total) TICK(); if (!ukl_grid_scan_n(g, (n), C32(in), C32(out), C32(total), C32(SUMS), tg.scratch, &tg.flag)) return
#define SEG(op, n, in, heads, out) TICK(); if (!ukl_grid_seg_scan_n(g, (op), (n), C32(in), C32(heads), C32(out), C32(SUMS), tg.big, tg.big + 256u, &tg.flag)) return
#define SORT(n, bits, keys, va, kb, vb, hist) TICK(); if (!ukl_grid_sort_u64(g, (n), (bits), true, C64(keys), C32(va), C64(kb), C32(vb), C32(hist), C32(SUMS), sl, tg.scratch, &tg.flag)) return
// A count the passes before wrote.
#define N(s) AT(const uint, s)[0]

// A group's threadgroup memory: the barrier's flag, the scans' scratch, and the sorts' counts or
// a sorted tile (the segmented scans' two rows alias them: no phase runs two of them).
constant uint PP_TG_BIG = UKL_GRID_SORT_TILE_WORDS > UKL_RADIX_GROUPS * UKL_RADIX_DIGITS + 3u * UKL_RADIX_DIGITS
                              ? UKL_GRID_SORT_TILE_WORDS : UKL_RADIX_GROUPS * UKL_RADIX_DIGITS + 3u * UKL_RADIX_DIGITS;
struct PpTg {
    uint flag;
    uint scratch[UKL_TG_SCRATCH];
    uint big[PP_TG_BIG];
};

// The stage word of `GRID` (after the barrier's three): 0 the queue's first phase, 1 its second,
// 2 done.
constant uint PG_STAGE = 3u;

#define PP_GRID_PARAMS                                                                                 \
    device uchar* arena [[buffer(0)]], device const uchar* data [[buffer(1)]],                         \
    device const uint* lay [[buffer(2)]], uint lid [[thread_position_in_threadgroup]],                 \
    uint group [[threadgroup_position_in_grid]], uint groups [[threadgroups_per_grid]],                \
    uint lsz [[threads_per_threadgroup]], uint lane [[thread_index_in_simdgroup]]

#define PP_GRID_BEGIN                                                                                  \
    threadgroup PpTg tg;                                                                               \
    UklGrid g;                                                                                         \
    g.ctl = AT(uint, GRID); g.groups = groups; g.group = group; g.lid = lid; g.lsz = lsz;               \
    UklGridSortLocal sl;                                                                               \
    sl.counts = tg.big; sl.h = tg.big + UKL_RADIX_GROUPS * UKL_RADIX_DIGITS;                           \
    sl.before = sl.h + UKL_RADIX_DIGITS; sl.base = sl.before + UKL_RADIX_DIGITS; sl.tile = tg.big;     \
    device const PpFile* files = AT(const PpFile, FILES);                                              \
    uint nf = W(N_FILES);                                                                              \
    uint stop = 0u, phase = 0u;                                                                        \
    (void)lane; (void)data; (void)files; (void)nf; (void)sl; (void)stop; (void)phase;

kernel void pp_setup(PP_GRID_PARAMS) {
    PP_GRID_BEGIN
    stop = W(STOP);
    if (ukl_grid_failed(g)) return;
    uint last = W(LAST);
    // --- The front: the nodes, the names, the directives, the guards -------------------------
    EACH(f, nf) {
        pp_counts(files, data, AT(uint, COUNTS), AT(atomic_uint, BAD), nf, f);
        pp_dir_counts(files, data, AT(uint, DIR_COUNTS), AT(atomic_uint, BAD), nf, f);
    }
    BARRIER();
    SCAN(nf, COUNTS, BASES, TOTAL);
    SCAN(nf, DIR_COUNTS, DIR_BASES, DIR_TOTAL);
    EACH(n, N(TOTAL)) {
        pp_names(files, data, AT(const uint, BASES), AT(const uint, TOTAL), AT(ulong, KEYS), nf, n);
        if (W(NODES_FLAG) != 0u) pp_nodes(files, data, AT(const uint, BASES), AT(const uint, TOTAL), AT(uint4, NODES_OUT), nf, n);
    }
    EACH(f, nf) pp_dirs(files, data, AT(const uint, DIR_BASES), AT(PpDir, ROWS), AT(uint, DIR_GROUPS), AT(uint, GUARDED), AT(atomic_uint, BAD), nf, f);
    BARRIER();
    EACH(r, N(DIR_TOTAL)) pp_dir_keys(files, data, AT(const uint, BASES), AT(const ulong, KEYS), AT(const uint, DIR_BASES), AT(const uint, DIR_TOTAL), AT(const PpDir, ROWS), AT(ulong, DIR_KEYS), nf, r);
    BARRIER();
    EACH(f, nf) pp_guards(AT(const uint, GUARDED), AT(const uint, DIR_BASES), AT(const ulong, DIR_KEYS), AT(ulong, GUARDS), nf, f);
    if (last == 1u) return;
    // --- The definitions --------------------------------------------------------------------
    EACH(r, N(DIR_TOTAL)) pd_defs(files, data, AT(const uint, BASES), AT(const uint, DIR_BASES), AT(const uint, DIR_TOTAL), AT(const PpDir, ROWS), AT(PpDef, DEFS), AT(uint, PCOUNT), AT(uint, BCOUNT), nf, r);
    BARRIER();
    SCAN(N(DIR_TOTAL), PCOUNT, PFIRST, PTOTAL);
    SCAN(N(DIR_TOTAL), BCOUNT, IFIRST, ITOTAL);
    EACH(s, N(PTOTAL)) pd_params(files, data, AT(const uint, BASES), AT(const ulong, KEYS), AT(const uint, DIR_BASES), AT(const uint, DIR_TOTAL), AT(PpDef, DEFS), AT(const uint, PFIRST), AT(const uint, PTOTAL), AT(ulong, PKEYS), AT(atomic_uint, ROOM), nf, W(VA_LO), W(VA_HI), W(CAP), s);
    BARRIER();
    EACH(s, N(ITOTAL)) pd_items(files, data, AT(const uint, BASES), AT(const ulong, KEYS), AT(const uint, DIR_BASES), AT(const uint, DIR_TOTAL), AT(PpDef, DEFS), AT(const uint, IFIRST), AT(const uint, ITOTAL), AT(const ulong, PKEYS), AT(uint, ITEMS), AT(atomic_uint, ROOM), nf, W(CAP), s);
    if (last == 2u) return;
    // --- The visibility: the text map, the rows, the stretches, the lookups ----------------
    EACH(n, N(TOTAL)) pe_owner_in(files, data, AT(const uint, BASES), AT(const uint, TOTAL), AT(uint, OWNER_IN), nf, n);
    BARRIER();
    SEG(UKL_SEG_MAX, N(TOTAL), OWNER_IN, VHEADS, OWNER);
    EACH(n, N(TOTAL)) pe_text_flag(files, data, AT(const uint, BASES), AT(const uint, TOTAL), AT(const uint, OWNER), AT(uint, TFLAG), nf, n);
    BARRIER();
    SCAN(N(TOTAL), TFLAG, TRANK, TTOTAL);
    EACH(n, N(TOTAL)) pe_text_nodes(AT(const uint, TFLAG), AT(const uint, TRANK), AT(const uint, TOTAL), AT(uint, TEXT_NODES), n);
    EACH(j, W(ND)) pe_row_flag(AT(const PpInst, INSTS), AT(const uint, N_INSTS), AT(const uint, DACTIVE), AT(const uint, N_DIRS), AT(const uint, DIR_BASES), AT(const PpDir, ROWS), AT(const PpDef, DEFS), AT(const ulong, DIR_KEYS), AT(uint, RFLAG), j);
    BARRIER();
    SCAN(N(N_DIRS), RFLAG, RAT, RTOTAL);
    EACH(j, W(ND)) pe_rows(AT(const PpInst, INSTS), AT(const uint, N_INSTS), AT(const uint, DSLOT), AT(const uint, N_DIRS), AT(const uint, DIR_BASES), AT(const PpDir, ROWS), AT(const ulong, DIR_KEYS), AT(const uint, RFLAG), AT(const uint, RAT), AT(ulong, SORT_KEYS), AT(ulong, NAME_KEYS), AT(uint, VALUES), W(N_BUILTIN), j);
    EACH(j, W(NG)) pe_gap_keys(AT(const uint, GSLOT), AT(const uint, N_GAPS), AT(ulong, GKEYS), j);
    ONE(i) pe_rows_total(AT(const uint, RTOTAL), AT(uint, N_ROWS), W(N_BUILTIN), i);
    BARRIER();
    SORT(N(N_ROWS), 64u, SORT_KEYS, SORT_VA, SORT_KB, SORT_VB, SORT_HIST);
    SORT(N(N_GAPS), 32u, GKEYS, GSORT_VA, GSORT_KB, GSORT_VB, GSORT_HIST);
    TICK();
    EACH(p, W(NG)) pe_gap_counts(AT(const PpInst, INSTS), AT(const uint, N_INSTS), AT(const uint, GACTIVE), AT(const uint, N_GAPS), AT(const uint, DIR_BASES), AT(const uint, DIR_TOTAL), AT(const PpDir, ROWS), AT(const uint, BASES), AT(const uint, TOTAL), AT(const uint, TRANK), AT(const uint, TTOTAL), AT(uint, GCOUNTS), AT(uint, GFIRSTS), AT(const uint, GSORT_VA), AT(const ulong, GKEYS), AT(uint, SSLOT), nf, p);
    TICK();
    BARRIER();
    SCAN(N(N_GAPS), GCOUNTS, GAT, GTOTAL);
    EACH(s, N(GTOTAL)) pe_lookups(files, data, AT(const uint, BASES), AT(const ulong, KEYS), AT(const uint, TEXT_NODES), AT(const uint, GAT), AT(const uint, N_GAPS), AT(const uint, GFIRSTS), AT(const uint, SSLOT), AT(const uint, GTOTAL), AT(const ulong, SORT_KEYS), AT(const uint, SORT_VA), AT(const uint, N_ROWS), AT(const ulong, NAME_KEYS), AT(const uint, VALUES), AT(const uint, FLOORS), W(N_UNITS), AT(uint, LOOKUPS), AT(atomic_uint, VBAD), nf, s);
    if (last == 3u) return;
    BARRIER();
    // --- The top level: depth, the close table, the calls, the outermost, the lists --------
    uint nt = N(GTOTAL);
    EACH(s, nt) px_top_parens(files, data, AT(const uint, BASES), AT(const uint, TEXT_NODES), AT(const uint, GAT), AT(const uint, N_GAPS), AT(const uint, GFIRSTS), AT(const uint, GTOTAL), AT(uint, THEADS), AT(uint, DELTA), AT(uint, CLOSE), nf, s);
    BARRIER();
    SEG(UKL_SEG_ADD, nt, DELTA, THEADS, DEPTH);
    SCAN(nt, CLOSE, CLOSE_AT, CLOSE_TOTAL);
    EACH(s, nt) px_close_keys(AT(const uint, DEPTH), AT(const uint, CLOSE), AT(const uint, CLOSE_AT), AT(const uint, GTOTAL), AT(ulong, CLOSE_KEYS), AT(atomic_uint, CTL), s);
    BARRIER();
    SORT(N(CLOSE_TOTAL), W(CLOSE_BITS), CLOSE_KEYS, CSORT_VA, CSORT_KB, CSORT_VB, CSORT_HIST);
    device const ulong* closes = ukl_grid_sort_in_input(W(CLOSE_BITS)) ? AT(const ulong, CLOSE_KEYS) : AT(const ulong, CSORT_KB);
    TICK();
    EACH(s, nt) px_top_calls(files, data, AT(const uint, BASES), AT(const PpDef, DEFS), AT(const uint, LOOKUPS), AT(const uint, TEXT_NODES), AT(const uint, GAT), AT(const uint, N_GAPS), AT(const uint, GFIRSTS), AT(const uint, GTOTAL), AT(const uint, DEPTH), closes, AT(const uint, CLOSE_TOTAL), AT(uint, CEND), AT(atomic_uint, CTL), nf, s);
    BARRIER();
    SEG(UKL_SEG_MAX, nt, CEND, EHEADS, REACH);
    EACH(s, nt) px_top_outer(AT(const uint, CEND), AT(const uint, REACH), AT(const uint, GTOTAL), AT(uint, STATE), AT(uint, OFLAG), s);
    BARRIER();
    SCAN(nt, OFLAG, OID, OTOTAL);
    EACH(s, nt) px_top_lists(AT(const uint, LOOKUPS), AT(const uint, CEND), AT(const uint, STATE), AT(const uint, OID), AT(const uint, GTOTAL), AT(const uint, SSLOT), AT(const uint, GAT), AT(const uint, N_GAPS), AT(PxList, LISTS), AT(atomic_uint, SLOTS), AT(atomic_uint, CTL), AT(const PpDef, DEFS), W(LIST_CAP), s);
    ONE(i) px_seed(AT(const uint, OTOTAL), AT(atomic_uint, QCTL), AT(atomic_uint, CTL), W(LIST_CAP), i);
}

// The queue's workers: each SIMD group of the grid one, until the queue is idle.
#define PX_WORK()                                                                                      \
    px_worker(files, data, AT(const uint, BASES), AT(const ulong, KEYS), AT(const PpDef, DEFS), AT(const uint, ITEMS), \
              AT(const ulong, SORT_KEYS), AT(const uint, SORT_VA), AT(const uint, N_ROWS), AT(const ulong, NAME_KEYS), \
              AT(const uint, VALUES), AT(const uint, TEXT_NODES), AT(const uint, GAT), AT(const uint, N_GAPS),  \
              AT(const uint, GFIRSTS), AT(const uint, GTOTAL), AT(const uint, DEPTH), AT(PxList, LISTS),          \
              AT(uint, TOKENS), AT(PxArg, ARGS), AT(uint, OUT), AT(atomic_uint, SLOTS), AT(atomic_uint, QCTL),    \
              AT(atomic_uint, CTL), AT(const uint, WORDS), AT(PxScratch, SCRATCH), closes, AT(const uint, CLOSE_TOTAL), AT(const uint, FLOORS), W(N_UNITS), lane)

kernel void px_expand(PP_GRID_PARAMS) {
    PP_GRID_BEGIN
    if (ukl_grid_failed(g)) return;
    device atomic_uint* stage = (device atomic_uint*)(AT(uint, GRID) + PG_STAGE);
    device atomic_uint* qctl = AT(atomic_uint, QCTL);
    device const ulong* closes = ukl_grid_sort_in_input(W(CLOSE_BITS)) ? AT(const ulong, CLOSE_KEYS) : AT(const ulong, CSORT_KB);
    uint nt = N(GTOTAL);
    uint at = atomic_load_explicit(stage, memory_order_relaxed);
    if (at == 0u) {
        PX_WORK();
        BARRIER();
        // Work remains (its workers gave up waiting): the next launch takes it up. Every group
        // reads the same count: no worker runs after the barrier.
        if (atomic_load_explicit(&qctl[2], memory_order_relaxed) != 0u) return;
        // --- The top level's rollback, its sizes and places --------------------------------
        EACH(s, nt) px_top_xflag(AT(const uint, STATE), AT(const uint, OID), AT(const PxList, LISTS), AT(const uint, GTOTAL), AT(uint, XFLAG), s);
        BARRIER();
        SCAN(nt, XFLAG, XAT, XTOTAL);
        EACH(s, nt) px_top_xlist(AT(const uint, XFLAG), AT(const uint, XAT), AT(const uint, GTOTAL), AT(uint, XPOS), s);
        BARRIER();
        if (g.group == 0u) px_top_cover(AT(const uint, XPOS), AT(const uint, XTOTAL), AT(const uint, OID), AT(PxList, LISTS), AT(uint, XCOVER), AT(atomic_uint, CTL), g.lid, lane);
        BARRIER();
        EACH(s, nt) px_top_sizes(AT(uint, STATE), AT(const uint, OID), AT(PxList, LISTS), AT(const uint, GTOTAL), AT(uint, V), AT(const uint, XPOS), AT(const uint, XTOTAL), AT(const uint, XCOVER), s);
        BARRIER();
        SCAN(nt, V, OBASE, VTOTAL);
        EACH(s, nt) px_top_place(AT(const uint, STATE), AT(const uint, OID), AT(PxList, LISTS), AT(const uint, GTOTAL), AT(const uint, OBASE), AT(const uint, TEXT_NODES), AT(const uint, GAT), AT(const uint, N_GAPS), AT(const uint, GFIRSTS), AT(uint, OUT), AT(atomic_uint, SLOTS), qctl, AT(atomic_uint, CTL), W(OUT_CAP), W(Q_CAP), s);
        BARRIER();
        if (g.group == 0u && g.lid == 0u) atomic_store_explicit(stage, 1u, memory_order_relaxed);
    } else if (at != 1u) {
        return;
    }
    // --- The queue's second phase: the places, to the output -----------------------------------
    PX_WORK();
    BARRIER();
    if (g.group == 0u && g.lid == 0u && atomic_load_explicit(&qctl[2], memory_order_relaxed) == 0u) atomic_store_explicit(stage, 2u, memory_order_relaxed);
}

// The refusals, one thread: the queue's (work left, or it overflowed), the output's room, and a
// failed barrier (the grid was not resident together: the host runs it again with fewer groups).
kernel void pp_check(device uchar* arena [[buffer(0)]], device const uint* lay [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    device atomic_uint* ctl = AT(atomic_uint, CTL);
    device const uint* grid = AT(const uint, GRID);
    if (grid[2] != 0u) px_refuse(ctl, X_BARRIER);
    else px_check(AT(const atomic_uint, QCTL), AT(const uint, VTOTAL), ctl, W(OUT_CAP), 0u);
    if (grid[2] == 0u && grid[PG_STAGE] != 2u) px_refuse(ctl, X_QUEUE);
}
