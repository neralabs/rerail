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

This page extends Bitlex with a `tree` section. That section says, in masks, which nodes a file
has and where each starts and ends. Everything that differs between languages is in the Bitlex
file. The parts that do not differ are generic:

- **the compiler** reads any Bitlex file and writes two things: a block program (the masks,
  `10-bit-programs.md`) and a node table (one row per node kind);
- **the tape writer** reads the masks and the node table and writes the tape. It is one piece of
  code per executor, the same for every language.

So adding a language means writing its Bitlex file, and nothing else.

## 1. What the tree section declares

### 1.1 Nodes

A node kind is a start mask, an end rule and a rank:

```text
tree json {
  # Lines like these, in order: KIND start MASK end RULE.
  # A node starts at each bit of its start mask.
  node object  start code & lbrace   end at code & rbrace level depth
  node string  start opening         end at closing
}
```

- **Start.** Any mask. The node starts at each of its bits.
- **Rank.** Where several nodes start at one byte, the one listed first is outermost. A key's
  opening quote starts `pair`, `string` and `"`, listed in that order.
- **Tokens are nodes already.** Every `token` statement is a leaf node of its kind. Its start
  and end are the token's own, and it is ranked after the tree section's nodes at the same byte.
- **Zero-width nodes** are not allowed. A file that would need one goes to tree-sitter, as
  `01-branch-free-design.md` §2 decides.

### 1.2 End rules

A node's end comes from one of three rules. Each rule names masks only. The tape writer
implements each rule once, for every language.

| rule | the node ends | used for |
|---|---|---|
| `through B` | after its start's run of `B` bytes (a token's rule, §2.4 of `03`) | tokens |
| `at E` | after the next bit of `E` at or after its start | strings, comments |
| `at E level L` | after the next bit of `E` at the start's level of `L` | brackets, pairs, blocks |

**Levels.** A level is a `count` scan (`03` §2.7): `+1` at each byte of one mask, `-1` at each
byte of another, bounded by `max D`. `at E level L` is the nearest-smaller-value rule of
`01-branch-free-design.md` §3. The end is the first bit of `E` after the start where the level
comes back to the start's own level. A closing bracket matches its opening bracket this way.
So does a Python block's dedent, with indentation as the level.

### 1.3 Codes

A node's code is its kind, plus facts read at its start byte:

```text
  field key   = string & key_quote          # a mask: the node's field, where it is set
  field value = value_start & after_colon
  supertype _value = value_start            # the node is in the supertype where it is set
  alias ...                                 # a contextual alias, the same way
  extra comment                             # nodes of this kind carry the extra flag
```

- **Each fact is a mask at the node's start byte.** The compiler lists every (kind, field,
  alias, supertypes) combination the declarations allow. That list is the grammar's small code
  universe (`01` §2).
- **Names resolve against the grammar.** Kind, field and supertype names map to their ids in the
  grammar's own tables (tree-sitter's `parser.c` symbols and fields), as `ucode` tables do
  (`03` §2.6).
- **Codes are numbered by first use** in the tape. That numbering is the tape writer's job (§3.4).

### 1.4 Errors

The existing `error … when` statements still route a file to tree-sitter. The tree section
adds errors of its own, implicitly:

- a level that goes below zero or does not end at zero;
- a node whose end rule finds no end;
- a node that ends after its parent ends.

