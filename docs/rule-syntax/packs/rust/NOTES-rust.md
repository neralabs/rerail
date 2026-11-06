# rust: 33 Clippy rules, hand-written

The 33 rules have 889 lines in all, against 2,552 in the machine translation. They share 485 lines of library in
four files:

| file | what |
|---|---|
| `lib/rust/syntax.rr` | macro bodies and token trees, format strings, comments, parentheses, binary operators, attributes, `#[clippy::msrv]` (`msrv_older`), struct fields |
| `lib/rust/syntax-literals.rr` | number literals as Clippy's NumericLiteral splits them: digit groups, hex digits of a value, raw-string hashes, octal escapes |
| `lib/rust/syntax-patterns.rr` | patterns: `ref`/`mut` wrappers, `..`, `_` elements read through tokens, parentheses, tuple-like elements |
| `lib/rust/syntax-attrs.rr` | lint attributes (`allow`, `expect`), direct or out of a `cfg_attr`, and their `reason` |

## Scores

The scores come from `rrlab score rust ID --all`, matched by line and column. Two things were added on top of
rrlab, in a scratch wrapper, because rrlab models neither (proposals 1 and 2 below):

- Clippy's in-source `#[allow]` and `#[expect]`;
- the conformance `fixtures` globs, which confine a ui-toml rule to its own directory.

"ex" counts findings that `divergences.json` already explains; rrlab drops these from the score. Every one is a
parse error in tree-sitter-rust 0.24: `box` patterns, and an attribute on a tuple element.

