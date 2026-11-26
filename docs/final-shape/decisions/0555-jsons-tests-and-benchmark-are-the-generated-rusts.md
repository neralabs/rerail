# 0555: JSON's tests and benchmark are the generated Rust's; `rust_stream`'s are gone

Status: done by the implementer. The user asked for it (2026-11-21): port the JSON benchmark and remove this branch's
tests that use `rust_stream` (0550's rule: the branches that use the deprecated lowerings remove their uses). How it
is done is the implementer's choice. Follows 0554. Page 40, step 58.

## The decision

1. **The benchmark** (`json_tests::simdjson_form_against_simdjson`) times `rerail-json-aot`, the Rust the Bitlex
   compiler writes from the grammar (0551), tier 1 and tier 2, writing format 16 through one kept writer, against
   simdjson's DOM parser. It first checks that the tape is the JIT's. It reports tier 1 alone, and tier 2 with the
   writer on tier 1's records made once (`rerail_json_aot::parse_lexed`). The crate is built as the test is, so
   the measurement is a `--profile profiling` run. It no longer builds `rust_stream`'s source with rustc.
2. **Removed** as tests of `rust_stream`'s output:
   - `json_tests::simdjson_forms_rust_trees_are_the_jits`, with its helpers `language_of`, `aot_entry`, `flat`,
     `dispatch_resets`;
   - `aot_tests` (`grammars_rust_tapes_are_the_jits`), with nextest's entries for it and the dev-dependencies
     only it used (`rerail-kotlin-parser`, `libc`);
   - the by-word dump's two Rust files (`rust_stream::rust_source`, `clif_rust::translate`). The dump writes the
     generated tier 2 in their place, for the simdjson form.

   `json_rusts_tapes_are_the_jits` is the comparison with the JIT in their place, on the trees test's files.
3. **Found by the benchmark, fixed: tier 1 read a name the grammar defines later.** `json-simdjson.bitlex` has
   `class high` (a byte 0x80 and up), which the error "invalid UTF-8" reads, and a later `mask high` (a high
   surrogate's escape). `bitlex_lex_rust` evaluated the errors after every mask, by the names' last meaning, and
   bound a mask to the class's local name. So every file with UTF-8 of two bytes or more was refused
   (`twitter.json`, at byte 0). Now:
   - an error reads the names as they are at its statement;
   - a name defined again gets a local of its own;
   - the parts made of a name's old meaning are not reused for its new one.

   Grammars that define no name twice get the same output: Kotlin's `rust_is_current` passes, on a Linux host.

## Checks

- `json_rusts_tapes_are_the_jits`, with UTF-8 of two, three and four bytes, a `\u` escape, and seven invalid
  sequences: 204 tapes the JIT's, 120 refused by both. JSON's tests: 23. The parser tests: 90, without Metal.
- The benchmark on the M3 (a farm host; its load was not recorded), 30 runs a measure, median:

  | File | Ours, instr/byte | simdjson | Ours, GB/s | simdjson | Time ratio |
  |---|---|---|---|---|---|
  | `twitter.json` | 77.9 (tier 1 36.3, tier 2 41.2) | 5.8 | 0.16 | 4.03 | 24.8 |
  | `numbers.json` | 69.7 (tier 1 34.9, tier 2 34.8) | 14.3 | 0.30 | 1.66 | 5.5 |

  These are facts of this build, not targets. `rust_stream`'s form had measured 48.7 on `twitter.json`, with
  tier 1 on Cranelift at about 4.4 (page 40, steps 36-38). The generated tier 1 makes each mask in a pass of its
  own over the whole file; Swift-Kotlin-Scala-support's 0551 ("tier 1's masks are one pass over the blocks")
  rewrites that generator, and is to be merged here.

## Not checked, not done

- Why tier 2 is 41 instructions a byte: no profile was taken.
- `rr_aot` and rr_parser's tier 1 Rust tests still use the deprecated modules: `.rerail`'s parser needs the direct
  generator to cover its grammar first.