## 2. JSON's tree section

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

  # A pair ends with its value. The level rule skips the ends of values nested deeper, and the
  # key's end is not a value end, so the first value end at the pair's level is its own.
  node document start first_value     end at last_value_end
  node pair     start key_quote       end at value_end level depth
  node object   start code & lbrace   end at code & rbrace level depth
  node array    start code & lbracket end at code & rbracket level depth
  node string   start opening         end at closing

  field key        = string & key_quote
  field value      = value_start & after_colon
  supertype _value = value_start
  extra comment
}
```

`prev_structural` and `next_structural` are not Bitlex functions yet. They test the structural
byte before and after, skipping whitespace and comments: a `fill` forward and a `back_fill`
backward. Each one is a line of masks in this file, not a new primitive. `first_value` and
`last_value_end` are left to the differential test (step 1 of §5).

## 3. The generic tape writer

The tape writer has no language in it. Its input is the node table and the masks the table
names. One row of the table is:

```text
kind, rank, start output, end rule (through | at | at with level), end output,
level outputs (up, down), code facts (field, alias, supertype outputs)
```

It writes each record into the tape at its slot `ord`, with no list of positions built first.
The rest of this section is the same for every language.

### 3.1 `ord`: a prefix count

A node's preorder number is the count of nodes that start before it, plus its rank:

```text
ord = C[start] + rank        C[p] = the number of nodes that start before byte p
```

A lane, which owns 64 bytes on the GPU, finds `C` at its first byte:

1. its count: one popcount per start mask in the table;
2. `simd_prefix_exclusive_sum` across the SIMD group;
3. its group's offset, by the bounded look-back between groups. This is what the extract pass
   already does (`10-bit-programs.md` §4.1).

Inside the lane, the count below bit `j` is a masked popcount per start mask. The list of start
masks is fixed by the table, so this is straight-line code.

### 3.2 `code` and `off`: written by the start

- **`code`.** The kind and its facts, read at the start byte, give the grammar code. A table
  lookup gives the code's number (§3.4).
- **`base` and `off`.** Preorder never decreases a start, so a 16-node block's least start is the
  start of its first node:

  ```text
  base[b] = start of node 16 b
  off     = start - base[ord / 16]
  ```

  The node whose `ord` is a multiple of 16 writes `base`. Every node writes `off`. For that it
  needs the start of the node `ord % 16` places before it, which is in its word or close
  behind.

### 3.3 `size` and `tail`: written by the end

```text
size = C[end] - ord
tail = end - (start of node C[end] - 1)        the last node to start before the end
```

The lane that holds a node's end knows `C[end]`, because it is its own prefix count. So the
end writes `size` and `tail` into the same slot, `ord`. For that it must know the node's `ord`.
Each end rule finds it its own way:

- **`through B`.** The end is in the start's word or a few words on. The start lane finds it by
  a count of trailing zeros, and writes all five values itself.
- **`at E`.** The next bit of `E` after the start. It is found in the start's word, else in
  the next words' masks, or by a carried "next bit of `E`" scan: a `back_fill` of `E`. The
  start lane writes all five values.
- **`at E level L`.** The end finds the start instead. The start is the nearest earlier byte
  whose level is below the level at the end. The search goes from near to far:
  1. in the lane's word, by the level change below each bit;
  2. in the SIMD group, where each lane publishes its lowest level and a ballot names the nearest
     lane under the target;
  3. across groups, through each group's lowest level, published next to its look-back state.

  The start's `ord` is then `C[start] + rank`, where `C[start]` is the start lane's prefix
  count. The look-back writes that to a per-lane array. The start lane is earlier, so its value
  is there or soon will be.

### 3.4 Guessed sizes, checked in the same dispatch

Some of the tape's properties are known only once every node is counted:

- its widths, each 2 or 4 bytes from the column's maximum;
- the codes' first-use numbering;
- its length.

The tape writer guesses them, writes with the guess, and checks the guess in the same
dispatch. In the common case the masks and the tape are then one dispatch.

| what | guess | check |
|---|---|---|
| node count, for the allocation | from the file's length, at the density the language's corpus shows, with a margin | the look-back's total |
| `size` width | 2 bytes if the guessed count is below 65,536 | the total |
| `off` and `tail` widths | 2 bytes | an `atomic_max` per column |
| first-use numbering | the order the language's files usually use their codes | an `atomic_min` of `ord` per code, compared on the host (a few dozen values) |

- **Wrong guess.** The host writes the tape again with the true values, in a second command
  buffer.
- **Numbering alone wrong.** A pass over the `code` column renumbers it.
- **The guesses are per language, but they are data.** The compiler measures them on the
  language's corpus and stores them in the node table. They are not code.

### 3.5 Bounded waits

Every wait is a bounded poll, as in the carries' look-back:

- for a group's offset;
- for an earlier lane's prefix count;
- for a group's lowest level.

A lane that runs out of polls marks its group deferred and writes nothing. The host then writes
the deferred groups' nodes in a second command buffer. It uses the counts the first dispatch
wrote, so the masks are not run again.

### 3.6 On the CPU

The same table drives a CPU form. Cranelift's loop over blocks visits the blocks in order, so
every look-back answer is already known when it is needed. The nearest-smaller-value search
becomes a stack of the open starts' `ord`s per level, bounded by the level's `max D`.

## 4. What changes where

| part | change | language-specific? |
|---|---|---|
| Bitlex | the `tree` section: `node`, `field`, `supertype`, `alias`, `extra` | the file is |
| the reference evaluator (`tools/bitlex`) | evaluates the tree section, and writes a tape byte for byte like `TapeImage::finish` | no |
| the compiler | Bitlex to a block program in `rir`, and the node table as new `rir` rows of the same program | no |
| block programs | a `count` level: the up and down masks are outputs; the tape writer builds the level from them | no |
| the tape writer | §3, once per executor (CPU, Metal) | no |
| per-language data | the guesses of §3.4, measured by the compiler | data only |

The node table is part of the transducer's `Program`, as the block program is (`12` §3). It is
covered by the digest, checked by `validate`, and printed and assembled by `rir`. It is not a
separate format.

## 5. Proposed steps

Each step closes with its check passing.

1. **The tree section in the reference evaluator,** for JSON.
   *Check:* the evaluator's tape equals today's `TapeImage` byte for byte, on every data-layer
   JSON file. This settles `01` §5's unknowns. Nothing is compiled yet.
2. **The compiler:** Bitlex to a block program and a node table, for the masks `json.bitlex`
   uses.
   *Check:* the block program's outputs equal the evaluator's masks.
3. **The tape writer on the CPU,** from the node table.
   *Check:* tapes equal to step 1's, byte for byte.
4. **The tape writer on Metal**, one dispatch in the common case.
   *Check:* the same; the fallback count reported; time per block next to the masks alone.
5. **Python's tree section,** with indentation as a level. *Check:* as step 1, on the
   data-layer Python files.
