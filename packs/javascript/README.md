# Rerail rule pack: JavaScript and TypeScript (ESLint core)

72 code rules for JavaScript and TypeScript source, ported from ESLint 10.11.0's core rules with their default options.
22 start on; 50 start off (`"on": false`) because `eslint:recommended` does not turn them on, so they are experimental.
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

### `eslint-accessor-pairs`

Port of ESLint accessor-pairs. Default options: setWithoutGet true, getWithoutSet false, enforceForClassMembers true, enforceForTSTypes false. Covers object literals, classes (static and instance members paired separately) and property descriptors passed to Object.defineProperty, Reflect.defineProperty, Object.create or Object.defineProperties. Static keys pair by ESLint's getStaticPropertyName; other keys by their tokens.

### `eslint-arrow-body-style`

Port of ESLint arrow-body-style, default as-needed. Fixes only when ESLint's comment and ASI guards allow and the rewrite is a template of whole nodes.

### `eslint-capitalized-comments`

Port of ESLint capitalized-comments, default "always". Tests the first non-whitespace, non-asterisk character for Unicode Ll/Lu case; skips directive comments and URLs. Fix converts the first letter's case.

### `eslint-class-methods-use-this`

Port of ESLint class-methods-use-this, default options. Flags non-static, non-constructor methods and class fields initialized with a function/arrow whose own body never uses this or super. Nested functions, static blocks and nested class field initializers have their own this; arrows share it. An accessor field (AccessorProperty) is not treated as a field.

### `eslint-default-case`

Port of ESLint default-case. Only non-empty switches; exempt when the last comment after the last case matches "no default" (case-insensitive, configurable via commentPattern).

### `eslint-default-param-last`

Port of ESLint default-param-last on JS files. A rest parameter neither counts as required nor is reported (ESLint isRequiredParameter). TypeScript optional params are handled by typescript-eslint-default-param-last.

### `eslint-dot-notation`

Port of ESLint dot-notation, default options (allowKeywords true). Keys: string, static template, boolean/null literal that is a valid identifier. Reported at the key; no fix when comments sit inside the brackets.

### `eslint-eqeqeq`

ESLint 10.11.0 eqeqeq, default options (always, null always). Autofixes only when an operand is typeof or both operands are literals of one type.

### `eslint-func-name-matching`

Port of ESLint func-name-matching, default mode always. Compares against variable, assigned property, object key or class field; string keys only when they are escape-free identifier names. module.exports is exempt by default.

### `eslint-func-style`

Port of ESLint func-style (default "expression"). Default exports and TypeScript overload implementations are exempt.

### `eslint-grouped-accessor-pairs`

Port of ESLint grouped-accessor-pairs, default order anyOrder. Reports the latter of a getter/setter pair (only getter and setter of their key in an object, class instance or static members, or TS type) when another member sits between. Static keys pair by getStaticPropertyName, other keys by tokens.

### `eslint-guard-for-in`

Port of ESLint guard-for-in. Accepted bodies: empty, an if statement, or a block whose first statement is an if that is either the only statement or whose consequent is continue / { continue; }.

### `eslint-id-length`

Port of ESLint id-length, default min 2 (graphemes), no max, properties always. Covers variables, parameters, functions, classes, members, imports, destructuring targets and assigned properties.

### `eslint-max-classes-per-file`

Port of ESLint max-classes-per-file, default max 1. Reported at the file's first statement; counts declared and evaluated (expression) classes.

### `eslint-max-depth`

Port of ESLint max-depth, default 4. Depth counted per function, static block or program.

### `eslint-max-nested-callbacks`

Port of ESLint max-nested-callbacks with max 10. A callback is a function/arrow passed as a call argument; a callback nested inside more than max callbacks (itself included) is reported at its function head. Nesting depth is not interpolated into the message.

### `eslint-max-params`

