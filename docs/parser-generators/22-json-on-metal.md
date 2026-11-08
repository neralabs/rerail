# Parser generators: JSON's pass 1 on Metal

## Intro

Pass 1 of the JSON parsers (doc 21) is one block program: bit operations over the bytes, with
carries from each block to the next. The same program that Cranelift compiles for the CPU is
printed as Metal Shading Language and run on the GPU. No step is written by hand for Metal.

This page covers:

- what the kernel looks like (§1);
- how it is launched, and how many workers fit (§2);
- how fast it runs on a batch of files (§3);
- where the time goes, measured by removing one part at a time (§4);
- what would make it faster (§5).

All figures are from an Apple M5 Max with 40 GPU cores. The batch is simdjson's 17 benchmark
files, each repeated 4 times: 68 files, 62 MB. Every run's token starts, last bytes and totals
are checked against Cranelift's. Throughput is the batch's bytes over its GPU time, the
fastest of 3 runs. The machine was shared, at load averages of 20 to 80, so figures vary by about
10% from run to run.

## 1. The kernel

Each **lane** (one GPU thread) owns W bytes of a file: 16, 32 or 64. Its values are words of
W bits, one bit a byte. A SIMD group of 32 lanes is a **group**: it covers 32·W bytes.

Values that depend on bytes before the lane come in three kinds:

- **Windowed.** A `prev` of a value made from the bytes alone. The lane recomputes that value
  over the 64 bytes before it, from memory. No other lane is needed.
- **Halos.** A `prev` of any other value. A lane takes the previous lane's word by a SIMD
  shuffle; lane 0 takes the last word of the group before.
- **Scans.** A carry, such as "inside a string". Each lane makes a small table of what its
  bytes do to the carry. The tables are composed across the group's lanes, then across groups,
  by decoupled look-back: a group publishes its table, and the next one reads it.

A value can depend on a halo or a scan, so the cross-group steps come in order. Each carried
value gets a **level**: one more than the highest level among its operands. All the halos and
scans of one level are published and read together, in one **exchange**: one word per lane,
one poll per lane. For JSON:

| | step 14 | step 15, with comments |
|---|---|---|
| operations | 307 | 363 |
| windowed `prev`s | 20 | 27 |
| halos | 28 | 38 |
| scans | 6 | 8 |
| exchanges | 14, then one for the totals | 18, then one |
| record a group, in words | 96 | 129 |
| MSL | about 1,600 lines | about 1,800 |

Before levels, every halo and scan waited on its own: 34 waits for step 14, not 14.

Groups whose bytes are all inside the file, except the first, run a second copy of the body. In
it, values known to be all ones or all zeros in a full block are constants. The CPU uses the
same known-bits analysis.

## 2. Launch

The kernel is **persistent**. A fixed number of workers, each one SIMD group (one threadgroup of
32 threads), take tickets from an atomic counter. A ticket is one group of one file. Every
launch covers the whole batch.

- **Pipeline limits.** 1,024 threads a threadgroup at most, SIMD width 32, no threadgroup
  memory. Everything shared goes through device memory.
- **Ticket order.** Tickets go round robin over the files: the first group of every file, then
  the second, and so on. A group waits only on the group before it in its file, which took its
  ticket a round earlier. In file order, the groups in flight were mostly one file's next ones,
  each waiting on the one before. Round robin took step 14 at 64-byte lanes from 7.3 to
  9.0 GB/s, at 512 workers.
- **Polls.** A lane polls a predecessor's word a bounded number of times. If the word is still
  missing, the group ends unfinished and the launch is repeated for what is left. No loop is
  unbounded. 64 polls ran out often on full batches: the predecessor was slow, not missing.
  65,536 polls removes the repeats.
- **Workers.** Every worker must be resident. A ticket held by a worker that is waiting to be
  scheduled stalls the workers behind it until their polls run out.

GB/s at 64-byte lanes, step 14, 65,536 polls:

| workers | 384 | 512 | 768 | 1,024 | 1,536 | 2,048 | 4,096 |
|---|---|---|---|---|---|---|---|
| GB/s | 7.3 | 9.1 | 11.4 | 12.5 | 4.8 | 2.4 | 1.4 |

