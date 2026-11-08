# Parser generators: the IR a Bitlex file compiles to

Status: 2026-11-08. This page states exactly what the Bitlex compiler emits and what
the transducer's pipeline receives. It also states the rule every backend is held to.

## Intro

A Bitlex file describes a language's lexer and tree. The Bitlex compiler turns it into
one transducer `Program`. Every executor then runs that program: the interpreters,
Cranelift on the CPU, and Metal on the GPU.

The program has two parts:

- **Tier 1, a block program.** Straight-line operations on bit masks, one bit per
  source byte. It finds the tokens.
- **Tier 2, words.** A state machine over tier 1's token stream. It checks the
  tokens and builds the tree.

Neither part names a language. Each instruction has one meaning, which the interpreter
defines. A backend may only lower instructions: each instruction, by its operands, into
machine code with the interpreter's meaning.

Earlier backends broke that rule. They held a tape format, a width policy and a call
protocol that no instruction states. Those backends were deleted. What they held is
now in the program: outputs at their delays, extracts and totals as tier-1 rows, the
stream each word reads, and the node table (§4.2, §6). A guard refuses any generated
function that calls out or loops over single bytes (§6).

The ladder (§9) checks that this is not JSON's alone. It is thirteen languages, from
one keyword to JSON with comments, each a Bitlex file, each parsed with no Rust
between the bytes and the nodes.

## 1. The path

```
Bitlex file
  -> bitlex_compile        (tier 1 rows, tier 2 words, as rir text)
  -> rir::assemble         (text -> Program; validate)
  -> Program               (digest, plan id, fusion, rir round trip)
  -> native_ir::lower      (Program -> NativeModule)
  -> backends              (each lowers NativeModule's instructions)
```

- The compiler's only output is `rir` text. The assembler and the verifier are the
  transducer's own; a program they refuse does not run.
- `Program` holds tier 1 in `block_programs` and tier 2 in `words`, `switches` and
  `strings`. The digest covers all of it.
- `native_ir::lower` gives each backend the same `NativeModule`:
  - tier 1 as a `BlockNative`;
  - tier 2 as `structural`, one `StructuralBlock` per word.
- The reference evaluator (`tools/bitlex`) runs Bitlex itself, without the compiler.
  It is the oracle for what the compiled program must compute (§7).

## 2. The rule for backends

A lowering is generic when its code depends only on:

1. the instruction;
2. its operands;
3. the target (vector width, lane width).

It is **hardcoded** when the backend holds meaning that no instruction states. Examples:

- a data format, such as a tape's columns and their widths;
- a numbering, such as codes given at first use;
- a policy: buffer sizes, a guess and rerun, when a file goes to tree-sitter;
- a protocol between two programs, such as how a check is called;
- a fused algorithm that several instructions together would state.

What a backend may not do, it may not do in host Rust either. Host code may allocate
the buffers the IR declares, call the function, and hand the buffers over.

## 3. Tier 1: the block program

### 3.1 Values

A value is a mask over the stream: bit `i` is byte `i`. Operation `i` defines value
`%i`. Operands are earlier values, so a program is straight-line with no cycle.

Executors split the stream into blocks (64 bytes on the CPU; 16, 32 or 64 per lane on
Metal). A *carried* operation passes state from block to block. No operation loops
over bytes.

### 3.2 Operations

Here `b[i]` is byte `i`, and the byte before the stream is `0x00`.

| op | rir | bit `i` is | carry |
|---|---|---|---|
| `Eq(c)` | `eq 0x22` | `b[i] == c` | none |
| `Class(t, m)` | `class t0 0x05` | `lo[b[i] & 15] & hi[b[i] >> 4] & m != 0` | none |
| `Pair(u, t, m)` | `pair t0 t1 0x01` | `b[i-1]` is in `u`'s classes, `b[i]` in `t`'s, sharing a class of `m` | none |
| `Const(p)` | `const 0x5555…` | bit `i % 64` of `p` | none |
| `Not(a)` | `not %a` | `!a[i]` | none |
| `And`, `Or`, `Xor`, `AndNot` | `and %a, %b` | `a[i] op b[i]`; `AndNot` is `a & !b` | none |
| `Prev(a, k)`, `k` in 1..=63 | `prev %a, 3` | `a[i-k]`, 0 before the stream | the last 64 bits |
| `Toggle(a)` | `toggle %a` | XOR of `a[0..=i]` | parity |
| `Escaped(a)` | `escaped %a` | byte `i` follows an odd run of `a` | a borrow |
| `Fill(s, e)` | `fill %s, %e` | `s[i] | (f[i-1] & !e[i])` | whether a fill is open |
| `Within(d)` | `within 3` | `d <= i < len + d` | none |
| `Phase(k, r)`, `k` in 1..=63 | `phase 5 2` | `i % k == r` | none |

