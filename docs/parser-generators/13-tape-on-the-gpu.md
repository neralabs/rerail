# Parser generators: the tape from a Bitlex file

Status: proposal (2026-11-06). Nothing here is built yet.

## Intro

A Bitlex file (`03-bitlex.md`) defines a lexer as masks: one bit per byte for each fact. Its
output today is a token stream. A parser's output is the tape: one record per node, in
preorder, with these columns:

| column | what it holds |
|---|---|
| `code` | the node's folded code (kind, field, flags, alias, supertypes), numbered by first use |
| `size` | how many nodes its subtree holds, the node included |
| `base` | for each 16 nodes, the least start among them |
| `off` | the node's start, minus its 16-node block's `base` |
| `tail` | the node's end, minus its last descendant's start |

This page extends Bitlex with a `tree` section, which says in masks where each node starts and
ends. All of a language's logic is in its Bitlex file. The rest is generic and has no language
in it:

- **the compiler** turns a Bitlex file into a block program (`10-bit-programs.md`): its masks,
  and one `node` row per node kind;
- **the executors** run the block program and write the tape from its `node` rows. Generated
  code (Cranelift, MSL) writes the records in the same loop or kernel as the masks.

There is no intermediate table, and no list of positions. The tape is regular enough to write
straight from the masks (§2).

## 1. The tree section

### 1.1 Nodes

A node kind is a start mask and an end mask, with a level where nesting matters:

```text
tree json {
  # KIND start MASK end MASK [level SCAN], one line per kind, outermost first.
  node object  start code & lbrace   end code & rbrace   level depth
  node string  start opening         end closing
}
```

- **Start.** A node of the kind starts at each bit of its start mask.
- **End.** The node's last byte is a bit of its end mask: the first one at or after its start
  (§1.2).
- **Rank.** Where several nodes start at one byte, the one listed first is outermost. A key's
  opening quote starts `pair`, `string` and `"`, listed in that order.
- **Tokens are nodes already.** Every `token` statement is a leaf node of its kind. The compiler
  gives it the token's own start mask, and the mask of each token's last byte as its end mask.
  At the same byte, tokens rank after the tree section's nodes.
- **Zero-width nodes** are not allowed. A file that would need one goes to tree-sitter, as
  `01-branch-free-design.md` §2 decides.

### 1.2 The one end rule

Every node ends the same way:

> A node starting at byte `s` ends at the first bit `e ≥ s` of its end mask where the level
> just after `e` equals the level just before `s`.

- **Without a level,** the level is constant, so the rule is "the first bit of the end mask at
  or after the start". A string ends at its closing quote.
- **With a level,** the rule is the nearest-smaller-value rule of `01-branch-free-design.md` §3.
  Bits of the end mask inside deeper nesting are skipped.
- **A level** is a `count` scan (`03` §2.7): `+1` at each byte of one mask, `-1` at each byte of
  another, bounded by `max D`.

Some worked cases:

| node | start | end mask | level before `s` | level after `e` |
|---|---|---|---|---|
| `{ … }` | `{` | `}` | d | d (the `}` closes what the `{` opened) |
| a pair whose value is `{ … }` | the key's quote | value ends | d | d (after the value's `}`) |
| a pair whose value is a number | the key's quote | value ends | d | d (a number inside a nested value is at d + 1, so it is skipped) |
| a string | `"` | closing quotes | (none) | (none) |
| a token such as `true` | `t` | the token's last byte | (none) | (none) |

Python's blocks would use indentation as their level.

### 1.3 Codes

A node's code is its kind, plus facts read at its start byte:

```text
  field key        = string & key_quote     # the node has this field where the mask is set
  supertype _value = value_start            # the node is in this supertype where it is set
  alias …                                   # a contextual alias, the same way
  extra comment                             # nodes of this kind carry the extra flag
```

- **Each fact is a mask at the start byte.** The compiler lists every combination the
  declarations allow (kind, field, alias, supertypes). That list is the grammar's small code
  universe (`01` §2).
- **Names resolve against the grammar.** Kind, field and supertype names become their ids in
  tree-sitter's tables (`parser.c`), as `ucode` tables do (`03` §2.6). The block program holds
  the ids; the names are left in its comments.
- **First-use numbering** is the writer's job (§3.3).

### 1.4 Errors

The existing `error … when` statements still route a file to tree-sitter. The tree section adds
some errors of its own, implicitly:

