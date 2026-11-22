# 0560: The `.rerail` parser's tier 1 is written from the grammar

Status: done by the implementer, first step. The direction is the user's (2026-11-21): the `.rerail` parser's Rust
(`rr_aot`) and `rr_parser`'s tier 1 Rust tests used the deprecated lowerings (`clif_rust`, `rust_stream`, decision
0550), and "need the new generator to cover .rerail's grammar first". How it is done is the implementer's choice.
Follows 0550, 0551, 0556.

## What was there

- `rerail-rr-aot`'s build script wrote both tiers from the JIT's IR (`clif_rust` for tier 1, `rust_stream` for tier
  2), behind its `generated` feature. Nothing depended on the crate.
- `rr_parser` had two tests on `clif_rust`'s output of JSON's tier 1: `json_tier1_has_no_label_switch`, and
  `json_tier1_by_pass`'s `RERAIL_JSON_RUST` dump.
- `bitlex_lex_rust` refused `rr.bitlex`. It had no `scan … = count(…)`, no `toggle`, and no mark `else escape`. Its
  reader also left out a region's `else E bytes M`.

## The decision

1. **Tier 1 covers `rr.bitlex`** (`bitlex_lex_rust`):
   - **`scan N = count(U, D) max M`**: a walk of the bits of `U | D` after the token loop. It refuses the file where
     the count passes `M`. A count whose name a mask, escape or skip reads is refused by the generator (none does in
     `rr.bitlex`).
   - **`toggle`**: a stage 1 carried node, the word's prefix XOR and its carry, as `escaped` is carried. The helper
     (`TOGGLE`) is written only into a file that uses it.
   - **A mark's `else escape`**: after the token loop, a test of the mark's pending bit and of its mask from the last
     grammar token to the end. It refuses where the mark would fall past the file's last token.
   - **A region's `else E bytes M` in a reader**: the region's arm takes `E`'s openers too. With no closer after
     them (`E`'s meaning), the token is `M` bytes, as the records' reader's `ReadNone::Bytes`.
2. **`rerail-rr-aot` is the generated tier 1**, as `rerail-kotlin-aot` and `rerail-json-aot` hold theirs:
   `src/tier1.rs`, checked in, with no build script and no `generated` feature. It is written from the compile
   `rr_parser::compiled` makes (the tape's kind and field ids, `Shape::Rules`), carried as `Compiled::lexer_rust`, so
   its codes are the JIT's.
3. **`rr_parser`'s `rust_tests`** replace the `clif_rust` tests:
   - `rust_is_current`: the crate's file is the compiler's output (`RERAIL_BLESS=1` writes it).
   - `tier1s_records_are_the_interpreters`: on the samples and files made to reach each rule above, tier 1 as Rust
     refuses where the interpreter's tier 1 does. Where neither refuses, each record's first byte, last byte and
     code are the interpreter's.
   - `corpus_tier1s_records_are_the_interpreters` (by name): the same over the repository's `.rerail` files.
4. **Two kinds of difference from the interpreter are allowed, each named in the test**:
   - **Marks only refused**: a hidden mark that is only refused (`refuse M in L`: `json_bad`) has its local token
     tests moved by the lowering to tier 2 (`kind_checks`). So the interpreter's records lack its bit, which tier 1
     as Rust sets. The test leaves those bits out of the comparison, and checks that tier 1 as Rust sets every bit
     the interpreter does.
   - **Escapes the lowering moves to tier 2**: tier 1 as Rust refuses the file, the interpreter's tier 1 does not,
     and the JIT refuses it.
5. **`json_tier1_has_no_label_switch` is removed**, and so is `json_tier1_by_pass`'s `RERAIL_JSON_RUST`. Both
   checked `clif_rust`'s output. JSON's tier 1 as Rust is `rerail-json-aot`'s (0551).

## Checks

- `rust_tests`: 32 files' records are the interpreter's, and 9 are refused by both.
- The corpus: 1784 files, none differ. The first run found the empty backtick literal (``` `` ```, the `else` of
  region `t2`): tier 1 as Rust refused it.
- `rr_parser`'s suite on a Linux host: 46 pass, and 4 fail there only because they run tier 1 on Metal. Those 4 pass
  on a farm Mac (M1 Max).
- `rerail-kotlin-parser`'s `rust_is_current` (on a Linux host) and `json_rust_is_current`: Kotlin's and JSON's
  generated files are unchanged. `rerail-compiler`'s tests: 151.

## Not checked, not done

- **Tier 2 is not generated yet.** `bitlex_rust::tier2` refuses `rr.bitlex` at its first `relabel` (`test_tail`),
  and the loop form at its marks, extras, flags and readings. So `rerail-rr-aot` has no `parse`, and no tape is
  compared.
- The tests compare records and refusals, not the JIT's tapes.
- Speed: nothing was measured.
- The count walk is a pass over the file's words after the token loop, not stage 1's. Its cost was not looked at.
- `rerail-pp-aot` and `c_directives` still use `clif_rust`. They are Go-C-and-C-support's (0550).
