# Open issues in the .rr drafts

Found by reading the code while commenting it (2026-11-03). Nothing here has been run; each item is a likely bug to
confirm once a parser and engine exist. None is fixed yet.

## lib/tsx/names.rr, lib/tsx/jsx.rr
1. `ref` excludes identifiers in a `name` field, which is where JSX tag identifiers sit: tags get no `ref_site`/`sym_of`.
   This breaks `fragment`, `provider_el` and the router example.
2. `decl_site` "pattern" needs a declarator that already has a `decl_site`; the var/let rules accept identifiers only,
   so `const {a} = …` declares nothing.
3. Params match `identifier` only: `function Layout({ children })` and `x => …` (field `parameter`) declare nothing.
4. `ns_target` follows `* as` imports only, so `React.Fragment` via `import React from "react"` does not resolve.
5. var/let classification uses `nearest` to any declaration ancestor, so a nested function's `let` inside a `var`
   initializer is tagged "var".
6. `export default function App(){}` is likely exported as `App`, not `default`.
7. `export * as ns from "m"` is treated as plain `export *`.
8. `export { a as b }` without a source does not follow the local `a`.
9. `tsconfig` baseUrl, `package`, `package_export` are unused; paths join the tsconfig dir rather than baseUrl;
   `resolve_base` lacks `.d.ts`, `/index.jsx` and the ESM `.js` → `.ts` mapping.
10. `def_of`/`decl_site` miss `export var`, `enum`, `namespace`, abstract classes, generator and class expressions.
11. jsx: `render_stop` includes `statement_block` (block-bodied map callbacks end the climb); `child_callback` rejects any
    attribute ancestor at any depth; `render_attr` takes the nearest attribute at any depth.
12. `directive_global` and `es_global` are `.decl` with no rules; they should be `.native` or shipped facts.

## lib/python/*
1. names: `imported_sym`, the `site_*`/`scope_kind` block and `.decl builtin` appear twice.
2. names: `live_def` never drops a decorated definition (the `(f, f)` row of `defstmt` is never shadowed).
3. names: `site_latest` misses a later `def`/`class` (strict `within` against the statement itself).
4. imports ↔ names depend on each other with one `.use`; imports reads `def_name`, `qualname`, `type_checking_block`.
5. Fixed (2026-11-04, found by the new stratification check): `exports` → `resolve_export` → `!… exports …` was
   negation through recursion. resolve_export now negates `exported(m, x)`, which is defined without
   resolve_export. A second cycle, import_time → `!type_checking_block` → sym_of → resolve_export → exports →
   import_time, is broken by testing TYPE_CHECKING by qualified name (`qn`), not by resolved symbol. The cost:
   `from typing import TYPE_CHECKING as T` re-exported through another module is not recognised.
6. Fixed in the Soufflé conversion: `fdef` (function-or-lambda in imports, function-only elsewhere) is now `fn_or_lambda` in
   imports, and each library declares a relation once.
7. calls: `isasync` duplicates `is_async`; a library carries `.output`; the W5 `ASYNC_BLOCKING` comment has no rules.
8. calls: `posarg` counts all argument children, so keyword or splat arguments before it shift the index.
9. Changed: flow's `always_raises` follows if/elif/else chains as positive recursion (no `ordered by`).
   flow: `stmt_of`/`before` cover `block` nodes only (no module level); `in_branch` leaves out loop bodies.
10. names: `nonlocal` targets the nearest function, not the nearest one binding the name; `scope_name` uses any
    enclosing class; `rel_base` lacks the `p != ""` guard.

## Rule files
1. post-op-no-internal-dispatch 001: "first comparison" uses `precedes`, which needs a shared parent. Needs a
   source-order test, else nearly every repeat is reported.
2. function-level-imports b1/b2: the allowlist names `load_python_module`; the oracle skips
   `_load_python_module_uncached`. b2 matches via `scope_name`, which adds a `Class.` prefix.
3. Fixed: function-level-imports b1 and b2 carry `scope` as a column of the finding; one per statement.
4. function-level-imports b1: findings keyed by statement, so `import os, sys` counts once; the oracle counts two.
5. Fixed: function-level-imports b2 no longer redefines `import_time`, `runs`, `reaches`; it uses python/imports.
6. public-api-triples: `direct_wrapper_bypasses_lazy_owner` accepts a listed set of `*_lazy` helpers (oracle: any
   `*_lazy`/`lazy_*`, one finding per group); `same_params` drops `self`/`cls`; `exemplar` uses the smallest name.
