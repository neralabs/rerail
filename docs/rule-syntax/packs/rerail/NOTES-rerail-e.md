# rerail pack, batch E: notes

Five rules of `packs/rerail/rules/*.json`, each with the JSON's directives (revision + 1), regexes and messages
copied exactly, and tests that include the pack.json example (caught and passes).

## rerail-no-message-classification (2 policies)

- `rerail-no-message-classification.rerail` (typescript, `src/**/*.ts`) and `.rust.rerail` (rust,
  `native/src/**/*.rs`): the JSON's clauses split by language, each with the JSON's excludes for its files.
- The JSON's patterns (`$M.$F($A)`, `$M?.$F($A)`, `$R.test($M)`, `$R.exec($M)`, Rust `$M.$F($S)`) stay pattern
  atoms; `$A === $B` and its kin are a `binary_expression` with that operator (`op`). `nthChild: 1/2` is `child(n, c,
  0/1)`; `inside` without `stopBy` is the direct parent.
- `resolves.binding` is tsx/names' `ref_site` + `binding_value`; `exported: false` is `!site_exported`; the
  `occurrence … inside new/throw` exclusion is `site_occurrence` + `inside(o, _, {new_expression, throw_statement})`.
- Rust: the receiver regex is tested only on the kinds whose text can match it (identifier, field, call, `*`/`&`
  expression), which grounds the relation; no other kind's text can match the regex. Test code (`follows` an
  attribute through attributes and comments) is rust/syntax's `attached`, with the JSON's attribute regex.

## rerail-generator-placement (4 policies)

- `.rerail` (typescript: .js/.mjs/.cjs/.ts/.mts/.cts), `.python.rerail`, `.php.rerail`, `.shell.rerail` (text
  only: no grammar reads shell; `Bash` is a test-only grammar in language.rs). Each takes the JSON's includes for its
  suffixes and the excludes that can apply to them (`test_*.py`/`*_test.py` only in Python, `*Test.php` only in PHP).
- Anchored callee regexes are globs where exactly equal (`{fs,fsp,…}{.promises,}.…`, `**.write_text`); PHP's
  optional leading `\` stays a regex.

## rerail-ui-api-precedence, rerail-ui-pure-modules (1 policy each, javascript)

- Node regexes: a whole match where the JSON anchors both ends, `(?s:.*)…(?s:.*)` searches otherwise.
- pure-modules: the JSON's shadowing exclusion is kept as it is: a global of a group (the I/O names; document;
  window/globalThis/self; location; history) is exempt under an ancestor that binds ANY name of that group, as an
  arrow's single parameter, an identifier anywhere in a parameter list (defaults included), or a declarator
  directly in a block or the program. A destructured parameter binds no `identifier`, so both sides report
  `function here({ window }) { return window.location; }` (checked on the box: same, 2 findings). The JSON's
  stated intent would exempt it; the translation follows the JSON.

## rerail-ui-color-tokens (3 policies)

- `.rerail` (css, `ui/**/*.css`, excluding `ui/tokens.generated.css`), `.javascript.rerail` (`ui/**/*.js`,
  `ui/**/*.mjs`), `.html.rerail` (`ui/**/*.html`): every file has a grammar, so no text-only policy.
- The seven `file.content` regexes are one library, `lib/rr/rerail-ui-colours.rerail` (new, this batch's), shared
  by the three policies; each reports at the JSON's capture with `match_group`. The messages' `$colour` / `$token`
  are `{c}` / `{t}`, the capture's text.

## Library additions

- `lib/rr/rerail-ui-colours.rerail`: `raw_hex`, `data_uri_hex`, `colour_function`, `named_colour`, `effect_colour`,
  `nera_token_defined`, `nera_token_set` (each `at: span`).

## Tooling

- A new library file is found only with `RERAIL_RR_LIB` pointing at this tree's `lib/` (`#include` reads the
  engine checkout's library otherwise: `#include "rr/rerail-ui-colours.rerail": no such file`). The checks ran
  with it set; the translation suite sets it to the checkout's own `lib/`.

## Validation (box, engine at 7efcac9c67)

- test-rr: 30 passed, 0 failed (10 + 7 + 3 + 4 + 6 over the 11 files).
- Original vs translation, findings keyed (path, line, code):
  - rerail-no-message-classification: caught same (1), passes same (0), repo same (11).
  - rerail-generator-placement: caught same (1), passes same (0), repo same (0).
  - rerail-ui-api-precedence: caught same (2), passes same (0), repo same (7).
  - rerail-ui-pure-modules: caught same (2), passes same (0), repo same (0).
  - rerail-ui-color-tokens: caught same (2), passes same (0), repo same (0).
- Every test's fixtures as a corpus of its own (30 corpora): same on all 30.
- `site/check_rr.py`: no problem in these files (14 pre-existing elsewhere).
