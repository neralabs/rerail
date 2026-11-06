# Parser generators: a branch-free path from source to tape

Status: design proposal, 2026-11-06. Nothing is built yet. Measurements and the fidelity hazards
are in `00-problem.md`.

## Intro

simdjson is fast because it does not branch on its input. Each byte becomes a few bits of fact.
Facts are combined with logical operations and prefix scans, and output positions come from
prefix counts. The work per byte is the same whatever the bytes are.

This page applies that rule to the whole path, from the source bytes to Rerail's tape image,
including the tape. It shows that, once each node's start, end and code are known, the five v9np
columns follow from one prefix sum and a few gathers. It then shows that the starts and ends come
from one kind of query, the nearest smaller value, which can itself be computed as a prefix scan.
It lists the primitives a stream transducer needs, works through JSON, and sketches Python.

## 1. What "no branching" means here

- **Allowed:**
  - loops whose trip count is a size: blocks of a file, the popcount of a mask, the levels of a
    scan;
  - selects (`a & m | b & ~m`), table lookups, gathers and scatters;
  - one decision per file: if any error bit is set, the file goes to tree-sitter.
- **Not allowed:**
  - a branch per byte or per token that depends on the data;
  - recursive descent;
  - an LR automaton's shift/reduce loop;
  - simdjson's stage-2 state machine.

A loop with a data-dependent trip count is a branch. So "pop until the stack top is smaller" is
not allowed either. Section 3 replaces it.

## 2. The tape follows from extents

Describe each node by its *extent*: its start byte, its end byte, its folded code, and a rank
among the nodes that start at the same byte, outermost first. The columns then follow, with no
tree walk:

```text
n[p]   = number of nodes starting at byte p        (a per-byte count)
C[p]   = n[0] + … + n[p−1]                          (one exclusive prefix sum)

ord    = C[start] + rank                            (the node's preorder)
size   = C[end] − ord                               (nodes starting inside it, minus its ancestors)
tail   = end − start[ord + size − 1]                (a gather)
base   = min of start over each 16 consecutive ords (a 16-lane reduction)
off    = start − base[ord / 16]
widths = 2 or 4 bytes, from the maximum of each column (a reduction)
```

**Why `size` holds.** The nodes that start in `[start, end)` are the node, its descendants, and
its ancestors that start at the same byte. `C[start] + rank` already counts those ancestors. A node
after it in preorder that is not a descendant starts at or after `end`. Zero-width descendants at
`end` break the formula, so a file with one is routed to tree-sitter. In tree-sitter they are
MISSING nodes, which are errors anyway.

**The code table.** Each node has a grammar-wide code `g` from a small universe: one entry per
(kind, field, flags, alias, memberships) combination the grammar can produce. The tape numbers
codes in order of first use. So:

1. `first[g] = min ord` over the nodes with code `g`, a scatter-min;
2. sort the few codes that occur by `first`;
3. `code[ord] = rank of g`, a gather.

The result is the same image `TapeImage::finish` writes, byte for byte.

On the device, the tape scatter writes the columns straight into a resident slab (decision 0032).
The device's tape view reads that layout already.

## 3. Extents come from nearest smaller values

A node's start is usually a local fact: "this byte opens a string", or "this token is a keyword
at the start of a logical line". Its end is not local. It is where something closes, and in each
case that is a *nearest smaller value*:

| Structure | Key per position | End of the node at `i` |
|---|---|---|
| brackets and strings | nesting depth, a prefix sum of +1 and −1 | next position whose depth is below `depth[i]` |
| Python blocks | indentation width of each logical line | next logical line indented no deeper than the block's header |
| expressions | operator precedence, ties broken by associativity | the right boundary in the precedence (Cartesian) tree: the next operator that binds more loosely |

The expression row needs a word of explanation. For a flat run of operands and binary operators,
the parse tree under precedence is the Cartesian tree of the operator precedences. Each
operator's operands extend to its nearest more loosely binding operator on each side. Nodes such
as `comparison_operator`, which tree-sitter makes n-ary for `a < b < c`, merge runs of equal keys.

