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
1. `.gate`, `.budget … from base`, `.on parse_error` and `code {c}` are specified but have no engine yet.
2. Signature verification is Datalog over `@ed25519_ok`; the two layouts in lib/data-layer/signing.rr were checked
   against real signatures at the pinned revision, the rules themselves were not run.
3. file-length follows the CLI's line count (splitlines); the suite plugin counts `\n`. They differ only on files
   with other line separators.
