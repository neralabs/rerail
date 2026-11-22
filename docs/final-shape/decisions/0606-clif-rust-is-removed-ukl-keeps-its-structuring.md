# 0606: `clif_rust` is removed; UKL keeps the structuring it used

Numbered 0570 where it was written (Parser-generators-final-shape); 0606 at its merge of the hub, whose 0570 is another.

Status: done by the implementer. The direction is the user's (2026-11-21): "delete the rest of clif_rust and fix the
parts that think they need it. Parts which are not about rust generation need to be either folded into their
callers or rewritten." How it is done is the implementer's choice. Follows 0550 and 0565.

## What was there

- `rerail_clif::rust` (1391 lines) translated Cranelift IR to Rust (`translate`), with a text fingerprint. The
  compiler re-exported it as `clif_rust`.
- Its users:
  - `rerail-pp-aot`: the C preprocessing words' IR as Rust, built by a build script and run by
    `native_cpu_pp::run_aot`. It was `c_directives`' CPU path, under its default `generated` feature. Without the
    feature, the path was already the JIT's code of the same IR.
  - `c_directives`' tests of that build, and the rustc row of its corpus benchmark.
  - `clif_ukl`, the IR as UKL for the device. It used the translator's control-flow structuring (`Shape`, `Order`,
    `successors`, `returned_call`) and `absorbed`, a matcher of NEON patterns. It printed C through the structuring's
    `c` flag.
  - `native_cpu_pp::ukl_source`, which used `fingerprint`.
- Left in the JIT for tier 1's Rust (removed in 0560): the `keep_ir` option, which nothing set; the IR it kept
  (`CompiledBlock::ir`, `CompiledStream::ir`); `entry_ids`, `entry_id`, `tables_words` and `tables_span`.

## The decision

1. **`rerail_clif::rust`, the compiler's `clif_rust` and the `rerail-pp-aot` crate are deleted.** So are
   `native_cpu_pp::rust_source`, `run_aot` and `ABI`, and `c_directives`' `generated` feature and `aot_build`. The
   preprocessor's CPU path is the JIT's code.
2. **The structuring moves into its one caller, as `clif_ukl::shape`** (private), and prints only C:
   - the Rust printing goes: labels, `Names` and `Labels`, the `match` dispatch;
   - `threaded` folds into `of`, since its one caller passed no block that only jumps, so `thread` did nothing.
3. **`clif_ukl` no longer calls `absorbed`.** Its `ty` refuses every vector type for any value it declares, so a
   function with a vector test was refused there already. Only the error's text changes.
4. **`fingerprint` moves to `clif_ukl`**, for `ukl_source`.
5. **The JIT's leftovers for tier 1's Rust are removed**: `keep_ir`, the kept IR, the entries' ids, and the record
   tables' words and span. `Bytewise::window_function` is now built for tests only: those tests read the window's IR.
6. **The translator's tests**:
   - Moved: `the_dominators_are_the_definitions`.
   - Rewritten without Rust:
     - `a_loop_with_two_entries_is_structured`: the node is split, with no dispatch, and the C has a loop and each
       block;
     - `a_returned_call_is_the_return`: tests `returned_call` directly.
   - Removed with the Rust they read: `bits_against_zero_are_one_test`, and the compiler's aarch64 test of the NEON
     narrowing shift in Rust.

## Checks

- `rerail-compiler`'s tests: 152 (`clif_ukl::shape`'s three among them). `rerail-clif`'s: 13. JSON's: 23.
  `rr_parser`'s `rust_tests`: 3.
- On a farm Mac (M1 Max): `the_compiled_words_are_the_interpreters` (the preprocessor on the JIT, against the
  interpreter) and `the_device_words_are_the_interpreters` (the UKL `clif_ukl` prints, on Metal) pass.
- The libraries build with no warnings, and so do their tests.

## Found on the way

- Removing `pub mod rust;` left its `#[cfg(feature = "ir")]` on `view`. A Linux build without `ir` found it.
- **`c_directives`' grammar does not compile here.** 8 of its tests fail, the same on Linux and on this Mac, with
  "`ellipsis`: no byte left that no token begins with, for its reader's kind". The check is decision 0509's (a reader's
  kind takes a byte no token begins with, `bitlex_masks`). It came into this branch with the Swift-Kotlin-Scala
  merge, and none of this change touches it. `c_directives` is Go-C-and-C-support's. It was not fixed here.

## Not checked, not done

- The UKL text before and after was not compared byte for byte. The device test passing is the check.
- The preprocessor's speed: the rustc build was its fastest CPU path (decision 0348, t44: 0.44× the interpreter's
  cycles against Cranelift's code). It is gone, and nothing was measured after.
- The `c_directives` tests that need the grammar, which fails as above, did not run.