**Branch-free over a small alphabet.** If the keys come from a small set of size `L`, keep a vector
`last[0..L]` of the latest position holding each key. The previous smaller value of `k` is the
maximum of `last[0..k]`: a masked vector max, with constant work per element. Scanning backwards
gives the next smaller value. Precedence levels number about 20, and indentation widths per file
and bracket depths are bounded (Python's deepest node in the corpus is at depth 83).

**As a scan.** Combining two `last` vectors by element-wise maximum is associative. So the nearest
smaller value is a prefix scan with a vector-valued state. A GPU computes it the same way as a
sum, with no stack.

## 4. The stream transducer

Today's transducers read tapes and emit matches (`docs/transducer-design.md`). A parser needs a
second kind that reads bytes and writes a tape. It is a dataflow graph over streams: per byte,
then per token, then per node.

| Operation | What it does | CPU (NEON) | Metal |
|---|---|---|---|
| `map(table)` | byte or token → class or fact bits | nibble lookups (`vqtbl1q_u8`) | one lane per element |
| `shift(k)` | a neighbour's fact | shift across 64-bit masks | read at `i ± k` |
| `scan(+)` | counts, depth, ordinals | word-level prefix sums | `simd_prefix_exclusive_sum`, then a threadgroup scan, then decoupled look-back between groups |
| `scan(xor)` | toggles, such as inside a string | carry-less multiply (`vmull_p64`) | as for `+` |
| `scan(compose)` | a DFA's state per byte: lexing | transition maps composed by table shuffles | each lane maps its chunk's start states, then a scan composes the maps (open item O66) |
| `scan(vector max)` | nearest smaller value | masked max over a `last` vector | as for `+`, with a vector state |
| segmented forms | the same, restarted at flags | the same, with flags | the same, with flags |
| `compact(mask)` | set bits → a dense list | count-trailing-zeros, write 8, advance by popcount | prefix count, then scatter |
| `gather`, `scatter` | read or write at computed positions, including scatter-min and scatter-add | indexed loads and stores | device memory |
| `reduce` | error bits, maxima for widths | across lanes | threadgroup reduction |

- **Combining transducers** means fusing these operations. A chain of `map`, `shift` and `scan`
  steps runs as one pass over each 64-byte block, as simdjson's stage 1 does.
- **The final-shape rules apply as for tape transducers.** Every operation exists on both CPU and
  Metal, the interpreted form is available at once, and the JIT is built in the background.
- **One scan serves both uses.** The `scan(compose)` lexing primitive is the same one open item
  O66 needs for chunked text matching on the device.

## 5. JSON, worked through

The tree-sitter-json grammar has the nodes `document`, `object`, `pair`, `array`, `string`,
`string_content`, `escape_sequence`, `number`, `true`, `false`, `null` and `comment`. Its anonymous
tokens are `{ } [ ] , : "`. It has one supertype, `_value`, and no conflicts.

**Facts per byte**, as in simdjson:

- `quote`, `backslash` and `escaped`;
- `in_string`, the prefix XOR;
- `structural`, `whitespace` and `scalar_start`;
- `depth`, a prefix sum over brackets outside strings;
- `next_structural` and `prev_structural`, the structural characters around a byte, from a
  forward and a backward scan;
- `content_start`, the first byte of each run of string content between quotes and escapes.

**Nodes per byte.** A rank is the order among nodes that start at the same byte.

| Node | Starts where | Rank | Ends where | Field and membership |
|---|---|---|---|---|
| `pair` | an opening quote whose string is followed by `:` | 0 | the end of its value: the last non-space byte before the next `,` or `}` at its depth | none |
| `object`, `array` | `{`, `[` | 1 | after the matching close (nearest smaller depth) | `value` if the previous structural is `:`; `_value` |
| `string` | an opening quote | 1 | after the next unescaped quote | `key` before `:`; otherwise `value` after `:` and `_value` |
| `number`, `true`, `false`, `null` | `scalar_start` | 1 | the next whitespace or structural byte | as for a value string |
| `{ } [ ] , : "` | that byte | 2 (1 for a closing `"`) | the next byte | none |
| `string_content` | `content_start` | 1 | the next quote or backslash | none |
| `escape_sequence` | an unescaped backslash | 1 | two bytes on | none |
| `document` | the first value | 0 | the end of the last value | none |

All of these are masks, shifts, scans and gathers. For example, the opening quote of a key starts
three nodes at one byte: `pair`, `string` and `"`.

**Errors.** Validation masks set an error bit, and one OR over the file reduces them:

- a depth that goes negative or does not end at zero;
- a byte that cannot start a scalar;
- two values with no separator between them;
- a bad escape.

A set bit routes the file to tree-sitter. The routing rule is measured as the share of files the
fast path takes (`07-viability.md`).

**Comments.** tree-sitter-json's comments are extras. Tested on its parser, a comment's parent is
the lowest common ancestor of the tokens before and after it, or `document` at either end of the
file. So a comment between a key and its `:` is in the `pair`, and one after a value and before
`,` is in the `object` or `array`. Its rank and parent follow from the extents like any token's.
`lexers/json.bitlex` lexes comments.

**Unknowns the differential test settles:** where tree-sitter's `document` node starts and ends
around leading and trailing whitespace, and the exact cursor memberships.

## 6. Python

Python is worked through in `02-python.md`. It needs the same five-stage shape, plus:

- a lexer automaton over a bounded stack of modes, for f-strings;
- blocks and clause chains as nearest smaller values over indentation;
- context facts that give the same tokens different nodes;
- expressions as Pratt binding powers resolved by nearest smaller values;
- a comment rule that reduces to block extents.

## 7. The grammar format

The format must be textual and declarative. It says, per grammar:

- **tokens** as regular expressions, which become one lexing DFA;
- **nesting pairs** (brackets and string delimiters) and the layout rule (indentation);
- **a precedence table** for operators;
- **node rules:** where a node starts (a fact), its rank, where it ends (a boundary from §3), and
  its field and memberships as facts of its context. Kind names follow tree-sitter, so codes
  follow directly.

Every tree-sitter grammar ships its compiled `src/grammar.json`, so an importer can read it without
Node. It can classify most rules:

| Rule shape in `grammar.json` | What it becomes |
|---|---|
| `TOKEN`, `PATTERN`, `STRING` | tokens |
| a `SEQ` that begins and ends with a matching literal pair | a nesting pair |
| `PREC_LEFT` or `PREC_RIGHT` over a binary `SEQ` | the precedence table |
| a `REPEAT` with a separator | a list rule |
| a `SEQ` led by a keyword | a statement rule |
| hidden rules, `ALIAS`, `FIELD`, supertypes | code rules |

It reports the rules that fit none of these. The external scanner is C, so its behaviour has to
be written in the format by hand: for Python, the indentation and string rules above.

The syntax itself is better fixed after JSON has been written in it by hand.

## 8. Where it sits in the final shape

- **A tape is a fact derived from a source fact** (architecture §1). The parser replaces
  tree-sitter plus `TapeImage::encode` behind the same image. The stored form is unchanged.
- **The device path** reads pinned source and writes the columns into a resident slab (decision
  0032). The edited file's tape is built where the device reads it.
- **Tape transducers and stream transducers** are two program kinds in one lowering pipeline.

## 9. Proposed sequence

1. **JSON reference.** A portable implementation of §4's operations over 64-bit masks, with the
   JSON rules of §5 written as data, emitting `TapeImage` bytes. It runs the differential test
   over the 14,669 JSON files and measures CPU time against tree-sitter plus `TapeImage::encode`.
2. **JSON on NEON, then on Metal**, both from the same graph.
3. **Python lexing.** Token boundaries and classes differentially tested against tree-sitter's
   leaves on 13,088 files and CPython's standard library.
4. **Python blocks, statements and expressions**, tracking coverage up to every file tree-sitter
   parses without error.
5. **The grammar format and the `grammar.json` importer**, written from what steps 1–4 needed.
