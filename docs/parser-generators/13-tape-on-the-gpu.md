# Parser generators: writing the tape on the GPU, node by node

Status: proposal (2026-11-06). Nothing here is built yet.

## Intro

The mask passes end with planes: one bit per byte for each fact. A parser's output is the
tape: one record per node, in preorder. The tape has these columns:

| column | what it holds |
|---|---|
| `code` | the node's folded code, numbered by first use |
| `size` | how many nodes its subtree holds, the node included |
| `base` | for each 16 nodes, the least start among them |
| `off` | the node's start, minus its 16-node block's `base` |
| `tail` | the node's end, minus its last descendant's start |

Each column is 2 or 4 bytes wide, chosen from its largest value.

This page shows how each GPU lane can write the records of the nodes that start in its bytes,
straight into the tape. No list of positions is built first, and simdjson's index is not used.
Every value a record needs comes from a prefix operation: a sum, a count below a bit, or the
nearest smaller value. In the common case, the masks and the tape are one dispatch.

JSON is the worked case. Its nodes and their start and end rules are in
`01-branch-free-design.md` §5.

## 1. Which nodes start where

Each node kind has a start plane, built by the mask pass from stage 1's facts:

```text
; JSON, per byte (01 §5's table):
;   S_object  = '{' outside strings      S_array = '[' outside strings
;   S_string  = opening quote            S_pair  = opening quote of a key
;   S_token   = each of { } [ ] , : "    S_scalar = simdjson's scalar starts
;   S_content = string content starts    S_escape = unescaped backslashes
```

Several nodes can start at one byte. Their order there is fixed by kind (the `rank`). A key's
opening quote starts `pair` (rank 0), `string` (1) and `"` (2).

The count of nodes at byte `p`, `n[p]`, is the sum of the start planes' bits at `p`.

## 2. `ord`: a prefix count

A node's preorder number is the count of nodes that start before it, plus its rank:

```text
ord = C[start] + rank        C[p] = n[0] + ... + n[p-1]
```

A lane gets `C` at its first byte as the extract pass gets its offset
(`10-bit-programs.md` §4.1):

1. the lane's count: one popcount per start plane;
2. `simd_prefix_exclusive_sum` across the SIMD group;
3. the group's offset, by the bounded look-back over earlier groups.

Inside the lane, the count below bit `j` is a masked popcount per plane:

```text
// Lane code for the nodes starting at bit j of the lane's word.
below = 0
for each start plane S:          // a fixed list: no data-dependent loop
    below += popcount(S & ((1 << j) - 1))
ord = lane_offset + below + rank(kind)
```

The lane writes the node's `code` at `ord`, and its `off` (§5). Two of the record's values,
`size` and `tail`, depend on where the node ends.

## 3. `size` and `tail`: written by the node's end

A node's `size` is the count of nodes that start inside it:

```text
size = C[end] - ord
```

The lane that holds a node's end byte knows `C[end]`: it is its own prefix count. So the end
writes the record's `size` and `tail`, and the start writes the rest. Both write into the same
tape slot, `ord`, and nothing waits for anything.

For the end to write, it must know the node's `ord`. How it finds it depends on the kind of
node.

**Tokens, scalars, escapes and string content** end near their start:

| node | ends at |
|---|---|
| a token such as `{` or `,` | the next byte |
| a scalar | the next whitespace or structural byte |
| string content | the next quote or backslash |
| an escape | two bytes on |

For these the start lane writes all five values. The end is the next set bit of an end mask,
found with a count of trailing zeros in the lane's word. If the end is past the word, the
lane reads the following words in turn (§6).

**Containers** are objects, arrays, strings and pairs. Their end can be far away: the root
object ends at the end of the file. The end finds the start instead. A container's start is
the nearest earlier byte whose depth is below the depth at its end: a nearest smaller value
(`01-branch-free-design.md` §3). Depth is a prefix sum of +1 for an open bracket and -1 for
a close, outside strings. It is a carried scan, like `toggle`. The search for that byte runs
from near to far:

1. **In the lane's word.** It uses the depth change at each byte. That is a prefix sum inside
   the word: popcounts of opens and closes below each bit.
