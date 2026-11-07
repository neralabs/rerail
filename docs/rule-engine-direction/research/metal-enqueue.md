# Device-side enqueue on Metal for unbounded programs, without a CPU/JIT fallback

This extends `metal-recursion.md`.

- **Experiment sources and output:** `../experiments/metal-enqueue/`. `run1.txt` is the full run; `run-c2.txt` is the ICB visibility test.
- **Build and run:** `xcrun swiftc -O bench.swift -o bench`, then `C2SPLIT=1 ./bench <tape> kernels.metal`. The tape comes from `../experiments/metal-recursion/gen.py`.
- **Machine:** Apple M5 Max (40 GPU cores), macOS 26.4.1. Shaders are compiled at run time.
- **Checking:** every result is compared with a CPU BFS reference. All reported rows match unless noted.

## 1. Conclusion

Metal has no CUDA-style dynamic parallelism: a kernel cannot launch a kernel. These three facilities are enough instead:

1. **GPU-written indirect dispatch arguments.** A kernel sizes the next dispatch; a size of 0 makes it a no-op.
2. **GPU-encoded indirect command buffer (ICB) commands.** A kernel writes a whole compute command (pipeline, buffers, grid) into a pre-allocated slot, or resets it to a no-op. The slot executes in a *later* `executeCommandsInBuffer` call.
3. **A single-threadgroup persistent loop.** Forward progress is guaranteed within one threadgroup through `threadgroup_barrier`. Across threadgroups it is not: the grid-wide persistent queue failed in `metal-recursion.md`.

**Model: hybrid waves scheduled by the GPU.**
- While the frontier is wide, device-wide waves run, each dispatch sized by a one-thread GPU scheduler kernel (`prep`).
- At ≤ 1,024 frontier nodes, the GPU hands off to one 1,024-thread threadgroup, which runs all the remaining waves in a single dispatch.
- The CPU encodes slots and submits. It never makes a per-wave decision and never executes rule logic.

| Driver | Cost per wave |
|---|---|
| CPU, one command buffer per wave | 386–890 µs |
| Pre-encoded indirect dispatch | 5.9–7.9 µs |
| GPU-encoded ICB | 18–30 µs |
| Threadgroup loop | 1.13–1.18 µs |
| Hybrid | 1.2–1.6 µs, and the fastest on wide CPython |

A 100,000-wave fixpoint took 113 ms GPU in one command buffer. The wide CPython fixpoint (5.4M nodes, 27 waves) took 1.57 ms GPU with the hybrid.

## 2. What Metal offers for GPU-driven work generation

| Facility | What it does | Verified here |
|---|---|---|
| `dispatchThreadgroups(indirectBuffer:)` | Grid read from a kernel-written buffer; `(0,1,1)` is a no-op | Yes. 200,000 dispatches in one command buffer. An empty slot costs about 3.5 µs (CPython: 29 vs 1,029 slots, 1.61 vs 5.07 ms) |
| ICB compute commands | CPU-encoded (`MTLIndirectComputeCommand`), or GPU-encoded through MSL `command_buffer`/`compute_command` in an argument buffer: `set_compute_pipeline_state`, `set_kernel_buffer`, `concurrent_dispatch_threadgroups`, `set_barrier`, `reset`. Pipelines need `supportIndirectCommandBuffers` | Yes. `prep_icb` picks the pipeline, binds 9 buffers and sizes the grid |
| `executeCommandsInBuffer(range:)` | Executes a CPU-chosen range of slots | Yes |
| `executeCommandsInBuffer(indirectBuffer:)` | GPU-written range | Not needed; a reset slot gives the same early exit |
| GPU write to an ICB slot during the execute call that is running | **Not visible**: commands are fetched first | Measured. Ranges of 0 (one call), 2 and 8 all failed at 1/2,000 waves. Ranges of 1 with a barrier on the ICB were correct at 2,000/2,000 |
| ICB capacity | `maxCommandCount` | 200,000 commands allocated and run |
| Command buffer capacity | Dispatches per buffer | 200,000 dispatches (100,000 waves) worked. Encoding costs about 2 µs per wave on the CPU. A pre-encoded ICB is reusable across submissions (103 ms for 200,000 slots) |
| Grid-wide barrier or persistent grid | None. No forward progress across threadgroups or SIMD groups | A spinning queue starved (`metal-recursion.md`) |
| `threadgroup_barrier` loop in one threadgroup | Guaranteed progress | Yes: (e), and the tail of (f) |
| Ordering between dispatches | A `.serial` encoder orders dispatches and makes writes visible. ICB commands need `set_barrier()` and `memoryBarrier(resources:[icb])` before a GPU-written slot is executed | Used throughout; all results match |
| Metal 4 | Command allocators, argument tables, explicit barriers. To the author's knowledge, no GPU kernel launch | Not verified |
| Long dispatches | The watchdog can kill command buffers that run for seconds on a GPU driving a display | Longest dispatch 113 ms; loops are bounded by `limit` |

