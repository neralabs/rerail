# Notes: batch rust-02 (five Clippy rules checked against their JSON originals)

Rules: clippy-many-single-char-names, clippy-needless-raw-string-hashes,
clippy-needless-raw-string-hashes-one-allowed, clippy-precedence, clippy-unnested-or-patterns.

Ground truth is Clippy: its recorded findings for the fixtures (`expected.json` of the `rust` recorded corpus) and
its source for cases outside them. Every key that differs was read and decided. In every case the translation agrees
with Clippy, so no `.rerail` changed. Each difference is one of two things:

- **(S) the suppression gap.** The key sits under a `#[allow(…)]`, `#![allow(…)]` or `#![expect(…)]` that names the
  rule's lint. Clippy and the original honour it. The translation declares the same `suppress clippy` lints as the
  JSON, but `rerail-engine run-policies` does not apply `suppress` settings: its diagnostics carry no suppressed
  state, so the suite sees them. This is the engine's gap, not the rule's logic; python's
  `misc-untranslated.result.json` (bandit `nosec`) records the same thing. Probe: with
  `#[allow(clippy::precedence)] fn f() { let _ = 1 << 2 + 3; }`, `#![allow(clippy::precedence)]` at the top of a file,
  or `#[allow]` on the `let`, run-policies still reports each line.
- **(O) the original is wrong.** The JSON reports or misses a key Clippy does not, and the translation follows Clippy.

## Per rule

### clippy-many-single-char-names: different (S, O)

- fixtures `tests/ui/many_single_char_names.rs:77`, translation only (S): `fn issue_3198_allow_works` carries
  `#[allow(clippy::many_single_char_names)]`. Clippy's recording has no finding there.
- rerail-native `native/src/code_rule_dsl/literal/phpdoc/lexer.rs:148`, original only (O). `fn token(s: &[u8], at: usize)`
  has at most three single-letter names in scope at once: `s`, a `let i` and an arm's `b`. The original also counts
  typed closure parameters (`|i: usize|`), because tree-sitter calls them `parameter`, the same kind as a function's.
  Clippy's `SimilarNamesLocalVisitor` records names only for a function's own parameters, `let` patterns and arm
  patterns. A closure's parameters are walked but never recorded (NOTES-rust.md (e) has the scratch probes).
  - Counterexample: `fn f(a: u8, b: u8, c: u8, d: u8) { let _ = |e: u8| e; }`. Clippy says nothing, and neither
    does the translation. The original reports `a`. With `let e = 0;` instead, both report.

### clippy-needless-raw-string-hashes: different (S)

- fixtures `tests/ui/needless_raw_string.rs:8, 12, 16, 38`, translation only: `r#"\s"#`, `br#"\s"#`, `cr#"\s"#`
  (no hash is needed) and `r##"SELECT * FROM "posts""##` (one is needed). The file starts with
  `#![expect(clippy::needless_raw_string_hashes, …)]`, so Clippy's recording has none of them. Without it, Clippy
  reports all four, as the translation does. The other 15 recorded keys in `tests/ui/needless_raw_string_hashes.rs`
  are on both sides.

### clippy-needless-raw-string-hashes-one-allowed: different (S)

- fixtures `tests/ui/needless_raw_string.rs:38`, translation only: `r##"SELECT * FROM "posts""##` has two hashes and
  needs one, so the one-allowed variant reports it too. It falls under the same crate-level `#![expect]` as above.

### clippy-precedence: different (S)

- fixtures, 12 keys, translation only. All of them are arithmetic operands of shifts and bitwise operators:
  - `rerail/precedence_bits.rs:50, 56, 57, 58, 59`, under `#![allow(…, clippy::precedence)]` (line 2);
  - `tests/ui/precedence_bits.rs:14–20`, under `#![expect(…, clippy::precedence)]` (line 2).

  The 30 keys on both sides are exactly Clippy's `precedence` recording for `rerail/precedence.rs` and
  `tests/ui/precedence.rs`.

### clippy-unnested-or-patterns: different (O)

- fixtures `tests/ui/unnested_or_patterns2.rs:18`, translation only:
  `if let box (0 | 1) | (box 2 | box (3 | 4)) = Box::new(0) {}`. Clippy reports it (the recording has line 18). The
  parenthesised alternative is itself an or-pattern, so after Clippy unwraps the parentheses every alternative is a
  `box` and nests. The original misses it.
- rerail-native `native/src/code_rule_dsl/compile/layout.rs:26`, original only:
  `(Some(_), Some(_), _) | (Some(_), _, Some(_))`. The two tuples differ in two positions. Clippy nests tuples only
  when all elements but one are equal (`eq_pre_post`), so it does not report this. The translation follows it.
  - Counterexample: `(Some(_), None, None) | (Some(_), None, Some(_))` differs in one position. Clippy reports it,
    and so does the translation. The two-position case above is reported by the original only.

## Library and core changes

No library file is needed or changed.

- **Engine: `run-policies` should apply a violation's `suppress` settings.** It should either drop the findings they
  silence or mark them suppressed, as the product does for the JSON's `violation.suppress`. For `clippy`, that is
  `suppress/clippy.rs`'s lint-level reader. Until then, every Clippy translation differs from its original on fixtures
  that `allow` or `expect` the lint. Here those are 18 keys across three rules, and none is a fault of the rule. This
  is proposal 1 of NOTES-rust.md, now about the suite's engine path rather than rrlab.
