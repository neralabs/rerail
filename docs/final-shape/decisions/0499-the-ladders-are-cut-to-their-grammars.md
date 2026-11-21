# 0499: The ladders are cut to their grammars

Status: decided (the user, 2026-11-20: "yes, remove the other ladders too", after Kotlin's, 0484). Follows 0484.

## Intro

**What a ladder was.** Each Bitlex language was built a construct at a time. Every step was a `.bitlex` file in a
`bitlex_ladder/` directory, and the tests checked every step against the reference before the next one started:

- JSON's: thirteen made-up languages (`vrai`, `liste`…), then JSON (step 14) and JSON with comments (step 15);
- `.rr`'s: 22 steps, from comments to a rule file's tests;
- the C directives': 6 steps.

**The problem.** The steps were a way of building, and the grammars are built. Each step was still compiled and
checked on every test run. The user asked for them to go, as Kotlin's did.

## The decision

1. **Each language keeps only the grammar it ships.**
   - JSON: `compiler/bitlex_ladder/14-json.bitlex` is `json_parser/json-rfc.bitlex`, and `15-jsonc.bitlex` is
     `json_parser/jsonc.bitlex`, beside the token loop's `json.bitlex` and `json-simdjson.bitlex`. Steps 01–13 are
     removed.
   - `.rr`: step 22 is `rr_parser/rr.bitlex`. Steps 01–21 are removed.
   - The C directives: step 6 is `c_directives/directives.bitlex`. Steps 01–05 are removed.
   - Each grammar's header now says what it reads, not what its step added.

2. **The checks stay, on the grammars.**
   - `ladder::check_ladder` checks a list of grammars, as it checked a ladder.
   - `ladder::grammars()` is JSON's two reference forms (`json-rfc`, `jsonc`). The token loop's forms have their own
     shape (0177) and their own tests (`rerail_parser_tests::json`).
   - The compiler crate's tests (`ladder_tests`, `tier2_tests`, `records_tests`) run on those two. Their fixed
     documents are now JSON in the same roles: keywords, lists, objects, strings with escapes, numbers, both comments,
     cut-off input, and long and deep documents past a block.
   - Floors on how much a test checked are lowered to what two grammars give, still well above zero.
   - `rr_tests` checks `rr.bitlex` against the engine's parser. Each case is marked `parses` or `never`, in place of
     the first step that parsed it. The checks that only an earlier step needed are removed: `NAMED_ONLY`,
     `TEST_BLOCKS` and the token-stripped comparison.
   - `aot_tests` runs three sets: JSON's two forms, `.rr`'s grammar and the three probes. It requires the Rust backend
     to take every grammar.
   - The C directives' `every_ladder_step_compiles` is removed. The crate's other tests compile `GRAMMAR`.

## Found on the way

- **The newline index on a token loop's tier 1.** The `.rr` grammar in the AOT test tripped 0493's debug check. A
  token loop's tier 1 stores no row of the newline output. Fixed in 0493 §5.
- **The split JSON form's store guard** (`json_split_stores_only_what_rows_are_read_for`) now counts the newline
  index as a reader of its rows (0493).
- **Metal on `jsonc`** (Q188): both JSON forms now run on Metal. On 42 of 378 `jsonc` files, all with comments, a
  comment's last bytes (extract 4) differ from the interpreter's. This predates both changes: it fails the same way
  with the old seeds and with the newline output taken out. Settled: the test compares extracts only where tier 1
  accepts the file (e6be1c0f92); it passes on Metal (roll-up 26, 2026-11-21).

## Tests

- `rerail-compiler`: the whole library;
- `rerail-rr-parser`: the whole library;
- `rerail-c-directives`: the whole library;
- `rerail-parser-tests`: `rr_tests`, `aot_tests` (each set), and JSON's quick tests;
- `metal_tests::every_grammar_on_metal_equals_the_interpreter`: red when this was written, as above; passes since
  e6be1c0f92.