Port of ESLint max-params (default 3) on JS files. Every param counts: plain, defaulted, destructured, rest. Reported at getFunctionHeadLoc's start; message names the function via getFunctionNameWithKind. countThis only affects TS this params; TS declared functions/function types are typescript-eslint-max-params's.

### `eslint-max-statements`

Port of ESLint max-statements (default 10, ignoreTopLevelFunctions false). Counts statements directly in blocks up to the nearest nested function or class static block; the message's count is only produced where the body holds no nested block.

### `eslint-no-alert`

Port of ESLint no-alert. Flags alert/confirm/prompt when unshadowed, or as members of unshadowed window, of globalThis when ecmaVersion or globals declare it, or of this at script global scope. Only globalThis honours /* global */ comments.

### `eslint-no-array-constructor`

Port of ESLint no-array-constructor. Only the global Array; exempt with exactly one non-spread argument or type arguments. Fix to an array literal adds a leading ; when the previous statement would otherwise continue (approximated from its last token).

### `eslint-no-await-in-loop`

Port of ESLint no-await-in-loop. Flags await expressions, for await loops and await using declarations in a loop's test, update or body with no function or for-await-of loop in between.

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

### `eslint-no-dupe-class-members`

Port of ESLint no-dupe-class-members. Static member names follow ESLint's getStaticPropertyName (via `compare` text `key`); duplicates are counted within the same class and the same staticness. A getter may pair with a setter; constructors and TypeScript overload signatures are ignored.

### `eslint-no-dupe-else-if`

