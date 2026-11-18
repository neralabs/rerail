# 0401: JSONC's comments end at their gaps

Status: decided (the user, 2026-11-18: "please work through these", the first being "JSONC keeps
its lasts extract … until the prover can handle that case"). Follows 0400.

## Intro

**The problem.** 0400 found each token's end from the gap after it. Bitlex proves that rule
grammar by grammar, and for JSONC it failed on the comment row. So JSONC kept its second
position list: 988 instructions and 33 stack accesses in its hot loop.

The rule holds on real documents. It fails only on files the fallback refuses, and a refused
place may lie any distance before the token's end:

- a comment opened inside another, such as `/* // */`;
- a quote inside a comment.

In such a file, two regions are open at one byte: the line comment's `fill` and the block
comment's, or a comment's and a string's `toggle`. The file is refused from where the second
region opens. The prover reads a window of bytes before the position. A window that starts
inside both regions does not show where they opened, so the prover could not tell that the
file was refused before it.

## The decision

**Two proof steps, both sound, in `bitlex_masks::gap_ends`.**

1. **The claim as its own hypothesis** (`Induction::never_inductive`). Take the first position
   where the claim holds. By definition it holds at no earlier position. So the step may assume
   the claim false at every lag of the window, not only at the lags the window reads at the
   position.
   - The values are made at every lag. They are the operations' own equations, so making them
     adds no assumption.
   - The bounded solve covers a stream that begins inside the window, and it uses the same
     hypothesis, which is true there too.
   - A claim proved this way joins the prover's lemmas for the claims after it.
2. **Lemmas of two regions at one byte.** For each pair of regions (`fill`s and `toggle`s) the
   claim is `a & b & ~refused & within 0`. It is proved by step 1 first, then kept as a lemma
   over the carries themselves, at their lags:
   - "both regions, not refused, inside the stream" holds at no byte;
   - a window's edge, where the carries were free, is now bound by it;
   - a lemma applies only where its position is in the stream.

Then the comment row's two claims prove: no comment's last byte in the skip, and a gap after
each one. JSONC's tier 1 makes no lasts.

**Rejected: a larger budget.** Raising the horizon to 16 bytes, or the solves and levels
(1,000 solves, 5 levels, 200 rounds), did not prove the claim. It made the compile take minutes.
The window's free edge is not a matter of size: no finite window shows where a region opened.

**Found on the way: the bounded witness was a sanity check's.** A counterexample that the
interpreter evaluates to zero seemed to show that the prover's model was looser than the
interpreter. It was not: the witness came from a check run in the same window, not from the
claim.

## Requirements, a test each

- R1. JSONC's rows all end at their gaps, and the CPU makes no lasts
  (`tier2_tests::a_tokens_end_is_its_gaps_as_the_lasts_extract`, which now asserts both 14-json
  and 15-jsonc).
- R2. The lasts and codes equal the old extract's on every document tier 1 accepts. The same
  test adds documents with comments that:
  - end in spaces;
  - touch a token;
  - end the file, or are unclosed at its end;
  - nest an opener or hold a quote, which are refused. Each JSON step must refuse at least one
    file.
- R3. Every proof stays sound. Every lemma is proved before it is kept, the two-region lemma
  states exactly what was proved (`within 0` included), and lemmas apply only inside the stream.
- R4. The CPU equals the interpreter. The compiler suite (105 tests) and the parser_tests guards
  (`json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`, `registers`,
  `stage1`, `vectors`, `on_every_executor`) all pass.
- R5. The rr JSON set: 129 parse the same, 216 are refused by both, 0 differ.

## What changed

| `15-jsonc` | Hot loop | Stack accesses | Tier 1's operations |
|---|---:|---:|---:|
| After 0400 | 988 | 33 | (not measured) |
| Now | **814** | **27** | 178 |

`14-json` is unchanged: 616 instructions, 5 stack accesses.

**Compile time.** The proofs of gap ends take up to about 0.9 s per grammar in a debug build
(JSONC's), and 0.1 to 0.4 s for the rest of the ladder.