| rule | lines | TP/FP/FN | deviations |
|---|---|---|---|
| allow-attributes | 21 | 25/0/0, 1 ex | cfg_attr predicates (a), (a') |
| allow-attributes-without-reason | 26 | 47/0/0, 1 ex | cfg_attr predicates (a), (a') |
| builtin-type-shadow | 19 | 1/0/0 | |
| decimal-literal-representation | 21 | 8/0/0 | (b), (c) |
| decimal-literal-representation-threshold | 22 | 1/0/0 | (b), (c) |
| double-parens | 39 | 25/0/0 | also reads std expression macros' arguments (d) |
| else-if-without-else | 15 | 4/0/0 | |
| inconsistent-digit-grouping | 18 | 9/0/0 | |
| inconsistent-digit-grouping-no-fractions | 17 | 1/0/0 | |
| int-plus-one | 26 | 14/0/0 | syntactic, as the JSON: floats too |
| large-digit-groups | 17 | 4/0/0 | |
| many-single-char-names | 95 | 3/0/0 | ASCII names only (e) |
| mixed-case-hex-literals | 15 | 4/0/0 | (c) |
| needless-arbitrary-self-type | 22 | 9/0/0 | |
| needless-else | 17 | 1/0/0 | |
| needless-raw-string-hashes | 23 | 15/0/0 | (f) |
| needless-raw-string-hashes-one-allowed | 21 | 3/0/0 | (f) |
| needless-raw-strings | 16 | 15/0/0 | |
| octal-escapes | 27 | 11/0/0 | format strings and concat! skipped, exact columns (g) |
| partial-pub-fields | 26 | 4/0/0 | |
| precedence | 26 | 30/0/0 | |
| precedence-bits | 22 | 24/0/0 | |
| redundant-at-rest-pattern | 19 | 6/0/0 | |
| redundant-field-names | 24 | 8/0/0, 1 ex | a macro-body case, listed in divergences.json |
| redundant-static-lifetimes | 40 | 31/0/0 | |
| single-char-lifetime-names | 15 | 9/0/0 | |
| single-component-path-imports | 41 | 5/0/0 | |
| unneeded-wildcard-pattern | 44 | 18/0/0 | |
| unnested-or-patterns | 83 | 33/0/0, 2 ex | struct patterns with `..` (h) |
| unreadable-literal | 19 | 16/0/0 | |
| unreadable-literal-no-fractions | 18 | 1/0/0 | |
| unusual-byte-groupings | 17 | 2/0/0 | |
| zero-prefixed-literal | 17 | 5/0/0 | follows Clippy, not the JSON (i), (c) |

All 33 score perfectly on their fixtures. I also ran each rule on scratch counterexamples beside Clippy 0.1.99
(`clippy-driver -W clippy::LINT`). Those runs agree except where noted below.

The two longest rules carry Clippy's algorithm:
- **many-single-char-names** keeps a stack of scopes with push times, shadowing and the report point.
- **unnested-or-patterns** has a nesting test for each pattern constructor.

## Deviations

- **(a) cfg_attr.** rustc expands `#[cfg_attr(pred, allow(x))]` before Clippy reads it, so the result depends on
  the build configuration.
  - As in the JSON, every predicate counts as true except three that a lint run never satisfies: `test`, `any()`
    and `not(clippy)`.
  - Nested cfg_attr is expanded the same way.
  - `allow_attributes` reports an `allow` that comes out of a cfg_attr at that `allow`, which is also where
    Clippy reports it.
- **(b) 19 digits and more.** `int_lit_value` handles up to 18 digits, because Soufflé numbers are 64-bit, so longer
  literals go unread.
  - Counterexample: `let _ = 18446744073709551615u64;` (0xFFFF_FFFF_FFFF_FFFF). Clippy reports it; this rule
    does not.
  - See proposal 4.
- **(c) macro_rules! bodies.** Clippy lints a literal in a macro_rules! body once for each use of the macro. Here a
  body is read as it is written, whether the macro is used or not, as the JSON does.
  - Counterexample: `macro_rules! m { () => { 0x1aB } }` with no use of `m!`. Clippy says nothing, and
    mixed-case-hex-literals reports it.
  - zero-prefixed-literal, decimal-literal-representation and octal-escapes behave the same way.
- **(d) double-parens.** The JSON reads macro_rules! transcribers. This rule also reads the arguments of std's
  expression macros (`assert_eq!`, `vec!`, `format!`, …), which Clippy lints after expansion.
  - This closes the `assert_eq!(((1, 2)), x)` case that divergences.json lists for line 64. That entry can now
    be dropped.
- **(e) many-single-char-names.**
  - Names are ASCII `[a-z]`. Clippy takes any single character that is not uppercase, a digit or `_`.
  - Clippy's model, which this rule follows:
    - the scopes are the function, each block and each match arm;
    - a `let`'s names are pushed after its value;
    - the `else` block of let-else counts as part of the let;
    - closure parameters, `if let` and `for` patterns are not counted.
  - Scratch probes for all of these agree with Clippy.
- **(f) needless-raw-string-hashes.** In Clippy, the lint's verdict on `r#"x"#` depends on whether
  needless_raw_strings is enabled.
  - With needless_raw_strings off (Clippy's default), `r#"x"#` gets a needless_raw_string_hashes finding.
  - The recording enables every lint, so the rule leaves such strings to needless_raw_strings.
  - Counterexample: `let _ = r#"x"#;` with only `-W clippy::needless_raw_string_hashes`. Clippy reports it; this
    rule does not.
  - Proposal 6 would make this exact.
- **(g) octal-escapes.** The JSON reported at a regex capture's span. This rule reports at the `\0` escape node, so
  its columns are exact. Clippy reads literal expressions only:
  - it does not report the format string of a printing macro: `println!("\033[0m")`, `write!(f, "\033")`,
    `assert!(c, "\033")`;
  - it does not report anything inside `concat!`, `stringify!`, `env!` or `include*!`;
  - it does not report C strings (`c"\033"`);
  - `println!("{}", "\033")` and `vec!["\033"]` are reported.

  The format argument is found by position (`format_string` in rust/syntax.rr). Literals in a macro_rules!
  transcriber are read, as in (c).
- **(h) unnested-or-patterns.**
  - Clippy 0.1.99 never nests struct patterns that have `..`, even where both sides have it:
    `S { x: 1, .. } | S { x: 2, .. }` is not reported, while `S { x: 1, y: 2 } | S { x: 2, y: 2 }` is. The rule
    follows Clippy.
  - Two shorthand fields `S { x } | S { x }` do not nest. One shorthand and one explicit field,
    `S { x } | S { x: 1 }`, do.
  - Subpatterns are compared by their text, ignoring whitespace and parentheses on elements and fields. Clippy
    compares ASTs. The two can differ only on exotic spellings, such as a comment inside a pattern.
- **(i) zero-prefixed-literal.** The JSON's regex reported `0_0` and missed `0__1`. Clippy tests the value, which must
  not be 0, and digits after a leading `0`, `_` included. This rule follows Clippy: `0__1` is reported and `0_0` is
  not.
- **(a') invalid attributes.** Two cases Clippy never sees, because rustc rejects them first, are still reported:
  - a non-literal reason, `#[allow(x, reason = concat!("a"))]`;
  - an inner attribute after a statement.

  `#[cfg_attr(feature = "x", allow(…))]` is read as live (a): which features are on depends on the build.
- **msrv.** `#[clippy::msrv = "…"]` in the source is honoured through `msrv_older(n, "1.53")`, and an outer
  attribute takes its item's msrv. Clippy also reads `msrv` from
  clippy.toml and `rust-version` from Cargo.toml; those are not read (proposal 5).

## Library and core changes

These are proposals. No existing file was edited.

1. **rrlab: Clippy's lint levels.** A finding should be dropped when an enclosing `#[allow(…)]` or `#[expect(…)]`, or
   the inner form `#![…]`, names one of the violation's `suppress clippy` lints. The scope runs from the first
   attribute of the item's run of attributes to the end of the item, because an item's lint level covers its own
   attributes. In `#[allow(clippy::allow_attributes, unused)]`, the attribute silences itself.
   - The wrapper `rrs.py` does this in about 40 lines, through `Run.suppress`.
2. **rrlab score: the conformance `fixtures` globs.** These globs confine a config-specific rule, such as
   `ui-toml/needless_raw_string_hashes_one_allowed/`, to its directory.
   - Without them, `--all` counts the default-config findings of every other file against the rule.
3. **rrlab: two lowering bugs found on the way.** The rules here avoid both.
   - **`has(a, n, set)` tests the set on `a`.** It lowers to `rr_has(a, n) :- rr_within(n, a), set(a)`, which
     agrees with core.rr's comment ("a is a strict ancestor of n whose kind is in set"). The name, though, reads as
     "a has a descendant n in set", and that is the form a rule wants for `has(u, s, "scoped_identifier")`.
     - Either the comment or the lowering should change. The descendant reading is the one JSON's `has` uses.
   - **A comma list in a string set is one kind.** `nearest(n, s, "block,match_arm")` matches the kind
     `block,match_arm`, that is nothing, and no error is raised. Either split the list on commas, or reject a set
     string that contains a comma.
4. **Core: a 128-bit integer value, or a hex functor.** `value(n, v)` for a Rust integer literal, as a decimal
   string up to u128, plus `@to_hex(v)`. This would remove the 18-digit limit in (b).
5. **Core/config: Clippy's msrv sources.** Facts for `clippy.toml` `msrv` and Cargo.toml `package.rust-version`,
   keyed by crate directory, so that `msrv_of` can fall back to them:

   ```
   .decl clippy_conf(dir: symbol, key: symbol, v: symbol)
   .decl cargo_package(dir: symbol, key: symbol, v: symbol)
   ```
6. **Core: the enabled lint set.** A fact such as `.decl lint_enabled(tool: symbol, lint: symbol)` from the run's
   configuration would let one rule depend on whether another runs. needless_raw_string_hashes needs it, see (f).
7. **Library (next wave): expressions inside std macro arguments.** A std macro's arguments are a token tree, so
   expression lints never see them. This affects int-plus-one, precedence, precedence-bits and
   else-if-without-else.
   - Examples Clippy reports and these rules miss: `assert!(x >= y + 1)` and `println!("{}", 1 << 2 + 3)`.
   - Proposal: a core fact `parsed(t, "rust_expr_args", root)` (the `parsed` relation core already has for
     Python string annotations). It would reparse the token tree of `assert*!`, `debug_assert*!`, `vec!`, the
     printing macros and `dbg!` as a comma-separated expression list, so the expression rules apply unchanged.
     double-parens reads the token trees directly today.
8. **rrlab: `suppress clippy "a", "b"`.** rrlab reads only the first string of the list. The rules list them all,
   as the JSON does.
