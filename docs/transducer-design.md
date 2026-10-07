# Tape transducer language

Status: redesigned language proposal. This replaces the previous VM design.

## Scope

A transducer reads a syntax tape and produces match records with captures.
It describes selection, tests, navigation, alternatives, and output. It is not a general-purpose VM.

There are no registers, arithmetic instructions, calls, stack operations, mutable memory, or host callbacks.
Storage, SQL rule composition, scheduling, and backend implementation are outside the language semantics.
The implementation section records the current CPU and Apple Silicon backends.

## Input and output

Input is an immutable tape, optional source bytes, and an ordered set of starting nodes.
A tape node exposes kind, incoming field, flags, byte span, parent, and ordered children.
Node identity is its tape identity and ordinal, not its byte span.

Each starting node begins an independent path at instruction zero.
A path consists logically of an instruction position, a current node, and an ordered capture sequence.
These are transducer semantics, not a prescribed runtime data structure.

A test either retains or rejects the path.
Navigation changes its current node. A navigation with several results creates one path per result.
Alternatives create independent paths with the same current node and captures.
Only `EMIT` produces an output record.

An output record contains the starting node, output label, and ordered captures.
Each capture contains its label, node identity, and half-open UTF-8 byte span.
Repeated capture labels append values. They do not overwrite earlier values.
Rejected paths produce no output, including their captures.

## Exactly one word per instruction

Every instruction occupies exactly **32 bits**:

```text
31                    16 15          8 7           0
+-----------------------+-------------+-------------+
|      operand:u16      |   mode:u8   |   opcode:u8 |
+-----------------------+-------------+-------------+

word = opcode | (mode << 8) | (operand << 16)
```

There are no extension words, operand words, escape prefixes, or variable-length instructions.
The next instruction is always the next word.
Instruction positions count words, not bytes.
Serialized words use little-endian byte order.

Most arithmetic is 16-bit: instruction positions, local table indices, labels, and small numeric comparisons.
Checked arithmetic rejects overflow. It never wraps into another instruction or table entry.
The encoding supports at most 65,536 instructions and 65,536 entries in each local table.
An oversized program produces an explicit compilation error, not truncated code.

Tape positions and source spans are exceptions. Large inputs retain their native-width positions through the tape iterator.
A file with more than 65,535 nodes is not truncated or divided by the instruction encoding.
This does not require general wide arithmetic in the language.

## Tables

Each transducer has four local tables:

| Table | Contents |
| --- | --- |
| Strings | Arbitrary-length byte strings. Capture and output names use UTF-8 entries. |
| Predicates | Pure text tests: equality, prefix, or regex, with a string index and declared text semantics. |
| Labels | Capture and output names, each referring to a string entry. |
| Long integers | Unsigned 64-bit constants for numeric tests that exceed 16 bits. |

String lengths and backing-storage offsets are not limited to 16 bits.
Strings can contain embedded NUL bytes. Neither instructions nor table indices contain inline string data.
Regex entries identify their regex dialect and flags. An unsupported dialect is an error, not a failed match.

The `SIZE` instruction has a long-constant mode.
In that mode, its 16-bit operand indexes the long-integer table instead of holding the number directly.
It remains one instruction word.
Other instructions have no implicit long mode.
Tables contain data, not hidden instruction sequences or arbitrary executable expressions.

## Instructions

An instruction normally continues at the next word.
Unused mode and operand fields must be zero.
Kind and field operands use the input grammar's IDs. Field zero means no incoming field.

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

Flags use bits `0=named`, `1=extra`, `2=missing`, and `3=error`.
All other flag bits are reserved.
A flag cannot be both required and forbidden.
Node selection mode is `0=all nodes` or `1=named nodes`.
`NEXT named` skips anonymous siblings, rather than rejecting at the first anonymous sibling.

`SIZE` comparison modes are `0=equal`, `1=at least`, and `2=at most`.
Bit 7 selects a long-integer table reference. All other mode bits are zero.
For example, `SIZE at-least,long 3` compares against `longs[3]`.
The span length retains its native width during comparison. It is never narrowed before the test.

