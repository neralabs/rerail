# 0194: Every language's names library speaks main's binding terms; main's other packs pass the reader

Status: decided (2026-11-13, in the work to run main's rules directly: "please address these", on what decision
0191 left open).

## Intro

Decision 0191 gave Python's and TypeScript's names libraries main's binding terms: `bind_name(name, site)`,
`main_form(site, form)` and the site filters. Two things stayed open:

- Python had no site for main's `global` and `deletion` bindings.
- Main's other packs had never been checked. Of the 31 JavaScript, TypeScript, React, Vue, Rust and SQL rules
  that use `binds`, 28 failed the reader.

The failures had four causes:

```
#include "rust/names.rr": no such file            // no names library for Rust, Vue or SQL
relation site_scope is not declared                // tsx/names.rr lacked site_scope, binding, site_referenced, …
no grammar "eslintDirective"                       // main parses `/* global x */` comments with its own grammar
line 158: n                                        // translator output with an ungrounded variable
```

## Decision

1. **Each language library defines the binding interface.**
   - `lib/rust/names.rr`, `lib/sql/names.rr` and `lib/vue/names.rr` (which includes `tsx/names.rr`) are new.
   - `lib/tsx/names.rr` gains `binding`, `site_scope`, `site_referenced`, `site_exported`, `site_builtin`,
     `site_annotation` and `site_directives`.
   - Each one is written after main's scope code for that language (`scope/rust.rs`, `scope/sql.rs`), and comments
     where it approximates.
2. **Python's `global` and `deletion` sites.** `global x` in a function or class binds x there (`glob_site`). An
   unconditional `del x` unbinds it (`del_site`): `del x, y`, `del (x, y)` and `del [x]` too. A `del` under an
   `if`, `while` or `match`, a loop's or `try`'s `else`, or an `except`, is Ruff's conditional deletion, a
   different binding, and gets no site.
3. **`resolves: {unresolved: true}` is a library term outside Python**: `unresolved_ref(r)`. In tsx, r resolves
   to no declaration and names no global, neither the environment's nor a `/* global */` comment's. Python keeps
   its own lowering.
4. **`eslint-directive` is a guest dialect**, like the format strings of decision 0176.
   - It is the registry language `languages/eslint-directive.json` (family `format`, built with `typescript`,
     tape tag 17).
   - Main's directive parser (`literal::eslint_directive`) writes its tree: `directive`, `item`, `setting` and
     `description`.
   - The literal tape parser (`literal::format_tape`) now names no grammar. A dialect's grammar is the registered
     literal grammar its label names in camel case. So a grammar another crate holds needs no edit there.
5. **The translator's output is grounded, or is not written.**
   - A util no finding reaches is not emitted. A relation counts as reached wherever it is named, called or
     passed as an argument (`within_stop(c, p, stop_40)`).
   - A capture named only inside an aggregate or a negated atom (main's `not`) does not leave it. Inside an
     aggregate it becomes the aggregate's own variable; in a negated atom it is `_`; outside, it is `unset`.
   - A `stopBy` of negations only ranges over the nodes. Simplify keeps `node(_, x, …)` unless a positive literal
     outside an aggregate names x.
   - A relational clause (main's `relational`: evidence queries joined as rows) is not translated yet. It is
     written as a declared finding with no rules, and a note says so. Before, it became a text rule over an
     empty JSON term, which the reader refused.
   - Clippy's suppressions (`lints`) are read like ESLint's `rules` and noqa's `codes`.
   - A message that is an `$option` reads as its default text.
   - An `anchor: end` finding is at the node's last line.

## Consequences

- 30 of the 31 rules pass the reader. All 537 of Python's still do. The Python pack changes in 28 files (dead
  utils removed, `binds: {}`, stop sets), with the same findings on the one-file corpus. The Ruff fixture
  comparison is unchanged: 11 equal.
- On their probes: AL05 gives every expected finding on its 47 upstream fixtures; n-handle-callback-err matches
  all 17 invalid fixtures; prefer-const and immutability give the expected findings. The new Python sites give
  lines 3, 5, 7, 10 (both names) and 15 on a probe of 15 lines.

Open:

- typescript-eslint-explicit-module-boundary-types fails the reader. Its utils pass `LATE` through a recursive
  group (`exported`, `checked`, `checked_higher_order`), and the demand pass makes no demand carrying `LATE` for
  any of them, so `early-enough`'s line comparison is ungrounded.
- **Main's `typescript` reads `.js`; the engine's does not.** The registry's TypeScript language has the
  `typescript` and `tsx` dialects. `.js` is the `javascript` language, with its own grammar. So the JavaScript
  pack's rules (`.language typescript`, `.js` scope) read no file. Either the translator picks the language from
  the scope, or `.js` is also read as TypeScript; that choice is open.
- Vue: main's rules match its view, one program made from the scripts and the template (`vue/view.rs`). The engine
  builds no such guest, so the Vue rules pass the reader but find nothing. The translator also drops `embedded`.
- Rust: clippy-many-single-char-names hits the 2 GB cap while compiling its 6 KB rule body. `redefines` does not
  tie the name to the site, which matters where a site binds several names (Rust's `let (a, b)`).
- SQL: `compare` with `text: "name"` reads `value`, which SQL identifiers do not have.
- ESLint: `directives: "eslint"` is written as its own conjunct. In main it widens `builtin` and `exported`.
  `site_directives` holds for every site until it does.
- Several of the 30 have run-time refusals of their own (no-eval's `scriptDeclarations`, no-redeclare's
  `redefines` with `every`, no-shadow's `unset`). None of these packs is in the checked set yet.
