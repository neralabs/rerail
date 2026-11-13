# Parser generators: the tape from a Bitlex file

Status: proposal (2026-11-06). Step 1 of §7 is built: the reference evaluator runs the tree
section (§4). Step 2 is under way: the compiler (`transducer/packed/bitlex_compile`) compiles all
of `json.bitlex` (classes, masks, counts as bit-planes, frames, tokens, keywords, errors,
escapes, and the tree section's `node` and `code` rows). Its rows equal the evaluator's on every
executor. Both Metal executors run a batch of files, each under its own language's program, in
one launch (§5.4, decision 0066). Step 3 is built for the masks' output (decision 0068): the CPU
writer's tapes equal tree-sitter's byte for byte on 771 of 774 data-layer JSON files, and the
other 3 go to tree-sitter.

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
  At the same byte, tokens rank after the tree section's nodes. `untoken KIND` drops a token
  kind from the nodes, for a grammar whose node is not the token.
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
  field key of string = key_quote           # a string starting at a bit of key_quote has `key`
  supertype _value of object array … = value_start
  alias …                                   # a contextual alias, the same way (not built yet)
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

`lexers/json.bitlex` ends with this section. The reference evaluator runs it (step 1 of §7),
and its nodes equal tree-sitter's on every JSON file tried.

```text
tree json {
  # Bytes between tokens that carry no structure: whitespace and comments.
  mask gap         = (code & ws) | in_cmt
  mask solid       = ~gap
  mask first_solid = solid & ~prev(fill(solid, empty))
  mask last_byte   = ~next(~empty)

  # Bracket depth, after each byte.
  scan depth = count(code & (lbrace | lbracket), code & (rbrace | rbracket)) max 100000

  # The first solid byte after a ':', and after a '{' or ','.
  mask after_colon = fill(prev(code & colon), prev(solid)) & solid
  mask after_open  = fill(prev(code & (lbrace | comma)), prev(solid)) & solid

  # A key is known from what comes before it, as in simdjson's stage 2: a string that opens
  # inside an object, right after its '{' or a ','. Its closing quote ends the key.
  mask key_quote   = opening & after_open & depth.frame(code & lbrace)
  mask key_end     = closing & prev(fill(key_quote, closing))

  mask value_start = (code & (lbrace | lbracket)) | (opening & ~key_quote) | wstart
  mask value_end   = (code & (rbrace | rbracket)) | (closing & ~key_end) | lasts(word)

  node document start first_solid       end last_byte
  node pair     start key_quote         end value_end            level depth
  node object   start code & lbrace     end code & rbrace        level depth
  node array    start code & lbracket   end code & rbracket      level depth
  node string   start opening           end closing

  field key of string = key_quote
  field value of object array string number true false null = value_start & after_colon
  supertype _value of object array string number true false null = value_start
  extra comment
}
```

**No backward carry.** A key is decided from what comes before it, as simdjson's stage 2
decides it from its stack: a string that opens inside an object, right after the `{` or a `,`.
The section's only lookahead is `next` by one byte (`lasts`, `last_byte`).

**What the differential test settled** (tree-sitter-json 0.24.8, runtime 0.27.1):

- `document` starts at the first byte that is not whitespace and ends at the end of the file,
  trailing whitespace included;
- `string_content` is split around escapes in the tree, as the tokens are, so no `untoken` is
  needed;
- a value has the supertype `_value`, a key the field `key`, and a pair's value the field
  `value`;
- a comment is an extra, the child of whatever node it sits in. The rank and nesting rules place
  it with nothing added.

**Results.** The evaluator's nodes are compared with tree-sitter's, node by node in preorder:
kind id, field id, supertypes, extra flag, span and the generic rule's `size`.

| corpus | files | equal | fall back | tree-sitter errors |
|---|---:|---:|---:|---:|
| a data-layer checkout: tracked JSON files | 900 | 899 | 0 | 1 |
| the same checkout: every JSON file outside `.git`, `node_modules` and `.venv` | 28,470 | 28,467 | 1 (an empty file: a zero-width root) | 2 |

These are the tape's inputs. The columns follow from them by `TapeImage::finish`'s formulas.
Comparing the image itself byte for byte needs the engine, and is part of step 3.

## 5. Carries in both directions

Every carry so far runs forward: a block reads what the blocks before it left. Some facts depend
on what comes after a byte. This section says how they are found, in one launch.

### 5.1 simdjson's way: forward state instead

simdjson's stage 1 has no backward carry. It asks whether the byte before was a scalar, not
whether the next byte is one. Its only lookahead is writing each block's structurals one block
late. Facts that depend on context, such as whether a string is a key, are left to stage 2. Stage
2 walks the structural indexes in order with a stack, so it knows a key is due before it reads
one.

A Bitlex file can often do the same. JSON's tree section decides a key from what comes before it
(§4), not from the `:` after it. That needs two kinds of operation:

- **Bounded lookahead:** `next(a, k)`, and `lasts`, which uses it. The compiler retimes the
  program instead of looking ahead. Each value is computed `d` bytes late, where `d` is the most
  lookahead on any path to it. A `next(a, k)` is then `a` computed `k` bytes less late, and the
  outputs come `D` bytes late, where `D` is the largest `d`. The tape writer subtracts `D` from
  every position. No new carry is needed; reading bytes late is a `prev`.
- **The innermost open frame:** `C.frame(a)`, for count `C`. Its carry is a stack: one bit per
  open frame (whether `a` opened it), and the depth. A word's effect on the stack is to pop some
  frames and push others. Two such effects combine into one, so the carry is a scan like the
  others. It passes between groups by the look-back, forward. The stack is one 64-bit word, so
  a file nested deeper than 64 goes to tree-sitter.

### 5.2 When a fact really depends on what follows

Some grammars have facts that only a later byte decides. In Python, an `assignment` starts at
`x` but is known only at the `=`. These need a backward carry: `back_fill`, run from the end of
the file to its start.

- **A summary per chunk.** A backward carry's effect on a chunk is a table from the carry coming
  in on its right to the carry going out on its left. The table depends only on the chunk's own
  masks. Several backward carries in one pass compose into one table, as the fused forward
  rounds' tables do.
- **Sweeps.** Each direction is a sweep over the chunks. The compiler orders the carried
  operations into sweeps by their dependencies. All the carries of one sweep run in the same
  direction; operations with no carry are recomputed wherever they are needed. A program with
  one backward fact runs two sweeps:
  1. **forward:** the forward carries, and the backward operations' tables, are published per
     chunk;
  2. **backward:** each chunk takes the backward carry from the chunk on its right, then computes
     the final masks and writes the tape.

### 5.3 One launch for every sweep

Metal has no barrier across threadgroups, so the sweeps cannot be separated by a barrier. They
are separated by order instead:

- **Persistent threadgroups.** The launch starts as many threadgroups as stay resident. Each one
  loops, taking a work item from an atomic counter. Each loop is bounded.
- **Items in sweep order.** The counter numbers every chunk of every sweep:
  - the forward sweep's chunks come first, from the first chunk to the last;
  - the backward sweep's come next, from the last chunk to the first.
- **Every wait is on a smaller number.** An item waits only on items with smaller numbers. Those
  were handed out earlier, to threadgroups that are running, so a bounded poll cannot deadlock.
- **A poll that runs out** leaves its item unfinished. The host launches again. Finished items
  return at once, and the others find their predecessors' states already published. Each
  launch finishes at least the first unfinished item, so the launches are bounded.
- **States are self-checking words.** A carry state is one 32-bit atomic: 2 status bits and 30
  value bits. A `prev`'s 64-bit halo is three such words. A reader never needs a fence.
- **Inside a chunk,** threadgroup barriers and SIMD-group prefix operations resolve the carries.
  A chunk can be large (1,024 lanes, 64 KB), so the look-back between chunks is rare.
- **Masks between sweeps.** The compiler chooses, mask by mask, whether to recompute a mask or
  keep it. A kept mask costs 8 bytes written and read per 64-byte block. A Python lexer has
  hundreds of masks, so it keeps the operands of the later sweep that are expensive to rebuild,
  and recomputes the rest.
- **The tape in a backward sweep.** Node counts there accumulate from the right. The writer
  places the records right-aligned in a buffer of the guessed capacity, and the tape starts at
  `capacity − total`. If the tape view cannot take that offset, a third sweep, forward again,
  writes the records in order instead, in the same launch.

The JSON program needs only the forward sweep. Sweeps are built when a lexer first needs a
backward fact: Python's.

### 5.4 A batch of files in one launch

Most launches parse many files, in several languages (decision 0066). One launch takes the batch
and a group of programs, one per language:

- **A group of lanes holds one file.** So it runs one program, chosen by a switch that is the same
  in every lane. The padding to whole groups costs 0.8% to 4.1% of the bytes.
- **A file table** gives each file its bytes, its program and where its states and outputs go.
- **The bytes are read where they are resident** (decision 0067). Each file is admitted to a
  slab once, keyed by its hash, and the launch binds up to four slabs. A file table row names
  its slab and its offset in it. A file already resident is not copied again.
- **The CPU executors take the same batch.** The interpreter runs each step once over every
  file of a program. Cranelift's function loops over the files and then over their blocks.
- **A batch over the device's buffer budget is split in halves** (`run_split`), never refused.
  A file that alone does not fit is the error.

**Timings.** JSON's program, 578 instructions with the tree rows and the fallback, ran over
the 774 tracked JSON files of a data-layer checkout. That is 39.4 MB and 615,787 blocks. Two
files go to the fallback. Every executor's results equal Cranelift's.

GPU time is the best of 3. Cranelift's is thread CPU time, the best of 3, from a debug build.
The machine was heavily loaded: load average about 100 on 18 cores.

| executor | lanes | launches | ns per block | GB/s |
|---|---:|---:|---:|---:|
| Cranelift, one call | — | — | 194.6 | 0.33 |
| generated MSL, lookups | 16 B | 5 | 120.2 | 0.53 |
| generated MSL, lookups | 32 B | 3 | 69.4 | 0.92 |
| generated MSL, lookups | 64 B | 2 | 44.3 | 1.44 |
| generated MSL, planes | 16 B | 5 | 114.5 | 0.56 |
| generated MSL, planes | 32 B | 3 | 65.4 | 0.98 |
| generated MSL, planes | 64 B | 2 | 40.6 | 1.57 |
| Metal interpreter | 16 B | 5 | 157.9 | 0.41 |
| Metal interpreter | 32 B | 5 | 112.8 | 0.57 |
| Metal interpreter | 64 B | 5 | 80.9 | 0.79 |

The launches column counts the halves the budget split the batch into. One launch over all
runs was repeated because a look-back ran out of polls. Narrow lanes need more groups, so more
look-back state and more output lanes, and the budget splits them more often.
- **Carries stay inside a file.** A file's first group starts from the identity; its other groups
  look back only to earlier groups of the same file.
- **Generated MSL** has a function per program and one kernel. **The interpreter** reads each
  program's steps from a table.

## 6. What changes where

| part | change | language-specific? |
|---|---|---|
| Bitlex | the `tree` section: `node`, `field`, `supertype`, `alias`, `extra` | the file is |
| the reference evaluator (`tools/bitlex`) | evaluates the tree section, and writes a tape byte for byte like `TapeImage::finish` | no |
| the compiler | Bitlex to a block program: the masks, the `node` and `code` rows, the guesses | no |
| block programs | `node` and `code` rows; `count` levels as up and down masks | no |
| the executors | write the tape from the rows (§3.2): generated code on Cranelift and Metal, step by step in the interpreters | no |

## 7. Proposed steps

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
6. **Sweeps (§5.2, §5.3),** built when Python's section first needs a backward fact.
   *Check:* the tapes still equal tree-sitter's; one launch; the fallback count reported.
