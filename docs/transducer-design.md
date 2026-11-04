# Tape transducer language

Status: a design with a test-only implementation; production rule evaluation does not use it.

<details>
<summary>What is compiled, and where</summary>

The interpreter in `native/src/code_rule_dsl/transducer.rs`, with `execute_batch` and the ast-grep pattern compiler,
compiles only for tests. Its arm64 JIT and Metal backends compile only for tests with the `bench-backends` feature on macOS.
Production uses only that file's per-tape stream of node kinds, stored as a search index.

</details>

## Scope

A transducer reads a syntax tape and produces match records with captures; it is not a general-purpose VM.

It describes selection, tests, navigation, alternatives, and output.
It has no registers, arithmetic instructions, calls, stack operations, mutable memory, or host callbacks.
Storage, SQL rule composition, scheduling, and backend implementation are outside its semantics.

## Input and output

Paths walk an immutable tape; only `EMIT` turns a path into an output record.

- **Tape:** immutable input, plus optional source bytes and ordered starting nodes.
- **Path:** each starting node begins one at instruction zero; it holds a position, a current node, and ordered captures.
- **Record:** the starting node, output label, and ordered captures of a successful path.

<details>
<summary>Nodes, paths, and captures in detail</summary>

A node exposes kind, incoming field, flags, byte span, parent, and ordered children.
Its identity is tape identity plus ordinal, not byte span.
A path's parts are transducer semantics, not a prescribed runtime data structure.

- **Tests** retain or reject the path.
- **Navigation** moves the current node; several results create one path each.
- **Alternatives** create independent paths with the same node and captures.

A capture is a label, node identity, and half-open UTF-8 byte span.
Repeated labels append; they never overwrite. Rejected paths output nothing, including their captures.

</details>

## Exactly one word per instruction

Every instruction is exactly **32 bits**; there are no extension words, operand words, escape prefixes, or variable lengths.

```text
31                    16 15          8 7           0
+-----------------------+-------------+-------------+
|      operand:u16      |   mode:u8   |   opcode:u8 |
+-----------------------+-------------+-------------+

word = opcode | (mode << 8) | (operand << 16)
```

The next instruction is always the next word. Positions count words, not bytes. Serialized words are little-endian.

<details>
<summary>Exact limits</summary>

Positions, local table indices, labels, and small numeric comparisons are 16-bit.
Checked arithmetic rejects overflow; it never wraps into another instruction or table entry.
A program holds at most 65,536 instructions and 65,536 entries per local table.
More is an explicit compilation error, not truncated code.

Tape positions and source spans are the exception: they keep native width through the tape iterator.
A file with more than 65,535 nodes is not truncated or split, and the language needs no general wide arithmetic.

</details>

## Tables

Each transducer has four local data tables, referenced by 16-bit index.

| Table | Contents |
| --- | --- |
| Strings | Arbitrary-length byte strings. Capture and output names use UTF-8 entries. |
| Predicates | Pure text tests: equality, prefix, or regex, with a string index and declared text semantics. |
| Labels | Capture and output names, each referring to a string entry. |
| Long integers | Unsigned 64-bit constants for numeric tests that exceed 16 bits. |

<details>
<summary>Table rules</summary>

- String lengths and backing-storage offsets are not limited to 16 bits. Strings may contain NUL bytes.
- Neither instructions nor table indices contain inline string data.
- Regex entries name their dialect and flags; an unsupported dialect is an error, not a failed match.
- Tables hold data, never hidden instruction sequences or arbitrary executable expressions.
- Only `SIZE` has a long-constant mode: its 16-bit operand indexes the long-integer table, still in one word.
  Other instructions have no implicit long mode.

</details>

## Instructions

Fourteen opcodes cover tests, navigation, capture, branching, and output; each continues at the next word unless noted.

- **Tests** (`KIND`, `FIELD`, `FLAGS`, `TEXT`, `SIZE`) retain or reject the path.
- **Navigation** (`CHILD`, `NEXT`, `PARENT`, `DESCEND`) moves the current node, one path per result.
- **Capture and output:** `CAPTURE` appends the current node under a label; `EMIT` emits one record and ends the path.
- **Control:** `FORK` continues at the next instruction and the target; `GOTO` jumps; `STOP` ends without output.

<details>
<summary>Opcode table and mode encoding</summary>

| Opcode | Instruction | Mode | Operand | Effect |
| --- | --- | --- | --- | --- |
| `00` | `STOP` | 0 | 0 | End this path without output. |
| `01` | `KIND` | 0 | kind ID | Retain the path if the current kind equals the operand. |
| `02` | `FIELD` | 0 | field ID | Retain the path if the incoming field equals the operand. |
| `03` | `FLAGS` | required flag mask | forbidden flag mask | Require every required flag and no forbidden flag. |
| `04` | `TEXT` | 0 | predicate index | Retain the path if the current node's source bytes satisfy the predicate. |
| `05` | `SIZE` | comparison, optionally long | value or long index | Compare the current span's byte length with the constant. |
| `10` | `CHILD` | node selection | 0 | Continue once for each selected direct child. |
| `11` | `NEXT` | node selection | 0 | Move to the next selected sibling, or reject this path. |
| `12` | `PARENT` | 0 | 0 | Move to the parent, or reject this path. |
| `13` | `DESCEND` | node selection | 0 | Continue once for each selected proper descendant in preorder. |
| `20` | `CAPTURE` | 0 | label index | Append the current node under this label. |
| `30` | `FORK` | 0 | target instruction | Continue both at the next instruction and at the target, in that order. |
| `31` | `GOTO` | 0 | target instruction | Continue at the target with the same node and captures. |
| `40` | `EMIT` | 0 | label index | Emit one record and end this path. |