7. plan-scope-side-effects: the `may fx` regex runs on the receiver's whole text, arguments included.
8. swallowed-exceptions: the body summary says "other statements" where the oracle lists the first five types.

## Soufflé form
1. Fixed: every generated pack rule is grounded. Captures of option-dependent regexes, JSON `transform`s and
   option-valued operands are lowered; a metavariable an alternative leaves empty is bound to core's `unset`
   (578 rules), as ast-grep leaves it empty; a variable used once under `!` is written `_`.
2. Relation names share one namespace per program (rule file, includes, core). `site/check_rr.py` reports duplicate
   declarations and ungrounded rules in the hand files; it prints ok today.
3. Additions to Soufflé that an engine must support: pattern atoms `` n ~ `…` ``, named-group captures in `match`
   (span or symbol), `match_all`, `must, may` relations, `.violation`/`fix`/`=>`/`.edit`, and a relation name
   as the stop-set argument of `within_stop`, `nearest`, `precedes_stop`; the `join` aggregate; and
   `.decl r(…) ordered by earlier` (data-layer/DESIGN.md).
4. `site/check_rr.py` also checks stratification: no relation depends on itself through `!` or an aggregate unless
   it is `ordered by earlier` and the use is tied by `earlier(m, n)`. It found the two cycles above.

## Data-layer design (data-layer/DESIGN.md)
1. The findings layer is designed (data-layer/FINDINGS-LAYER.md), not built: outcomes per (policy, file), keys
   and ids from keys, `code {c}`, `tag {t}`, the gate stratum, `.on syntax_error`, `.budget … from base`, and the
   verdict. Three things to measure once it runs:
   - how far Unknown spreads in cross-file policies when one file does not parse (the over-approximate places of
     section 2): if a typical edit with a syntax error makes whole policies Unknown, consider keeping the
     document's last parsed facts for the other files;
   - the cost of the extra evaluation that finds Unknown places, on a run with artifacts missing;
   - whether withholding whole unparsable documents loses findings that the engine's error-tolerant clauses find
     today (`source-partial`).
2. Signature verification is Datalog over `@ed25519_ok`; the two layouts in lib/data-layer/signing.rr were checked
   against real signatures at the pinned revision, the rules themselves were not run.
3. file-length follows the CLI's line count (splitlines); the suite plugin counts `\n`. They differ only on files
   with other line separators.
4. PEP 696 type-parameter defaults: Rerail's vendored tree-sitter-python already parses them (`type_default`,
   PATCHES.md patch 1). python/ast.rr's `type_param` was written from its grammar.json, not checked on a parse:
   confirm the shapes (`type` around the name, `constrained_type`, `splat_type`) with a tree dump once the
   engine runs .rr files.
5. Run artifacts are designed (data-layer/RUN-FACTS.md), not built: `.artifact`, `artifact=`, `rerail facts
   stamp`, `--facts`, `--require-facts`. The five producers are sketches over the data layer's existing
   functions. Only linter_runtime's is new logic, a loop over `DL_SUITE`; it belongs in the data-layer
   repository. test-budget.rr is the worked port; the other four checks are not ported yet.
6. `.budget M by k from base` over a metric relation was dropped: a metric ratchet is a rule
   (noqa-sec-ratchet.rr, test-budget.rr). The coverage table's 16 `.budget … from` uses include metric ratchets.

## Alignment with the other branches (ALIGNMENT.md)
1. The final-shape `.rdl` parser would accept this notation with small changes: bare variables, `number` and
   `symbol`, `glob(p, s)` argument order, relations of names in place of `{…}` sets, `@dot` for its `@cat`, the
   magic-set pragma for `.demand`, and `syntax_error` with line and message. Those changes belong on that branch.
2. Globs are exact but not always short: `.*` becomes `**`, because the translator does not know whether a subject
   can hold `/`. On a name, `*` would read better and mean the same. With the typed text relations (`name`,
   `str`), the translator could print `*` when the subject is a name.
3. Soufflé's lattice declaration is confirmed from its grammar (`.lattice NAME<> { OP -> arg }`, operators Bottom,
   Top, Lub, Glb, Leq). How the `Lub` functor's arguments are written is still to check against Soufflé's tests.
4. `word(w, s)` is specified (0037: whole words, any separators), but the converter emits none: regexes with `\b`
   stay `match`. Teaching it `\bWORD\b` (and `(?i)\bA\s+B\b` as `iword("a b", s)`) is the next exact case.
5. Ordered first-match reachability (the Data-layer branch's `reachability` stage) is recorded as an accepted
   difference under the findings contract, not reproduced.
