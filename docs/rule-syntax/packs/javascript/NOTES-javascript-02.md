# Notes: batch javascript-02 (six ESLint rules made to report what their JSON originals report)

Rules: eslint-no-empty-function, eslint-no-extra-bind, eslint-no-irregular-whitespace, eslint-no-lonely-if,
eslint-no-magic-numbers, eslint-no-redeclare. Each was translated already and `different` at the start; each is
`same` now (`translations:check --pack javascript --rules <id>`, every corpus: examples, fixtures, rerail-typescript).

Ground truth is the pack's tool, ESLint 10.11.0 (`conformance.json`), run with default options on each counterexample
(and ESLint 9.39.5 where it differs, below). In every case below the original agreed with ESLint and the translation
was wrong, so each fix follows both. No library file changed; no new library.

## Per rule

### eslint-no-empty-function (56 fixtures original-only → same)

`function foo() {} // allow: functions`: tree-sitter hangs a line comment that follows the `}` of a function
declaration inside its `statement_block` (the block's text runs to the line's end), and `empty_block`
(`lib/tsx/eslint-a-functions`) counts every child, so the body looked commented. ESLint reports it. The rule now has
its own `empty_body(b)`: a child counts only when a `}` is in a structural gap after it (comments are nodes, so a gap
never holds a comment's text). The fix (`explain`) now applies only to a block with no child, since it rewrites the
whole block and would drop a hung comment.

Library request: `lib/tsx/eslint-a-functions.rerail`'s `before_close(b, x)` says "x starts before b's `}`" but is
`child(b, x, _)`; its comment says the `}` is the block's last character, which tree-sitter's trailing line comment
breaks. Define it as this rule's `before_brace` does (`child(b, y, j), j >= i, structural_after(y, g),
contains("}", g)`), and `eslint-no-empty-function` can go back to `empty_block`.

### eslint-no-extra-bind (8 fixtures translation-only → same)

`var a = function() { return this.b }.bind(c)` was reported: `!uses_this(f)` held for every function. Two engine
behaviours (below, E1 and E2) made the old `count : { uses_this(f, t) } = 0` and a `nearest(t, r, frame),
fn_region(f, r)` form both wrong. `uses_this(f)` now starts from `bound(f, …)` and asks for a `this` within f with no
`this_frame` (common's: a non-arrow function, a class field's value, a static block) strictly between.

Frames follow ESLint 10.11: its no-extra-bind enters a scope at `PropertyDefinition > *.key:exit` and `StaticBlock`,
so `var a = function() { class C { field = this.x; } }.bind(b)` is reported (fixtures 047–049). ESLint 9.39.5 has no
such handlers and does not report it; the pack's tool is 10.11, so the rule follows it.

### eslint-no-irregular-whitespace (5 original-only, 2 translation-only → same)

ESLint's lines end at U+2028 and U+2029 as well as `\n`: `foo<LS><LS>` gives findings at 1:4 and 2:1; `var a = 'b',<LS>c
= 'd',\ne = 'f'<LS>` gives 1:13 and 3:8. The tape's lines end at `\n` only. Each finding's line is now the tape line
plus the separators before it in the document (`es_line`), and a finding whose line moves sits at `@place(path, line)`
(the violation is `at: loc`).

A second fault, found by a counterexample and not in the corpora: `'<LS>'; y<NBSP>` missed y's space (ESLint: 2:5).
`end_line` counts the U+2028 inside the string, so the string seemed to end on line 2 and to cover y's space. The skip
test now measures a skipped node's end from its own text, counting `\n` only (`stops`).

### eslint-no-lonely-if (2 fixtures translation-only → same)

`if (a) if (a) {} else { if (b) {} } else {}` was reported; ESLint does not. `areBracesNecessary` asks whether the
token after the block is `else`; `needs_braces` (`lib/tsx/statements.rerail`) asks it only when the block is an if's
own consequence. Here the block is an inner if's else, which ends the outer if's consequence. The rule adds
`else_bound(b)`: a dangling if in b, and b ends on the same byte as an if's consequence that has an else. Checked with a
comment before the outer else, and with `if (a) { if (x) f(); else { if (c) bar(); } } else baz();` (reported, as ESLint).

Library request: give `needs_braces`'s third clause this form (end byte shared with a consequence that has an else),
so curly gets it too: `if (a) if (b) {} else { if (c) d(); } else e();` keeps its braces in ESLint's curly.

### eslint-no-magic-numbers (894 fixtures and 112 rerail-typescript translation-only → same)

Every number was reported, `var a = 5` and `{ a: 5 }` included, and both `-42` and `42` for `-42`. Every negation in
the rule held, because its nodes came out of a relation that joins `value(n, v)` (engine behaviour E3). The rule now
takes its nodes from the tree alone (`num(n, f)`: the literal, or the unary holding it), its message text from `text`
(`raw_of`), and values only where the options compare them (`signed(f, v)`, used positively). The sign test for `-0`
compares values instead of negating `zero_value`.

### eslint-no-redeclare (5 fixtures original-only → same)

- `/*globals\nArray */` reports Array on line 2. The rule built `@place(f, l)` inside `listed(c, x, at)`, a relation;
  a place is made only in a violation's head (ALIGN-FINAL-SHAPE: "`@place` as a value outside a violation's head"), so
  the row never existed. The comment findings now go through `comment_redeclares(c, l, msg)`, and two heads put them at
  the comment or at `@place`.
- `/*globals a */ /*globals a */` reports the second: the duplicate test used `earlier(o, c)`, AST visit order, which
  has no comments. It compares start offsets now.
- `off` items follow ESLint's linter (`addDeclaredGlobals`): every comment that lists x declares it, `x:off` ones
  too, unless the last of them says `off`. `/*globals a:off */ /*globals a */` reports the second (ESLint: 1:30);
  `/*globals a */ /*globals a:off */` reports nothing.

## Engine behaviours met (each worked around in the rule; proposed for the engine)

- E1. A relation over `this_owner` alone, negated, held for no node: `has(f) :- this_owner(_, f).` then
  `bound(f, …), !has(f)` held for a function whose `this` is its own (and `count : { uses_this(f, _) } = 0` and `= 1`
  both held for it). With `bound(f, …)` and `within(t, f)` in the negated relation's body it is right.
- E2. `nearest(t, r, own_frame), fn_region(f, r)` with f bound first held for a `this` inside a nested function
  declaration (r should have been the inner function's region).
- E3. `lv(n, v) :- number_lit(n), value(n, v).` then `lv(f, _), !kind(f, "number")` and `lv(f, _), !is_num(f)` held
  for every number; the same with `text(n, t)` in place of `value` was right.
- E4. A `@place` built in a non-violation relation yields no row and no error.

All four were seen with `~/pt/bin/rerail-engine run-policies` on the fleet box; minimal probes are the snippets quoted.