## 3. Design

**Admissibility.** The set stage is stratified Datalog over finite node sets. The extensional database is the tape columns (`kind`, `size`, derived `par`).

- **Within a stratum:** positive recursion only. It terminates in ≤ |nodes in scope| waves, because each node enters the frontier at most once.
- **Between strata:** negation and counting.
- **Scope:** any axes, either direction, any depth. Only unstratifiable programs are rejected. This is wider than `metal-recursion.md`'s rule of monotone recursion in one direction.

**Per stratum:**

1. **Seed kernel.** Computes the base relation, marks the seeds, and writes frontier F0 and its count.
2. **Wave slot k.** CPU-encoded, in a `.serial` encoder:
   - `expand` is an indirect dispatch sized by the previous `prep`. Each frontier node visits its axis neighbours (parent, children via `c += size[c]`, siblings), tests the rule, marks the result atomically and appends it.
   - `prep` is one thread. It reads the produced count, consumes the source frontier, and writes the next `expand` arguments.
   - When the count is 0, `prep` sets `done`.
   - When the count is ≤ H, `prep` sets the handoff mode and writes `(1,1,1)` to the loop's arguments.
   - In both of those cases the remaining slots become no-ops, at about 3.5 µs each.
3. **Loop slot.** `tg_loop` is one indirectly dispatched 1,024-thread threadgroup. It alternates frontiers with device-scope barriers until the frontier is empty or `limit` is reached, then continues in the next segment.
4. **Strata** run in order in the same command buffer. Their number is fixed by the program.

**When a wave must choose a kernel** (a fused-group variant or an axis kernel), `prep_icb` encodes ICB slot k or resets it, and a CPU-encoded `executeCommandsInBuffer(icb, k..<k+1)` executes it.
- That costs about 18–20 µs per wave, against 6–7 µs for plain indirect dispatch.
- Use it only when the choice of kernel really depends on the data.
- Each slot needs its own execute call, because of the visibility limit in §2.

**Waves per submission:**

- **Narrow, deep recursion:** one loop dispatch. 100,000 waves took 113 ms.
- **Wide waves:** W pre-encoded slots per stratum, with W taken from statistics (maximum depth or diameter of the scoped documents). CPython needed 27.
- **Live frontier at the end of a command buffer:**
  - the submitter appends another chunk of slots in a new command buffer (variant d);
  - frontier, counts and marks stay resident, and the CPU sees only "not done";
  - cost: about 6.4 µs per wave with chunks of 1,024, correct across up to 1,564 sequential command buffers.

**Memory:**

- **Marks:** 1 bit per node with `atomic_fetch_or`. The bench used u32.
- **Frontiers:** 2 × 4 B × nodes worst case. Each node enters once per stratum, so frontiers cannot overflow.
- **Outputs:** exact, via count → scan → scatter.
- **Live sets:** a posting list (4 B per element) or a bitmap (n/8 bytes), chosen from the statistical bound. A posting list converts to a bitmap on the GPU above n/32 elements.
- **Arena:** computed exactly at plan time from document node counts.
- **Batches:** cut so the arena fits the budget, for example 50% of `recommendedMaxWorkingSetSize`. Splitting a batch by documents is always correct.
- **Scale:** a 1M-node document needs about 9 MB.
- **Chunking:** programs local to a subtree can be chunked along preorder ranges, because subtrees are contiguous. Recursion that crosses a chunk boundary needs the whole document resident; that is the only hard limit.

**Termination:**
- ≤ n waves per stratum;
- each wave makes progress or ends the stratum;
- loops are bounded by `limit`;
- no spinning, and no waits across threadgroups.

## 4. Measurements

Units are wall and GPU time in ms. µs/wave is wall time divided by waves.

**Chain, depth 10,000** (20,001 nodes, 10,000 waves):