`Phase` gives `aligned(a, k)` its residues when `k` does not divide 64. For a divisor,
a `Const` pattern repeats every 64 bytes and serves.

A class table is a pair of 16-byte nibble tables (`table T lo=… hi=…`), up to eight
classes each.

### 3.3 Rows

```
block 0 table 0 lo=… hi=…
block 0 %0 = eq 0x22
block 0 %1 = toggle %0
block 0 output %1             (an output at the program's delay)
block 0 output %2 delay 1     (an output with its own delay)
block 0 extract %1            (the positions of an output's set bits)
block 0 delay 2               (outputs are this many bytes late)
block 0 check 0 …             (a token check: a block program of its own)
```

- **Outputs** are the program's results: one mask each.
- **Delay.** Bitlex's `next` reads later bytes. The compiler delays values instead, so
  bit `i + d` of a value at delay `d` is byte `i`'s. The program runs `delay` bytes
  past the stream. Each output names its own delay.
- **Extract** is an output's set bits as ascending `u32` byte positions. These are the
  token stream tier 2 reads: `stream start` and `stream last`.
- **Checks** are block programs over one token's bytes. Their one output, `fail`,
  refuses the token where it has a bit.

The verifier refuses:

- an operand that is not an earlier value;
- an output or extract that names no value (an extract must be an output);
- a table out of range;
- a `prev` past 63, or a delay past 63;
- a check that is not one output at delay 1 or less, with no extract and no check of
  its own.

### 3.4 The native IR

`BlockNative` has the same operations as `BlockInst`, plus two that only lowering
makes: `Plane(k)` (bit `k` of each byte) and `PlaneBefore(k)` (the same, one byte
back). `planes_form` rewrites classes as formulas over bit-planes, for Metal.

## 4. Tier 2: the words

### 4.1 The machine's state

| register | meaning |
|---|---|
| `next` | the next token in `starts` |
| `taken` | the next last byte in `lasts` |
| `start`, `end` | the current token's bytes |
| `limit` | the next token's start, or the file's length |
| `last_end` | the token before's end |
| `at_end` | the stream is used up |
| `nodes` | the nodes written, in preorder |
| `open` | a stack of open nodes |
| `returns` | a stack of return states |

### 4.2 Words

Each word is 32 bits: an opcode, a mode and a 16-bit operand.

| word | does | refuses when |
|---|---|---|
| `TOKEN e` | takes the next start of extract `e`; sets `start = end = s` and `limit` | past the end; `s < end` |
| `SPAN m=0 n` | `end = start + n` | `end > limit` |
| `SPAN m=1 e` | `end = lasts[taken] + 1` from extract `e`; `taken += 1` | no last byte; it is before `start`; `end > limit` |
| `EMPTY t` | — | tier 1's total `t` is not zero (the file goes to tree-sitter) |
| `SWITCH T` | goes to `T`'s target for byte `start + offset`, or to its end target | the target is `refuse` |
| `WORD_IS m=0 S` | — | the token's bytes are not string `S` (8 bytes at most) |
| `WORD_IS m=1 S` | a test: if the token is not `S`, skips the next word | — |
| `CHECK C` | runs check `C` over the token | its `fail` output has a bit |
| `OPEN K` | pushes a node of kind `K` at `start` | — |
| `CLOSE m` | pops a node; its end is `last_end` (m=0) or `end` (m=1) | no open node |
| `LEAF K` | a node of kind `K` over the token, size 1 | — |
| `NODE_FIELD F` | the last node's field is `F` | — |
| `NODE_SUPER K` | adds supertype `K` to the last node | — |
| `PUSH P` | pushes return state `P` | — |
| `RETURN m` | pops `P`; goes to `P + m` | no return state |
| `ACCEPT` | the root ends at the file's end | not at the end; open nodes but the root; returns left; lasts unread |
| `GOTO P`, `STOP` | jump; refuse | `STOP` always |

`TOKEN` and `SPAN m=1` name the extract they read, so the stream is the program's,
not a convention of the backend. `EMPTY` reads a **total**: a block row
`total %v` asks tier 1 for the number of set bits of output `%v`. The compiler makes
each fallback output a total, and the words begin with an `EMPTY` per total. So the
decision to send a file to tree-sitter is the program's own first words, not a
caller reading outputs.

The nodes go to a **node table**, one row per node in preorder, with columns kind,
field, start, end and size, and a second table of supertypes. Each backend writes
the same table. It is sized from the program: the longest path between two
`TOKEN`s, times the number of tokens.

Its tables:

- **Switches:** a target for each of the 256 byte values, an end target, and a byte
  offset. Written as `switch N 0xLO-0xHI TARGET`.
- **Strings:** `WORD_IS`'s strings.
- **Checks:** the block program's checks.

