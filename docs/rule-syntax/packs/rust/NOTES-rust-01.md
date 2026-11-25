# rust-01: five Clippy rules against their JSON originals

Ground truth: Clippy's UI tests in the recorded fixtures (`//~^ lint` marks each expected finding) and Clippy's
source (`clippy_lints/src/literals/decimal_literal_representation.rs`, master). No `.rerail` changed: every
remaining difference is either the JSON being wrong (the translation follows Clippy) or the suite's run of the
translation not applying `suppress clippy`.

## Engine gap: run-policies applies no `suppress clippy`

The originals go through the product path, which drops a finding where `#[allow]`/`#[expect]` of a listed lint is
in force (`suppress/clippy.rs`, `ClippyIndex`; an item's attributes cover the item and its own attributes).
`rerail-engine run-policies` returns the translation's diagnostics with no suppression applied or marked, so the
suite counts every finding Clippy's lint levels silence. Checked on a box with the integration engine:

```rust
#[allow(clippy::double_parens)]
fn f() { let _ = ((1)); }        // clippy-double-parens translation still reports line 2
```

The rules declare the same `suppress clippy` lints as their JSON; the fix belongs in run-policies (apply or mark
the policy's `suppress` lines as the product path does), as NOTES-misc-untranslated.md (python, `# nosec`) also
records. Not worked around in the rules: re-deriving rustc's lint levels per rule would duplicate `ClippyIndex`.

## clippy-allow-attributes: different, original right

fixtures, 2 translation-only, both the gap above:
- `allow_attributes.rs:26` `#[allow(clippy::allow_attributes, unused)] struct Allowed;`: the attribute allows its
  own lint (rustc sets an item's levels before its attributes are checked). No `//~^` in the UI test.
- `allow_attributes.rs:30` `#[allow(unused)]` under `#[expect(clippy::allow_attributes)]` on the same item.

## clippy-decimal-literal-representation, -threshold: different, translation right

rerail-native, 2 translation-only each, `native/src/code_rule_dsl/compile/config.rs`:
- 1062 `9_007_199_254_740_993u64` = 0x20_0000_0000_0001;
- 1071 `-9_007_199_254_740_992i64`: the literal is 9_007_199_254_740_992 = 0x20_0000_0000_0000 (the `-` is a
  unary operator; Clippy's `check_lit` sees the literal).

Clippy formats `val` (a u128) as hex and lints when the first digit is 1/2/4/8 and the middle digits are all 0
(`check_text`): both qualify, at either threshold (16384, 0xFFFFFF). The JSON caps its `value` tests at
`lt 9007199254740992` (2^53, a JavaScript number's exact range), so it misses every literal from 2^53 up. The
translation reads values up to 10^18 (`int_lit_value`); longer literals stay unread (NOTES-rust.md, (b)).

## clippy-double-parens: different, translation right

fixtures, 1 translation-only: `double_parens.rs:64` `assert_eq!(((1, 2)), (1, 2), "Error");`, which the UI test
marks `//~^ double_parens` (Clippy lints the expansion). The JSON reads only macro_rules! transcribers, not std's
expression macros' arguments; the translation reads both (NOTES-rust.md, (d)).

## clippy-inconsistent-digit-grouping-no-fractions: different, original right

fixtures, 1 translation-only: `ui-toml/lint_decimal_readability/test.rs:6` `100_200_300.123456789` inside a
function whose `#![allow(clippy::inconsistent_digit_grouping)]` silences it; no `//~^` there. The same literal at
line 17, unsuppressed, is reported by both. The run-policies gap above.

## Library changes

None.
