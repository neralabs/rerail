# 0484: The provers know a walk's regions never overlap

Status: decided (the user, 2026-11-19: "fix the kotlin speed, starting with tiny examples", after
Kotlin's ladder steps took 5 to 7 minutes each to compile). Follows 0412 (0453 on Parser-generators-final-shape).

## Intro

**The problem.** The Bitlex compiler proves facts about a grammar's bit program before it lays out
tier 2. For example, a token's row is proved never to start inside a comment, or two token rows
are proved never to hold at one byte. Each proof is a search over the streams of bytes the
program could see (`bitlex_prove`). On Kotlin's ladder these proofs took almost all of a step's
compile time:

| Step | Bitlex compile, CPU |
|---|---:|
| 02 | 1.3 s |
| 05 | 4 s |
| 09 | 31 s |
| 12 | 57 s |
| 35 | minutes |

Cranelift took 1 to 3 s of each.

**What made it slow.** Kotlin has a walk (`block::Walk`): its strings, characters and comments
are regions that the walk opens one at a time. A region opens only after the last one's closer,
so two of the walk's regions never hold at one byte. The provers didn't know this. A fixed
point's equations don't show it: each region's fill is its own carried value. So every proof that
needed "a comment's byte is not a string's byte" searched for a stream where both held. Such a
stream doesn't exist, and the prover used up its budget looking. A proof that failed this way
also kept tier 2 bigger than it needs to be, because the fact went unproved.

## The decision

1. **A walk's kinds are lemmas** (`Lowering::induction`). For each pair of kinds in each walk,
   the fixed-point prover (`bitlex_prove::Induction`) is given the lemma "these two fills never
   hold at one place". This is true of every walk by construction: the walk opens a region only
   past the last one's closer. Both fills are in the walk's group, at one delay, so the lemma is
   at lag 0 in both. Every prover the lowering makes (gap ends, marks, row modes) is made this way.

2. **A row whose start never holds is dead** (`row_modes`). Where both `s & ~v` and `s & v` are
   proved, the row's start `s` never holds, so the row meets no state. Before the lemmas this
   never came up. With them, such rows were read as "mode unknown" and met every state, and
   tier 2 grew past its 65,533 words. Read as dead, step 35's tier 2 is 15,424 words; it was
   23,146 before the lemmas.

3. **The lowering and the provers hash with a fixed seed** (`foldhash::fast::FixedState`, in
   `bitlex_masks`, `bitlex_prove`, `bitlex_tier2` and `grammar`). A proof's search order comes
   from map iteration. With std's random seed, one grammar compiled to programs of 15,420,
   15,422 or 15,424 words from run to run. Now one grammar compiles to one program.

4. **Gap ends: a claim a real stream breaks ends the check before the lemmas** (`gap_ends`,
   `Induction::refuted`). A bounded solve that finds a stream starting inside its window has
   found a real stream, and no lemma rules a real stream out. So the check gives up before it
   proves the region lemmas, which it did before only to fail on the same claim afterwards.

## Tried and not kept

**Runs of witness streams before each proof.** Any byte string is a stream. A run of the
program (`BlockProgram::rounds_values`) that sets a claim's bit refutes the claim without a
search. The provers' found streams were kept and run over each claim before it was proved. At
step 35 this made no difference: 13.4 s with the runs and 14.5 s without, unloaded. Under load
the runs cost more. It was removed. The answers were the same either way, since a sound prover
fails on a refuted claim anyway.

**A memo of the plain prover's `eval`** was slower, and was reverted.

## Measured

Bitlex compile, CPU, on this machine (M-series). "After" is the final code, from the ladder run below, at load about 18:

| Step | Before | After |
|---|---:|---:|
| 02 | 1.3 s | 0.09 s |
| 05 | 4 s | 0.39 s |
| 09 | 31 s | 1.8 s |
| 12 | 57 s | 1.9 s |
| 21 | 32 s | 9.1 s |
| 28 | | 8.1 s |
| 35 | minutes | 14.5 s |

The whole ladder, all 35 steps (`aot_tests::ladder_steps_rust_tapes_are_the_jits`, `RERAIL_AOT_LADDER=kotlin`,
load about 18): 420 s wall, the Rust's build 69 s of it; the Bitlex compile 165 s of CPU over the 35 steps, Cranelift
29 s. Every step's tapes are the JIT's. It took about 3 hours before, over farm hosts.

The four changes were measured together, not one by one.

## Checks

- Kotlin's 47 tests against the lean parser pass. The corpus (495 files) gives 429 the same,
  62 refused, 0 differ, as with the walk lemmas off and no runs.
- `kotlin_parser::tests::a_cold_compile_makes_the_shipped_answers_program`: the grammar compiles
  twice to the same program, cold and with its shipped answers (`kotlin.answers`, 0414; Q371).
- `kotlin_parser::tests::walk_lemmas_keep_the_tapes` checks that the lemmas make the program
  smaller, and that with them off (`bitlex_masks::NO_WALK_LEMMAS`) it reads each text the same.
  Without the lemmas the grammar takes minutes to compile, so the test is in nextest's slow tier,
  where Q371 put the step test it replaces; it is not ignored.
- The compiler's prover, SAT and grammar tests pass, as do `parser_tests`' compile, JSON, block
  and `.rerail` tests.

## What is left

- At step 35, about 4.6 s of pair checks use up the plain prover's budget and prove nothing.
  `row_modes`' proofs take about 8 s.
- `rr_tests::rr_cases_parse_from_their_steps` compiles all 22 `.rerail` steps, in 124 s of CPU.
  Step 22's Cranelift compile takes 23.6 s of it.
  `rr_tests::rr_steps_parse_as_the_reference_and_the_engine` takes 173 s: 59 s of Bitlex compile, 42 s of Cranelift.
  Both tests are in the quick tier, past its 20 s line.

## Kotlin's ladder removed

The user, 2026-11-20: "stop keeping the whole ladder ... I said stop keeping them at all". Steps
01 to 34 are deleted (steps 1 to 35 here, where 0422 had kept step 36 alone). The last step is the
grammar, `kotlin_parser/kotlin.bitlex`: step 36's grammar under the grammar's header. The AOT test
takes it as Kotlin's only step: 82 s, most of it rustc's build. The nine
`what_step_N_does_not_read_is_refused` tests are one, `what_the_grammar_does_not_read_is_refused`.
The step timings above are from before the removal.