## Alternatives, repetition, and ordering

`FORK` and navigation enumerate paths. They do not change another path's captures.
A path does not expose an undo stack or a register environment.
An implementation can share immutable prefixes without changing these semantics.

Logical output order is depth-first path order:

1. Starting nodes follow their supplied order.
2. `CHILD` follows sibling order. `DESCEND` follows preorder.
3. `FORK` follows its next-instruction path before its target path.
4. Captures follow the order of `CAPTURE` instructions along that path.

Parallel execution must preserve that observable order.
Different successful paths remain different outputs, even if their captures are equal.
The language does not silently deduplicate matches or choose a longest match.
A query frontend must account for its own selection semantics rather than assume they match this language.

A repeated sibling match uses `FORK`, `NEXT`, and a backward `GOTO`.
For example, this fragment emits every nonempty prefix of consecutive named identifiers:

```text
0  KIND identifier
1  CAPTURE item
2  FORK 5
3  NEXT named
4  GOTO 0
5  EMIT sequence
```

For siblings `a, b`, it emits `[a,b]` before `[a]`.
The shorter path never receives the longer path's capture of `b`.

A valid cycle must advance through a finite tape domain.
For this fragment, every return to instruction zero requires a later sibling.
A cycle containing only tests, captures, and branches is invalid.
Parent/child oscillation is also invalid. A changed node alone does not prove progress.
Compilation rejects cycles without a monotonic traversal proof.
Zero-width nodes still advance by ordinal.

## Worked example

Select Python calls whose direct `function` child is the identifier `eval`.
Capture the call and its function child, then emit a `match` record.

For this example, resolved IDs and tables are:

```text
kind call = 0, identifier = 1
field function = 1
strings = ["eval", "callee", "call", "match"]
predicates[0] = byte-equality(strings[0])
labels = [strings[1], strings[2], strings[3]]
```

These kind IDs are example assignments, not claims about a particular grammar binary.

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
For `eval(x)`, the output captures are `call=[0,7)` followed by `callee=[0,4)`.
For `other(x)`, the text test rejects the path.
For `obj.eval(x)`, the function child is an attribute, so the kind test rejects the path.
An argument child fails the field test regardless of its text.
A repeated matching field creates independent paths. It does not overwrite a capture.

A long-constant example is also exactly one word:

```text
longs[3] = 100000
SIZE at-least,long 3 = 00038105
```

The word contains opcode `05`, mode `81`, and table index `0003`.
The integer `100000` exists only in the table.

## Errors and language boundary

Invalid opcodes, modes, table indices, grammar IDs, branch targets, and non-progressing cycles are errors.
A reachable fallthrough past the last instruction is an error, not an implicit `EMIT` or a wrapped instruction position.
Missing source bytes for `TEXT` are an input error, not a negative text result.
Cancellation or exhausted resources mean incomplete output, never a complete negative result.

This language does not define SQL operators, rule-level negation, binding environments, rewrites, or a general expression evaluator.
It does not claim that arbitrary Tree-sitter queries or ast-grep rules lower directly to these primitives without semantic adaptation.
Those frontends and execution backends remain separate work.

The encoding checks read the example words directly from this document:

```bash
python3 rerail/scripts/test_transducer_design.py
```

They check one-word encoding and its boundaries.
The native implementation is `native/src/policy_dsl/transducer.rs`.
It implements every instruction above, table validation, path ordering, capture isolation, cycle checks, and shared kind dispatch.

## Implemented ast-grep comparison

The first ast-grep compiler accepts exact root-terminal patterns with Smart strictness.
It rejects internal patterns, metavariables, and other strictness modes with `UnsupportedPattern`.
It never calls ast-grep from inside the transducer.

The controlled benchmark compares three match paths:

1. ast-grep-core 0.45.3 over native Tree-sitter trees.
2. The same ast-grep matchers over `TapeDoc`.
3. Compiled fixed-word transducers over the same `TapeDoc` values.

