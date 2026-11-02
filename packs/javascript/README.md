# Rerail rule pack: JavaScript and TypeScript (ESLint core)

80 code rules for JavaScript and TypeScript source, ported from ESLint 10.11.0's core rules with their default options.
23 start on; 57 start off (`"on": false`) because `eslint:recommended` does not turn them on, so they are experimental.
Each shipped rule is scored against ESLint's own findings on ESLint's test cases and on their rename variants (each
case with its identifiers and literal contents renamed, labelled by ESLint again: `holds-under-rename`), and against the
labelled lines of real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's
production workspace path, with every disagreement declared in `divergences.json`. The conformance check
([conformance/README.md](../../conformance/README.md#the-gate)) gates the shipped rules and reports the candidates.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack javascript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group, and when the pack is recommended |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's (and candidate's) upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools |
| `divergences.json` | Declared disagreements with ESLint |
| `upstream/eslint/` | ESLint's copied test cases (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `upstream/generate.mjs` | ESLint's generator |
| `upstream/eslint-plugin-unicorn/` | eslint-plugin-unicorn's copied test cases with their rename variants, its expected findings, its generator (`generate.mjs`), and its real-code samples' `samples.lock` and `SAMPLES.md` |
| `upstream/<tool>/gates.yaml` | The tool's upstream manifest: its pins, approved configurations, the file types it lints and the facts of every rule the pack ports ([docs/upstream.md](../../docs/upstream.md)) |
| `candidates/rules/` | Ports of ESLint and eslint-plugin-unicorn rules still being evaluated: listed in `conformance.json`, scored and reported, never gated, never shipped |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

The pack is recommended for a repository with JavaScript or TypeScript source files. Files under test, script and
fixture folders, test and spec files, and configuration files (`*.config.*`, `.*rc.*`) do not count.

This pack and the [TypeScript pack](../typescript/README.md) split ECMA files by type. The 21 core rules that
typescript-eslint 8.70.1 replaces with an extension rule, or turns off for TypeScript in its `eslint-recommended`
configuration (`no-redeclare`, `no-undef`, `no-unused-vars`, `no-dupe-keys` and others), run on `.js`, `.jsx`, `.mjs`
and `.cjs` files only, and are scored on ESLint's JavaScript test cases only (`fixtures` in `conformance.json`). On
TypeScript they report overloads, declaration merges and type names, and with both packs added a typescript-eslint
port would report the same violation again. `no-loss-of-precision` and `prefer-spread`, which typescript-eslint leaves
to the core rule on TypeScript, run on every ECMA file.

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/javascript.ts`) runs each tool's generator
against the pinned tool versions and upstream commit (the generator's header lists them):

```sh
npm run fixtures:derive -- --pack javascript                                 # every tool
npm run fixtures:derive -- --pack javascript --tool eslint-plugin-unicorn    # one
npm run fixtures:stage -- --pack javascript --tool eslint-plugin-unicorn     # copy what changed into upstream/eslint-plugin-unicorn/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack javascript --rule eslint-use-isnan --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-arrow-body-style`

Port of ESLint arrow-body-style, default as-needed. Fixes only when ESLint's comment and ASI guards allow and the rewrite is a template of whole nodes.

### `eslint-capitalized-comments`

Port of ESLint capitalized-comments, default "always". Tests the first non-whitespace, non-asterisk character for Unicode Ll/Lu case; skips directive comments and URLs. Fix converts the first letter's case.

### `eslint-default-case`

Port of ESLint default-case. Only non-empty switches; exempt when the last comment after the last case matches "no default" (case-insensitive, configurable via commentPattern).

### `eslint-default-param-last`

Port of ESLint default-param-last on JS files. A rest parameter neither counts as required nor is reported (ESLint isRequiredParameter). TypeScript optional params are handled by typescript-eslint-default-param-last.

### `eslint-dot-notation`

Port of ESLint dot-notation, default options (allowKeywords true). Keys: string, static template, boolean/null literal that is a valid identifier. Reported at the key; no fix when comments sit inside the brackets.

### `eslint-eqeqeq`

ESLint 10.11.0 eqeqeq, default options (always, null always). Autofixes only when an operand is typeof or both operands are literals of one type.

### `eslint-func-style`

Port of ESLint func-style (default "expression"). Default exports and TypeScript overload implementations are exempt.

### `eslint-guard-for-in`

Port of ESLint guard-for-in. Accepted bodies: empty, an if statement, or a block whose first statement is an if that is either the only statement or whose consequent is continue / { continue; }.

### `eslint-max-depth`

Port of ESLint max-depth, default 4. Depth counted per function, static block or program.

### `eslint-max-nested-callbacks`

Port of ESLint max-nested-callbacks with max 10. A callback is a function/arrow passed as a call argument; a callback nested inside more than max callbacks (itself included) is reported at its function head. Nesting depth is not interpolated into the message.

### `eslint-max-params`

Port of ESLint max-params (default 3) on JS files. Every param counts: plain, defaulted, destructured, rest. Reported at getFunctionHeadLoc's start; message names the function via getFunctionNameWithKind. countThis only affects TS this params; TS declared functions/function types are typescript-eslint-max-params's.

### `eslint-max-statements`

Port of ESLint max-statements (default 10, ignoreTopLevelFunctions false). Counts statements directly in blocks up to the nearest nested function or class static block; the message's count is only produced where the body holds no nested block.

### `eslint-no-array-constructor`

Port of ESLint no-array-constructor. Only the global Array; exempt with exactly one non-spread argument or type arguments. Fix to an array literal adds a leading ; when the previous statement would otherwise continue (approximated from its last token).

### `eslint-no-caller`

Port of ESLint no-caller; the arguments object is matched through parentheses.

### `eslint-no-case-declarations`

Port of ESLint no-case-declarations. using declarations, which tree-sitter-typescript 0.23.2 parses as assignments with a using token, count as lexical declarations.

### `eslint-no-compare-neg-zero`

ESLint 10.11.0 no-compare-neg-zero, default options. -0 is a unary minus on a numeric literal of value 0. Scope only matters to ESLint's Object.is suggestion, which is not a fix.

### `eslint-no-cond-assign`

Port of ESLint no-cond-assign, default except-parens: an assignment that is itself the test of if/while/do-while/for/ternary unless wrapped in an extra pair of parentheses (for "for", any pair).

### `eslint-no-console`

Port of ESLint no-console, default allow []. Flags member access on the global console (resolved), reported at the member expression.

### `eslint-no-delete-var`

Port of ESLint no-delete-var. Reports delete applied to a bare identifier, looking through parentheses.

### `eslint-no-div-regex`

Port of ESLint no-div-regex. Reports a regex literal whose pattern starts with `=`; the fix (as ESLint) replaces the first `=` with `[=]` (M12b `fix.range` over a regex capture group).

### `eslint-no-dupe-keys`

Port of ESLint 10.11.0 no-dupe-keys, default options. Static keys via ESLint's getStaticPropertyName (`compare` text `key`); a getter may pair with a setter; a non-computed `__proto__: value` is not counted as a key.

### `eslint-no-duplicate-case`

Port of ESLint 10.11.0 no-duplicate-case, default options. A case test (inside any parentheses) is token-equal (ESLint's equalTokens, via `compare` text `tokens`) to an earlier case of the same switch.

### `eslint-no-duplicate-imports`

Port of ESLint no-duplicate-imports. With includeExports, re-exports are also checked. Only flags when ESLint's merge rules allow merging (namespace vs named, `export *`, type-only default vs named). Module names are paired by cooked value trimmed the way JavaScript trims (`transform` with `value: true`), and the message names the trimmed value. Options (includeExports, allowSeparateTypeImports) live in the `options` define; a variant is a config patch.

### `eslint-no-empty`

Port of ESLint no-empty. Function bodies, class static blocks and TS module bodies are not checked; a block or switch with a comment is not empty. Option allowEmptyCatch in the `options` define.

### `eslint-no-empty-function`

Port of ESLint no-empty-function, default options (allow: []). A body holding a comment is not empty; constructors with parameter properties are exempt.

### `eslint-no-empty-pattern`

Port of ESLint no-empty-pattern. Option allowObjectPatternsAsParameters in the `options` define.

### `eslint-no-empty-static-block`

Port of ESLint no-empty-static-block. A comment inside the braces makes it non-empty; a trailing comment the grammar attaches after the closing brace does not count.

### `eslint-no-eq-null`

Port of ESLint no-eq-null. Reports `==` and `!=` with a `null` operand, looking through parentheses.

### `eslint-no-extend-native`

Port of ESLint no-extend-native. Flags assignment to a property of `<Builtin>.prototype`, or passing it to Object.defineProperty/defineProperties, for the ECMAScript builtins ESLint lists, filtered by the configured ecmaVersion and globals (`ast.languageOptions`). The builtin must resolve to the global (`resolves`). Option exceptions (default []).

### `eslint-no-extra-label`

Port of ESLint no-extra-label. Fix removes the label when the statement holds no comment.

### `eslint-no-inline-comments`

Port of ESLint no-inline-comments. A comment is reported when a token or comment ends on its start line before it, or starts on its end line after it (`layout` gaps, comments included). Exceptions: ESLint directive comments (isDirectiveComment prefixes), and a comment alone in a JSX expression container whose line holds only `{` before and `}` after. Option ignorePattern.

### `eslint-no-iterator`

Port of ESLint no-iterator. Member accesses whose static property name (getStaticPropertyName) is `__iterator__`: a non-computed name (identifier escapes decoded), or a string or substitution-free template index by its cooked value.

### `eslint-no-lonely-if`

Port of ESLint no-lonely-if. Only when the else block's braces are not necessary; fixes to `else if` under ESLint's comment and ASI guards (ASI guard approximated conservatively).

### `eslint-no-multi-str`

Port of ESLint no-multi-str. Not reported when the string's parent is a JSX attribute or JSX expression container.

### `eslint-no-negated-condition`

Port of ESLint no-negated-condition. Flags `if` with a plain (non-else-if) `else`, and ternaries, whose test is `!x`, `!=` or `!==`.

### `eslint-no-nested-ternary`

Port of ESLint no-nested-ternary. Consequent or alternate checked through parentheses.

### `eslint-no-new`

Port of ESLint no-new. An expression statement whose expression (through parentheses) is a `new` expression.

### `eslint-no-new-wrappers`

Port of ESLint 10.11.0 no-new-wrappers, default options. Global String/Number/Boolean resolved with `resolves`.

### `eslint-no-object-constructor`

Only flags `Object()` / `new Object()` without arguments when `Object` resolves to the global. ESLint offers only a suggestion, so no fix is declared.

### `eslint-no-proto`

Matches member accesses whose static property name (ESLint getStaticPropertyName) is `__proto__`: a non-computed name with identifier escapes decoded, or a string or substitution-free template index by its cooked value.

### `eslint-no-return-assign`

Default `except-parens`: an unparenthesized assignment whose nearest statement/function/class ancestor is a `return` or an expression-bodied arrow, reported at that ancestor.

### `eslint-no-script-url`

Matches string literals and untagged, substitution-free template literals whose value starts with `javascript:` case-insensitively: cooked value, or for JSX attribute strings the raw text with HTML character references decoded. Default options of ESLint 10.11.0.

### `eslint-no-self-compare`

Operands compared by tokens, ignoring comments and parentheses around an operand (as ESTree drops them). Covers equality and relational operators.

### `eslint-no-sequences`

Default allowInParentheses: true. Exempts for-loop init/update and sequences wrapped in parentheses (doubled parentheses where the grammar already requires them, e.g. in an if condition).

### `eslint-no-setter-return`

Covers class/object `set` accessors and the `set` function of a descriptor passed to Object.defineProperty, Reflect.defineProperty, Object.defineProperties or Object.create, including an arrow setter's expression body. `Object` and `Reflect` must resolve to declared globals.

### `eslint-no-template-curly-in-string`

Value checked cooked, or for JSX attribute strings raw with HTML character references decoded.

### `eslint-no-unexpected-multiline`

Flags a line break before the `(` (or `<` of type arguments) of a call with arguments, the `[` of a computed member access, the template of a tagged template, or the first `/` of a division whose second `/` is directly followed by regex flags. Optional calls and members are skipped.

### `eslint-no-unneeded-ternary`

Fixes follow ESLint: invert the test, coerce with `!!`, or collapse; default-assignment form (`x ? x : y`) becomes `x || y`.

### `eslint-no-unsafe-finally`

A jump counts when the nearest finally lies before a function/class boundary; for unlabeled break/continue also before any enclosing loop or switch, and for a labeled jump before the labeled statement it targets.

### `eslint-no-unsafe-negation`

Relational operators (<, >, <=, >=) are only checked when enforceForOrderingRelations is enabled (off by default).

### `eslint-no-unused-labels`

Fix removes the label only when no comment sits between label and body, the body cannot become a directive, and removal creates no ASI hazard.

### `eslint-no-unused-private-class-members`

Uses `binds.used: false` on the private name: writes alone don't count as use for fields and methods, any reference counts for accessors. Reported at the last declaration. ESLint 10.11.0 defaults.

### `eslint-no-useless-call`

thisArg must be token-equal (ESLint equalTokens) to the object `f` is read from, or null/undefined when `f` is not a member access.

### `eslint-no-useless-catch`

Catch whose first statement rethrows its own identifier parameter: reports the whole try statement when there is no finally, else just the catch clause.

### `eslint-no-useless-concat`

Port of ESLint no-useless-concat. Flags a `+` whose nearest operands (looking through nested `+` chains and parentheses) are both string or template literals, and only when the right literal starts on the line where the left one ends (multi-line concatenation is allowed).

### `eslint-operator-assignment`

Default option `always`. References compared like ESLint's isSameReference without static computed keys; the operator's left operand is checked first, then the right for commutative operators. ESLint's canBeFixed gates the fix.

### `eslint-prefer-numeric-literals`

Flags `parseInt` / `Number.parseInt` (global, scope-resolved) with a string literal and a radix literal of 2, 8 or 16; fixes to 0b/0o/0x only when the string is a valid literal body.

### `eslint-prefer-spread`

Flags `f.apply(thisArg, args)` with a non-literal argument list when thisArg is null/undefined for a plain function, or has the same tokens as the applied method's object (ESLint equalTokens).

### `eslint-prefer-template`

Reports the outermost `+` concatenation chain that mixes a string literal with a non-string operand.

### `eslint-radix`

ESLint 10.11.0 radix, default options. Flags global parseInt / Number.parseInt with no arguments, no radix, or a radix that is not an integer 2..36 (any literal not such a Number, signed Number literals likewise, or the global `undefined`). The deprecated 'always'/'as-needed' option changes nothing.

### `eslint-sort-imports`

Each import compared with the previous declaration: member-syntax group order (none, all, multiple, single), then first local name within a group; specifiers sorted by local name (first unsorted one reported). No fix (ESLint's fix sorts a list).

### `eslint-sort-keys`

Default options: asc, case-sensitive, minKeys 2. Compares each key with the previous named key; a spread resets. Static key names; a computed identifier key compared by its name.

### `eslint-sort-vars`

Flags an identifier declarator when an earlier identifier declarator of the same declaration sorts after it. ESLint's sorting autofix is not reproduced.

### `eslint-use-isnan`

ESLint 10.11.0 use-isnan defaults: enforceForSwitchCase true, enforceForIndexOf false. NaN and Number are scope-resolved so a shadowing binding does not fire.

### `eslint-valid-typeof`

Default requireStringLiterals false. Also flags the global `undefined` (scope-resolved) compared with typeof.

### `eslint-yoda`

Options ["never", {"default": true}] i.e. exceptRange false. Literals: string, number/bigint, boolean, null, regex, substitution-free template, negated number; parentheses transparent.
