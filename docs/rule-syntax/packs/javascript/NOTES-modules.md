# Notes: tsx/modules, eslint-strict, eslint-sort-imports

> The core additions proposed below are now in lib/core.rr (2026-11-05).

These are hand-written versions of two ESLint rules. The machine translations from
`packs/javascript/rules/*.json` run to about 200 lines each. The vocabulary they lacked
(directive prologues, strict mode, module-ness, the shape of an import) is now
`lib/tsx/modules.rr`, and each rule is a page of Datalog over it.

| File | Lines |
|---|---|
| `lib/tsx/modules.rr` | 216 |
| `packs/javascript/eslint-strict.rr` | 89 |
| `packs/javascript/eslint-sort-imports.rr` | 79 |

## Assumptions

- **Options.** Each JSON option is read with `option(name, v)`. Booleans are `"true"` or unset.
  `eslint-strict`'s string option is `option("option", m)`, named as the JSON names it.
- **ESLint's environment.** The JSON's `when` reads ESLint's `languageOptions`, which Rerail
  cannot see. These are read as policy options instead:
  - `sourceType`: `"module"`, `"commonjs"` or `"script"`;
  - `globalReturn` and `impliedStrict`: `"true"` or unset.
- **The default source type** follows the engine's `SourceType::of_file`
  (`native/src/code_rule_dsl/language/adapter.rs`) when no `sourceType` option is set:
  - `.cjs`/`.cts` are CommonJS; `.mjs`/`.mts`, `.d.ts` and every other extension except
    `.js`/`.jsx`/`.ts` are modules;
  - a `.js`, `.jsx` or `.ts` file follows the nearest package.json's `type`, and is a module
    when there is no package.json (ESLint's flat-config default);
  - under a package.json with no valid `type`, Node's syntax detection decides: module syntax
    makes a module, otherwise CommonJS. `module_syntax` checks top-level import and export
    statements, leaving out TypeScript's erased forms and `import x = require()`. It skips the
    rest of Node's test: `import.meta`, top-level await, and top-level const/let/class named
    after the CommonJS wrapper's names.

  So nothing is a script unless the option says so. ESLint itself does not read package.json.
  The engine does, and so does this library, through the proposed `package_for` and
  `package_field`.
- **Explicit `"safe"`.** It means the same as no option, as in ESLint. The JSON has clauses only
  for an unset option, so it reports nothing for an explicit `"safe"`.
- **`memberSyntaxSortOrder`** is a list, so it arrives as one `option_item(name, i, v)` row per
  element (proposed below). `syntax_pos(s, k)` is then each syntax's place in the list, and the
  check is one comparison: `x < y`. Any order works. The JSON handled only the default and two
  other orders, one clause per pair.
- **Comparison.** `<` on symbols is Soufflé's code-point order, and `ignoreCase` uses `@lower`.
  JavaScript compares UTF-16 units, so the two orders differ only between characters
  U+E000–U+FFFF and astral characters.
- **Comments as nodes.** `child` lists comments, as tree-sitter does. `scope_stmt` skips them,
  and so does the `#!` line.

## Semantic differences from the JSON

Findings are keys (policy, code, path, line). The codes are the same (`strict`,
`sort-imports`). Messages are ESLint's texts, so their wording differs from the JSON's.
`sort-imports` adds the line of the other import to its messages.

### eslint-strict

1. **The environment comes from options, else the file and its package.json** (above), as the engine
   decides it for the JSON's `when`. `globalReturn` and `impliedStrict` are options only.
2. **Explicit `"safe"`** is reported like the default. The JSON reports nothing for it.
3. **`#!` line.** The JSON counts it as a statement. It then reports `global` at the `#!`
   line, and misses every program-level `"use strict"` after it. Here the line is skipped, as
   ESLint skips it.
4. **Parameters.** `simple_params` counts a TypeScript `this: T` parameter as simple, as ESLint
   does. The JSON's test agrees on this case but is built differently. Two cases may differ:
   - parameter properties (`constructor(private a)`) are simple here, but typescript-eslint
     makes them non-simple;
   - some exotic parameter forms (TS decorators on parameters) may be classified differently.
5. **The fix** uses `delete(u)`: the statement goes, and its line too when it was alone there.
   The JSON's `replace(n, \`\`)` leaves an empty line. It is offered for the same five message
   ids (`multiple`, `unnecessary`, `unnecessaryInClasses`, `module`, `implied`).
6. **`wrap` names a function only when it is named and not async**, as in the JSON. ESLint
   also names methods, arrows and async functions ("method 'm'", "async function 'f'"). This
   changes only the message.
7. **`("use strict")`** counts, as in ESLint and the JSON: ESTree has no node for the
   parentheses. `use_strict` keeps this quirk. `directive`, the spec's notion, does not count
   it, so `strict_scope` does not either.
8. **`'use\x20strict'`** counts: `use_strict` compares `value`, as ESLint compares the value.
   The JSON's regex reads the raw text and misses it. `directive` keeps the raw text, as the spec
   does.
9. **Computed method names.** For "function"-mode `function`/`wrap` and for `unnecessary`, a
   method encloses only its parameters and body (`fn_region`). So in
   `({ [(function(){})()]() {} })` the inner function counts as top-level, as in ESLint and the
   JSON.
10. **Method anchors.** A method's finding sits at its `<T>` type parameters when it has them,
    else at its parameters: where ESLint's FunctionExpression starts.
11. **`<!--` comments** (tree-sitter's `html_comment`) are skipped like other comments.

Everything else follows ESLint's `strict.js` case by case, as the JSON does:
- modes: `module` and `implied` override the option, and `safe` resolves to global or function;
- the directive cases: `multiple`, `nonSimpleParameterList`, `unnecessary` (an enclosing
  function is strict) and `unnecessaryInClasses`;
- the function cases: `function` and `wrap`, only for functions in no function or class body,
  reported at the parameters for methods;
- `global` reported at the first statement.

A hand-built fact set for a function-mode script, and for the global, never and module modes,
gave exactly ESLint's reports.

### eslint-sort-imports

1. **Option gates.** Several JSON clauses ignore `ignoreDeclarationSort` and
   `allowSeparatedGroups` (those for non-default orders). They would report even with
   `ignoreDeclarationSort: true`, or across groups. Here both options apply everywhere, as in
   ESLint.
2. **Groups.** Under `allowSeparatedGroups` any line between two imports starts a new group:
   a blank line, a comment or code (ESLint's `getNumberOfLinesBetween`). The JSON tests for
   two line breaks before the previous import. That likely agrees on blank lines, and may
   differ on comment-only lines.
3. **`ignoreCase` under `allowSeparatedGroups`.** The JSON's alphabetical clauses compare
   case-sensitively when `allowSeparatedGroups` is true. Here `ignoreCase` always applies.
4. **The first local name** is the ESTree first specifier. For `import D, { a } from "m"` it
   is `D`. The JSON's pattern binds both `D` and `a`, which can add or lose findings.
5. **`import x = require("m")`** is not an import declaration here (TSImportEqualsDeclaration
   in ESLint). The JSON classifies it as a `none` import.
6. **Fix.** The JSON has none. ESLint's member fix is added as a suggestion (not `safe`), with
   ESLint's limits:
   - only the names in braces move, and each is replaced by the name that belongs at its
     position, so separators and line breaks stay;
   - it is not offered when any comment is between the braces. This is more conservative than
     ESLint, which declines only for comments touching a specifier;
   - the sort is stable, so equal keys under `ignoreCase` keep source order. ESLint's
     comparator never returns 0, so its order for equal keys depends on its sort.
7. **Imports in `declare module` blocks.** The previous import is the nearest earlier import
   declaration in the file, in traversal order (`import_prev` over `visit_index`), as in ESLint.
   It is not only the previous sibling. The JSON's `follows` sees siblings only.

## What could not be expressed

- **ESLint's configuration** (`languageOptions`, `ecmaFeatures`) is not a Rerail input. It is
  modelled as policy options, and the file is used when no option is set.
- **A rank aggregate.** The fix needs each name's position in sorted order. Here that is
  `lo + count(smaller keys) + count(equal keys earlier)`: two counts, no recursion. A `rank`
  form beside `join` would read better, but it is not needed.
- **Fix titles cannot be templated.** The suggestion title is fixed text. The message carries
  the member name.

## Proposed core additions

They are in the block `// ---- proposed for core.rr (to be moved) ----` at the top of
`lib/tsx/modules.rr`.

| Addition | What | Why |
|---|---|---|
| `.decl option_item(name: symbol, i: number, v: symbol)` (`.input`) | Element `i` of a list-valued option, from 0 | `option(name, v)` holds one value and loses list order. Any list option needs it (`memberSyntaxSortOrder`; also `import/order` groups, allowlists in options). |
| `.decl package_for(path: symbol, dir: symbol)`, `.decl package_field(dir: symbol, key: symbol, v: symbol)` (`.input`) | The nearest package.json for a file, as `tsconfig_for` does; that file's top-level string members | The engine and Node give a `.js` file a module system from its package's `type`. Core's `package(dir, name)` reads only the name. Other uses: `engines`, `private`, `main`. |
| `.functor lower(s: symbol): symbol` | JavaScript `toLowerCase` | `ignoreCase` options compare lower-cased text. The `i*` text tests ignore case but cannot order, and `@convert(…, "lowerCase", …)` also splits words. |

Two notes on `core.rr` itself, not changed:
- **`has` is ambiguous.** Its comment says "a is a strict ancestor of n whose kind is in set".
  The guide's table and the translator use `has(n, c, "k")` with the kind on the descendant
  `c`. These files use it that way, once: `!has(b, _, "comment")`.
- **`code "literal"`** on `.violation` (as in `code "sort-imports"`) is what the generated
  `.rr` uses. Core documents only `code {c}`.

## Checks

ESLint 9.39.3 (with @typescript-eslint/parser 8.56.1), run through the reviewer's harness,
confirms the expected reports for the reviewer's cases:
- no report for `a.js` by default;
- lines 2 and 3 for the computed-key method;
- line 3 for the import in `declare module`;
- line 1 for `m<T>`;
- `module` for `'use\x20strict'`.

These rules cannot run on real code until the engine runs `.rr`. So each of those cases was
checked against the rules by reading them, not by running them.


- **`python3 site/check_rr.py`** prints `ok`. Its groundedness pass globs `**/*.rr`, so it
  covers `packs/`. Its strata and duplicate-declaration passes glob only `*.rr`,
  `data-layer/**` and `lib/**`, which leaves out the two rules. The same functions were run on
  the rule files directly (each program with its includes and core): no problems.
- **Soufflé 2.5** parses, type-checks and runs both programs (core, names, modules and the
  rule) after a mechanical strip of the additions:
  - `.violation` became `.decl`;
  - rules with `=>` were dropped;
  - set arguments were quoted;
  - `prefix`/`suffix` were renamed to declared relations.

  On hand-built facts, strict gave ESLint's reports in each mode: no package.json and
  `type: "commonjs"` (the latter needs suffix facts in the scratch run).
  On hand-built facts, sort-imports gave `sort_member` at `a` in `import { b, a, c }`, and
  `sort_syntax(all, multiple)` for a namespace import after it.