The workload contains eight Python identifier patterns over all 5,725 corpus files.
All paths produced 12,967 ordered ranges with this SHA-256:

```text
c0e5803bcd890944c94f6610fc1061ca780c1e63fc482f02ce2324217161f66f
```

Five measured iterations followed one warmup.
The median matching times were:

| Path | Median | Relative to transducer |
| --- | ---: | ---: |
| ast-grep on Tree-sitter trees | 12,608.58 ms | 35.10 times slower |
| ast-grep on tapes | 817.68 ms | 2.28 times slower |
| transducers on tapes | 359.22 ms | baseline |

Preparation was separate from matching.
Tree-sitter parsing took 8,653.04 ms.
Tape encoding and restoration took 4,574.91 ms.
The source corpus contained 95,697,853 bytes.

The benchmark artifact was recorded during development and is not part of this repository.
This result applies only to the exact root-terminal compiler subset.
It does not establish structural-pattern, metavariable, relation, or full-policy speedups.

## Batch and anchor contract

`execute_batch` accepts many programs and many tapes in one native operation.
Tests and benchmark backends call it; production runs node-test programs one node at a time.
Its result shape is file, then program, then ordered outputs.
Without anchors, each tape gets one shared leading-kind scan.

A caller can supply one ordered root-ordinal list for each file and program pair.
An empty list proves that the pair has no candidate roots.
The executor starts only at supplied ordinals and does not search for replacements.
Lists must contain valid, strictly increasing ordinals.
Invalid lists return `InvalidAnchor`.

Anchor construction is outside transducer execution.
The contract accepts preorder ordinals from the existing anchor index or another sound source.
Missing or incomplete index coverage must use unanchored execution rather than an empty list.

## CPU and Apple Silicon backends

The arm64 CPU JIT emits native comparison code for each supported root-terminal program.
Generated pages are writable during emission and read-executable during use.
Compilation rejects unsupported instructions or constants.
The JIT never calls the interpreter from generated code.

The generic Metal backend reads the same fixed 32-bit instruction words.
Its current capability set is `KIND`, literal `TEXT`, `CAPTURE`, and `EMIT`.
The Metal JIT generates a specialized kernel for the same program batch.
Metal compiles that kernel at runtime.
Both backends reject unsupported programs before command submission.

All four backends consume the same externally supplied anchor lists.
They preserve file, program, root, and capture order.
The parity fixture covers multiple files, multiple programs, empty anchors, and invalid anchors.
It also compares CPU interpreter, CPU JIT, Metal interpreter, and Metal JIT output.

### Full-corpus anchored comparison

The Apple Silicon comparison used 5,725 files and the same eight literal patterns.
The supplied anchors were complete exact kind-and-value postings.
All backends produced 12,967 ordered ranges with the earlier SHA-256.
Five measured iterations followed one warmup.

| Backend | Median | Relative to CPU interpreter |
| --- | ---: | ---: |
| CPU interpreter, no anchors | 380.07 ms | not applicable |
| CPU interpreter, exact anchors | 4.93 ms | baseline |
| arm64 CPU JIT, exact anchors | 5.14 ms | 1.04 times slower |
| Metal interpreter, exact anchors | 49.86 ms | 10.11 times slower |
| Metal JIT, exact anchors | 33.15 ms | 6.72 times slower |

Exact anchors reduced interpreted execution by 77.04 times.
Anchor construction took 301.35 ms and stayed outside execution timing.
Thus, one build plus one execution took 306.29 ms in this local run.
CPU JIT compilation took 0.07 ms.
Generic Metal compilation took 181.02 ms.
The specialized Metal compilation call took 1.74 ms with the system shader cache warm.

GPU medians include packing, buffer creation, submission, synchronization, reads, and ordered output reconstruction.
The selective workload is too small to recover those costs.
Exact anchors also leave too little work to recover each CPU JIT call.
No JIT or GPU speedup is claimed for the exact-anchor execution.

The benchmark artifact was recorded during development and is not part of this repository.
