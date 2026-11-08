# Parser generators: the analysis, and how scans and barriers are represented

Status: proposal, for review (2026-11-07). Nothing here is built. It is the compiler half of
`14-tier-2-design.md`: how the Bitlex compiler finds JSON's plan from the Bitlex file, and the
form that plan takes before it is lowered to Cranelift and Metal.

## Intro

Today the Bitlex compiler gives a block program (`block.rs`) with two parts:

- **tier 1:** straight-line operations over 64-bit masks, some of them carried from block to
  block (`toggle`, `escaped`, `fill`, `prev`);
- **tier 2:** rows that state rules (`NodeRow`, `CodeRow`, `KeywordRow`). They say what a node
  is, not how to compute it.

Doc 14's plan (two passes over the bytes, three scans, entries of four candidates) was worked
out by hand. This page proposes how the compiler finds it, and what it writes down. There are
two new levels between the block program and the backends:

```
Bitlex file
  -> block program     tier-1 ops, tier-2 rows               (exists)
  -> dataflow          every value, tier 1 and tier 2, as an op;
                       every carried op names its scan       (new: the analysis reads this)
  -> plan              passes, scans, barriers, entries,
                       candidates                            (new: the backends read this)
  -> Cranelift, MSL    generated code
```

The plan is derived from the block program. It is not stored, and it is not hand-written. `rir`
prints it after the program, so tests read the compiled form (doc 12 §2.4).

## 1. The dataflow: tier 2 as values

Tier 2's rules become ops over a second domain, the tokens. Tier 1's ops stay over the bytes.

```
# bytes: 64 per word, as today
%1  = escaped %0               # carried: scan fn
%4  = toggle %3                # carried: scan xor
# tokens: one per token start, in order
t   = tokens %starts           # the token domain; a token's text is its bytes
%k  = kind t                   # keywords: local, from up to 8 bytes
%lk = last %k                  # the previous token's kind: scan last, forward
%nk = next %k                  # the next token's kind: scan last, backward
%s  = stack %k {'{' '['} {'}' ']'} max 13 pair   # open brackets: scan dyck
%p  = top %s                   # the parent's kind: local, given %s
%n  = starts(rows) %k %lk %nk %p   # how many nodes start at this token: local
%o  = count %n                 # ord: scan add
```

Each tier-2 row lowers to these ops:

| row | ops |
|---|---|
| `start '{'`, `start opening` | `kind`, or a tier-1 mask read at the token's start |
| `when after (…)` | `last kind` |
| `and parent object` | `top stack` |
| `nested max 13` | an open and a close of `stack` |
| `end with value` | the pair bit of `stack` |
| `check number is json_number` | a local check of the token's text |
| a node's place on the tape | `count` of node starts |

## 2. Scans are ops with a monoid

Every carried op names a **monoid**: what is carried, and how two carries combine. A scan is
one op in the dataflow, whatever its direction and however it is lowered.

| monoid | carry | combine | JSON uses it for |
|---|---|---|---|
| `xor` | 1 bit | xor | `toggle` |
| `fn k` | a function of `k` state bits | compose | `escaped`, `fill`, and S1 (§3) |
| `add w` | `w` bits | add | token and node counts, `ord` |
| `or` | 1 bit | or | errors |
| `last w`, forward or backward | `w` bits and a present bit | the later present value | the previous kind, the next kind, the last token start |
| `dyck kinds max` | 64 bits (doc 14 §3) | cancel, then concatenate | the stack |

Each monoid has four parts, and all four are tested:

- an identity and a combine, which must be associative;
- a step: one element applied to a carry, for the CPU and inside a lane;
- a block summary: a whole block at once, from bit operations where it can (`toggle`'s
  popcount parity), else by a walk over the block's set bits (`dyck`'s brackets);
- a sequential reference, the test oracle.

Then one scan is lowered at four levels. The plan places each level:

| level | holds the carry | combines by |
|---|---|---|
| lane: one 64-byte block | registers | the block summary, or steps |
| SIMD group: 32 lanes, 2 KB | registers | `simd_prefix_*`, or shuffles for wide carries |
| threadgroup: the entries | threadgroup memory | a scan over entries, between barriers |
| chunk: the file in pieces | device memory | a carry from chunk to chunk, in order |

On the CPU only the first level exists: a scan is a running carry in the one loop.

## 3. The analysis

The analysis gives every value a **class** (local, forward or backward) and a **depth**: how
many scans must finish before it is known.

- An op that is not carried has its operands' greatest depth.
- A carried op's block summary depends on earlier scans only through their carries at the
  block's edge: the left edge for a forward scan, the right edge for a backward one.
- If those carries are a few bits, `k`, the summary is computed in the pass before, once for
  each of their `2^k` values. These are the **candidates**. The scan picks one after the
  earlier scan is done. The scan's depth is one more than the earlier scan's, but it needs no
  pass over the bytes between them.
- If they are wide, as the stack is, a summary that needs them waits for another pass over the
  bytes.
- Carries of small state functions that feed each other are merged into one `fn` scan. JSON's
  `escaped` feeds `toggle`, so S1 is one scan over a function of 2 bits.
- A dependency that enters a summary only by adding to it, as the key bit adds 1 to the node
  count, is stored as its own field. It is not a candidate.