| Variant | Wall ms | GPU ms | µs/wave |
|---|---|---|---|
| (a) CPU per wave | 4,968 | 348 | 497 |
| (b) pre-encoded (+1,000 empty slots) | 71–78 | 55–61 | 7.1–7.9 |
| (d) chained, 64 / 1,024 waves per command buffer | 73 / 59 | 51 / 56 | 7.3 / 5.9 |
| (e) threadgroup loop | 11.8 | 11.2 | 1.18 |
| (f) hybrid, H = 1,024 / 8,192 | 15.5 / 14.1 | 12.7 / 12.5 | 1.55 / 1.41 |
| (c) GPU ICB | 204 | 130 | 20.4 |
| (c2) self-encoding ICB, split 1 | 296 | 150 | 29.6 |

**Chain, depth 100,000** (200,001 nodes, 100,000 waves):

| Variant | Wall ms | GPU ms | µs/wave |
|---|---|---|---|
| (a) | 38,589 | 3,118 | 386 |
| (b) | 712–728 | 507–510 | 7.1–7.3 |
| (d) 64 / 1,024 | 923 / 641 | 512 / 523 | 9.2 / 6.4 |
| (e) | 113 | 113 | 1.13 |
| (f) | 119–123 | 114 | 1.19–1.23 |
| (c) | 1,866 | 1,232 | 18.7 |
| (c2) | 2,227 | 1,415 | 22.3 |

**CPython stdlib** (5.4M nodes; seeded at Lambda; passes every kind except Module; 1.40M marked; 27 wide waves):

| Variant | Wall ms | GPU ms | Note |
|---|---|---|---|
| (a) | 24.0 | 1.71 | |
| (b) 29 / 1,029 slots | 1.89 / 7.15 | 1.61 / 5.07 | |
| (d) 64 / 1,024 | 3.18 / 10.6 | 1.99 / 8.51 | |
| (e) | 8.61 | 8.34 | a wide frontier on one threadgroup is slow |
| (f) H = 1,024 | **2.65** | **1.57** | 9 tail waves ran in the loop |
| (f) H = 8,192 | 8.65 | 8.30 | handed off too early |
| (c) | 3.90 | 1.82 | |
| (c2) | 2.88 | 1.93 | |

**Readings:**
- The CPU round trip costs 50–400× per wave, and removing it is the point.
- The threadgroup loop suits narrow tails and does badly on wide frontiers. H ≈ 1,024 works for both shapes.
- Empty slots cost about 3.5 µs each, so W should be sized from statistics rather than with a large margin.
- A GPU-encoded ICB costs 2.5–3× more per wave, so it is only worth it when the kernel choice really depends on the data.

**Caveats:**
- The tape is from Python `ast`, not a tree-sitter CST.
- Marks are u32, not bit-packed.
- These are single runs on a busy machine.

## 5. Resource outcomes instead of a fallback

**No CPU/JIT route at run time.** The CPU interpreter and JIT exist as test oracles, and as the executor on machines with no Metal at all.

**`Unknown(resource)` reasons**, per (rule, document), never silent:

1. `doc_exceeds_device`: the document's resident requirement exceeds the budget.
2. `batch_split_exhausted`: a single-document batch still does not fit.
3. `device_lost` / `command_buffer_error`: a bug or driver fault, reported with a diagnostic dump.
4. `plan_bound_violation`: observed sizes exceed the plan's bound, and re-planning is still over budget.

**Metric.** `metal_coverage = 1 − Unknown(resource) pairs / evaluated pairs`. It is reported per rule and in aggregate, with `unknown_by_reason` and the largest document size.

**CI gate.**
- `metal_coverage` must be 1.0 for every admitted rule on three corpora: the full data-layer repository, CPython plus the 335 patterns, and the rerail repository.
- Any `Unknown(resource)` or `device_lost` fails the gate.
- A rule that cannot pass the gate is not admitted.

**The verifier rejects, with a reason:**

- **Recursion through negation or aggregation** (unstratifiable), e.g. `U := A AND NOT_EXISTS(has U)`. The reason names the cycle. The author restates the rule in stratified form (e.g. depth k as k strata) or as an explicit priority.
- **Recursion through anything other than set-stage axes and local tests**, such as string-producing transforms or facts from other documents. Cross-document closure belongs to the SQL graph producers.
- **Live sets that cannot fit:** live-set count × bound exceeds the budget for the largest document in the corpus statistics. Rejected at plan time, with the numbers.
- **Constructs with no lowering yet:** reported as tracked, counted compiler gaps, never executed elsewhere.

**Effect.** Every gap is either a compile-time rejection that names the construct, or a counted resource outcome on a gated metric. An incomplete Metal path cannot pass as complete.