Port of ESLint no-dupe-else-if. Flags an else-if test that repeats an earlier test of the chain, is an operand of an earlier `||` test, or is an `&&` whose operand repeats an earlier test (one level only). Tests are compared by tokens, with `||`/`&&` operands in either order (ESLint's equal). Divergence: ESLint's full or/and set-cover over the whole chain is not expressed.

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

### `eslint-no-eval`

Port of ESLint no-eval, default allowIndirect false. Indirect uses flagged: references to the global eval; eval read from a global object (global/window/globalThis, when declared by the configured languageOptions or at the top level of a script); and `this.eval` where `this` is the global object (top level of a script or CommonJS file, or a non-strict function via `resolves.this` with `sloppyFunctions`, strictness from languageOptions) whose call binds the default `this`. Which calls rebind `this` (methods, bind/call/apply, thisArg callbacks, capitalized constructors) is ESLint's heuristic, approximated structurally. Shadowing resolved with `resolves`.

### `eslint-no-extend-native`

Port of ESLint no-extend-native. Flags assignment to a property of `<Builtin>.prototype`, or passing it to Object.defineProperty/defineProperties, for the ECMAScript builtins ESLint lists, filtered by the configured ecmaVersion and globals (`ast.languageOptions`). The builtin must resolve to the global (`resolves`). Option exceptions (default []).

### `eslint-no-extra-bind`

Port of ESLint no-extra-bind. Only `.bind(x)` with exactly one argument; the fix removes the call when the argument is side-effect free and no comment is inside.

### `eslint-no-extra-label`

Port of ESLint no-extra-label. Fix removes the label when the statement holds no comment.

### `eslint-no-implied-eval`

Port of ESLint 10.11.0 no-implied-eval, default options. setTimeout/setInterval/execScript called directly or through window/self/global/globalThis, with a first argument that is a string (followed through effectively-constant bindings: `value.follow`), a template literal, or a `+` with such an operand. Those names count only when the configured ecmaVersion and globals declare them (`ast.languageOptions`) and nothing shadows them (`resolves`).

### `eslint-no-inline-comments`

Port of ESLint no-inline-comments. A comment is reported when a token or comment ends on its start line before it, or starts on its end line after it (`layout` gaps, comments included). Exceptions: ESLint directive comments (isDirectiveComment prefixes), and a comment alone in a JSX expression container whose line holds only `{` before and `}` after. Option ignorePattern.

### `eslint-no-irregular-whitespace`

Port of ESLint no-irregular-whitespace. Reports every maximal run of irregular whitespace and every U+2028/U+2029 (`finding.each` over a regex group): inside tokens ESLint does not skip (comments, regex literals, template chunks), between tokens via a lexer regex over the program text and each template substitution that consumes non-code (comments, strings, templates, regex literals at the start or after a punctuator), and in the file's leading whitespace before the first token (only its first three runs or line terminators). Options skipComments/skipTemplates/skipRegExps/skipJSXText/skipStrings.

### `eslint-no-iterator`

Port of ESLint no-iterator. Member accesses whose static property name (getStaticPropertyName) is `__iterator__`: a non-computed name (identifier escapes decoded), or a string or substitution-free template index by its cooked value.

### `eslint-no-labels`

Port of ESLint no-labels. Options allowLoop and allowSwitch in the `options` define. A candidate: on the real-code samples (`docs/upstream.md#samples`) it misses findings in Flow-annotated files the engine's grammar cannot parse.

### `eslint-no-lonely-if`

Port of ESLint no-lonely-if. Only when the else block's braces are not necessary; fixes to `else if` under ESLint's comment and ASI guards (ASI guard approximated conservatively).

### `eslint-no-loss-of-precision`

Port of ESLint no-loss-of-precision. Binary literals whose significant bits span more than 53; decimal literals whose significant digits differ from the stored number's (String(v) through `transform` with `value: true`, overflow to Infinity and underflow to zero included); hex and octal literals checked like binary by significant-bit span. A candidate: on the real-code samples (`docs/upstream.md#samples`) it reports 28 decimal literals of 2^63 (`9223372036854775808`), which a double stores exactly and ESLint does not report, because their digits differ from `String(v)`'s.

### `eslint-no-magic-numbers`

Port of ESLint no-magic-numbers. Numeric literals (with a leading unary +/-) are flagged except variable initialisers, object properties, member assignments, parseInt radixes and JSX containers. Options (ignore, ignoreArrayIndexes, ignoreDefaultValues, ignoreClassFieldInitialValues, enforceConst, detectObjects, ...) in the `options` define.

### `eslint-no-multi-assign`

Port of ESLint no-multi-assign. Flags an assignment used (through parentheses) as a variable initializer, a class field value or another assignment's value. Option ignoreNonDeclaration.

### `eslint-no-multi-str`

Port of ESLint no-multi-str. Not reported when the string's parent is a JSX attribute or JSX expression container.

### `eslint-no-negated-condition`

Port of ESLint no-negated-condition. Flags `if` with a plain (non-else-if) `else`, and ternaries, whose test is `!x`, `!=` or `!==`.

### `eslint-no-nested-ternary`

Port of ESLint no-nested-ternary. Consequent or alternate checked through parentheses.

### `eslint-no-new`

Port of ESLint no-new. An expression statement whose expression (through parentheses) is a `new` expression.

### `eslint-no-new-func`

Port of ESLint no-new-func. Calling or constructing the global Function, or calling its apply/bind/call (resolved with `resolves`).

### `eslint-no-new-wrappers`

Port of ESLint 10.11.0 no-new-wrappers, default options. Global String/Number/Boolean resolved with `resolves`.

### `eslint-no-obj-calls`

Port of ESLint `no-obj-calls`. Flags calling or constructing Atomics, JSON, Math, Reflect, Intl or Temporal directly, through a global object (globalThis/window/self/global), or through an effectively constant variable whose binding holds one (resolved recursively via `resolves.binding`). Names count only when the configured ecmaVersion and globals declare them (`ast.languageOptions`) and nothing shadows them.

### `eslint-no-object-constructor`

Only flags `Object()` / `new Object()` without arguments when `Object` resolves to the global. ESLint offers only a suggestion, so no fix is declared.

### `eslint-no-proto`

Matches member accesses whose static property name (ESLint getStaticPropertyName) is `__proto__`: a non-computed name with identifier escapes decoded, or a string or substitution-free template index by its cooked value.

### `eslint-no-prototype-builtins`

Port of ESLint no-prototype-builtins. A candidate: on the real-code samples (`docs/upstream.md#samples`) it misses findings in Flow-annotated files the engine's grammar cannot parse.

### `eslint-no-redeclare`

Flags each definition after the first in the same scope (`binds.redefines`); with builtinGlobals, definitions of a configured or `/* global */`-declared global (`binds.builtin`). Inside `/* global */` comments it reports per item (M12a eslintDirective parse): an item naming a builtin/configured global, one an identifier declares in the global scope, or one an earlier global comment names.

### `eslint-no-return-assign`

Default `except-parens`: an unparenthesized assignment whose nearest statement/function/class ancestor is a `return` or an expression-bodied arrow, reported at that ancestor.

### `eslint-no-script-url`

Matches string literals and untagged, substitution-free template literals whose value starts with `javascript:` case-insensitively: cooked value, or for JSX attribute strings the raw text with HTML character references decoded. Default options of ESLint 10.11.0.

### `eslint-no-self-assign`

Covers `=`, `&&=`, `||=`, `??=` for identifiers and member chains with equal tokens, or one level deep equal objects and static property names (props: true). Destructuring follows ESLint's eachSelfAssignment to any depth through recursive utilities: i-th array elements compared by index with holes counted, nothing after a right spread, a rest against a last spread; object properties by static key after the right object's last spread. Parenthesized elements on the left (`[(a)] = [(a)]`) are not followed. A candidate: on the real-code samples (`docs/upstream.md#samples`) it misses findings in Flow-annotated files the engine's grammar cannot parse.

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

### `eslint-no-unsafe-optional-chaining`

Short-circuit propagates through parentheses, await, ||/??/&& operands, the last sequence element and ternary branches. Unsafe positions: callee, member object, tagged template, new, destructuring source, extends, for-of, non-object spread, in/instanceof, with. Arithmetic on optional chains only with disallowArithmeticOperators (off by default).

### `eslint-no-unused-expressions`

Unwraps parentheses and TS `as`/`<T>`/`!`/instantiation wrappers. Allowed: call, new, assignment, update, await, yield, import(), void, delete; directive prologue strings exempt. When ecmaVersion is at most 3 (no directive prologues), directive-shaped strings are reported too.

### `eslint-no-unused-labels`

Fix removes the label only when no comment sits between label and body, the body cannot become a directive, and removal creates no ASI hazard.

### `eslint-no-unused-private-class-members`

Uses `binds.used: false` on the private name: writes alone don't count as use for fields and methods, any reference counts for accessors. Reported at the last declaration. ESLint 10.11.0 defaults.

### `eslint-no-unused-vars`

Covers variables, parameters, caught errors, imports, types and type parameters. Reported where ESLint reports (the last write from the variable's own function, else its first definition) via `binds` and `resolves.write: "last"`; options filter on those. Also flags names matching an ignore pattern that are in fact used. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (first three names of a comment).

### `eslint-no-useless-call`

thisArg must be token-equal (ESLint equalTokens) to the object `f` is read from, or null/undefined when `f` is not a member access.

### `eslint-no-useless-catch`

Catch whose first statement rethrows its own identifier parameter: reports the whole try statement when there is no finally, else just the catch clause.

### `eslint-no-useless-computed-key`

Default enforceForClassMembers: true. Object-literal exemptions for `__proto__`/`constructor`/`prototype` by cooked value. Fix replaces with the literal unless a comment sits in the brackets.

### `eslint-no-useless-concat`

Port of ESLint no-useless-concat. Flags a `+` whose nearest operands (looking through nested `+` chains and parentheses) are both string or template literals, and only when the right literal starts on the line where the left one ends (multi-line concatenation is allowed).

### `eslint-no-useless-constructor`

Flags an empty constructor only in a class without `extends`; otherwise a constructor whose body is only a `super(...)` call passing its parameters through unchanged. Pass-through follows ESLint's isPassingThrough for any number of simple parameters: each parameter and super argument matched by index and name, including a spread of a rest parameter, or a single spread of `arguments`.

### `eslint-one-var`

Port of ESLint one-var with default options (`always`). var scope is the enclosing function, let/const scope the enclosing block. `using` declarations are not modelled (grammar gap).

### `eslint-operator-assignment`

Default option `always`. References compared like ESLint's isSameReference without static computed keys; the operator's left operand is checked first, then the right for commutative operators. ESLint's canBeFixed gates the fix.

### `eslint-prefer-destructuring`

Checks declarations and assignments, array and object. Object destructuring only where the target name equals the property name. Array: plain (non-optional) access with an integer literal index (Number.isInteger); super and #private accesses skipped. Fix (object declarations only) as ESLint: only when no comment lies outside the object.

### `eslint-prefer-exponentiation-operator`

Flags global `Math.pow` including `globalThis.Math.pow`, computed and optional-chain forms; Math/globalThis resolved via scope. Fix split per parenthesization of base, exponent and result; cases ESLint fixes using token-adjacency spacing or a leading `;` get no fix.

### `eslint-prefer-numeric-literals`

Flags `parseInt` / `Number.parseInt` (global, scope-resolved) with a string literal and a radix literal of 2, 8 or 16; fixes to 0b/0o/0x only when the string is a valid literal body.

### `eslint-prefer-object-has-own`

Flags `Object.prototype.hasOwnProperty.call`, `Object.hasOwnProperty.call` and `({}).hasOwnProperty.call` when Object is the global. The `({})` form has no Object token to resolve and keeps an approximate scope graph.

### `eslint-prefer-spread`

Flags `f.apply(thisArg, args)` with a non-literal argument list when thisArg is null/undefined for a plain function, or has the same tokens as the applied method's object (ESLint equalTokens).

### `eslint-prefer-template`

Reports the outermost `+` concatenation chain that mixes a string literal with a non-string operand.

### `eslint-radix`

ESLint 10.11.0 radix, default options. Flags global parseInt / Number.parseInt with no arguments, no radix, or a radix that is not an integer 2..36 (any literal not such a Number, signed Number literals likewise, or the global `undefined`). The deprecated 'always'/'as-needed' option changes nothing.

### `eslint-require-await`

Reports an async non-generator function with a non-empty body that contains no await expression, for-await or await-using of its own.

### `eslint-sort-imports`

Each import compared with the previous declaration: member-syntax group order (none, all, multiple, single), then first local name within a group; specifiers sorted by local name (first unsorted one reported). No fix (ESLint's fix sorts a list).

### `eslint-sort-keys`

Default options: asc, case-sensitive, minKeys 2. Compares each key with the previous named key; a spread resets. Static key names; a computed identifier key compared by its name.

### `eslint-sort-vars`

Flags an identifier declarator when an earlier identifier declarator of the same declaration sorts after it. ESLint's sorting autofix is not reproduced.

### `eslint-strict`

Default `safe` mode: `global` for CommonJS or globalReturn, else `function`. Each check gated on the case's languageOptions as ESLint picks the mode: module code reports every directive as 'module'; impliedStrict (outside modules) as 'implied'.

### `eslint-use-isnan`

ESLint 10.11.0 use-isnan defaults: enforceForSwitchCase true, enforceForIndexOf false. NaN and Number are scope-resolved so a shadowing binding does not fire.

### `eslint-valid-typeof`

Default requireStringLiterals false. Also flags the global `undefined` (scope-resolved) compared with typeof.

### `eslint-vars-on-top`

A var must sit directly in a program, function body or static block, preceded only by declarations (after a leading run of directives and imports, except in static blocks).

### `eslint-yoda`

Options ["never", {"default": true}] i.e. exceptRange false. Literals: string, number/bigint, boolean, null, regex, substitution-free template, negated number; parentheses transparent.
