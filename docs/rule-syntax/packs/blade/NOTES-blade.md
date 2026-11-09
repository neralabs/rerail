# blade: 10 rules, hand-written (tlint 9.7.0, stillat/blade-parser 2.1.0, Bladestan 0.11.7)

The 10 rules have 191 lines, against 190 in the machine translation (which is short because it leaves the JSON's
`host` and `embedded` terms as opaque `ast(…)` atoms). They share two new library files:

| file | what |
|---|---|
| `lib/blade/syntax.rr` | the engine's Blade tree (Forte's kinds, fields and leaves, as `native/src/code_rule_dsl/blade/forte/tree.rs` documents them) and the compiled PHP program, reached as `parsed(document, "compiled-view", program)`: directives by name, directive blocks, echoes, `blade_code` |
| `lib/blade/values.rr` | `php/values.rr` over the compiled program, with any compiled-view marker (`;`) counted as writing any variable |

The Bladestan rules reuse `lib/php/values.rr` (php_if_const, php_identical) as they are: the program is PHP in
tree-sitter-php's kinds.

## Scores

**None of the 10 can be scored.** rrlab has no Blade grammar (its README: "Blade: no grammar is vendored or in the
cargo registry, so it is skipped"); the engine's Blade tape is Forte's parse plus the compiled view, both native
Rust (`native/src/code_rule_dsl/blade/`). What was checked:

- Review fix: the compiled PHP is not the Blade document's last child. The engine keeps it as a derived document
  of its own (`tape.rs` `DerivedImage`, `embed/compiled_view.rs` `Via::CompiledView`; "no tape holds another"), so
  the first version's `blade_code` (`inside(n, p, "program")` under `document`) could never hold, and the three
  rules over the program and the marker rule were vacuous. `blade_code` now reaches the program as `lib/vue/code.rr`
  reaches guest trees: `parsed(document, "compiled-view", program)`. That needs the engine to give `parsed` rows
  for compiled views (request 3 below).

- `python3 site/check_rr.py` (grounding, strata, declarations): ok for all 10.
- The three rules over the compiled program (duplicate-condition-expressions, if-always-true,
  identical-always-true) were run with `rrlab run` on hand-compiled PHP (`if (c): … elseif (c): … endif;`, as
  `blade/view.rs` writes it), with `blade_code` taken as true: each reports what its opening comment says, and the
  last-elseif and `!`/isset exclusions hold. A `;` in the scope stops `$n = 1; if ($n)` from being followed.
  The real rules compile (run on a PHP file, where `blade_code` is empty: 0 findings).
- `tlint-spaces-around-blade-render-content`'s `after_at` (the JSON's test of the text before the echo) never holds
  in the engine's tree, where `@{{ }}` is an escape; it is kept for parity with the JSON and says so.
- The seven rules over the Blade tree are written against the kinds, fields and leaf texts tree.rs documents; they
  were not run.

| rule | lines | scored | deviations |
|---|---|---|---|
| blade-parser-component-shorthand-variable-parameter | 22 | no | |
| blade-parser-debug-directive | 16 | no | |
| blade-parser-duplicate-condition-expressions | 22 | no (PHP proxy run) | |
| blade-parser-for-else-structure | 17 | no | |
| bladestan-identical-always-true | 23 | no (PHP proxy run) | values without types (a) |
| bladestan-if-always-true | 19 | no (PHP proxy run) | values without types (a) |
| tlint-no-dump-directives | 14 | no | |
| tlint-no-ray-directive | 14 | no | |
| tlint-space-after-blade-directives | 22 | no | reads directives, not the compiled structures (b) |
| tlint-spaces-around-blade-render-content | 22 | no | |

## Deviations

(a) As for the PHP pack's if-always-false (packs/php/NOTES-php.md, (d)): a variable is followed to its one write;
PHPStan reads types. A directive Blade does not compile (an application's own `Blade::directive`) is a marker that
may write every variable (blade/view.rs). A marker's text is `;` whatever directive it stands for (view.rs
`marker` emits `;` over the directive's bytes), so `lib/blade/values.rr` cannot pick out the `@props`, `@aware` and
unknown-directive markers: it treats every empty statement of the compiled view as writing any variable of its
scope. That is sound but loses reports: after an `@foreach` (whose marker only defines `$loop`), `@error` or a
written `;`, no variable of that scope is followed. Request 2 would narrow it.

(b) The JSON finds the compiled `if`/`for`/`foreach`/`while` statement and tests the text of the Blade directive it
came from (`host`), plus a second clause for directives inside attribute values. The rule tests the Blade
`directive` nodes themselves, which is the same set wherever a directive compiles to a structure. It differs for a
control directive Forte leaves unpaired inside an attribute value: the view does not compile it, the JSON skips it,
and the rule reports it (`<div class="@if($a)">` with its `@endif` elsewhere).

## Library and core changes

1. **rrlab (proposal): a Blade tape.** `rrlab-facts` could link the engine's `blade::forte` and `blade::view`
   modules (or a small crate split out of them) to write Blade facts, the document tree and its compiled view (as a
   guest tree, with a `parsed(document, "compiled-view", program)` row), as the engine's tape does. Until then this pack, and `complexity-cyclomatic-blade`, cannot be scored.
2. **Core (proposal): `.decl blade_host(n: node, h: node)`**, the template node a compiled PHP node stands for
   (the JSON's `host`). With it `lib/blade/values.rr` would count only the markers that may write any variable:
   `php_dynamic(s) :- kind(m, "empty_statement"), blade_code(m), blade_host(m, h), blade_directive(h, x),
   (x = "props" ; x = "aware" ; !blade_known(x)), nearest(m, s, php_scope_kind).` Until then every marker counts.
3. **Engine requirement: `parsed` rows for compiled views.** `parsed(r, "compiled-view", p)`, with r the Blade
   `document` and p the compiled program's root, as guest trees give `parsed(raw, "typescript", p)` in a .vue file.
   `blade_code`, and through it three blade rules, the values markers and `complexity-cyclomatic-blade`, need it.

## Duplicates to merge

None.
