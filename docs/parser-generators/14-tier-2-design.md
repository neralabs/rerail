# Parser generators: JSON's two tiers on Metal, designed before they are built

Status: design, for review (2026-11-07). Nothing here is built. It is designed for JSON, the
simdjson case, first. §10 lists what must generalise.

## Intro

A Bitlex file compiles to two tiers (decision 0049):

- **tier 1** is bit operations on every byte: classes, strings, each token's first and last
  byte, per-byte errors;
- **tier 2** is the per-token rules: kinds, keys, nesting, checks, the tape.

Both are generated code, compiled from the Bitlex file. Tree-sitter's tapes are the reference.

On Metal the work is split by what each value depends on. Most values depend only on nearby
bytes. A few depend on the whole file before a byte, or after it. Those few are resolved by
prefix scans over small per-chunk entries, and everything else runs in parallel.

The plan for JSON, from the analysis in §1:

```
pass A    every byte, once:  tier 1, and each 2 KB's entry (two candidates)
barrier
scans     over the entries:  S1 (strings)  ->  S2 (stack, keys)  ->  S3 (counts)
barrier
pass B    every byte again:  exact carry-ins; each block's tokens; node records
barrier                      (with one mid-pass barrier to match closes to opens)
pass C    every node:        the tape's columns, at their final widths
```

So the bytes are read twice, and the longest chain of dependent scans is three. No
threadgroup waits on another.

## 1. The analysis: what depends on what

Every value of the compiled program gets one class:

- **local:** a function of the bytes within a few positions;
- **forward:** depends on everything before it, resolved by a forward scan;
- **backward:** depends on everything after it, resolved by a backward scan.

A carried value's **depth** is the number of scans that must run before it. The analysis
orders the scans by depth. Scans at the same depth run together, in either direction.

JSON's values:

| value | inputs | class | resolved by | depth |
|---|---|---|---|---:|
| byte classes: `\`, `"`, op, ws, newline | bytes | local | | 0 |
| `escaped` | backslashes | forward borrow | S1 | 1 |
| `inside` (string toggle) | quotes, `escaped` | forward parity | S1, fused | 1 |
| token starts and last bytes | the above | local, given S1 | | 1 |
| `prev(content)`, `prev(word)` | the lane before | local: a SIMD shuffle | | 1 |
| a token's kind | its first byte, or up to 8 bytes | local | | 1 |
| open brackets at a byte | brackets outside strings | forward stack | S2 | 2 |
| key: a string followed by `:` | the next token after its close | backward | S2 | 2 |
| field `value`: after a `:` | the token before | forward, last present | S2 | 2 |
| a node's `ord` | node starts before it | forward count | S3 | 3 |
| an open's `ord`, for its close | the stack, `ord`s | forward, per level | in pass B | 4 |
| errors | anything | OR | any scan | |

Two choices made by this table:

- **Keys are found backward.** A key is a string whose next token is `:`. That depends only on
  tier 1, so it is depth 2. The forward rule ("after `{` or `,`, when the parent is an
  object") needs the stack, which makes keys and so every `ord` one scan deeper. The two rules
  agree on valid JSON, and pass B's checks refuse a file where they would not.
- **Pairs need no stack entry.** Every open object below the innermost has an open pair, since
  the deeper container is that pair's value. Only the innermost object's pair is in doubt: one
  bit, "after a `:`, before the next `,` or `}`".

### 1.1 Fusing a scan into the pass before it

S2's entries depend on S1 only through the string state at a block's first byte. Each entry
holds both candidates, and the scan phase picks one once S1 is done. So S1 and S2 need no pass
over the bytes between them.

- **The string state:** in a string at the block's start or not, two candidates.
- **The escape state** at the first byte matters only when the byte before the block is a
  backslash. The lane sees that byte in its halo. Only then does an entry need four candidates,
  and that case is rare.

The general rule, for later: fuse when the dependent summary is small and the earlier carry
reaches it through `k` bits at the edge, so `2^k` candidates, and when computing the
candidates costs less than another pass over the bytes. Otherwise add a pass.

## 2. The entries

One entry per SIMD group of 32 lanes, so per 2 KB. A lane takes a 64-byte block. Inside a
SIMD group, lanes combine their values in registers with SIMD prefix operations. So only the
entries are stored between passes.

| field | bits | combine |
|---|---:|---|
| S1: the escape and string carries, as a function of the carry-in | 4 | compose |
| *per string-state candidate (×2):* | | |
| the stack effect (§3) | 64 | §3 |
| first and last token kinds | 6 | first / last present |
| keys: is the string open at the end a key? (needs the right) | 2 | backward borrow |
| token count; node count, and +1 if the open string is a key | 12 + 13 + 1 | add |
| error | 1 | OR |
| codes used (JSON has fewer than 32) | 32 | OR |
| total | about 35 bytes | |

S1's 4 bits are a function from the escape carry-in to (escape carry-out, string parity).
Two such functions compose into one, so S1 is one scan for both of tier 1's carries.

**Where entries go.** In threadgroup memory while they fit, and in device memory past that.
32 KB, less pass B's 3.4 KB (§4), holds about 800 entries, so 1.6 MB of a file at a time.
Larger files go in chunks (§6).

## 3. The 8-byte stack effect

A block's effect on the stack of open brackets is the brackets it closes that opened before
it, then the brackets it opens and leaves open. JSON's brackets are `{` and `[`, so one bit
each, at most 13 deep (`nested max 13`):

| bits | field |
|---:|---|
| 4 | closes: how many |
| 13 | closes: the kinds they expect, innermost first |
| 4 | opens: how many |
| 13 | opens: their kinds, innermost first |
| 1 | the innermost object's pair is open at the block's end |
| 1 | the pair bit is set by this block, not passed through |
| 2 | error: a kind mismatch, or deeper than 13 |
| 26 | free |

**Combine `a` then `b`.** The first `m = min(a.opens, b.closes)` of `b`'s closes cancel `a`'s
innermost opens. Then:

- `a`'s top `m` kinds must equal `b`'s first `m` expected kinds, or the error is set;
- the closes are `a`'s, then `b`'s beyond `m`;
- the opens are `a`'s below `m`, then `b`'s;
- depth past 13 sets the error.

All of these are shifts, masks and one compare.

**Per block** the lane makes its effect by a walk over its block's bracket bits. That is
usually 0 to 4 iterations, and at most 64.

## 4. Pass B: every input from a scan

Pass B re-reads the bytes. For each lane it rebuilds the exact carry-ins: the entry's scan
result, plus a SIMD prefix over the lanes before it in the group, plus a SIMD suffix for the
backward key bit. Then it walks the lane's tokens in order.

Each input the walk needs, and where it comes from:

| input | source |
|---|---|
| a token's index and start position | the token count's prefix, then a popcount below its bit; positions are written compacted, as simdjson's structural indexes |
| a node's `ord` | the node count's prefix, then a popcount of node starts below its bit, plus its rank at its byte |
| a token's kind | its first byte; a keyword compare of up to 8 bytes |
| key or not | the key bit: local, or the backward scan's for a string still open at the block's end |
| field `value` | the last token kind before it: local, or the forward scan's |
| the parent's kind, for the checks | the stack at the block's start, then the brackets walked so far |
| an open's `ord`, for its close | below |

The node starts at one byte, with their ranks:

| byte | nodes, outermost first |
|---|---|
| the first token | `document` |
| `{` or `[` | `object` or `array`, then the punctuation token |
| a key's opening quote | `pair`, `string`, `"` |
| any other opening quote | `string`, `"` |
| any other token start | the token |

**Matching a close to its open** needs the open's `ord`. That is the latest open at the same
level before the close. JSON has 13 bracket levels, 13 pair levels and the open string, so 27
values:

- **in the same lane:** in registers, from the walk;
- **in an earlier lane of the same pass:** each lane offers its last unmatched open per level.
  One SIMD prefix max per level runs over the lanes, since `ord`s grow. Then one value per
  SIMD group and level goes to threadgroup memory: 32 × 27 × 4 bytes, 3.4 KB. Barrier.
- **in an earlier pass:** the 27 values carried from pass to pass.

So pass B has one barrier in its middle. Before it, the walk writes every node's start record.
After it, the closes whose open is outside the lane write `size` and `end` into the open's
record. The second half walks only the lane's closes again.

**Records** go to scratch in device memory, one per node: start, end and size (4 bytes each),
code (2) and level (1). `atomic_min` of `ord` per code gives first use. `atomic_max` per
column gives the widths.

**Checks** in the walk, any failure setting the file's error:

- each token's kind against the one before, given the parent: simdjson's stage 2 table;
- numbers with `json_number`, over their bytes in 8-byte chunks;
- keywords;
- the stack's errors.

## 5. Pass C: the tape

One thread per node, reading the scratch records:

- `code`: renumbered by first use, from a table of a few dozen codes sorted by one thread
  after pass B;
- `size`;
- `base` for each 16 nodes, the start of node `16g`, and `off = start − base`;
- `tail = end + 1 − start[C[end + 1] − 1]`, a gather from the starts;
- each column at its final width.

The only guess is the host's buffer capacity, from the file's length at JSON's measured node
density. A file that overflows it runs again with the true size, which the first run knows.

## 6. Large files: sequential chunks

A file larger than the entry budget runs in chunks, in order, in the same threadgroup. Each
chunk is passes A, B and C over its bytes, then a carry written to device memory for the next
chunk:

- S1's state at the boundary;
- the totals: tokens, nodes, codes used;
- the open brackets as a stack, with the pair bit;
- the 27 open `ord`s;
- the last token's kind;
- the error.

**The backward dependency at a chunk's end.** A string at the end of a chunk may be a key, and
that is known only from the next token, which may be in the next chunk. So a chunk commits
only up to its last block boundary before such a string. The carry records that boundary, and
the next chunk starts there. It re-reads a few hundred bytes, the unresolved tail. Everything
before the boundary is final.

In the meantime, until chunks are built, entries past threadgroup memory spill to device
memory, as §2 says.

## 7. Many small files in one threadgroup

The corpus's median file is 3.7 KB, so a threadgroup takes a run of files up to a chunk.
Every scan is segmented: a file's first block resets every combine. Records, totals and errors
are per file.

## 8. The CPU

One generated function per program. Blocks in order, tokens in order, every carry in a
register, the bracket stack in registers bounded by 13. The CPU needs no scans:

- keys by the forward rule, since the stack is at hand;
- `ord` by a running count;
- records written at their final place.

## 9. Checks for the build

- Tapes equal tree-sitter's byte for byte on the corpus, or the file goes to tree-sitter.
- Metal's tapes equal the CPU's.
- Shape guards:
  - pass A and pass B read the bytes once each;
  - the scan chain is S1, S2, S3;
  - one case per token kind in the walk, and no interpretation of rows;
  - nothing allocated per token;
  - entries within budget;
  - no wait on another threadgroup.
- CPU and GPU time next to simdjson's stages: 9 to 13 ns per 64 bytes for stage 1, and about
  12 for stage 2.

## 10. What must generalise afterwards

- The analysis of §1, run by the compiler over any program's values, with backward carries.
- The stack effect for any `nested` kinds: bits per kind, and the bound from `max`.
- The pair bit is JSON's "a child that ends its parent" (`end with`). In general that is one
  bit per such kind at the innermost level.
- The backward key rule is JSON's. In general a rule may read the next token, and the analysis
  chooses forward or backward by depth.
- The chunk commit point, for any backward carry.

## 11. Open questions

1. **Pass B's mid barrier.** Matching closes inside a pass needs it. The alternative is a
   fourth scan over the entries, for the 27 open `ord`s. That needs 108 bytes per entry, about
   four times the entry, but drops the barrier and the second walk.
2. **The four-candidate case** (a block after a backslash): store four candidates always, or
   flag the entry and re-run its block in the scan phase?
3. **Scratch records then pass C, or pass B writing the tape directly** once the widths are
   known. They are known after S3 for `size`, but not for `off` and `tail` until every start is
   written.