- a level that goes below zero, or does not end at zero;
- a node whose end rule finds no end before its level drops below its start's;
- a node that ends after its parent ends.

## 2. Why the tape can be written straight from the masks

Records are in preorder, and preorder is "by start byte, then by rank". So a node's slot is a
prefix count:

```text
ord = C[start] + rank        C[p] = the number of nodes that start before byte p
```

Every column is then one write at `ord` of a value the node can find:

| column | value |
|---|---|
| `code` | the kind's code, from the code masks at the start byte |
| `off` | `start − base[ord / 16]` |
| `base` | the start of node `16 b`, written by that node (starts never decrease in preorder) |
| `size` | `C[end + 1] − ord` |
| `tail` | `end + 1 − (start of node C[end + 1] − 1)`: the last node to start inside it |

So the writer needs, for each node:

- its start and its rank, given by the `node` rows;
- its end, given by the end rule;
- the prefix count `C` at its start and just after its end.

Nothing else is needed, and none of it depends on the language.

## 3. The block program

### 3.1 `node` rows

The compiler writes the tree section as rows of the block program. These are the same `rir`
rows that already hold the masks and `extract`:

```text
; One row per node kind, outermost first. The masks are the program's outputs.
block 0 node 0 kind 7 start %41 end %42 level %40 %43     ; object: `{` … `}`, depth +%40 -%43
block 0 node 1 kind 9 start %51 end %52                   ; string: opening … closing quote
block 0 code 1 field 2 when %60                           ; string: field `key` where %60
block 0 code 1 supertype 4 when %61                       ; string: in `_value` where %61
```

- **`node`** gives a kind's grammar id, its start and end masks, and the level's up and down
  masks if it has one. The row's place in the list is its rank.
- **`code`** gives a fact of a kind's code and the mask where the fact holds.

The rows are part of the transducer's `Program`, like the masks. They are:

- covered by its digest;
- checked by `validate`: every mask an output, at most one level per node, the ids within the
  grammar's ranges;
- printed and assembled by `rir`.

### 3.2 What the generated code does

The executors write the tape from the `node` rows in the same pass as the masks:

- **On Metal,** in `tempo_block`, as the extract pass already does.
- **On the CPU,** in Cranelift's loop over blocks.
- **The interpreters** follow the same rows, one step at a time, as the reference.

The generated code is specialised to the rows. Each one is a fixed number of straight-line
statements, and the only data-dependent loop is over a word's node starts.

For each lane's word (64 bytes) the generated code does the following:

1. **The counts.** One popcount per start mask gives `n`, the number of nodes starting in the
   word. A prefix sum across the SIMD group and the bounded look-back between groups then give
   `C` at the word's first byte. This is the extract pass's offset (`10` §4.1).
2. **Each start.** For each node starting at bit `j`:
   - `ord` is `C` at the word, plus a masked popcount of each start mask below `j`, plus the
     node's rank;
   - the node writes `code`, `off`, and `base` when `ord` is a multiple of 16.
3. **Each end.** At each bit of an end mask, the code finds the node that ends there by the end
   rule (§3.3). Then it writes that node's `size` and `tail`, from `C` just after the end and
   the node's `ord`.

So the start and the end of a node each write their own columns of the same slot, `ord`, and
neither waits for the other.

### 3.3 Finding the node that ends at a byte

This is the one search in the writer. It is a prefix operation:

- **Without a level,** the node is the latest start of its kind at or before the end. A count of
  leading zeros finds it, in the word or in the words before it. A string's opening quote is the
  previous set bit of its start mask.
- **With a level,** the node is the latest start whose level-before equals the level-after at the
  end. That is the nearest smaller value of the level. The search goes from near to far:
  1. in the word, by the level change below each bit;
  2. in the SIMD group, where each lane publishes its lowest level and a ballot names the nearest
     lane under the target;
  3. across groups, through each group's lowest level, published next to its look-back state.

The found start's `ord` is its `C` plus its rank. Its `C` is in a per-lane array written by the
look-back. The start lane is earlier in the stream, so the value is there or soon will be.

On the CPU the blocks run in order, so a stack of open starts per level replaces the search. It
is bounded by the level's `max D`.

### 3.4 Guessed sizes, checked in the same pass

Some of the tape's properties are known only once every node is counted:

