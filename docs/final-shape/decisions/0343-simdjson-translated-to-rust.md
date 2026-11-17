# 0343: simdjson, translated to Rust, is the reference

Status: decided (2026-11-17). Corrects 0341's premise. Follows 0337–0341.

## Intro

**The problem.** The generated JSON tier 1 and tier 2 are Rust (the AOT), read beside what
they should become. Until now they were read beside simdjson's C++ (stage 1) and `kernel.inc`'s
C++ walk (stage 2, which writes the tape). C++ beside Rust shows every difference: a different
language, a different compiler (clang against rustc), and different intrinsics. Only some of
those differences are ours.

**The answer.** A Rust crate, `rerail-json-lean` (`native/crates/json_lean`), translates both
line for line:

- `stage1`: simdjson 5.0.5's arm64 `json_structural_indexer::index<64>` and everything it
  calls (`simd8x64`, `classify`, the escape, string and JSON scanners, the UTF-8 checker,
  arm64's own `bit_indexer::write`), with simdjson's names;
- `stage2`: `kernel.inc`'s `walk`, which writes the v9np tape;
- `number`: `kernel.inc`'s `number_end` (simdjson's number check);
- `tables`: simdjson's tables, copied by `scripts/json_lean/tables.py`.

It is a reference, not a parse path: the engine still reads JSON through `rerail-simdjson`.
arm64 only, the implementation translated. JSONC is left out.

## Equal to simdjson: what was checked

**Outputs** (`tests::stage1_is_simdjsons`, `tests::tape_is_the_kernels`):

- over JSONTestSuite, simdjson's `jsonexamples` and made inputs (each length 0..200, depth
  1023–1025, escapes at 32-byte edges, a 70 KB string), 1461 files: stage 1's count and index
  are simdjson's;
- 715 accepted files: the tapes are the kernel's, byte for byte; every refusal is the
  kernel's code.

**Machine code.**

- *Stage 1.* The block's masks are clang's, instruction for instruction (registers aside), when
  built with rustc 1.99.1; see the toolchain note below. The indexer has the same structure.
- *Walk.* The same structure: 8 `backslash_quote` sites in each (4 fast paths, 4 escape
  loops), and 6 `hex_to_u32` sites in each.

**Instructions a byte** (`tests::against_simdjson`, least of 41 rounds; C++ against Rust):

| Input | Stage 1 | Tape (stage 1, the walk, the columns) |
|---|---|---|
| `jsonexamples` | 3.10 / 3.11 | 12.86 / 11.87 |
| long ASCII strings | 2.14 / 2.17 | 3.11 / 3.12 |
| long UTF-8 strings | 3.26 / 3.22 | 4.21 / 4.17 |
| white space | 2.14 / 2.17 | 2.38 / 2.39 |
| numbers | 4.50 / 4.50 | 27.93 / 24.68 |
| short strings | 4.50 / 4.50 | 21.90 / 19.86 |

The Rust walk retires fewer instructions where the walk dominates. Clang keeps more of the
C++ walk's state in stack slots: 547 stack loads against 363 in the Rust.

Cycles are the same to within the machine's noise: the load average was 30–36, and the same
code's cycles moved by up to 30% from one run to the next.

## The decisions in the translation

**A `goto` is a loop, not a label switch.** `kernel.inc`'s walk is a `goto` graph, and Rust
has no `goto` (`become`, the explicit tail call, is unstable on 1.99.1). The first
translation, `loop { label = match label { … } }`, compiled to a switch that every
transition passed: 19 jumps went to one binary search over the labels. LLVM's jump threading
does not thread an edge into a loop header, because the result would be irreducible.

Each container's cycle is now a loop of its own:

- `object_key` to `object_continue`;
- `array_value` to `array_continue`;
- `scope_end`'s closes.

Their transitions are direct branches. The switch is passed only where a container opens or
closes, which is where `kernel.inc` reads `object[d]`.

**Hints in place.** `kernel.inc`'s `simdjson_likely` and `simdjson_unlikely` are
`core::hint::cold_path()` on the unlikely side. `code_of`'s first use is cold in place, not a
cold `#[inline(never)]` helper: the helper's call made the walk spill its caller-saved
registers.

**State in registers.** The walk's scalars are a struct's fields (`Walk`). The `ids` array
the walk indexes is a local of its own, so the struct holds no array: LLVM's SROA does not
split a struct with a dynamically indexed array, and it would keep the scalars in memory.

**Values not made.** As `kernel.inc` (`no_values`), a number is checked and not converted.
A float is still computed where finiteness decides the answer. `parse_float_fallback` is
Rust's `parse::<f64>` (correctly rounded, as simdjson's `from_chars`).

