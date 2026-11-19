# Parser generators: compiling, lowering and interpreting Bitlex

Status: survey and proposed shape, 2026-11-06. Nothing here is built yet, apart from the
reference evaluator (`03-bitlex.md` §5) and the operation counts in §8. It follows
`01-branch-free-design.md` §4 (the stream transducer) and `06-block-parallel.md` (carries and
hardware levels).

## Intro

`tools/bitlex` runs a `.bitlex` lexer as an oracle. Every operation runs over a whole file's mask,
one operation after another, with heap vectors in between. That makes it right and easy to check,
but slow: about 3 MB/s on Python.

The aim is code in the shape of simdjson's stage 1:

- 64 bytes at a time, with masks in registers and carries passed in registers;
- a few ergonomic SIMD wrappers, so the generated code reads like the hand-written scanners;
- one implementation per architecture, with its own register width and step size.

The same program must also run in three more places:

- an interpreter, available at once;
- a JIT, built in the background;
- Metal.

Ideally all of them go through the final shape's transducer pipeline.

The proposal in one paragraph:

- Lower every `.bitlex` file to one IR, a **stream program**. It is a graph of the
  stream-transducer operations of `01-branch-free-design.md` §4 (`map`, `shift`, `scan`,
  `compact` …) over 64-byte blocks.
- Every carry is explicit and typed, with the summary, combine and apply parts of
  `06-block-parallel.md` §2.
- Rounds become passes.
- Four executors read that one IR:
  1. a chunk-at-a-time interpreter;
  2. generated Rust in simdjson's shape, with a module per architecture;
  3. Cranelift, emitting the same shape at run time;
  4. Metal, first as an interpreter kernel, then as generated MSL.

  They are four bindings of one plan identity, as decision 0012 sets for tree transducers.
- The stream program is a second kind of transducer program, beside the tape-walking one. It
  shares the pipeline's machinery: plan ids, background build and swap, pipeline caches, launch
  budget and residency slabs. It does not share the instruction set (§7).

## 1. simdjson's shape, and what to copy

From simdjson's sources (`generic/stage1/*.h` and `arm64.cpp`, `haswell.cpp`, `icelake.cpp`,
`fallback.cpp`):

| piece | what simdjson does | what Bitlex takes |
|---|---|---|
| implementations | one namespace per target (`arm64`, `haswell`, `icelake`, `westmere`, `ppc64`, `lsx`, `fallback`), chosen at run time by `get_active_implementation()` | one generated module per target, chosen once per process |
| `simd8<T>` | one register of bytes, with `splat`, `load`, `==`, `<=`, `lookup_16`, `saturating_sub` … | the same wrapper, written once per target by hand |
| `simd8x64<T>` | one 64-byte block as `NUM_CHUNKS = 64 / sizeof(simd8)` registers: 4 × 16 on NEON, 2 × 32 on AVX2, 1 × 64 on AVX-512. A `static_assert` checks the count | the same; the block stays 64 bytes on every target, because a mask is a `u64` |
| `to_bitmask` | NEON: AND with a bit-position mask, then four `vpaddq`; AVX2: `movemask` | `eq(b)`, `lteq(b)`, `in_class(T) -> u64` |
| `STEP_SIZE` | `index<64>` on arm64, `index<128>` on haswell and icelake: two blocks per step, so two independent block computations overlap | `STEP` per target: 1, 2 or 4 blocks per step (§5) |
| `buf_block_reader<STEP>` | full steps read in place; the tail is copied into a step padded with spaces | the same, padding with a byte that no class holds; masks are trimmed at `n` |
| scanners | small structs with one carried field each (`next_is_escaped`, `prev_in_string`, `prev_scalar`), composed in `json_scanner::next` | one generated struct per lexer, one field per carry, one `next(&block)` per pass |
| `classify` | two nibble lookups (`vqtbl1q_u8`) and an AND | `class` statements packed into nibble tables by the compiler, up to 8 classes per table pair |
| delayed write | the indexer writes the previous block's structurals, so every mask can see one block ahead | `next(a, k)` reads one block ahead in the same way |
| `fallback` | a separate scalar loop, byte by byte | not needed: the portable target is `u64` SWAR with the same code, so every target runs the same algorithm |

