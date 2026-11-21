# 0565: `rust_stream` is removed; the JIT keeps the facts it read from it

Status: done by the implementer. The direction is the user's (2026-11-21): delete `rust_stream`, and the parts of
`clif_rust` that can go now. How it is done is the implementer's choice. Follows 0550, 0560, 0561.

## What was there

- `rust_stream` (5480 lines) wrote tier 2 as Rust from the native IR. Decision 0550 deprecated it, and 0560 and 0561
  replaced its last product use (`rerail-rr-aot`).
- The JIT (`native_cpu_stream`) still called four of its items:
  - `direct_keys`: whether a format 16 run passes each node to the writer at its word, and each node word's key;
  - `open_bound`: the most nodes such a run holds open;
  - `marks`: whether rows carry marks;
  - `word_form`: a local check's byte classes.

  The first two are answered by the program's facts (`facts`), the analysis `rust_stream` made before it wrote a
  program.
- The JIT also kept a harness for the Rust's run: `CompiledStream::set_entry`, `Parser::sixteen_mut`, the
  `asks_feed` flag, the `refusal` codes the Rust returned, and `STATE_SLOTS`. `tape_format` kept `WRITER_RUST`, the
  writer's source for the Rust to include. `clif_rust::rust_source` (tier 1 as Rust) had no caller.

## The decision

1. **`rust_stream.rs` is deleted.** The items the JIT reaches go to a private module of the compiler,
   `native_cpu_facts`: about 1820 lines. Every item no caller reaches is gone, found by the compiler's dead-code
   warnings until none was left. That includes unused fields of the facts and `WordForm::need`.
2. **The facts take no widths or tag.** Nothing in them read either, so `direct_keys(native)` and
   `open_bound(native)` take the program alone.
3. **The Rust run's harness is removed from the JIT.** A refused run is now named by the word that refused. Tier 2
   always has tier 1's records made first (`fed_all`), as it did for every run but the Rust's.
4. **`WRITER_RUST` and `clif_rust::rust_source` are removed.** The `clif_rust` shim keeps the translator's
   re-export and its aarch64 test, for `rerail-pp-aot` and `c_directives` (Go-C-and-C-support's, 0550).

## Checks

- A probe compiled every `.bitlex` file in the repository in both shapes (`Machine`, `Rules`): 1285 programs. For
  each it wrote `direct_keys`, `open_bound` and `marks`. Its output before the pruning (the moved code as it was)
  and after is the same byte for byte. Only JSON in simdjson's structure (`json-simdjson.bitlex`, `Machine`) runs
  direct (173 keys, open bound 3074). Three programs' rows carry marks. The probe takes 100 s, so it is not in the
  tree.
- `rerail-compiler`'s tests: 150. `rust_stream`'s one test (`a_check_word_is_its_places_values`, of its word
  arithmetic in Rust) went with it.
- JSON's tests (`parser_tests`, `json`): 23. `rr_parser`'s `rust_tests`: 3.
- On a Linux host: `rerail-kotlin-parser`, 48. `rerail-rr-parser`, 47, plus its 4 Metal tests, which pass on a
  farm Mac (M1 Max).

## Not checked, not done

- **The facts still build Rust text.** They find a switch's successors (`succ`) from the plans that wrote its
  `match` (`Plan::refs`). In some places the text decides a successor: a known leaf whose code ends in the refusal
  for a node with no number (`UNNUMBERED`) goes on nowhere. To answer from the program alone, the analysis has to
  be rewritten. Only one program reaches the direct run, so the probe above is a weak test of such a rewrite.
- Several of the facts' fields are always empty in the JIT's use (`numbers`, `strings`, `ends`, `reserved`,
  `empties`, `methods`), because only `rust_stream`'s writer set them. The code that reads them still runs, on
  empty values.
- `Ctx::more` and `Ctx::feed` stay in the context's layout and are now always 0.
- The translator (`rerail_clif::rust`) stays. `rerail-pp-aot` and `c_directives` use it, and `clif_ukl` uses its
  control-flow structuring (`Shape`).
- Speed: nothing was measured.