The analysis then groups scans by depth into steps. Scans of one depth run together, forward
and backward.

**What the analysis finds for `json.bitlex` as written.** The pair row says
`when after ('{' | ',') and parent object`. A pair starting at a token depends on the stack,
which is wide, so the node count waits for another pass:

```
pass bytes, scan S1, scan S2 (stack), pass bytes (counts), scan S3, pass bytes (tape)
```

That is three passes over the bytes. With the key found from the next token, as doc 14 §1
chose, the node count depends on the stack nowhere, and the plan is doc 14's two passes. The
choice is the Bitlex file's, so the file says it (§6, question 1). The analysis does not prove
two rules equal.

## 4. The plan: passes, scans, barriers, entries

The plan is a list of steps. Each step names the stores it reads and writes:

| step | runs | reads and writes |
|---|---|---|
| `pass bytes` | one lane per 64 bytes, looping over the chunk | the bytes, entries, the tape |
| `pass tape` | one thread per node | the tape |
| `scan` | over the entries, in threadgroup memory | entries |
| `barrier` | `threadgroup_barrier` | |
| `chunk` | the loop over chunks; carries in device memory | the chunk carry |

**Barriers are checked, not guessed.** A store written by one thread and read by another needs
a barrier between the write and the read. The plan's verifier walks the steps and refuses a
plan where one is missing. SIMD-group operations need none: a SIMD group runs in step.

**Entries** are laid out by the plan: one per SIMD group, a field per scan's summary, and the
fields that depend on candidates repeated four times. The plan states the width of every field.
The verifier checks that the entries of a chunk fit the threadgroup memory left after the passes'
own use.

**Candidates** are a field's annotation, `cand S1`: the field has four copies, one per value of
S1's carry at the entry's first byte. The scan that reads the field picks the copy.

## 5. JSON's plan, printed

This is what `rir` would print for `json.bitlex`, with the key read from the next token and
`match barrier`. The numbers are doc 14's.

```
plan json
  # Entries: one per SIMD group (2 KB), four candidates on S1's carry.
  entry 2048
    s1      fn 2                       4 bits      # escaped and toggle, merged
    stack   dyck {'{' '['} max 13 pair 64 bits  cand s1
    kinds   last 3 first, last 3       6 bits  cand s1
    start   last 11                    11 bits cand s1   # the last token start, for tail
    key     last 1 backward            2 bits  cand s1   # is the open string a key?
    tokens  add 12                     12 bits cand s1
    nodes   add 13, +1 key             14 bits cand s1
    error   or                         1 bit   cand s1
    # 4 + 4 x 110 bits = 444 bits, about 56 bytes

  pass bytes A                     # tier 1; each lane's summaries; SIMD-group reduce
    writes entry
  barrier
  scan S1 forward s1               # depth 1
  scan S2 forward stack kinds start, backward key   pick s1   # depth 2
  scan S3 forward tokens nodes                      pick s1   # depth 3
  barrier
  pass bytes B                     # exact carry-ins; one case per token kind; the tape
    reads entry, writes tape
    barrier                        # match barrier: opens offered per level
    closes outside the lane: size, tail
  barrier
  pass tape C                      # off; narrowing of off and tail; codes renumbered
  chunk carry s1 stack kinds tokens nodes opens27 error, commit before open key
```

With `match scan` the mid-pass barrier goes, and the entry gains a field
`opens  last 32 x 27 forward, 108 bytes, cand s1`.

On the CPU the same plan prints as one step:

```
plan json cpu
  pass bytes   # every scan a running carry; keys by the next token; tape at ord
```

## 6. To agree before building

1. **The key rule is written in the Bitlex file.** I propose
   `node pair start string when before ':'`, with `before` reading the next token's kind (a
   backward `last`). The forward rule becomes a check, refused files going to tree-sitter. The
   alternative is a compiler that proves two rules equal. I do not propose it.
2. **The plan is derived, and printed by `rir`.** The assembler recomputes it from the program
   and refuses a printed plan that differs. So a hand-written `rir` file can pin a plan, and
   nothing stores a second copy.
3. **Grammar checks are missing from Bitlex.** simdjson's stage 2 checks each token against
   the one before, given the parent (`[1 2]` is an error). `json.bitlex` has no such table, so
   such a file would give a wrong tape. The table is local, given `last kind` and `top stack`,
   so it changes no depth. It can be added after the plan is built.

## 7. Tests, written first and red

In `block_plan_tests.rs`, reading the derived plan for `json.bitlex`:

- **classes and depths:** byte classes local 0; `escaped` and `inside` forward 1; token starts
  local 1; `last kind`, `stack` and the next kind 2; `ord` 3;
- **the chain:** scans S1 → S2 → S3, S2 holding the one backward scan;
- **the passes:** two over the bytes, one over the tape; and three over the bytes for the
  forward key rule (the current file), so the choice shows in the plan;
- **the barriers:** the verifier accepts JSON's plan, and refuses it with any barrier removed;
- **the entry:** four candidates; the stack field 64 bits; at most 57 bytes in all;
- **`match scan`:** no mid-pass barrier, and the entry 108 bytes per candidate larger;
- **the CPU plan:** one pass, no barriers;
- **the monoids:** each combine associative, and each scan's summaries, combined, equal to its
  sequential reference over random blocks. These are the next step after the plan.
