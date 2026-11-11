# Tier 1 of `.rr` on Metal: where the time goes

## Intro

JSON's tier 1 runs on Metal at 13.8 to 15 GB/s (docs 22 and 23). `.rr`'s runs at 0.44 GB/s,
in the same segment-and-lanes kernels. This page finds out why.

It covers:

- how the benchmark is run, with no compiling inside the timed runs (§1);
- the numbers: Metal against the CPU (§2);
- what the tooling could and could not measure (§3);
- where the GPU's time goes, kernel by kernel (§4);
- the causes (§5);
- what would fix them, and in what order (§6).

The machine is an Apple M5 Max with 40 GPU cores. The batch is the repository's 1,362 `.rr`
files, 8.85 MB. The machine was shared, at load averages of 25 to 85, so the wall times are
loose. GPU times come from the command buffers, and CPU times from the thread's clock.

## 1. The benchmark

`rr_metal_bench` (in `rr_parser`, run by hand) works like this:

- **Prewarm first.** `serial::prewarm` runs the program over a sample until a run makes no new
  source and no new kernel. A run can learn which scans to make exact (`learn`), and the next
  run compiles what it learned, so one run is not enough. A process would call `prewarm` once
  a program, on a thread of its own, off the parse's path.
- **Count compiles.** `serial::compiles()` counts the MSL sources generated and the kernels
  compiled. The benchmark reads it around the timed runs, and those must make none.
- **The sample is the corpus.** What a run learns depends on the files. A sample of every
  fourth file left 12 kernels to the timed runs, so the default sample is every file.
- **Digests come from the caller.** `run_resident_digested` takes each file's SHA-256, as the
  engine's residency does. A run hashes no bytes, and it admits the files once, not once a
  pass.
- **Sources can be kept on disk.** `RERAIL_SERIAL_CACHE=dir` keeps the generated MSL by key.
  It is for runs by hand, such as traces, which would otherwise generate the sources again in
  every process. With it, the probe's whole process takes 6 s instead of 26 s.

The first call's cost, now out of the timed runs:

- generating `.rr`'s MSL: 26 to 48 s in a debug build;
- compiling it: about 22 s on first sight. The system's shader cache serves it after that.
- prewarm over the corpus: 35 to 93 s.

## 2. The numbers

CPU time a pass on one thread, GPU time from the command buffers:

| | 8.85 MB |
|---|---|
| Rust reader (`rr::tape`), optimized | 234 to 263 ms |
| compiled grammar, file by file, CPU | 106 to 125 ms |
| tier 1 alone, one CPU batch | 55 to 73 ms |
| tier 1 alone, Metal, GPU time | 19.3 to 20.2 ms (0.44 to 0.46 GB/s) |
| tier 1 alone, Metal, host CPU | 36 to 55 ms |

- **Batch size does not matter.** A budget raised from 256 to 512 MB, to run the corpus as one
  batch, gave 19.3 ms against 20.2 ms. The corpus twice over gave 48 ms. GPU time grows with
  the bytes, about 2.2 ms a MB. Launch overhead is not the cost.
- **The host's CPU is now small.** Before the files were admitted once a run, it was 205 ms a
  run, most of it SHA-256 of every file at each of the five passes.

## 3. The tooling

What Instruments gave (`xcrun xctrace`, Xcode 16):

- **Metal System Trace** records each command buffer's GPU interval and the encoders' labels.
  An encoder is now labeled with its kernel's name, from the pipeline's label.
- **The compiler's spill events.** The big pass's command buffer spills 1.17 KB a thread. The
  small passes' main kernel spills 624 bytes.
- **The hardware counters are refused.** The Metal GPU Counters instrument says "Selected counter
  profile is not supported on target device". This holds for both templates and for every
  counter profile value tried. Game Performance records one counter only, "RT Unit Active". So
  there are no limiter, occupancy or bandwidth counters on this M5 Max from the command line.

What stood in for them:

- each kernel's GPU time alone (`RERAIL_SERIAL_TIMES`);
- each pipeline's `maxTotalThreadsPerThreadgroup`, which falls when a kernel's registers are
  many. Every kernel here allows 1,024, so no kernel's registers limit its occupancy.
- turning parts off (`RERAIL_SERIAL_CONE=0`) and changing the budget.

## 4. Where the GPU's time goes

Tier 1 of `.rr` is nine passes. The forward passes:

