# 0430: The preprocessor as array programs: a transducer side program, on the interpreter and Metal

Status: draft (2026-11-18). Follows 0421 (the spike of the shape) and the user's choice of its
placement (a): the passes become a transducer side program, and the directives' rounds are
written in the same operations. Numbered 0430 because other worktrees had reached 0429; the
code's first commits said 0422.

## Intro

0421 showed that a C or C++ preprocessor can read a translation unit (TU) as passes over
arrays. Each pass is a map, a scan, a sort, a search, a gather or a scatter. Only two pieces
stay serial: one `#if` test, and one macro invocation's expansion. Both are small and
independent of each other.

This decision makes the passes a program in the transducer's own format, so that the same
executors that run the rest of the transducer run them: an interpreter (the meaning), Metal,
and later Cranelift. The two serial pieces become *calls*. Each call runs the preprocessing
words of 0348 on one element (one test, one invocation), as 0408 runs them on the device.

## The format: array programs

An array program (`rerail_compiler::array`) is held in `Words.array_programs`, beside the
block programs (0066). It is covered by the program digest, `validate`, and the `rir` text and
assembler. Operation `i` defines the value `%i`, a column of `i64`.

| Operation | Meaning |
|---|---|
| `input k`, `param k`, `const c`, `iota %a`, `len %a` | the run's columns and scalars |
| `map OP %a %b`, `select %m %a %b` | element by element; a length-1 operand is broadcast |
| `scan add\|max\|min [excl] %a [seg %f]` | a prefix scan; a flag resets it |
| `sort %k1 %k2 …` | the stable permutation that sorts by the keys |
| `gather %s %i`, `scatter HOW %src %i %n %init` | moves; a scatter skips an index outside `0..n` |
| `search le\|lt %sorted %q` | each query's place in a sorted array |
| `compact %m`, `expand %c`, `concat %a %b` | lengths made by data |
| `call f %args… [env %e…]`, `part %c k` | function `f` on each element; its result rows |
| `carry %init`, `rounds %c %n …` | a local fixed point over a span of operations |

A run is refused, with the operation and the reason, where its operands do not fit: unequal
lengths, an index outside its array, two writes of a `set` scatter to one element, a search of
an array not sorted, a negative count, a fixed point past 1,000 rounds.

The structural guard: no operation loops over elements serially except `call`, whose work per
element is bounded by the words' bounds.

## The passes as two programs

`rerail_c_directives::passes` holds two programs, built in Rust as rows:

- **ROUND** (P1 of 0421): one round of the directives' fixpoint. The layout of the include
  tree, the definitions sorted by (name, position) and searched per use, push/pop matched by
  depth, each test called at its position, a chain's first true arm by a segmented scan, a
  group's activity by gathers, includes called on the host.
- **TEXT** (P2–P4): the active stretches, the candidates, the outermost invocations (a max
  scan and a compact), each invocation called alone, and the output assembled by counts, a scan
  and a scatter.

The host runs ROUND until the state it returns is the state it read, then TEXT once.

**The memo.** A test is called again only if it is new or a definition it looked at has
changed. A test is called only where its group is active in the last round's state (a mask).
On the heaviest C++ unit, tests fell from 231,776 to 40,450; rounds rose from 24 to 38.

## The calls on the words

A call reads one element with the words of 0348, on a file made for it
(`passes/words.rs`):