## Corrected: 0341's premise

0341 said simdjson's arm64 `prefix_xor` is one `vmull_p64`. It is not. simdjson 5.0.5's arm64
`prefix_xor` (`native/vendor/simdjson-5.0.5/simdjson.cpp`, line 14306) is the six shift-XOR
rounds, and the `vmull_p64` line is commented out:

```cpp
// We could do this with PMULL, but it is apparently slow.
// …
bitmask ^= bitmask << 1;
bitmask ^= bitmask << 2;
// … 4, 8, 16, 32
```

The rounds use the general registers, which are otherwise idle in stage 1; the multiply
needs two moves between the register files. The translation keeps the rounds, as simdjson.
Whether tier 1 keeps `clmul` (0341's fork) is open, and is the user's decision.

## Measured: the rounds against the multiply

`tests::prefix_xor_forms` runs stage 1 with each form of `prefix_xor` (`index::<PMULL>`; the
two indexes are checked equal in `stage1_is_simdjsons`). It also runs the form alone: its
latency (each mask a function of the one before) and its throughput (none dependent). Each
figure is the least of 61 rounds, three runs each, on an M5 Max at load 9–18 and an M3 Max at
load 22–27.

In stage 1 the quote mask is made in the general registers (`quote & !escaped`). So the
multiply is `fmov d, x`, `pmull`, `fmov x, d`: two moves between the register files, as
simdjson's comment says. The rounds are six `eor x, x, x, lsl #k`.

**Stage 1, instructions / cycles a byte, rounds against `vmull_p64`:**

| Input | M5 Max | M3 Max |
|---|---|---|
| `jsonexamples` | 3.11 / 0.55 against 3.06 / 0.59 | 3.11 / 0.57 against 3.06 / 0.58 |
| long ASCII strings | 2.17 / 0.40 against 2.13 / 0.41 | 2.17 / 0.40 against 2.13 / 0.41 |
| numbers, short strings | 4.50 / 0.90 against 4.45 / 0.86 | 4.50 / 0.98 against 4.45 / 0.93 |

**The form alone, cycles a mask, rounds against `vmull_p64`:**

| | M5 Max | M3 Max | M1 (Firestorm), from Dougall Johnson's tables |
|---|---|---|---|
| latency | 13.0 against 13.5 | 13.0 against 13.5 | 12 against `fmov` + 3 + `fmov` (each move "≤10") |
| throughput | 1.67 against 0.70 | 1.64 against 0.53 | 2.0 against 0.5 |

On the M1, a shifted `eor` has latency 2 and throughput 0.333, and `pmull` (1Q) has latency 3
and throughput 0.25. A move from the vector registers to the general ones has throughput 0.5.

So on every Apple core measured, the multiply's latency is the rounds'. Its throughput is two
and a half to three times theirs. In stage 1 neither form is on the loop's carried path
(`prev_in_string` is), and the difference is within ±6%. The multiply is ahead where the
blocks are regular (numbers and short strings, 4–5%) and behind on the M5's corpus (7%).
Simdjson's arm64 choice (the rounds) costs about nothing on Apple cores. The 3-cycle `pmull`
does not show "slow"; simdjson's comment names the moves on Neoverse N1 and Cortex-A72.

## Toolchain: `addp` needs rustc 1.99.1

simdjson's `to_bitmask` is `and` with the bit weights, then four `vpaddq_u8`.

- clang, and rustc 1.99.1 (LLVM 23), emit `addp`.
- rustc 1.95.0 (the repository's `rust-toolchain.toml`) and 1.97.1 (the AOT's) lower stdarch's
  `vpaddq_u8` (a shuffle and an add) to `uzp1`/`uzp2` and `orr`, or `xtn`/`uzp2` and `add`:
  more instructions a block.

So the translation is read at 1.99.1 (`cargo +1.99.1 test -p rerail-json-lean --profile
profiling`). The generated AOT, built at 1.97.1, has the same lowering against it.

## Tests

- `rerail-json-lean`: `tests::stage1_is_simdjsons`, `tests::tape_is_the_kernels`
  (`RERAIL_JSON_CORPUS`, `RERAIL_JSON_SUITE` add those files); `tests::against_simdjson`
  (ignored: a benchmark, `RR_ROUNDS`).
- `rerail-simdjson-native` gains `Kernel::tape_unhashed`, the kernel's tape with no hash, and
  `rerail-simdjson` gains `tape_unhashed_with`, so the benchmark compares like with like.
- `scripts/json_lean/loops.py` lists an objdump file's loops (backward branches), to find a
  hot loop in both.