2. **In the SIMD group.** Each lane publishes the lowest depth it reaches. A ballot of the
   lanes below the end's lane whose lowest depth is under the target depth names the nearest
   one: the highest set bit, by `clz`.
3. **Across groups.** Each group publishes its lowest depth next to its look-back state. The
   search walks back over those values, then over the found group's lanes as in step 2.

The start's `ord` is then `C[start] + rank`. `C[start]` is the start lane's prefix count. The
look-back writes it to a per-lane array, and that lane is earlier in the stream, so its
value is ready or soon will be. The wait is bounded, and a fallback covers it (§7).

`tail` needs the start of the node's last descendant. That descendant is the last node that
starts before the end: node `C[end] - 1`. Its start is the nearest earlier set bit of the
union of the start planes, which the end finds as it found the node's start.

## 4. Depth at the ends of strings and pairs

Strings and pairs are containers that brackets do not mark:

- **A string** ends at its closing quote. That is the next set bit of the quote plane after
  its start. A string is never nested, so a forward search for that bit finds it (§6).
- **A pair** ends where its value ends. That is the byte before the next `,` or `}` at the
  pair's depth. The end of the value writes the pair's record together with its own.

## 5. `base` and `off`: by 16 nodes

Preorder never decreases a start, so a 16-node block's least start is the start of its first
node:

```text
base[b] = start of node 16 b
off     = start - base[ord / 16]
```

- The node whose `ord` is a multiple of 16 writes `base`.
- Every node writes its own `off`. It needs the start of the node `ord % 16` places before it.
  That node is in the same word or close behind. The search is a count back over the union of
  the start planes, with each byte's `n[p]` as its weight.

## 6. Widths, codes and size: guess, then check

The tape's widths, its code numbers and its length are known only at the end. A second pass
over every node would be needed to wait for them. Instead the write uses a guess and checks it
in the same dispatch:

| what | guess | check, in the same dispatch |
|---|---|---|
| node count, for the allocation | from the file's length, with a margin, at the density measured on the corpus | the look-back's total |
| `size` width | 2 bytes if the guessed count is below 65,536 | the total |
| `off` and `tail` widths | 2 bytes | an `atomic_max` per column |
| code numbers by first use | the grammar's usual first-use order (for JSON, the order a typical file uses them) | an `atomic_min` of `ord` per grammar code, then compare the order on the host (a few dozen values) |

- **Wrong guess.** If a check fails, the host runs the tape write once more with the true
  values. That needs a second command buffer.
- **Code numbers alone.** If only the code numbers are wrong, a small renumbering pass over the
  `code` column is enough. It runs in a second command buffer that does no mask work.

## 7. Bounded waits and the fallback

Every wait in this design is a bounded poll, as in the carries' look-back:

- for a group's offset;
- for an earlier lane's prefix count (§3);
- for a group's published lowest depth (§3).

A lane that runs out of polls marks its group deferred and writes nothing. The host then
writes the deferred groups' nodes in a second command buffer. It uses the prefix counts the
first dispatch already wrote, so the masks are not run again.

## 8. What is lane code and what is not

- **Lane code, straight-line:** the masks, the start planes, the counts below a bit and the
  look-back state updates. This code has no branches.
- **Data-dependent work:** the loop over a word's node starts, and the searches of §3 and §5.
  Each step of a loop is bounded by the word's width or a fixed number of polls. Each search
  costs in proportion to the distance it covers.

## 9. Proposed steps

Each step closes with its check passing.

1. **JSON's start planes and depth,** as rir in the stage 1 fixture.
   *Check:* each plane equals a reference built from today's tape for the corpus files.
2. **`ord` and `code` written by the starts,** with a guessed code order.
   *Check:* the `code` column equals today's tape byte for byte.
3. **`size` and `tail` written by the ends.** Leaves first, then containers with the
   nearest-smaller-value search.
   *Check:* the `size` and `tail` columns equal today's tape.
4. **`base` and `off`.**
   *Check:* the whole image equals `TapeImage::finish`'s output byte for byte, on every JSON
   corpus file. One dispatch in the common case, with the fallback count reported.
5. **The CPU form.** The same records, written by Cranelift's loop over blocks. There the scan
   order makes every look-back immediate.
