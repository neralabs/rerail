# 0571. A Metal kernel's barriered functions are dynamic libraries of their own

Status: built (audited 2026-11-23). Accepted (2026-11-21, the user, on per-function dylibs for pp's Metal kernel:
"yes"). The engine's Metal compile path (`accel/src/gpu/kernels.rs`) and UKL's source (`rerail_ukl::split`); every
backend's source is unchanged.

## Intro

`pp_setup`, the preprocessor's persistent grid (0468 §4), took 166 s to make its pipeline on the M3 Max (1.4 s of
front end, the rest the back end). The spikes (tmp/spike, the M3, 2026-11-21) found why:

- Metal's back end inlines a function with a threadgroup barrier at every call site, whatever its `noinline`, and
  inside functions that are themselves inlined. A function with a barrier called from 1, 4 and 16 sites took 98,
  493 and 2,961 ms; the same function four times larger, 0.53, 3.9 and 33.8 s. A plain function stayed at about
  95 ms whatever its sites.
- The size optimization level does not stop it, and it gave three "internal compiler error" failures on the real
  kernel.
- A dynamic library (`MTLDynamicLibrary`) is the one boundary the back end does not inline across. One a function,
  the same 16 sites took 85 ms once, then 6 ms.

The monolithic build also fails on Metal (0568's record): every 32-group run of the launch test, where the
per-function build of the same source passes.

## Decision

1. **A source with barriered `UKL_ONCE` functions is split** (`rerail_ukl::split::split`, over the front end's text,
   macros expanded):
   - **a unit a function**: the source with that function defined and exported (its `static noinline` dropped),
     every other unit's function an `extern` declaration, and no kernel (nor its `[[...]]` attributes);
   - **kept local**: a `UKL_ONCE` function with no threadgroup barrier in its reach (itself, its helpers and the
     functions it calls) and under `LOCAL_MOST` (8 KB) with its helpers stays in every unit that calls it, and in
     the kernel. A call into a dynamic library costs more than one within a unit, and the tape's small reads are
     called a thread an element: the spike's first split put them in units and ran 1.53x slower, the rule brought
     it to 1.19x. A function that calls a unit's is a unit itself;
   - **the kernel**: the source with every unit's function an `extern` declaration;
   - **levels**: a level's callees are all in lower levels; **`depth`** is the deepest chain of `UKL_ONCE` calls
     from a kernel.

   The scan knows C's comments, literals and line markers, and the top level: a name, its parameters, then a body
   or `;`.
2. **The units are compiled level by level, a level's at once** (one thread each). Each unit is a dynamic library
   with an absolute install name, linking the units it calls. Then the kernel's library is compiled from source,
   linking them all. The pipeline's `maxCallStackDepth` is the split's `depth`: Metal's default of 1 page-faults
   once a dynamic library calls another, and pp's calls nest 6 deep.
3. **The store keeps the units** where their install names point, `<store>/<device>/units/<split>/<name>.metallib`:
   - `<split>` is the digest of every unit's source and the kernel's, so a changed source, or a changed rule,
     is a new directory;
   - Metal keeps no dynamic library across processes. A later process loads each one, and compiles one again where
     it does not load (another driver's, damaged, pruned). Without a store they go to a temporary directory;
   - a use touches the file, for the store's pruning, as an archive's does.
4. **Not shipped**: `kernels ship` makes no AIR or archive of a split source. Its units compile in seconds where it
   runs.
5. **A pipeline the archive does not take** (one linking dynamic libraries) is the back end's compile alone, whose
   error, if it has one, is the kernel's. It is not archived: with its units loaded, the compile takes 33-39 ms on
   the M3 Max (the system's shader cache).
6. `RERAIL_METAL_UNITS=off` compiles a split source whole, as before. It is a measurement aid, beside
   `RERAIL_DIAG_MSL_NONCE`.
7. **A split source that declares the function constant** (`UKL_CONSTANT_U32`, Metal's function constant 0) is a
   library a value of the constant: the value is baked into its units and its kernel (`constant uint32_t name =
   value;`), and its functions are made with no constant values. A dynamic library holds no unresolved function
   constant ("cannot lower module with unresolved function constants"). The JIT's batch kernel is such a source
   (0576): its barriered step functions read the batch's tape format. The source's split is kept once, and each
   value's library is made from it (`kernels::library`). A source that does not split keeps one library, its
   constant given at the pipeline. The test is `kernels::tests::a_split_source_with_a_function_constant_is_a_library_a_value`.

pp's setup splits into 14 units in 6 levels:
- level 0: `ukl_grid_barrier`, `ukl_group_scan_n`, `ukl_group_seg_scan_n`, `px_r_work`;
- then the grid scans;
- then the sort and the `px_r_*` phases;
- last, `px_drain`.

Its 13 tape reads (`clif_tape_*`, `node_*`) stay local. The test is
`pp_arena::tests::the_setup_splits_one_unit_a_barriered_function`.

## Measured

The M3 Max, 2026-11-21, a debug build, the farm's load as it was:

| | Monolithic | Split |
|---|---|---|
| pp_setup's first compile (cold) | 166 s | 14.3 s: the whole launch test, its units compiled |
| A later process's kernels (warm) | the library's front end, and the pipeline from its archive | 0.17 s: preprocess 0.09 s, split 0.055 s, 14 loads and the kernel's library, then the pipeline 0.04 s |
| `pp_arena`'s three evaluation tests, warm | 1.78-1.93 s each | 0.33-0.46 s each |
| GPU time (the spike, the suite's 520 evaluations) | 1 | 1.19x (the median evaluation 1.2x), about the host's noise |

Results: the eval tests 32 of 32 under the 1 s profile on Metal, the `pp_arena` tests 10 of 10, and the launch test
(0568) passes on Metal. The split's first version scanned the source once a function, for its depth, in 0.37 s at
O0. It is one forward pass now.

## Found by the first Metal runs: the pool lost blocks under contention

`pp_arena::tests::the_pool_splits_a_larger_block` (0478 §10) had been checked on UKL's CPU backend only. On Metal it
failed in both builds: 4,021 and 3,891 takes met, with room for 7,239.

- **The defect.** `ukl_pool_push`, a split's give-back during takes, swapped the head in a loop bounded by
  `UKL_POOL_TURNS` (64), and dropped its piece when it lost them all. 8,192 takers on a few heads lost about half
  the pool.
- **The fix.** A give never fails now (`ukl/pool.h`):
  - the block's link is marked `UKL_POOL_LINKING`, then fenced;
  - an exchange makes the block the head, then the link is written;
  - a taker that reads the mark reads the head again, a bounded turn.

  No ABA follows: a piece's first unit is never a head twice in one take phase, as before. `ukl_pool_push` goes:
  `ukl_pool_give` is the one push.
- **The test's claim was more than the pool's contract.** "Every take met up to the room" does not hold where a take
  finds every class empty while others hold blocks mid-split. Its caller then takes from the store's front. What
  must hold is conservation: the takes met, plus the blocks of the class still in the pool (walked from its heads),
  equal the room, with no unit both taken and in the pool. Where the room exceeds the demand, every take is met.
  The test passes on Metal (3 of 3) and on the CPU backend.