The verifier refuses any other word, operands out of range, and any cycle without a
`TOKEN`. So the machine's only loop is over the stream.

`WORD_IS m=1` is for keywords that share a first byte with the row's other tokens.
For `null` and a word `n…`, the `n` arm reads:

```
SPAN m=1                 ; the token's end, from tier 1
WORD_IS m=1 "null"       ; not null: skip the GOTO
GOTO @null_arm
CHECK word               ; any other n-token is a word
LEAF word ...
```

A keyword the state has no arm for jumps to a `STOP`. Its token has the keyword's
kind, so the word's arm must not take it.

## 5. What reaches the transducer

| `Program` field | from the compiler | in the digest | in `NativeModule` |
|---|---|---|---|
| `block_programs[0]` | tier 1 rows | yes | `block_programs[0]`, a `BlockNative` |
| `.checks` | the `check` statements | yes | `BlockNative::checks` |
| `words` | tier 2 | yes | `structural` |
| `switches` | tier 2's switches | yes | `switches` |
| `strings` | keywords | yes | `strings` |

Output names, such as `fallback`, `stream start` and `stream last`, are the compiler's
labels for outputs (`Compiled::labels`). They are not in the program. A caller finds
an output by its label.

## 6. Generic or hardcoded: where things stand

| part | interpreter | Cranelift | Metal | generic? |
|---|---|---|---|---|
| tier 1 operations | yes | per op | per op | yes |
| output delays | its own undelay | in the loop: a held word per delayed output | `write_output`: two atomic ORs per word | yes |
| extracts | yes | in the loop: eight set bits at a time | `count_sink`: a look-back sum, then the positions | yes |
| totals | yes | a running count | `count_sink`; the file's last group writes it | yes |
| tier 2 words | yes | per word, one function | none | Metal open |
| `CHECK` | runs the check | the check's block body inlined, a loop over 64-byte blocks | none | Metal open |
| fallback | `EMPTY` words | `EMPTY` words | tier 1's totals | yes |

The CPU interpreter's undelay is how the interpreter is written: it is the
reference, not a backend.

**The compiled path** (`native_cpu_stream::Parser`) is two generated functions: tier
1's, then tier 2's, reading tier 1's buffers as tier 1 wrote them. No Rust runs
between the file's bytes and the node table.

**The shape guard** (`native_cpu_shape`). Each generated Cranelift function is
checked before code generation, and refused if:

- it has a call instruction of any kind;
- it has a loop the lowering did not name. Each loop head is named with what one
  turn steps over: a file of the batch, a 64-byte block, eight set bits of a mask,
  or a token. With the named heads taken out, the graph must have no cycle.

So a loop over single bytes cannot be generated, and neither can a call out to the
host.

**Still open.** Tier 2 on Metal. The words lower the same way (one block of code
per word, a loop over the stream), but nothing generates it yet.

## 7. Checking generated code: the corpus

Each executor must compute what the Bitlex file means, not only what the interpreter
says. The corpus checks both. It is in
`native/src/code_rule_dsl/transducer/packed/bitlex_corpus/`, one Bitlex file per
program, from one construct alone up to combinations of many.

For every file and every input:

- **Tier 1.** Each `mask`, from `tools/bitlex`'s evaluator, against the same output
  on:
  - the CPU interpreter;
  - Cranelift;
  - the Metal interpreter at 16-, 32- and 64-byte lanes;
  - generated MSL at each width, by lookup and by planes.
- **Tier 2,** for a file with a tree. The evaluator's nodes (`tools/bitlex::tree`)
  against the words' interpreter over the interpreter's stream. A file the evaluator
  sends to tree-sitter must be refused too.

The parameters it covers:

- every operation, alone and in combination;
- every `prev`, `next` and `span` distance, 1 to 63;
- classes:
  - one byte;
  - ranges;
  - high bytes and `0x00`;
  - up to many products, so several tables;
- delays:
  - a value at each delay;
  - outputs at different delays in one program;
- carried state: none, one word, and more than one;
- counts of each bound, and frames;
- tokens, keywords, `skip`, errors and escapes;
- trees: nodes, nesting, choices, supertypes, fields and checks.

Where it stands (2026-11-08):

- 580 tier-1 files on 220 inputs each: the interpreter equals the evaluator on every
  mask, and Cranelift equals the interpreter bit for bit. The Metal executors run by
  name (`corpus_metal_equals_the_interpreter`), since Metal takes about twelve
  minutes to compile generated MSL for the whole corpus. All nine Metal executors
  equal the interpreter on every file.
- 54 tree grammars over brackets, separators, atoms, keywords, checks, fields and the
  root's form. All 54 compile. On 2,947 documents each writes the reference's nodes,
  and sends a document to tree-sitter (1,589 of them) exactly where the reference
  does.

