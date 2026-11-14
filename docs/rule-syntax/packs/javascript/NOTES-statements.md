# Notes: tsx/statements, eslint-curly, eslint-one-var

> The core additions proposed below are now in lib/core.rr (2026-11-05).

These are hand-written versions of the two ESLint rules that `site/translate.py` produces from
`packs/javascript/rules/eslint-curly.json` and `eslint-one-var.json`. Both are written on top of a new
library, `lib/tsx/statements.rr`. The library supplies the words the generated text was missing, such as
"the body of this if", "a block that needs its braces", "a one-line statement", "removing these braces
would join two lines", and "an earlier declaration in the same scope". With those words, each rule reads
as ESLint's own description of it.

## Assumptions

- The `option` relation holds only configured values. `option("option", m)` is the string form
  (`"never"`), and the object form's keys are their own names (`option("var", "never")`,
  `option("separateRequires", "true")`). "No options at all" is `!option(_, _)`.
- Option names and value sets are kept as the JSON declares them: curly has `option` and `option2`,
  and one-var has `option`, `var`, `let`, `const`, `using`, `awaitUsing`, `initialized`,
  `uninitialized` and `separateRequires`.
- Every finding has code `curly` or `one-var`. curly's findings sit on the body, as ESLint's do.
  one-var's sit on the declaration's keyword token (`keyword(d, t)`). For a for-in/of head that is the
  `var`/`let`/`const` token, not the `for`. The `suppress` lines are copied from the JSON.
- `.pragma "magic-transform" "a,b,c"` takes a comma-separated list, as Soufflé's does.
- Node shapes were checked by parsing with tree-sitter-typescript 0.23.2 and tree-sitter-javascript
  0.25.0, built from the cargo registry:
  - TypeScript has no `using_declaration`. `using a = f(), b = g();` is an expression statement holding
    a sequence of assignments, and the first carries an anonymous `using` token. `await using` wraps
    the first assignment in an await_expression. The library reads that shape (`ts_using`) as a
    declaration whose declarators are the assignments. The JavaScript grammar's `using_declaration`
    is read too.
  - `declare let a;` is an `ambient_declaration` holding the declaration, and `decl_stmt` sees through it.
  - A namespace body is a `statement_block` under `internal_module` (or `module`).
  - Sloppy `for (var x = 0 in o)` puts a `value` field on the loop. A for head is its own declarator,
    so this one counts as initialized.
  - `<!--` comments are `html_comment` extras, and `is_comment` covers them wherever comments are skipped.

## Proposed for core.rr

The library has one block for core, at its top: tokens. The engine reads them off the tree-sitter tree
as spans, skipping comments, as ESLint's token store does.

```
.decl first_token(n: node, t: span)
.decl last_token(n: node, t: span)
.decl token_before(x: loc, t: span)
.decl token_after(x: loc, t: span)
```

curly needs them for four things. ESLint's one-line tests measure from "the token before the body" to
"the last token, not counting `;`". The unbrace safety test looks at the token before `}` and the token
after it, and asks whether that token starts with `( [ / \` + -`. The unbrace fix itself deletes the two
brace tokens. And "an `else` follows this block" is `token_after(b, t), text(t, "else")`. one-var uses
them to put a for head's finding on its `var` token: `token_before(left, t)`. None of these
can be told from named nodes: `)`, `else`, `;` and `}` are anonymous. The JSON gets the same answers
from ast-grep `layout` terms, which have no .rr form. No `wrap`/`unwrap` edit is proposed for core.
Wrapping is `replace(b, \`{$b}\`)`, and unwrapping is the library edit `unbrace(b)`, made of two
`replace`s on the brace tokens.

## Where the JSON deviates from ESLint (these rules follow ESLint 9.39)

curly:
- Under `consistent`, the JSON makes a chain want braces only when some body needs them (or, in
  multi-or-nest, when the body itself expects them). ESLint, and this rule, also count a body that
  expects braces under the mode, and a braced body where either form is allowed. So with `multi-line`
  or `multi-or-nest` plus `consistent`, this rule reports some "Expected {" that the JSON does not, and
  fewer "Unnecessary {".
- When unbracing is unsafe, the JSON's fix replaces the block with itself (a no-op). ESLint offers no
  fix there, and neither does this rule.
- The JSON computes one-line tests from layout. This rule uses ESLint's token definitions.
- Two fixes are deliberately safer than ESLint's. Where a keyword before the body runs straight into
  it, as in `else{c()}` or `do{x()}`, the `{` becomes a space: ESLint writes `elsec()`. And no unbrace
  fix is offered for a block holding a TypeScript enum or interface: ESLint reports these, and so does
  this rule, but its fix leaves `if (a) enum E {}`, which does not parse. `abstract class` counts as a
  declaration, so its braces are needed and are neither reported nor fixed.

one-var:
- ESLint 9.39's schema takes either the kind keys (`var`, `let`, `const`, `using`, `awaitUsing`,
  `separateRequires`) or `initialized`/`uninitialized`, never both. Some configurations it allows still
  make the JSON's `when` gates run var/let/const "always" checks where ESLint checks nothing:
  `{separateRequires: true}` alone, `{using: …}`, and `{uninitialized: "consecutive"}`. This rule
  checks nothing for a kind the object leaves out.
- `using` / `await using`: the JSON implements only always, and only on the JavaScript grammar's
  `using_declaration`, which TypeScript's grammar never produces. This rule reads both grammars, and
  never and consecutive apply to these kinds too, as in ESLint.
- Message texts: the JSON keeps ESLint's exact strings. Here the message id survives as the `c` column
  ("all", "initialized", "uninitialized"). Keys are unaffected.
- Neither the JSON nor this rule offers one-var's fixes. ESLint has join and split fixes.
- ESLint quirk, matched: a TypeScript namespace body opens no scope for let/const, so
  `let a = 1; namespace N { let b = 2; }` reports `let b` under always. `namespace_body` takes those
  blocks out of the lexical scopes.
- ESLint's scope bookkeeping is matched too. A declaration that one-var reports does not claim its
  scope. `crowded` is `ordered by earlier` (core's addition 6): it reads itself only at earlier
  declarations.

## Could not express / open

- An .rr policy has one `.language`. The JSON's using clauses name the javascript grammar. Which
  grammar runs on .js files is up to the engine, and the library reads both shapes.
- `tsx/names` defines `fn_scope`, which lacks `generator_function` and `class_static_block`. This
  library keeps its own `var_scope`/`lexical_scope`, following ESLint's stacks, instead of reusing
  `fn_scope`.

## Checks

- `python3 site/check_rr.py` reports `ok` for grounding, stratification (now including `packs/`,
  where `crowded`'s `ordered by earlier` tie is accepted) and duplicate declarations.
- End to end against ESLint 9.39.5, using the reviewer's runner. A scratch harness emits real
  tree-sitter facts (TypeScript or JavaScript grammar) with tokens, then lowers core + library + rule
  to Soufflé. For the lowering, `nearest` is defined over parent edges, and `ordered by earlier` is
  run to a fixpoint by feeding `crowded` back in. The harness applies curly's fixes and compares the
  reported lines. The reviewer's two case files (c1, c2: 26 cases) match ESLint's lines, and so do
  14 more cases. Those cover the `export` mixed-requires case, html comments, sloppy for-in
  initialisers, enum and interface blocks, TypeScript `using`/`await using` under never, always and
  consecutive, `var` in a namespace, and `declare` under always. Fix output matches ESLint except in
  the deliberate cases above.