Two simdjson habits matter more than any single operation:

- **The block is the unit of meaning; the step is a performance knob.** Code is written for one
  64-byte block. The step only decides how many blocks are in flight.
- **Each scanner owns its carry.** A carry is a field of a struct, read at the start of `next`
  and written at its end. Nothing else crosses a block edge.

## 2. The layers

```
 .bitlex text
     │  syntax.rs (exists)
     ▼
 AST: statements, mask expressions, scans, blocks, hypotheses, rounds
     │  expand: instantiate blocks, give hypotheses lanes, name every value
     ▼
 stream program (the IR)  ◄── checked against the reference evaluator, file by file
     │  passes: CSE, hoist round-invariant values, fuse, pack classes, allocate slots
     ├──► interpreter     chunk at a time, on the CPU             (available at once)
     ├──► Rust codegen    simdjson-shaped modules per target      (built ahead of time)
     ├──► Cranelift       the same shape, emitted at run time     (built in the background)
     └──► Metal           interpreter kernel, then generated MSL
```

The reference evaluator stays the oracle. It is the interpreter's specification, and every
executor is checked against it. Rules state the same thing for tree transducers ("the CPU
interpreter is the reference semantics and the test oracle", `spec-fusion.md`).

## 3. The IR: a stream program

### 3.1 Values

| type | per 64-byte block | where it comes from |
|---|---|---|
| `mask` | one `u64` | classes, logic, shifts, fills, toggles |
| `plane<k>` | `k` masks: a small number per byte, one bit-plane per bit | `reflect` (2 planes, a value mod 3), lane numbers in hypotheses |
| `count<w>` | 64 integers of width `w` (8, 16 or 32 bits) | `count`: depths, line numbers, prefix sums |
| `list` | a dense list of positions and payloads, any length | `compact`: token starts and ends, machine events |

Widths are inferred from the program: a depth with a bound is `count<8>`; a line count is
`count<32>`. `count` values are the only per-byte integers. Everything that tests them (`eq`,
`ge`, `frame`, `since`) yields masks again.

### 3.2 Operations

These are the operations of `01-branch-free-design.md` §4, each with a fixed cost per block:

| operation | in | out | carry between blocks |
|---|---|---|---|
| `classify(tables)` | bytes | several masks | none |
| `and or xor andnot not` | masks | mask | none |
| `shift(a, ±k)` | mask | mask | `k` bits, forwards or backwards |
| `scan_xor(a)` | mask | mask | 1 bit |
| `scan_fill(s, e)`, `scan_back_fill`, `odd_run(a)` | masks | mask | 1 bit (generate, propagate) |
| `scan_mod3(a, b)` | masks | `plane<2>` | 2 bits |
| `scan_add(up, down)` with floor | masks | `count<w>` | the count |
| `frame(c, m)`, `since(c, m)` | count, mask | mask | the open frames' flags, at most the depth bound |
| `scan_compose(maps)` | `list` of symbols | `list` of states | a map (≤ 64 bits) |
| `compact(m)` | mask | `list` | a write offset |
| `select(lane, …)` | `plane`, masks | masks | none |
| `seg_reset(op, flags)` | any scan | the same | the same, cleared at a flag |
| `reduce_or(m)` | mask | 1 bit per file | the running OR |

Each carried operation gives its carry type three parts: a summary, a combine and an apply, as in
`06-block-parallel.md` §2. A backend needs the combine only when it runs blocks out of order:
between CPU threads, and on Metal.

### 3.3 Statements to operations

| Bitlex | lowers to |
|---|---|
| `class` | one `classify`. The compiler packs all classes into nibble-table pairs, at most 8 classes per pair, as in simdjson's `op_table`/`ws_table` |
| `text("abc")` | `classify` for each byte, then `shift` and `and` |
| `prev`, `next`, `starts`, `lasts`, `after`, `span`, `aligned` | `shift` and logic |
| `toggle`, `fill`, `back_fill`, `after_odd_run` | the scan of the same name |
| `head(a, k)` | `k` shifts, unrolled |
| `head(a, C)` with a count `C` | an unroll up to `C`'s width bound, one `select` per step |
| `scan … = reflect(…)` | `scan_xor` for the sign, then `scan_mod3` on two planes |
| `scan … = count(up, down)` | `scan_add` with a floor |
| `machine` | `compact` the events, `scan_compose` the list, then one `scan_fill` per state bit to go back to bytes (`03-bitlex.md` §3.2) |
| `hypotheses … split E lanes …` | every lane's body, then `scan_compose` over the lane maps per segment, then `select` |
| `rounds N { … }` | a **pass group**: the body becomes a pass that is repeated, and `carry` values persist between passes (§4.3) |
| `block` and instances | expanded before the IR. Each instance is a copy with prefixed names, and CSE merges what the copies share |
| tokens, `munch`, `keywords` | `compact` of starts and ends, then per-token operations over the `list` |
| `error` | `reduce_or`, one bit per file |

### 3.4 Passes over the IR

- **CSE**, after instances are expanded. Blocks `f1` and `f2` share most of their classes and
  shifts.
- **Hoisting out of rounds.** A value that reads no `carry` is the same in every round, so it is
  computed once. Most classes and shifts are like this.
- **Dead values.** Only tokens, errors and `exit` scans are outputs.
- **Class packing**, as in §3.3.
- **Slot allocation.** Values get register or memory slots by liveness. The interpreter needs this
  to bound its memory (§4.1); the compilers get it from their own register allocators.

## 4. Two execution orders

The same IR runs in two orders. Their only difference is which loop is outside.

### 4.1 Operation-major: the interpreter

The outer loop runs over operations and the inner loop over the blocks of a chunk:

```rust
// One chunk: 256 blocks = 16 KB of source.
for op in &program.ops {           // dispatched once per operation, not once per byte
    match op.kind {
        Kind::And => for b in 0..blocks { slot[op.out][b] = slot[op.a][b] & slot[op.b][b] },
        Kind::Fill => {
            let mut carry = carries[op.carry];          // from the previous chunk
            for b in 0..blocks { (slot[op.out][b], carry) = fill_word(slot[op.s][b], slot[op.e][b], carry) }
            carries[op.carry] = carry;                  // to the next chunk
        }
        // …
    }
}
```

This is vectorised interpretation, as in MonetDB/X100:

- One dispatch covers 256 blocks, so the interpreter's overhead almost vanishes.
- The inner loops are tight and auto-vectorise.
- A slot is 2 KB (256 × `u64`). The Python lexer has hundreds of live values, so slot allocation
  (§3.4) has to keep the working set near L2. A larger chunk means less dispatch but more memory.

The interpreter is the tier that is ready at once. Its cost is memory traffic, not branches: every
operation reads and writes its slots.

### 4.2 Block-major: the compiled forms

The outer loop runs over blocks and every operation is inlined. Masks live in registers; carries
are struct fields. This is simdjson's order. The Rust codegen, Cranelift and MSL all emit this
order. Generated Rust for the start of `json.bitlex` would look like this:

```rust
// Generated from lexers/json.bitlex. One struct per lexer; one field per carry.
pub struct JsonPass0 {
    escaped: OddRunCarry,   // after_odd_run(backslash): one bit
    prev_q: u64,            // the previous block, for prev(…) and starts(…)
    // …
}

impl JsonPass0 {
    #[inline(always)]
    pub fn next<S: Simd>(&mut self, b: &S::Block64) -> Pass0Out {
        // classify: 2 nibble lookups for all 19 classes, then to_bitmask per class
        let c = b.classify(&TABLES);              // simdjson: vqtbl1q_u8 + vtstq + vpaddq
        let backslash = c.get(BACKSLASH);
        let escaped = self.escaped.next(backslash);
        let quote = c.get(QUOTE) & !escaped;
        // …
        Pass0Out { quote, escaped, /* … */ }
    }
}
```

`Simd` is the per-target trait, written once by hand, holding simdjson's ergonomic operations:

```rust
pub trait Simd {
    type Block64;                               // simd8x64: 4×uint8x16 on NEON, 2×__m256i on AVX2
    const STEP: usize;                          // blocks per step (§5)
    fn load(p: &[u8; 64]) -> Self::Block64;
    fn eq(b: &Self::Block64, c: u8) -> u64;     // compare, then to_bitmask
    fn lteq(b: &Self::Block64, c: u8) -> u64;
    fn classify<const N: usize>(b: &Self::Block64, t: &Tables<N>) -> [u64; N];
    fn prefix_xor(m: u64) -> u64;               // PMULL / PCLMUL, or the 6-step shift ladder
}
```

The scans (`fill`, `toggle`, `after_odd_run`) are already `u64` functions in
`tools/bitlex/src/mask.rs`. They move unchanged into `mask/word.rs` as functions of one word and a
carry, and both orders call them.

### 4.3 Rounds as passes, run as a wavefront

A round reads `carry` values that the previous round wrote for the whole file, so a round cannot be
done one block at a time. Each round is a **pass**: a block-major loop over the input.

Passes need not be whole-file sweeps. Chunk `k` in round `r` needs only two things:

- chunk `k`'s own values from round `r − 1`;
- chunk `k − 1`'s carries from round `r`.

So the order is a wavefront: for each chunk, run every round, keeping that round's out-carries for
the next chunk. The chunk stays in L1 or L2 for all its rounds, and the `carry` values between
rounds are one `u64` per value per block, stored for that chunk only.

**Early exit is allowed per chunk, never per byte.** Chunk `k` is settled after round `r` if two
things hold:

- its round-`r` carries equal its round-`r − 1` carries;
- its in-carries for round `r + 1` equal those for round `r`.

Then every later round repeats round `r`, so the loop stops. This branch is taken once per chunk
per round, about once per 16 KB, so a misprediction costs nothing measurable. Errors are read from
the last round only, as now.

Hypothesis lanes stay inside a pass. Every lane runs for every block, and `select` picks per byte
(`08-without-automata.md` §7).

## 5. Sizes per architecture

The block is always 64 bytes. Everything else is per target:

| level | NEON (Apple M) | AVX2 | AVX-512 | portable | Metal |
|---|---|---|---|---|---|
| register | 16 B: `simd8x64` = 4 regs | 32 B: 2 regs | 64 B: 1 reg | `u64` SWAR, 8 B | 1 thread holds a block's `u64`s |
| step | 1–2 blocks | 2 blocks | 2 blocks | 1 block | 1 block per thread |
| group | none | none | none | none | 32 threads = 2 KB (a SIMD group); carries by `simd_prefix_exclusive_*` |
| chunk | 16–64 KB per core | the same | the same | the same | a threadgroup: 256 threads = 16 KB |
| between chunks | a scan over chunk summaries, then a pass per chunk | | | | decoupled look-back with bounded waits (`06-block-parallel.md` §5) |

Notes:

- **Steps on NEON.** simdjson uses one block per step on arm64. Bitlex blocks do far more work than
  simdjson's stage 1 (§8), so two blocks per step may pay off: the two blocks' classify and logic
  overlap, and only the carries are serial. This is a measurement, not a guess.
- **Chunk size depends on the program.** The interpreter wants the largest chunk that keeps its
  live slots in L2. The compiled forms want a chunk whose carries for every round fit in L1.
- **Tails.** The last block is padded, and every mask is trimmed at `n`, as `Mask::trim` does now.

## 6. The four executors

### 6.1 Interpreter (CPU)

- It reads the IR, allocates slots and runs §4.1.
- It replaces nothing: the whole-file evaluator stays as the oracle.
- It is the first tier for any lexer converted at run time.

### 6.2 Generated Rust (ahead of time)

- `bitlex gen LEXER > json_gen.rs` emits §4.2's structs.
- The output is checked in or produced by `build.rs`, and compiled with the engine.
- The `Simd` trait has an `arm64` module, an `x86` module (AVX2, with AVX-512 later) and a
  `portable` module.
- This is the tier for the grammars the engine ships with (JSON, Python): built ahead of time, so
  there is no compile cost per process. Plan ids (§7) still name it.

### 6.3 Cranelift (at run time)

The final shape's CPU JIT is Cranelift 0.125.4 (`transducer/packed/native_cpu.rs`). Today it emits
scalar `I32`/`I64` code only. Every vector operation Bitlex needs exists in its instruction set:

| need | Cranelift instruction |
|---|---|
| nibble lookup (`vqtbl1q_u8`) | `swizzle` on `i8x16` |
| compare to a byte | `splat`, `icmp` |
| `to_bitmask` | `vhigh_bits` |
| `fill`'s add with carry in | `uadd_overflow_cin` |
| `back_fill`'s bit reversal | `bitrev` |
| `compact`'s advance | `popcnt`, `ctz` |
| `select` | `bitselect`, `select` |
| `prefix_xor` | no carry-less multiply, so the 6-step `ishl`/`bxor` ladder |

The generated code has the shape of §4.2: one function per pass, taking a `#[repr(C)]` context of
pointers and carries, in the style of `NativeTaskContext`. The background build and swap is
`compilation.rs`'s, keyed by the plan id. What compile cost buys has to be measured, because
prior work found that compile time must amortise (§9).

### 6.4 Metal

Two steps, as for tree transducers (`spec-transducer.md` §10):

1. **An interpreter kernel.**
   - One thread runs one block, and the loop runs over the IR's operations.
   - Every thread executes the same operation at the same time, so the `switch` is uniform: it does
     not diverge, and its cost is shared by 32 lanes.
   - Scans need a SIMD-group scan and, past 2 KB, a threadgroup barrier. These come only at scan
     operations.
2. **Generated MSL.**
   - It follows the pattern of `metal_register_codegen.rs`: source text compiled with
     `new_library_with_source` and cached by digest (the Metal crate's `runtime.rs`).
   - Each pass becomes one kernel. With early exit per chunk, a pass group is one dispatch with a
     bounded loop over rounds inside it.

Input comes from the residency slabs: the source bytes are already there, files one after another
(decision 0032). Every scan is segmented at file starts (`06-block-parallel.md` §5.1). The kernels
count against the launch budget (`KERNEL_LAUNCH_LIMIT`). A lexer costs one launch per pass group
for the whole corpus.

### 6.5 What is checked

- Each executor equals the reference evaluator on the full corpora: tokens and error bits.
- Check runs on random small inputs as well, as `spec-tooling.md` asks for tree transducers
  ("interpreter = JIT = Metal").
- Each block boundary is tested by splitting the same file at every offset in a window, so every
  carry crosses an edge at every phase.

## 7. Through the transducer pipeline

The final shape's transducer (`transducer/packed.rs`) walks a tree. It reads tape nodes, forks
paths, and keeps cursors and repeat counters. It has no byte input, no registers and no arithmetic
(`research/transducer-ir.md` §0).

Bitlex needs shifts, carries and per-byte integers. Adding those as opcodes to the tree walker would
break its rule 3 ("a new operation must earn its place"). It would also put two machines with
nothing in common behind one decoder.

So the stream program is a **second program kind**, as `01-branch-free-design.md` §4 proposed.

What it shares with tree transducers:

| shared | today, for tree transducers | for stream programs |
|---|---|---|
| plan identity | `PlanId::of_programs`, SHA-256 over digests (`compilation.rs`) | the digest of the IR, so all four bindings name one plan |
| interpreted first, JIT in the background | `resolve_plan` and a `rerail-compile` thread (decision 0012) | the same function, with a new binding kind |
| backend selection | `ExecutionBackend` | the same enum |
| text form and verifier | `rir`, `Program::validate` | a printed form of the IR; a verifier that checks carry types and the absence of data-dependent loops |
| Metal pipeline cache | `rerail-metal`'s `runtime.rs`, keyed by source digest | the same |
| launch budget | `metal_launch_budget.rs` | the same, one launch per pass group |
| device input | residency slabs | the same slabs, source bytes only |
| bitset steps | `SetStep` (`metal_set_steps.metal`): `And`/`Or`/`AndNot`/`Fill` over `u64` slots, one dispatch with barriers | the stream program's logic is a superset: shifts and scans added; the slot layout and barrier pattern can be copied |
| automaton scan | open item O66: chunk maps composed by a scan, for device regex | `scan_compose` is the same operation; one implementation serves both |

Where the two kinds meet: a stream program's output is a tape (`01-branch-free-design.md` §2,
`TapeImage::finish`), and tree transducers read tapes. The tape is the interface, so nothing has
to be fused across the two kinds. Later, a stream program could also emit facts directly, such as
token masks for text search, which is a separate decision.

Copying into the engine follows the current stage's rule: copy into place, then mutate.

- `tools/bitlex` stays the lab: parser, evaluator, IR, interpreter and code generator.
- When the IR and the interpreter equal the evaluator on the corpora, they are copied into
  `native/src/code_rule_dsl/` beside `transducer/`, as `stream/`. This happens on a sibling
  branch, never in the wolfeel worktree.

## 8. How much work per block

The evaluator now counts expression nodes as it runs; `bitlex tokens` prints the count on its
`ops per block` line. Each node runs once over every block of the file, so the count is the number of
mask operations per 64-byte block. It includes rounds as run, with the evaluator's early exit:

| lexer, file | blocks | operations per block |
|---|---|---|
| JSON | any | 123 |
| Python, a one-line file | 1 | 963 |
| Python, `encodings/koi8_r.py` | 216 | 963 |
| Python, `importlib/simple.py` | 6 | 963 |
| Python, `shutil.py` | 949 | 1,410 |
| Python, `asyncio/tools.py` | 164 | 1,558 |
| Python, `threading.py` | 950 | 1,780 |

What follows from it:

- **JSON is simdjson-sized.** 123 operations, most of them single `u64` instructions, is near
  simdjson's stage 1. Compiled block-major, it should run at GB/s.
- **Python's 963 is one pass over everything.** That is where each round converges in one round.
  Files above it took extra rounds.
- **Before CSE and hoisting.** The counts are with `f1`/`f2` expanded separately, nothing hoisted
  out of rounds, and every hypothesis lane evaluated in full. §3.4's passes should remove a large
  part. How large is the first measurement on the IR.
- **A rough bound.** 1,000–1,800 `u64` operations per 64 bytes, at 3–4 per cycle on an M-series
  core, is about 5–9 cycles per byte: 350–700 MB/s per core. Tree-sitter parses Python at tens of
  MB/s. So the compiled form has room even before optimisation.
- **The interpreter's bound** is set by memory traffic: about 1,400 operations × 2–3 slot reads
  and writes × 8 bytes per block. That is roughly 30 bytes of L1/L2 traffic per source byte.
  Expect 100–200 MB/s per core from the chunked interpreter, against 3 MB/s from the evaluator.

The bounds above are estimates from operation counts, not measurements of built executors. CPU
time is measured when each executor exists.

## 9. Prior work and the decisions it bears on

The prior-work indexes of the ast-index-spike (`prior-work/ast-index-spike-design-index.md`,
`prior-work/ast-index-spike-history-index.md`) were searched for JIT, code generation, Metal, SIMD
and transducers.

- **Compile cost must amortise.**
  - PostgreSQL's LLVM JIT cost 1.19 s against a 0.4 s plan (dead end D58).
  - For the planning statement it was 10× worse: 286 vs 2,935 ms (D59).

  A lexer is compiled once per grammar and process, and runs over every file, so it amortises.
  Shipped grammars are compiled ahead of time (§6.2) and pay nothing. Cranelift is for grammars
  converted at run time, and its compile time is reported apart from execution.
- **Specialised code beats a generic interpreter.** Specialised per-query SQL beat a generic
  interpreter relation by 12× (S250, dead end D73). The history index warns that a GPU kernel
  interpreting patterns faces the same cost (§2.23).

  This is why the interpreter dispatches per operation per chunk, not per byte (§4.1), and why
  compiled tiers exist at all. It is also why the Metal interpreter is the first step, not the
  last.
- **Dispatch dominates tiny kernels.** About 120 of 153 ns per native call was SQLite dispatch
  (S220). Hence the corpus runs as one stream, with one launch per pass group, not per file.
- **No prior evidence on SIMD byte lexing or GPU kernels.** Both indexes say so: "Data flow and GPU
  are the known gaps"; GPU and SIMD are "essentially absent from the spike". The choices in §4 and
  §5 rest on simdjson's published design and on this document's measurements.
- **Engine docs bind the shape.**
  - "Executors (interpreter, JIT, Metal) are swappable behind the same IR" (`spec.md`).
  - "Every admitted program runs on every backend. There is no fallback from Metal to CPU/JIT"
    (`spec.md` §8).

  A stream-program operation therefore exists on all four executors or not at all. Fallback to
  tree-sitter is per file and on error bits only. It is not a backend fallback.

These belong in a decision record when the work moves into the engine. That record would cover the
stream program as a second program kind, its four bindings under one plan id, and its operation
set.

## 10. Open questions

1. **Slot pressure.** Does the Python program, after CSE and hoisting, fit 16 KB chunks in L2? If
   not, interpret at 4 KB, or split a pass into sub-passes by liveness.
2. **Lanes.** Run all hypothesis lanes in every block, or only those a segment can reach? The first
   is branch-free; the second needs a compaction. Measure the waste first.
3. **`frame` and `since` on SIMD.** Matching by depth carries the open frames' flags. With depth
   bounded by the `count<8>` width, this is a shift register of flags per block. Its cost needs a
   prototype.
4. **NEON step 1 or 2.** Measured, as §5 says.
5. **Metal chunk per thread.** One block per thread maximises parallelism but makes every scan a
   SIMD-group scan. Four blocks per thread make scans cheaper and cut occupancy. Measure on the
   data-layer corpus as one stream.

## 11. Proposed sequence

1. **The IR and its expansion** from the AST, with a printer. Check: the IR run whole-file equals
   the evaluator on all corpora.
2. **CSE, hoisting and slot allocation.** Report operations per block before and after.
3. **The chunked interpreter**, with chunk-edge tests at every offset. Measure CPU time on the
   corpora.
4. **Rust codegen**, with the `Simd` trait for `arm64` and `portable`, for JSON first, then Python.
   Measure against the interpreter and against tree-sitter.
5. **Cranelift**, from the same IR, behind `compilation.rs`'s swap once copied into the engine.
6. **Metal**: the interpreter kernel, then generated MSL.
7. **The token and tree stages** to tapes (`03-bitlex.md` §6), as stream-program operations from
   the start.