What the corpus found, all fixed:

- **`next` of a `prev`.** The compiler took `prev` of a clean value to be clean, so
  `next(prev(digit, 5), 5)` gave `digit` where the stream ends early. Its last bits
  are past the end, so `prev` is dirty, and `next` cleans it.
- **`aligned(a, k)` for `k` not dividing 64.** It was refused, because the residues
  are not 64-byte patterns. They are now `phase k r`.
- **`phase` on the interpreter** used a block's number for its position.
- **Stray lane bits on Metal.** At 16-byte lanes, `toggle(prev(…, k))` for `k`
  from 25 to 31 counted bits above the lane's 16, which Metal's compiler kept in
  the register after the conversion to `ushort`. The carry helpers now mask a
  lane to its width (`lane_bits`) before a shift or a parity.
- **An empty root.** The grammar refused a root that may be empty, such as
  `value*`. A file with no tokens goes to tree-sitter, as in the reference, so the
  root may be empty.
- **Keywords sharing a first byte with other tokens.** The compiler refused them.
  They now compile to `WORD_IS m=1` tests (§4.2). The first version let a keyword
  with no arm in the state fall through to the word's arm, so `true: 1` parsed as
  a pair where `true` cannot be a key. It now refuses.

The inputs:

- every length to 200;
- lengths around 64-byte blocks;
- lengths around Metal's group spans (512, 1024, 2048) and twice them;
- random bytes;
- bytes that make carries long: backslash runs, quotes, nesting.

## 8. Extras

An **extra** (`extra comment`) is a token kind the grammar does not name. It may
come between any two tokens, as tree-sitter's comments do.

**Where it goes.** tree-sitter's parser puts an extra in the lowest node that holds
the tokens on either side of it, after the nodes that end before it. At either end
of the file it goes in the root, which then starts at the first extra. The reference
(`tools/bitlex::tree`) parses without the extras, then places each one so.

**In tier 2.** An arm closes the nodes that ended at the token before, then opens
the ones that start at its own token. So where an extra goes depends on which arm
comes next, unless every way on from the state closes the same number of nodes. A
pass over the state machine (`grammar::with_extras`) adds the extra's arm where it
does:

```
state S:   ',' -> close, leaf ...     ; both ways on close one node
           '}' -> close, leaf ...
extra in S -> close, leaf comment, to S'
state S':  ',' -> leaf ...            ; S without its close
           '}' -> leaf ...
```

- In state 0 nothing is open yet, so the extra's arm opens the root first.
- A state gets no extra arm where its ways on differ, or where a nested level may
  end or go on. An extra there sends the file to tree-sitter.
- A state gets no extra arm where its tokens begin with the extra's first bytes.
  Inside a string, a `/` starts the string's content; tier 1 makes no comment
  there anyway.

## 9. The ladder: one keyword to JSON with comments

The ladder is thirteen Bitlex files, in
`native/src/code_rule_dsl/transducer/packed/bitlex_ladder/`. Each step adds one
construct to the step before:

| step | the language | what is new |
|---|---|---|
| 1 | `true` | a token, a required keyword |
| 2 | `true` or `false` | a choice |
| 3 | `true, false` | a one-byte token, a repetition |
| 4 | and `null` | a third keyword |
| 5 | `[true, null]` | brackets |
| 6 | `[[true], []]` | a nested node, a supertype |
| 7 | strings | the string state, a prefix XOR |
| 8 | escapes | simdjson's escaped bytes |
| 9 | integers | a `check` over a token's bytes |
| 10 | numbers | the full number check |
| 11 | objects | fields: all of JSON |
| 12 | `//` comments | an extra |
| 13 | `/* */` comments | JSON with comments |

The test (`bitlex_ladder_tests`) knows no language. Each step's documents are drawn
from its own tree grammar, keyword lists and checks, with extras between tokens.
Each is then changed by a byte. On every document these give the same nodes, or all
refuse:

- the reference: the evaluator and its tree parser;
- the interpreters: tier 1, then the words;
- the compiled path: the two Cranelift functions;
- tree-sitter-json, on each file a JSON step accepts.

Tier 1 on Cranelift and on Metal's generated code equals its interpreter. Each step
also has the promised shape:

- tier 1's block body branches only for its loop and the extracts' flattening;
- tier 2 loops only over tokens, and over 64-byte blocks inside a `check`;
- neither function calls anything.

What it found, fixed:

- **A keyword longer than its word.** A row of keywords only took the word's length
  as the token's, so `truee` parsed as `true`. Such a row now reads its token's last
  byte and compares the whole token. JSON's escapes pay for it: one more carried bit.
- **First bytes past twelve free values** were every byte, so two rows clashed. They
  are now found in three-valued logic: still sound, and sharp enough.