- each column's width, 2 or 4 bytes from its maximum;
- the codes' first-use numbering;
- the tape's length.

The writer guesses them, writes with the guess, and checks the guess in the same pass. So in the
common case the masks and the tape are one dispatch.

| what | guess | check |
|---|---|---|
| node count, for the allocation | from the file's length, at the language's measured density, with a margin | the look-back's total |
| `size` width | 2 bytes if the guessed count is below 65,536 | the total |
| `off` and `tail` widths | 2 bytes | an `atomic_max` per column |
| first-use numbering | the order the language's files usually use their codes | an `atomic_min` of `ord` per code, compared on the host (a few dozen values) |

- **Wrong guess.** The host writes the tape again with the true values, in a second command
  buffer.
- **Numbering alone wrong.** A pass over the `code` column renumbers it.
- **The guesses are data.** The compiler measures them on the language's corpus and writes them
  as rows of the program, next to the `node` rows.

### 3.5 Bounded waits

Every wait is a bounded poll, as in the carries' look-back:

- for a group's offset;
- for an earlier lane's prefix count;
- for a group's lowest level.

A lane that runs out of polls marks its group deferred and writes nothing. The host then writes
the deferred groups' nodes in a second command buffer. It uses the counts the first dispatch
wrote, so the masks are not run again.

## 4. JSON's tree section

This is a sketch. A differential test against tree-sitter settles the unknowns that `01` §5
lists: where `document` starts and ends, and the exact memberships.

```text
tree json {
  # Bracket nesting outside strings and comments.
  scan depth = count(code & (lbrace | lbracket), code & (rbrace | rbracket)) max 1024

  # The structural byte before and after each byte, skipping whitespace and comments.
  mask after_colon = prev_structural(colon)
  mask key_end     = closing & next_structural(colon)
  mask key_quote   = opening & back_fill(key_end, closing)  # the key's closing quote, carried back

  # A value is an object, an array, a string or a scalar token.
  mask value_start = (code & (lbrace | lbracket)) | (opening & ~key_quote) | wstart
  mask value_end   = (code & (rbrace | rbracket)) | (closing & ~key_end) | lasts(word)

  # A pair ends with its value: the key's end is not a value end, and the level skips the ends
  # of values nested deeper.
  node document start first_value       end last_value_end
  node pair     start key_quote         end value_end            level depth
  node object   start code & lbrace     end code & rbrace        level depth
  node array    start code & lbracket   end code & rbracket      level depth
  node string   start opening           end closing

  field key        = string & key_quote
  field value      = value_start & after_colon
  supertype _value = value_start
  extra comment
}
```

- **Not yet in Bitlex:** `prev_structural` and `next_structural`. They test the structural byte
  before and after, skipping whitespace and comments: a `fill` forward and a `back_fill`
  backward. Each one is a line of masks in this file, not a new primitive.
- **Left to the differential test:** `first_value` and `last_value_end` (step 1 of §6).

## 5. What changes where

| part | change | language-specific? |
|---|---|---|
| Bitlex | the `tree` section: `node`, `field`, `supertype`, `alias`, `extra` | the file is |
| the reference evaluator (`tools/bitlex`) | evaluates the tree section, and writes a tape byte for byte like `TapeImage::finish` | no |
| the compiler | Bitlex to a block program: the masks, the `node` and `code` rows, the guesses | no |
| block programs | `node` and `code` rows; `count` levels as up and down masks | no |
| the executors | write the tape from the rows (§3.2): generated code on Cranelift and Metal, step by step in the interpreters | no |

## 6. Proposed steps

Each step closes with its check passing.

1. **The tree section in the reference evaluator,** for JSON.
   *Check:* the evaluator's tape equals today's `TapeImage` byte for byte, on every data-layer
   JSON file. This settles `01` §5's unknowns. Nothing is compiled yet.
2. **The compiler:** Bitlex to a block program with `node` and `code` rows, for what
   `json.bitlex` uses.
   *Check:* the block program's outputs equal the evaluator's masks.
3. **The CPU executors write the tape:** the interpreter, then Cranelift.
   *Check:* tapes equal to step 1's, byte for byte.
4. **The Metal executors write the tape**, one dispatch in the common case.
   *Check:* the same; the fallback count reported; time per block next to the masks alone.
5. **Python's tree section,** with indentation as a level.
   *Check:* as step 1, on the data-layer Python files.