- the definitions it can reach (the closure, by name, through the position's table), as
  `#define` lines in the unit's own tokens;
- `#undef` for a built-in undefined at the position;
- then the element: a test as `#if … 1 #else 0 #endif`, or an invocation's tokens with the
  token after it (when it is `(`) and a sentinel `)`;
- inside one synthesized file for each file of the include chain, so `__FILE__`,
  `__INCLUDE_LEVEL__` and `__has_include_next` read as they do in place.

A name the words did not find is reported back (`Env::asks`). If it is defined at the
position, the host adds it and reads again. An output without the sentinel or the follower
asked past the element: a merge, which refuses the unit for now (the corpus had none).

## On the device

`rerail_metal::array_program` runs a program's operations as kernels
(`array_program.metal`) over `i64` columns in shared memory:

- maps, selects and gathers: a thread an element;
- scans: a tile of 256 scanned in threadgroup memory, segment flags as pairs, the tiles'
  totals scanned the same way;
- sorts: a stable radix sort, a byte a pass, only the bytes where keys differ; each tile
  sorted by eight one-bit splits;
- searches: a binary search a query;
- scatters: a stable sort by index, a segmented scan, each index's last write;
- compact and expand: inclusive counts.

The first failing operation's error is kept by an atomic minimum, so a refusal names the same
operation the interpreter names.

**A batch.** `run_batch` runs a program for many units in the same launches. Each array is
its units' arrays end to end, with each unit's offsets on the host and on the device. A kernel
finds an element's unit by a binary search of the offsets, so each unit keeps its own lengths
and broadcasts, its scans reset at its first element, and its gathers and searches stay in its
own range. A sort takes the unit as its most significant key. A refused unit stays refused at
its first operation and drops out; the batch goes on. The interpreter's meaning is each unit
run alone, and a batch is checked against it.

The passes read a batch in lockstep (`passes::run_batch`): one ROUND run a round for every
unit still moving, then one TEXT run for all.

**What the host still does** (counted in `DeviceRun`):
- reads a count where a length is made by data (compact, expand, a scatter's `n`);
- reads which bytes of a sort key vary;
- reads each local fixed point's verdict;
- answers calls: reads the elements, makes each element's file, sends the rows back.

**The calls' readings on the device** (`Exec::MetalWords`). A call op hands every live unit's
elements to one host at once (`array::UnitCalls::call_units`; `Each` answers unit by unit
through `Calls::call_many`). The passes answer tests and expansions with
`words::tests_across` / `expands_across`: every element's file is made, and each wave of
readings, every unit's, runs as one lockstep batch of the words on the device
(`expand::on_metal_counted`, 0408's `pp_on`). An element that asked for names is made again
and read in the next wave, up to 8 readings. So a round costs about one word batch for each
call op, however many units the batch has.

Two things in `pp_on` were sized for TUs, not elements:
- `Bounds::rung` counted every tokenless file as embedded data, two tokens a byte. An
  element's headers are tokenless, carried for their text only, so a 30-token element got
  rooms for its headers' whole text (2 MB a job). A file counts as data now only if a file
  reaches it (`includes`, `named`). `Bounds::within` holds a rung at the job's own bounds.
- A batch's arena started at 64 MB and doubled while full. It now starts from the batch's
  measured need, and is split before any lease when the need is past the largest arena
  (256 MB). One call of 300 elements had made 6.6 GB of arenas that could not fit.

**The launch ledger.** Each command buffer is one execution under the launch ledger (200
launches, the user's bound of 2026-11-03). An array program runs many command buffers, with
the host's reads between them. The executor takes the execution lock and the ledger's scope
per command buffer, and ends a buffer at the ledger's bound. The bound itself is unchanged.

## On the CPU: Cranelift

`rerail_compiler::native_cpu_array` emits each shape of operation a program has (its map, its
combination, its flags) as one Cranelift function over a block of words: lengths, pointers,
and a mask by which an operand's index is taken (0 broadcasts a scalar). The host's driver is
the interpreter's: it chooses lengths, runs the rounds and answers the calls. Every loop over
elements is emitted code:
- maps, selects, iotas, copies;
- segmented scans;
- a stable LSD radix sort, a byte a pass, skipping a pass where every element has one byte;
- gathers, scatters (with a `set` scatter's written flags), searches (a sortedness check,
  then a binary search a query), compacts, expands;
- the fixed points' verdict.

`Exec::Cranelift` runs the passes on it; the calls run on the CPU.

## Measured

**Correct, the interpreter, the lean calls** (the serial pass's own expansion per element),
both corpora under clang's profile:
- C: 1,572 of 1,574 the same, 2 refused (`#line`; a call past its stretch), 0 differ.
- C++: 1,560 of 1,561 the same, 1 refused (`#line`), 0 differ.

**Correct, the calls on the words:** the passes' unit tests, 10 C units and 2 C++ units the
same. The heaviest C++ unit: 38 rounds, 40,450 tests, 18,763 expansions; under 13 s on the
interpreter, down from 106 s.

**Correct, on Cranelift:** the device's operation tests, the same programs and lengths, equal
the interpreter, refusals at its operation; every passes test; the first 50 C units.

**Correct, on Metal:** every operation equals the interpreter over lengths 0 to 70,001 (two
levels of tiles), and the refusals are at the interpreter's operation. The passes' 10 unit
tests equal the interpreter. The first C corpus unit reads the same.

**The shape on Metal**, the first C unit (8,674 tokens out, 29 rounds):

| | |
|---|---:|
| Program runs | 30 |
| Dispatches | 35,618 |
| Syncs | 2,534 |
| GPU time | 0.25 s |
| Elements the host called | 9,401 |

The syncs by cause: sort-key bytes 904, local fixed points 869, compact 409, expand 203,
calls 89, the rest 60.

**Batches**, the first C units, all read the same:

| | 10 alone | 10, one batch | 50, one batch |
|---|---:|---:|---:|
| Program runs | 137 | 30 | 30 |
| Dispatches | 138,003 | 41,684 | 43,248 |
| Syncs | 9,785 | 2,652 | 2,674 |
| GPU time | 1.13 s | 0.40 s | 0.36 s |
| Device bytes at the peak | 47 MB | 95 MB | 374 MB |
| Elements the host called | 29,657 | 29,657 | 141,641 |

With the readings on the device too (`Exec::MetalWords`), all the same:

| | 10, one batch | 50, one batch |
|---|---:|---:|
| Word batches (a wave of every unit's readings) | 38 | 40 |
| Their dispatches | 43 | 63 |
| Their GPU time | 0.24 s | 0.48 s |
| Readings (tests, expansions, again) | 10,881 | 51,441 |
| The programs' GPU time | 0.26 s | 0.36 s |

Before the readings were batched across units, the 10 units took 157 word batches (177
dispatches, 0.67 s of GPU time).

A batch costs about what its deepest unit costs alone: the launches, not the data. At 50 units
that is 7 ms of GPU time a unit. 0408's gate is about 0.15 s for the whole corpus on one core,
so the launches are still the cost to cut.

The 50-unit batch's dispatches by kernel: scans 12,630 (a scan of more than 256 elements is
three launches or more), maps 8,448, gathers 6,271, radix passes 3,506, a scatter's parts
about 4,400. That is about 250 operations a round, 30 rounds, one launch or more each.

**Timed** under a quiet-run lease on an M1 Max, `--profile profiling`, the first 200 C units,
the calls on the words (rustc's build of them on the CPU), every unit the same. The passes'
time only; settling each file under clang's profile comes before it.

| Executor | Wall | CPU (this thread) | Of it |
|---|---:|---:|---|
| Interpreter | 20.4 s | 20.4 s | |
| Cranelift | 15.7 s | 15.7 s | the words 2.5 s, the elements' files 0.6 s |
| Metal, the words on the device | 63.6 s | 41.6 s | GPU: the programs 3.9 s, the words 11.0 s; the words' host side 44.8 s |

Against 0408's gate (about 0.15 s for the whole corpus on one core, the serial pass), every
executor is two orders of magnitude or more away. Where the time goes:
- **Cranelift**, sampled: the round program's elementwise operations (maps, selects, gathers,
  copies) take about a quarter of the samples. Each round recomputes every moved operation
  over the unit's whole arrays, one pass over memory an operation, scalar loops. The rest is
  the host: the spike's unit tables (hashing, `reaching`, the lexer) that the calls read.
- **Metal**: the words' host side, laying out and finishing about 260,000 readings
  (`pp_on`'s prepare, copies and `complete`), and the M1 Max's budget: 50 units of a batch
  are past 512 MB, so the passes run 12 a batch (`Runner::cap`).

**Found on the way** (each by the device oracle, `RERAIL_PP_ORACLE=1`, which checks each
device run against the interpreter, unit by unit, and with `RERAIL_PP_ORACLE_OPS` names the
first operation whose value differs):
- a unit with no directive was refused (`stretches` gathered a directive's place at 0);
- the words handed a test's looked-at pairs back in a hash map's order, so the same test's
  pairs came back in another order call by call; the memo compares them in order;
- the corpus harness kept one directory of stub headers, remade for each file, so a batch
  settled before it was read lost its earlier units' stubs;
- a lease past the device's budget between launches left a command encoder open, and Metal
  aborted the process (an M1 Max): the executor ends it when dropped, and the passes split
  the batch.

## What is left

1. ~~Every unit of a batch in the same launches~~ (built, above).
2. **Less work a round**, on every executor: elementwise runs fused into one pass; only the
   elements a round changed recomputed, not the unit's whole arrays; on the CPU, more than
   one element a step. And the words' host side on the device: the element's layout made
   once, not prepared and finished a reading at a time.
3. **Fewer launches and syncs:** scans in one launch (a single-pass scan), elementwise runs
   fused into one kernel, a sort's varying bytes and a fixed point's verdict kept on the
   device.
4. ~~The calls on the device, through 0408's lockstep batch~~ (built, above). The element's
   files are still made on the host; making them by array operations is next.
5. ~~Cranelift, the CPU path~~ (built, above). Fused elementwise runs are not built yet:
   each operation is a loop of its own.
6. ~~Timing through `quiet-run`~~ (above): the gap is two orders of magnitude.
7. P0 (each file's tables) on the device; merges; an expansion cache.
