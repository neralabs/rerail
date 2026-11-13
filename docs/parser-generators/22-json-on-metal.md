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
fastest of 3 runs. The machine was shared, at load averages of 30 to 130, so figures vary by
about 10% from run to run.

## 1. The kernel

Each **lane** (one GPU thread) owns W bytes of a file: 16, 32 or 64. Its values are words of
W bits, one bit a byte. A SIMD group of 32 lanes is a **group**: it covers 32·W bytes.

Values that depend on bytes before the lane come in three kinds:

- **Local.** A `prev` of a value made from the bytes alone, such as "the byte before is a
  backslash". The lane takes it from the lane before, by a SIMD shuffle. Lane 0 has no lane
  before it in the group, so each group starts one lane early: a **warm-up lane** that reads
  the W bytes before the group, makes these values, and writes nothing. A group then covers
  31·W bytes, not 32·W.
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
| local `prev`s | 20 | 27 |
| halos | 28 | 38 |
| scans | 6 | 8 |
| exchanges | 14 | 18 |
| record a group, in words | 91 | 123 |
| MSL | about 1,900 lines | about 2,200 |

Before levels, every halo and scan waited on its own: 34 waits for step 14, not 14.

The outputs need no exchange:

- **Totals** are flags: 1 if the output has any bit. Each group ORs its own into the file's
  word, with one atomic. Before, a total was a count that went through an exchange of its own.
- **Token starts** are the one extract, written as positions. Each group needs the count of
  positions before it, so this count is the one value still exchanged after the levels.
- **Last bytes** are written as mask words, one bit a byte, with no count.

The body comes in three copies:

- **Edge groups.** The first group of a file and the one that holds its end. Every value is
  computed as written.
- **Full groups.** Every other group. Values known to be all ones or all zeros in a full block
  are constants. The CPU uses the same known-bits analysis.
- **ASCII groups.** A full group whose bytes, and the byte before it, are all below 128. The
  lanes agree on that with one `simd_all`. Values known from the ASCII bytes alone (all the
  UTF-8 checks, for JSON) are constants as well.

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
| 16 bytes | 5.0 | 6.5 | 3.6 | 4.9 |
| 32 bytes | 9.5 | 11.9 | 7.1 | 9.2 |
| 64 bytes | 13.8 | 16.7 | 10.4 | 12.9 |

"Split in 16" cuts each file into 16 independent pieces: 1,088 files of 57 KB on average. It
stands for a batch of many smaller files: more chains of groups that do not wait on each other.

Before the warm-up lanes, the same table read 3.0, 6.0 and 11.3 for step 14. What each change
gave step 14:

| change | 16 bytes | 32 bytes | 64 bytes |
|---|---|---|---|
| windows (before) | 3.0 | 6.0 | 11.3 |
| warm-up lanes | 5.0 | 9.0 | 12.7 |
| ASCII groups | 5.0 | 9.5 | 13.9 |

The warm-up lanes help most at narrow lanes: a window was 64 bytes of work for W bytes, a
warm-up lane is 1/32 of the group. The ASCII copy helps most at wide lanes, where the body is
most of the time.

At this load the launch is repeated once at every width: some group's polls run out while its
predecessor waits to be scheduled.

For comparison, one CPU core runs step 14's pass 1 at 2.2 GB/s, and simdjson's stage 1 at
7.8 GB/s (doc 21 §2), on a quiet machine.

## 4. Where the time goes

Each row removes one more part of the kernel. The outputs are then wrong, so these runs were not
checked. Step 14, 64-byte lanes, measured with windows, before round-robin tickets and the
longer polls:

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
  workers hide latency, and it reaches 17.8 GB/s.
- **The windows cost the most at narrow lanes.** Each lane redid 64 bytes of work for its
  W bytes. The warm-up lanes replaced them (§1).
- **Writing positions costs little**, about 8%.
- **Memory bandwidth is not the limit.** The kernel reads each byte once, plus 1/32 for the
  warm-up lane, and writes a few bytes a token: a few percent of what the chip's memory moves.

With today's kernel, one more part was removed: the number check, the CPU's one gated region
(doc 21 §4). Step 14 at 64-byte lanes went from 13.8 to 21.0 GB/s. That is the most a skipped
region could give, on a batch where most groups hold a number somewhere.

## 5. What would make it faster

What is done, from the list this page had before:

- **Warm-up lanes, not windows.** Done (§1). One warm-up lane covers every local `prev` of
  JSON: none reaches back more than W bytes.
- **An ASCII body.** Done (§1).

What is left, in the order of what §4 suggests they are worth:

- **Gated regions.** Up to 52% at 64-byte lanes (§4). A group could skip the number check when
  no lane holds a digit. The catch is a region that turns on late. Its gate can depend on a
  carry from the group before, which arrives at a later level. If the region was off at the
  early levels, the values it needs from them were never made. Skipping is only sound when
  the gate is known before the region's first value, or when the region can be rerun with
  everything it reads.
- **Several words a lane.** A lane that owns 2 or 4 words pays each exchange once for all of
  them, and the warm-up lane is a smaller share of the group. This needs the body printed for
  `ulong2`, or twice in each lane, with the carries between the two copies done in the lane.
- **Fewer carried words.** Every carried word is a halo or a scan. The CPU carries one packed
  word (doc 21 §4); here each halo and scan is still one word of the record. Packing them
  needs the same restated state as the CPU's next steps.

Fewer exchange levels is no longer on the list. The 14 levels are real dependencies: a value
at level k needs a carry that is only known once level k−1 is exchanged. A warm-up lane cannot
make those values: its own inputs would need halos from before it, and so on back to the start
of the file.

The ceiling with today's body and exchanges is about 21 GB/s at 64-byte lanes: §4, with no
number check. Going past that needs a smaller body or fewer round trips.

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
The launches are those of the last run.
