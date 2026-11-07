# 0049: Two tiers, as simdjson's two stages

Status: accepted (user, 2026-11-07: "the loop instructions needs to drop by at least an order of
magnitude … You need to implement the essence"; the plan, "okay, I agree with that plan"; the
tier-2 form, "Yes, this form"; numbers, "try them in tier 1 and tier 2. We should have a builtin
set of validators that lower to good code for integers, floats, numbers, etc")

## Intro

simdjson parses in two stages, and each does only what suits it.

- **Stage 1 works on every byte.** It does one classification, the string state and the token
  starts. It carries 3 bits and writes the token positions. On this machine its loop is 150
  instructions per 64 bytes, from the arm64 disassembly of Homebrew's libsimdjson 4.x.
- **Stage 2 works once per token.** It finds the token's kind from its first byte, checks
  numbers and literals, keeps a stack of open containers, and writes the tape.

JSON's block program had been doing stage 2's work as masks on every byte:
- 8 number rules and 3 keyword chains;
- a fill per nesting level;
- depth bit-planes;
- key and value masks;
- token ends.

That is 1939 loop instructions per 64 bytes, whatever the block holds. This record splits a
Bitlex file into the same two tiers.

```
tier 1 (block program, every byte):  classes, strings, token starts and lasts, per-byte errors
tier 2 (token rules, every token):   kinds, text checks, nesting, roles, the tape
```

## Prior work consulted

- **ast-index-spike indexes:** the spike's tape writer works per node, from a parse. Tier 2 is
  the same per-node writer, fed by tier 1's token positions instead of a parse.
- **Lessons ledger #28** (only CPU time counts). The guard counts instructions, which is a
  proxy. Every step is also timed against simdjson's full parse on the same files and load.
- **Doc 12 §3** (simdjson's primitives as a side program of the transducer). **Agrees:** tier 1
  is the block program. Tier 2 is the transducer's per-token side. It is run by the tape
  writer, which is language-independent and reads the rows that Bitlex compiles to.
- **Doc 13 §1–§3** (node rows, the one end rule). **Changed:** nesting comes from the writer's
  stack (`nested max D`), not from count bit-planes and `frame`.

## Decision

1. **Tier 1 may hold only stage 1's work.**
   - Byte classes.
   - One `after_odd_run` and one `toggle`, for strings.
   - Token starts, and token last bytes as `lasts` of a run.
   - Per-byte errors.
   - No `fill`, and no look-back beyond one byte.

   Guard: `json_tier1_is_simdjsons_stage1` requires at most 4 state bits, and a loop of at most
   150 instructions with no stack traffic.
2. **A token's last byte is a mask** (`token K = start S last L`). A run covered by `through`
   needs a fill, which tier 1 does not hold.
3. **Tier-2 rules** in the tree section, run by the tape writer:
   - `node K start S [when C]`: `C` is `after (K | …)` (the previous token's kind) and/or
     `parent K` (the innermost open node). `start K` names another node kind's starts.
   - `end E`, `end with F` (the node ends with its child that has field `F`), `first token` and
     `last token`.
   - `nested [max D]`: the node opens and closes on the writer's stack. An end must close the
     innermost open nested node, and that node must be of its kind. Deeper than `D` goes to
     tree-sitter.
   - `field F of K … = start of K2` and `… when C`: a fact from the node's place among the
     tokens, not from a mask.
   - `keywords K { "w" -> K2 … }`: a token of kind `K` whose text is a listed word is retyped.
     It is one compare of at most 8 bytes per token.
   - The tokens must tile the file, apart from skipped bytes, and no two may start at one byte.
     The writer checks both per token.
4. **The escape primitive is simdjson's `escaped`.** It marks every byte an escape covers,
   including an escaped backslash: in `\\n` it marks the second `\`, and `n` is text.
   `after_odd_run(x)`, the bytes after an odd run, is now Bitlex sugar for `escaped(x) & ~x`.
   Without escaped backslashes, an escape's last byte would need a fill.

   ```
   # The backslashes not escaped start escapes; the bytes escaped end them.
   mask escaped  = escaped(backslash)
   mask esc      = backslash & body & ~escaped
   mask esc_last = escaped & body
   ```
5. **Built-in validators**, `check K is V` (`integer`, `float`, `json_number`, …), each with two
   lowerings:
   - tier 1: masks;
   - tier 2: a per-token check over the token's bytes, in 8-byte SWAR chunks.

   Both are built for JSON's numbers, measured, and the faster one is kept.
6. **The CPU writer is tier 2's executor.** It is checked against tree-sitter's tapes on the
   corpus. On the GPU the same rules run in parallel per token: depth by a prefix sum, the
   parent by a nearest-smaller search, the previous token by index.

## Steps

1. Red guards: done (`9db221bd9`).
2. Stage 1's codegen: after tier 2. Cranelift has no `cmtst`, `pmull` or `tbx`. Stage 1 without
   UTF-8 is 186 instructions, against simdjson's 150 with four index writes. Whether to write our
   own emitter is decided after measuring.
3. Tier-2 rows in the format (`BlockProgram`, `rir`, digest, verify), the syntax, the compiler,
   and the writer.
4. JSON's Bitlex in two tiers. Its tapes must equal tree-sitter's on the corpus, and the tier-1
   guard switches on. Done, except the loop size (see Results).
5. Number validators in both tiers, timed against each other.

## Results so far

JSON in two tiers, on the 774 data-layer files: 771 tapes equal tree-sitter's, and 3 go to
tree-sitter. Two of those nest deeper than 13; tree-sitter finds an error in the third. None
differ.

| JSON tier 1 | one tier | two tiers | one structural class | stage 1's target |
|---|---:|---:|---:|---:|
| byte classes | 16 | 16 | 5 | 5 |
| operations | 483 | 65 | 41 | |
| state bits | 104 | 4 | 4 | 4 |
| loop instructions | 1939 | 483 | 253 | 150 |
| loop stack loads and stores | 164 | 6 | 0 | 0 |

- **An unterminated string** is no longer a per-byte error. Its `string` node has no end, so
  the writer refuses the file, as simdjson finds it from stage 1's last carry.
- **One class for the six structural bytes**, as simdjson's `op`. A token's kind is its
  keyword's, read from its text when it starts. So `after`, `parent`, `start '{'` and
  `end '}'` name kinds, not rows. A token row whose keywords are `required` has kind 0: a
  token that is no keyword sends the file to tree-sitter.

  ```
  token punctuation = start code & op
  keywords punctuation { "{"  "}"  "["  "]"  ","  ":" } required
  node object start '{' end '}' nested max 13
  ```
- **Escapes are checked per token**, the same way: `escape_sequence`'s keywords are
  tree-sitter-json's nine, `required`.
- **Bytes no token holds** are in a word: `word = code & ~ws & ~op`. A comment's `/` or a
  non-ASCII byte outside a string fails the number check, and the file goes to tree-sitter.
  That replaces three per-byte rules and their classes.

The loop's remaining excess over stage 1 is the code Cranelift makes around the program, not
the program itself:
- the `within` masks: about 45 instructions per block, all ones on every block but the last;
- an address `add` per output store and per load;
- the movemask weights, rebuilt each block.

### CPU time

Over the 774 files (615,787 blocks), best of 3 or 5, paired runs, load 24 to 33:

| stage | ours | simdjson |
|---|---:|---:|
| tier 1 / stage 1 | 14.0 to 15.6 ns per block | 12.7 ns per block |
| tier 2 / stage 2 | 7,300 to 7,600 ns per block | about 12.3 ns per block |
| both | | 25.0 ns per block |

- **Tier 1** is Cranelift's code, within 1.2x of stage 1. Before two tiers it was 114 to
  140 ns per block. Stage 1 also validates UTF-8, which tier 1 does not yet.
- **Tier 2** is the CPU writer, built in debug. It is 600x simdjson's stage 2:
  - it tests every row at every token;
  - it tests skipped bytes one at a time;
  - it allocates small vectors per token.

  It is correct, and checked against tree-sitter, but it is not yet stage 2's shape.

### Tier 1's codegen

The loop's code around the program, fixed in the Cranelift lowering (`native_cpu_block.rs`):

- **Full blocks get their own body.** A block wholly inside the file, and not the first, has
  every `within` all ones. Its body is the hot loop. The first block and the last ones take the
  general body. A head block picks one per block.
- **The pointers are induction variables.** Every load and store is `[pointer, #offset]`, with
  no address arithmetic per access.
- **The movemask weights and nibble mask are made once**, before the loop.
- **Bit logic folds as it is emitted.** A constant folds. A class stays complemented, as its
  `cmeq #0` makes it, until a use needs it plain: an `and` with it is one `bic`, and a `not` of
  it is free.

| JSON tier 1 loop | before | after |
|---|---:|---:|
| instructions | 253 | 178 |
| stack loads and stores | 0 | 0 |

Paired CPU timings, load 42 to 59: tier 1 takes 9.5 to 10.4 ns per block; simdjson's stage 1
takes 9.0 to 11.0. Stage 1 also validates UTF-8 and writes indexes.

The 28 instructions left over the target need instructions Cranelift lacks:
- `cmtst`: one instruction per class test, not an `and` and a `cmeq #0` (8 per block);
- `pmull`: the string toggle's prefix XOR in about two instructions, not thirteen.

### Tier 2 is generated code

The CPU writer (`tape_writer.rs`) interprets rows, and it is 600x simdjson's stage 2. That was
the wrong design. Tier 2 is generated from the Bitlex file, as tier 1 is: Cranelift on the
CPU, MSL on the GPU. A Rust version stays only as a test oracle.