| pass | instructions | toggles | fills | `prev`s | fixed points | exact levels |
|---|---|---|---|---|---|---|
| 0 | 271 | 0 | 0 | 67 | 0 | 0 |
| 2 | 724 | 8 | 22 | 148 | 2 (32 and 7 members) | 9 |
| 4 | 61 | 0 | 4 | 4 | 0 | 0 |
| 6 | 21 | 0 | 1 | 2 | 0 | 0 |
| 8 | 42 | 0 | 2 | 2 | 0 | 0 |

Passes 1, 3, 5 and 7 are backward fills.

Pass 2 is almost all of it. Its kernels, for a batch of 5.3 MB (3,286 segments):

| kernel | what it does | ms | share |
|---|---|---|---|
| `lanes_cone`, 8 launches | one level's scan tables | 5.2 | 41% |
| `lanes_group`, 2 launches | a fixed point's table, from each of 8 start states | 5.2 | 41% |
| `lanes_run` | the pass itself | 1.5 | 12% |
| prefixes, checks, the other passes | | about 0.9 | 7% |

So the pass's own body is an eighth of the time. The rest makes the carries exact before the
body runs.

## 5. The causes

### 5.1 A counter in eight toggles

Pass 2's toggles 220, 225, 233 and the five after them are a **binary counter**: the bracket
nesting depth, eight bits.

- Bit 0 toggles at every open and every close.
- Bit k toggles when the bits below it carry (an open with them all 1) or borrow (a close with
  them all 0).

Each bit reads the one below, so each is a level of its own. The segment form gives a level one
launch over all the bytes: eight launches for one counter.

The counter never forgets: a guessed depth is wrong to the file's end. Without levels
(`RERAIL_SERIAL_CONE=0`), the runs again take 138 rounds and 211 to 275 ms. So the carries must
be exact, and the levels are how the segment form makes them exact.

### 5.2 Every level runs the fixed point again

The toggles read the group (`Carry(0)` and its 32 members), a fixed point. A block's rounds run
the group until no carry changes. The levels' kernels each run a cone of the program, and every
cone after the group's level holds the group. So:

- each of the 8 levels runs the fixed point's loop again, to the same values;
- each group table runs it 8 times, once from each start state.

That is why a level's kernel, about 50 instructions on paper, takes 0.65 ms: 8 GB/s, against
27 GB/s for JSON's whole main pass.

### 5.3 Twenty launches in a row

Each level is a launch, then a prefix over each file's segments, and the next level waits for
both. Pass 2 is 20 dependent launches over the batch.

### 5.4 Slots

Tier 1 keeps 106 slots a block for the later passes and for tier 2: 848 bytes for 64 bytes of
input. So the 256 MB budget holds about 5 MB of input a batch. That does not cost time (§2),
but it does cap the batch.

## 6. What would fix it

In order of what each step removes:

1. **The fixed point once (5.2).** A level after the group's reads the group's values from
   memory instead of running the loop again: the group's members are stored once, as a pass's
   slots are. Likely most of the levels' 5.2 ms.
2. **The counter as one scan (5.1).** A ripple of toggles, each on the carry of the ones before,
   is an add of +1 or -1. Carried as one small integer, its block's effect is a delta, which
   composes by addition: one level, not eight. This is a compiler step (recognizing the chain)
   that every backend uses: Cranelift, the interpreter and Metal.
3. **One launch, lane-parallel, with look-back (5.3).** Doc 22's kernel (`run_split`) carries
   every level within one launch: a level is an exchange between SIMD groups, not a launch.
   JSON has 14 exchanges and runs at 13.8 GB/s. To take `.rr`'s pass 2, `run_batch` must
   accept three things it refuses today:
   - a pass's `Load`s and stores (the `Io` of the segment form);
   - fixed points: their lanes form exists (`group_loop`), but their state across SIMD groups
     does not. It is a lexical state of 8 states at most: a table of 3-bit entries, one word
     a group, which composes as the scans' tables do;
   - the backward fills between passes: these stay the segment form's kernels
     (`back_kernels`).
4. **Spills.** 1.17 KB a thread in pass 2, which the counters cannot attribute. Smaller
   kernels, after steps 1 to 3, are the first fix.

The ceiling: pass 2's body alone runs at about 3.5 GB/s (1.5 ms for 5.3 MB), and every pass's
body without levels takes 9 ms over the corpus (about 1 GB/s). Steps 1 to 3 remove the levels'
cost. The body's own speed is then the limit: 724 instructions against JSON's 307.