Past 1,024 workers it falls off a cliff: not all of them are resident. 1,024 groups of 32 are
32,768 threads, about 26 SIMD groups per core. Step 15 uses more registers, so fewer fit: it
ran faster at 768 workers than at 1,024. Metal gives no occupancy figure for a pipeline, so the
default is 768, the best for both.

## 3. Throughput

The default launch: 768 workers, 65,536 polls. GB/s over the 62 MB batch:

| lane width | step 14 | step 14, files split in 16 | step 15 | step 15, split in 16 |
|---|---|---|---|---|
| 16 bytes | 3.0 | 4.0 | 2.1 | 2.9 |
| 32 bytes | 6.0 | 7.7 | 4.6 | 6.2 |
| 64 bytes | 11.3 | 13.0 | 8.8 | 10.4 |

"Split in 16" cuts each file into 16 independent pieces: 1,088 files of 57 KB on average. It
stands for a batch of many smaller files: more chains of groups that do not wait on each other.

At 16-byte lanes some launches are still repeated, about twice as many as at the other widths.

For comparison, one CPU core runs step 14's pass 1 at 2.2 GB/s, and simdjson's stage 1 at
7.8 GB/s (doc 21 §2).

## 4. Where the time goes

Each row removes one more part of the kernel. The outputs are then wrong, so these runs were not
checked. Step 14, 64-byte lanes, measured before round-robin tickets and the longer polls:

| kernel | GB/s |
|---|---|
| the real kernel, 512 workers | 8.0 |
| exchanges of the carried values removed (totals kept) | 10.4 |
| all exchanges removed | 13.1 |
| the same, 2,048 to 8,192 workers | 17.8 |
| positions not written either | 19.3 |
| windowed `prev`s not recomputed either | 21.6 |

At 32-byte lanes, with all exchanges removed and no positions written, recomputing the windows
is most of the work: 12.3 GB/s with them, 29.8 without.

What this says:

- **The exchanges cost the most.** Without them, the kernel needs no residency limit: more
  workers hide latency, and it reaches 17.8 GB/s. The real kernel now runs at 11 to 13.
- **The windows cost the most at narrow lanes.** Each lane redoes 64 bytes of work for its
  W bytes: as much again at 64-byte lanes, four times as much at 16.
- **Writing positions costs little**, about 8%.
- **Memory bandwidth is not the limit.** The kernel reads each byte once, plus the windows, and
  writes a few bytes a token: a few percent of what the chip's memory moves.

## 5. What would make it faster

In the order of what §4 suggests they are worth:

- **Fewer exchange levels.** 14 levels means 14 round trips through device memory between
  neighbouring groups. Levels with only one halo could take that halo from the window instead,
  where its cone allows it.
- **Halos from a warm-up lane, not windows.** A group could start one lane early (or
  ⌈64/W⌉ lanes at narrow widths) and throw that lane's outputs away. The other lanes then take
  every windowed `prev` from the lane before, by shuffle. That costs 1/32 of the group, not the
  window's full recompute.
- **Several words a lane.** A lane that owns 2 or 4 words pays each exchange once for all of
  them.
- **Gated regions.** On the CPU, a block without a number skips the number check. A group could
  do the same if every lane agrees. A skipped region must still publish its carries, as
  constants.
- **An ASCII body.** The same, for groups with no byte above 127, including the 64 bytes
  before them.
- **Fewer carried words.** Every carried word is a halo or a scan: the CPU's list (doc 21 §4)
  applies here as well.

The ceiling with today's body is about 18 to 22 GB/s at 64-byte lanes: §4, with no exchanges.
Going past that needs a smaller body.

## 6. Rerunning it

`json_metal_bench`, in `native/src/code_rule_dsl/transducer/packed/bitlex_json_tests.rs`, is
ignored by default. Its variables:

- `RERAIL_JSON_CORPUS`: the directory of simdjson's benchmark files;
- `RERAIL_FORM`: `14` or `15`;
- `RERAIL_REPEAT`: how many times the corpus goes into the batch;
- `RERAIL_SPLIT`: how many pieces to cut each file into;
- `RERAIL_WIDTH`: one lane width, 16, 32 or 64, instead of all three;
- `RERAIL_WORKERS` and `RERAIL_SPINS`: the number of workers, and the polls of a predecessor;
- `RERAIL_ITERS`: runs per width.

It prints the kernel's shape (as in §1 and §2), then the GB/s and launches for each width.