Unused mode and operand fields must be zero.
Kind and field operands are the input grammar's IDs; field zero means no incoming field.

- **Flags:** bits `0=named`, `1=extra`, `2=missing`, `3=error`; others reserved. No flag may be both required and forbidden.
- **Node selection:** `0=all nodes`, `1=named nodes`. `NEXT named` skips anonymous siblings instead of rejecting.
- **`SIZE` modes:** `0=equal`, `1=at least`, `2=at most`. Bit 7 selects a long-integer reference; other bits are zero.
  `SIZE at-least,long 3` compares against `longs[3]`. The span length keeps native width and is never narrowed.

</details>

## Alternatives, repetition, and ordering

Branches enumerate isolated paths, emitted in a fixed depth-first order.

A repeated sibling match uses `FORK`, `NEXT`, and a backward `GOTO`.
This fragment emits every nonempty prefix of consecutive named identifiers:

```text
0  KIND identifier
1  CAPTURE item
2  FORK 5
3  NEXT named
4  GOTO 0
5  EMIT sequence
```

For siblings `a, b`, it emits `[a,b]` before `[a]`; the shorter path never sees the capture of `b`.

<details>
<summary>Ordering rules</summary>

`FORK` and navigation never change another path's captures; there is no undo stack or register environment.
Implementations may share immutable prefixes without changing these semantics.

Output order is depth-first path order:

1. Starting nodes follow their supplied order.
2. `CHILD` follows sibling order. `DESCEND` follows preorder.
3. `FORK` follows its next-instruction path before its target path.
4. Captures follow the order of `CAPTURE` instructions along that path.

Parallel execution must preserve this order.
Distinct successful paths are distinct outputs, even with equal captures: no silent deduplication, no longest match.
A query frontend must handle its own selection semantics rather than assume they match this language.

</details>

<details>
<summary>Cycle rules</summary>

A valid cycle must advance through a finite tape domain; in the fragment, each return to instruction zero needs a later sibling.
Cycles of only tests, captures, and branches are invalid, as is parent/child oscillation.
A changed node alone does not prove progress.
Compilation rejects cycles without a monotonic traversal proof. Zero-width nodes still advance by ordinal.

</details>

## Worked example

This program matches Python calls whose direct `function` child is the identifier `eval`.

It captures the call and its function child, then emits `match`. Resolved IDs and tables:

```text
kind call = 0, identifier = 1
field function = 1
strings = ["eval", "callee", "call", "match"]
predicates[0] = byte-equality(strings[0])
labels = [strings[1], strings[2], strings[3]]
```

The kind IDs are illustrative, not from a real grammar binary.

```text
position  instruction          word
0         KIND call            00000001
1         CAPTURE call         00010020
2         CHILD all            00000010
3         FIELD function       00010002
4         KIND identifier      00010001
5         TEXT 0               00000004
6         CAPTURE callee       00000020
7         EMIT match           00020040
```

Eight instructions occupy exactly 32 bytes.

<details>
<summary>Results for sample inputs</summary>

| Input | Result |
| --- | --- |
| `eval(x)` | Captures `call=[0,7)`, then `callee=[0,4)`. |
| `other(x)` | Rejected by the text test. |
| `obj.eval(x)` | Function child is an attribute; rejected by the kind test. |
| Argument child | Rejected by the field test, whatever its text. |

A repeated matching field creates independent paths, not overwritten captures.

</details>

<details>
<summary>Long-constant example</summary>

A long constant is also one word:

```text
longs[3] = 100000
SIZE at-least,long 3 = 00038105
```

The word holds opcode `05`, mode `81`, and table index `0003`; `100000` lives only in the table.

</details>

## Errors and language boundary

Invalid programs and inputs are errors, and the language stops short of rule-level logic.

<details>
<summary>Errors and exclusions</summary>

**Errors:**

- Invalid opcodes, modes, table indices, grammar IDs, branch targets, and non-progressing cycles.
- Reachable fallthrough past the last instruction (not an implicit `EMIT` or wrapped position).
- Missing source bytes for `TEXT` (an input error, not a negative text result).
- Cancellation or exhausted resources mean incomplete output, never a complete negative result.

**Out of scope:** SQL operators, rule-level negation, binding environments, rewrites, and a general expression evaluator.
The language does not claim that arbitrary Tree-sitter queries or ast-grep rules lower directly to these primitives
without semantic adaptation.

</details>

The encoding tests read the example words from this document and check one-word encoding and its boundaries:

```bash
python3 test/docs/test_transducer_design.py
```

The test-only interpreter in `native/src/code_rule_dsl/transducer.rs` implements every instruction above,
plus table validation, path ordering, capture isolation, cycle checks, and shared kind dispatch.
