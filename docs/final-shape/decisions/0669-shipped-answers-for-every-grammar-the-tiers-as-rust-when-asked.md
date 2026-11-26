# 0669: Shipped answers for every product grammar; the tiers as Rust only when asked

Status: done (the user, 2026-11-24: "please improve the other grammars to include shipped answers so that we can
keep the compilation costs don"). Follows 0414 (the answers kept), 0646 (the budget) and 0663.

## Intro

**Shipped answers.** A grammar's prover answers, keyed by question, can ship beside it (`kotlin.answers`, 0414):
a compile given them proves only what an edit asks anew. Only Kotlin shipped any. The other grammars the product
compiles at run time are the C directives, the C preprocessor tape and the C tokens (each for C and for C++),
`.rerail` (`rr.bitlex`) and JSON (`jsonc.bitlex`, in `rerail-lang-json`).

**What a cold compile of each costs** (a DGX Spark, `RERAIL_GPU=cpu`, the default budget of 2^20 steps):

| Grammar | Compile | Proving | Budget |
|---|---|---|---|
| `jsonc`, `json`, `json-rfc`, `json-simdjson` | 0.01 to 0.07 s | under 0.1 s | not reached |
| `rr` | 0.09 s | under 0.1 s | not reached |
| `pp` (C, C++) | 0.24 s each | 0.24 s | spent; lifted: 1.2 s, 20 answers more |
| `tokens` (C, C++) | 11.8 s each | 0.3 s | spent; lifted: 0.9 s, 16 answers more |
| `directives` | does not compile (below) | | |

So shipped answers save these grammars little time: their proving is short, and where the budget cuts it short
the answers would make the program sharper (the lifted compile's), not faster. The tokens' 11.5 s is not the
prover's. A sample (an M1 Max) puts 98% of it in `bitlex_lex_rust::tier1`: tier 1 written as Rust text
(`Gen::region_of`, which walks a mask again for every context it is read in, with no memo; `cubes_id`;
allocation). Every compile with a tree section wrote tier 1 and tier 2 as Rust, and only one caller reads it:
Kotlin's `tiers_rust_given` (the parser as Rust). The C crates, `rr`, JSON and Kotlin's own run-time compile
dropped it.

## The decision

- **The tiers as Rust only when asked** (`bitlex_masks::AsRust`, an argument of `compile_proved`; `compile` and
  `compile_shaped` say `No`). The Rust is written from the finished program and changes nothing else, so the
  program and the answers are the same. `tiers_rust_given` says `Yes`; Kotlin's `compile_timed` says `No`, and
  its `Compiled::rust`, which nothing read, is removed.
- **Every product grammar ships its answers**, beside its grammar: `directives.answers`, `bitlex_pp/pp.answers`
  and `bitlex_tokens/tokens.answers` (each file holds C's and C++'s), `rr.answers`, `json_parser/jsonc.answers`.
  Each compile goes through `answers::answered_shipped(family, text, shipped, …)`: the shipped answers and the
  machine's kept ones (0414), under a family a text (`c-pp`, `cpp-pp`, …).
- **One test of shipped answers** (`answers::check_shipped`): a compile of each of the grammar's variants from
  the file alone proves nothing. `RERAIL_BLESS_ANSWERS=1` writes the file again from a cold compile instead, and
  asks for the budget lifted (`RERAIL_PROVING_STEPS`), so the file holds proofs, not answers the budget cut short.
  Kotlin's test is ported to it, and `RERAIL_BLESS_KOTLIN_ANSWERS` is now `RERAIL_BLESS_ANSWERS`.

## Results

a DGX Spark, cold, the default budget:

| | Before | After |
|---|---|---|
| C tokens compile (each of C and C++) | 11.8 s | 0.31 s |
| C preprocessor compile (each) | 0.24 s | 0.26 s cold; from its answers, no proving |
| Kotlin's Bitlex compile from its shipped answers | 3.2 s | 1.3 s |
| `rerail-c-directives`' 22 tests | 70 to 80 s a test (the slow tier's note) | 9 s in all |

The quick tiers of `rerail-compiler`, `rerail-c-directives`, `rerail-rr-parser`, `rerail-kotlin-parser` and
`rerail-lang-json` pass (293 tests), and `rerail-parser-tests` too. The shipped Kotlin answers answer its
compiles as before (`the_shipped_answers_answer_the_grammar`).

## What it leaves

- **The directives grammar does not compile**: "`ellipsis`: no byte left that no token begins with, for its
  reader's kind", with the budget lifted too. It did not at the commit before 0663 either. A reader's kind that
  shares a first byte with another kind's takes a byte no token begins with (0177, 0509); `code` begins at any
  byte, so none is left. Eight of `rerail-c-directives`' tests fail on it (the crate is in the slow tier), and
  `the_directives_answers_answer_the_grammar` with them: `directives.answers` is empty until the grammar compiles.
- **A cold Kotlin compile within the default budget fails the same way** (`line_comment`), and so do
  `a_cold_compile_makes_the_shipped_answers_program` and `walk_lemmas_keep_the_tapes` (the slow tier). Both
  failed so at the commit before 0663. Past the budget a row's first bytes are every byte, which is sound, but
  tier 1's dispatch then has no byte left for a reader's kind, and the compile stops instead of making a less
  sharp program.
- **Tier 1 as Rust for the C tokens** still takes 11.5 s when asked: `region_of` has no memo by context.
