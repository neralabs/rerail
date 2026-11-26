# 0346: The string's state where simdjson keeps it

Numbered 0345 when first written; renumbered past another worktree's 0345.

Status: built (audited 2026-11-23). Decided (the user, 2026-11-17: "we should take the faster form; also do the two
state-placement fixes"; the carried state: "chains only"). Follows 0343 and 0341; amends 0070.

## Intro

**The problem.** Tier 1 makes a string's mask the way simdjson's stage 1 does: the quotes,
the escapes, the toggle between quotes. 0343 measured where those values live, in rerail's
Rust translation of simdjson:

- The quote mask is made in a vector register (`addp`).
- It leaves for a general register, for `& !escaped`.
- It goes back to a vector register for the multiply (`pmull`).

Multiplying the quotes where `addp` leaves them, and taking the escaped ones back out, was
the fastest form on the M5 Max and M3 Max, on every input.

Tier 1 had two more differences from simdjson in where it keeps the string's state:

- **The carries were packed.** The toggle's parity and the escape's borrow were bits of one
  packed word (0070): each block shifted them out and in again. simdjson keeps
  `prev_in_string` and `prev_escaped` in registers of their own.
- **The outputs went through memory.** Each block wrote its outputs to a record. Decision
  0347 takes this up.

## The decisions

**1. A toggle multiplies the raw quotes.** Prefix XOR is linear:

```text
prefix_xor(q & m) = prefix_xor(q) ^ prefix_xor(q & !m)
```

`native_cpu_block::toggle` finds the toggle's operand to be a bitmask's lane `q` ANDed with
terms (an escape, a gate). It then emits:

- the multiply of `q`, as the vector registers hold it;
- the rounds of the bits the terms take out: zero unless a quote is escaped or gated off.

The rounds are off the path from the bytes to the toggle. Without the multiply, or with no
lane under the terms, the toggle is `prefix_xor` as before.

The vendored Cranelift's aarch64 `clmul` reads a vector's low lane in place. A new lowering
rule matches `clmul (extractlane vec 0), y` (and the mirror) and gives `pmull` the vector's
register, with no `fmov` out and back. The fork has the same commit (neralabs/wasmtime
`nera/clmul-v38.0.4`, 85d8b2c1f). Its filetests cover the lowering and its results (the
interpreter, and aarch64 native).

The generated Rust, JSON's main function:

```rust
v238 = v221 & !v12;                              // the escape's borrow, its own variable
v250 = ((v13 as i64).wrapping_shr(63)) as u64;   // the parity: one arithmetic shift
v253 = vmull_p64(v237, -1);                      // the raw quotes, as `addp` leaves them
v251 = v237 & v247;                              // the escaped quotes, taken back out
```

**2. The string's chains have words of their own.** A `toggle`'s carry and an `escaped`'s
borrow each get a carried word to themselves (`Field::own`):

- The toggle's word is its value as made: no shift writes it, and its parity is `sshr #63`.
  Its look-backs read the top bits, as before.
- The borrow's word is the borrow itself.

Every other field stays packed as 0070 decided: the look-backs, the walks' state, and the
`fill`s. A `fill` has no counterpart in simdjson, and tier 1's own rules count one as a
violation (a run belongs to tier 2).

Six own words (the four number-check `fill`s with the two string chains) were measured first.
JSON's loop then carried 7 words where it had 2, and spilled 22 stack loads and stores: the
spills 0070 measured.

## Results

**Correct.**

- `parser_tests`: the escape, toggle, stage 1 (against simdjson) and UTF-8 checks on every
  executor, JSON's vectors, regions, stream and number check: all equal.
- `rr_parser`:
  - `jsons_cpu_records_are_the_interpreters`: 789 files, 1,165,507 tokens the same;
  - `jsons_tier2_split_is_tier1s`: 386 files;
  - `json_loop_against_step_14`: 129 the same, 216 refused by both, none differ.

**JSON's Cranelift loop** (`json_loop_keeps_values_in_registers`), against the budget of 690
instructions and 4 stack loads or stores:

| | Instructions | Stack loads and stores |
|---|---:|---:|
| before (0341, over budget already) | 699 | 6 |
| the toggle alone | 706 | 6 |
| six own words | 709 | 22 |
| the string's two own words alone | 697 | 10 |
| both (this decision) | 704 | 10 |

The two own words hold two more registers across the loop, and Cranelift spills two values
elsewhere. The budget is now 704 and 10. These counts are information, not the reason for
the decision, which follows the reference's shape (0343).

## Amends 0070

0070 packs every carried field first-fit into words. Now a chain of the string (a toggle's
carry, an escape's borrow) is a word of its own, as simdjson keeps it; the rest stays packed.
The guards (`json_loop_keeps_values_in_registers`, `stage1_loop_keeps_values_in_registers`)
check carried words = ⌈packed bits / 64⌉ + the chains' words.

## Left

- **The outputs in registers:** decision 0347.
- **The all-ones multiplicand:** it is moved to the vector registers each block (one `fmov`
  that reads no data).
- `json_compiled_path_equals_the_interpreters` was red before this decision (tier 2 makes 30
  calls of its own where it asserts none): O-json-tier2-own-calls.

## Tests

- `parser_tests`: `json_loop_keeps_values_in_registers`, `stage1_loop_keeps_values_in_registers`,
  `json_state_is_bits`, `escaped_equals_reference_on_every_executor`,
  `toggle_equals_reference_on_every_executor`, `stage1_equals_simdjson_on_every_executor`,
  `json_tier1_vectors_equal_the_interpreter`, `json_gated_regions_equal_the_interpreter`,
  `json_stream_equals_the_evaluator_on_every_executor`, `json_number_check_equals_the_reference`.
- `rr_parser`: `jsons_cpu_records_are_the_interpreters`, `jsons_tier2_split_is_tier1s`,
  `json_loop_against_step_14`.
- The fork: `runtests/clmul.clif`, `isa/aarch64/clmul.clif`.
