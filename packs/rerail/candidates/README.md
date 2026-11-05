# Rerail rule pack: Rerail repository, candidates

The rule documents in `rules/` here were drafted for the pack and not approved, so they don't ship. This file is not
published; the pack's [README](../README.md) covers the rules it ships, and [HISTORY.md](../HISTORY.md) the study
behind every rule that was considered.

## Candidates

| Rule | Why it waits |
|---|---|
| `rerail-cli-entry-no-decisions` | It catches none of its six recorded breaks, which were all written inline, and about half its matches are code it should leave alone |
| `rerail-tape-parse-once` | The compiler enforces it better: route `encode_tape` (`native/src/code_rule_dsl/workspace/evaluate.rs`) through `TapeImage::parse` and make the encoders private |

## Rule details

What each rule matches beyond its one-line description: exact matching and edge cases.

### `rerail-cli-entry-no-decisions`

A code rule in Log over `scripts/release/**`, `scripts/host/**` and `src/cli/**` (TypeScript and JavaScript, tests and
declaration files left out). It fires when a script whose top level does work when imported gains a branching function
that runs no command and writes no output, and does not export it: such a function belongs in a module beside the
script, where tests can import it (`scripts/release/packs.mjs`, `core-paths.mjs`). The six recorded breaks
([HISTORY.md](../HISTORY.md#rerail-cli-entry-no-decisions-a-script-that-runs-on-import-holds-no-decision-logic-not-approved))
were written inline in the script body rather than as a separate function, so this shape misses them.

### `rerail-tape-parse-once`

A code rule in Log over `native/src/**/*.rs`, leaving out `code_rule_dsl/tape.rs`, `code_rule_dsl/tape/`,
`code_rule_dsl/blade/forte/` and test modules. It flags calls to `TapeImage::encode`, `encode_blade` and
`encode_template`, and a second parse-then-encode of the same file, outside `TapeImage::parse`. Today it matches the
copy in `workspace/evaluate.rs`; #48 and #132 each had to edit both copies.
