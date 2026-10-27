# Rerail rule pack: JavaScript and TypeScript (ESLint core), candidates

The ports in `rules/` here are still being evaluated: scored and reported against the tools' recordings, never gated
and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-accessor-pairs`

Port of ESLint accessor-pairs. Default options: setWithoutGet true, getWithoutSet false, enforceForClassMembers true, enforceForTSTypes false. Covers object literals, classes (static and instance members paired separately) and property descriptors passed to Object.defineProperty, Reflect.defineProperty, Object.create or Object.defineProperties. Static keys pair by ESLint's getStaticPropertyName; other keys by their tokens.

### `eslint-class-methods-use-this`

Port of ESLint class-methods-use-this, default options. Flags non-static, non-constructor methods and class fields initialized with a function/arrow whose own body never uses this or super. Nested functions, static blocks and nested class field initializers have their own this; arrows share it. An accessor field (AccessorProperty) is not treated as a field.

### `eslint-default-param-last`

Port of ESLint default-param-last on JS files. A rest parameter neither counts as required nor is reported (ESLint isRequiredParameter). TypeScript optional params are handled by typescript-eslint-default-param-last.

### `eslint-dot-notation`

Port of ESLint dot-notation, default options (allowKeywords true). Keys: string, static template, boolean/null literal that is a valid identifier. Reported at the key; no fix when comments sit inside the brackets.

### `eslint-func-name-matching`

Port of ESLint func-name-matching, default mode always. Compares against variable, assigned property, object key or class field; string keys only when they are escape-free identifier names. module.exports is exempt by default.

### `eslint-grouped-accessor-pairs`

Port of ESLint grouped-accessor-pairs, default order anyOrder. Reports the latter of a getter/setter pair (only getter and setter of their key in an object, class instance or static members, or TS type) when another member sits between. Static keys pair by getStaticPropertyName, other keys by tokens.

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

### `eslint-no-dupe-class-members`

Port of ESLint no-dupe-class-members. Static member names follow ESLint's getStaticPropertyName (via `compare` text `key`); duplicates are counted within the same class and the same staticness. A getter may pair with a setter; constructors and TypeScript overload signatures are ignored.

### `eslint-no-dupe-else-if`

Port of ESLint no-dupe-else-if. Flags an else-if test that repeats an earlier test of the chain, is an operand of an earlier `||` test, or is an `&&` whose operand repeats an earlier test (one level only). Tests are compared by tokens, with `||`/`&&` operands in either order (ESLint's equal). Divergence: ESLint's full or/and set-cover over the whole chain is not expressed.

### `eslint-no-duplicate-imports`

Port of ESLint no-duplicate-imports. With includeExports, re-exports are also checked. Only flags when ESLint's merge rules allow merging (namespace vs named, `export *`, type-only default vs named). Module names are paired by cooked value trimmed the way JavaScript trims (`transform` with `value: true`), and the message names the trimmed value. Options (includeExports, allowSeparateTypeImports) live in the `options` define; a variant is a config patch.

### `eslint-no-empty-function`

Port of ESLint no-empty-function, default options (allow: []). A body holding a comment is not empty; constructors with parameter properties are exempt.

### `eslint-no-eval`

Port of ESLint no-eval, default allowIndirect false. Indirect uses flagged: references to the global eval; eval read from a global object (global/window/globalThis, when declared by the configured languageOptions or at the top level of a script); and `this.eval` where `this` is the global object (top level of a script or CommonJS file, or a non-strict function via `resolves.this` with `sloppyFunctions`, strictness from languageOptions) whose call binds the default `this`. Which calls rebind `this` (methods, bind/call/apply, thisArg callbacks, capitalized constructors) is ESLint's heuristic, approximated structurally. Shadowing resolved with `resolves`.

### `eslint-no-extra-bind`

Port of ESLint no-extra-bind. Only `.bind(x)` with exactly one argument; the fix removes the call when the argument is side-effect free and no comment is inside.

### `eslint-no-extra-label`

Port of ESLint no-extra-label. Fix removes the label when the statement holds no comment.

### `eslint-no-implied-eval`

Port of ESLint 10.11.0 no-implied-eval, default options. setTimeout/setInterval/execScript called directly or through window/self/global/globalThis, with a first argument that is a string (followed through effectively-constant bindings: `value.follow`), a template literal, or a `+` with such an operand. Those names count only when the configured ecmaVersion and globals declare them (`ast.languageOptions`) and nothing shadows them (`resolves`).

### `eslint-no-irregular-whitespace`

Port of ESLint no-irregular-whitespace. Reports every maximal run of irregular whitespace and every U+2028/U+2029 (`finding.each` over a regex group): inside tokens ESLint does not skip (comments, regex literals, template chunks), between tokens via a lexer regex over the program text and each template substitution that consumes non-code (comments, strings, templates, regex literals at the start or after a punctuator), and in the file's leading whitespace before the first token (only its first three runs or line terminators). Options skipComments/skipTemplates/skipRegExps/skipJSXText/skipStrings.

### `eslint-no-lonely-if`

Port of ESLint no-lonely-if. Only when the else block's braces are not necessary; fixes to `else if` under ESLint's comment and ASI guards (ASI guard approximated conservatively).

### `eslint-no-magic-numbers`

Port of ESLint no-magic-numbers. Numeric literals (with a leading unary +/-) are flagged except variable initialisers, object properties, member assignments, parseInt radixes and JSX containers. Options (ignore, ignoreArrayIndexes, ignoreDefaultValues, ignoreClassFieldInitialValues, enforceConst, detectObjects, ...) in the `options` define.

### `eslint-no-multi-assign`

Port of ESLint no-multi-assign. Flags an assignment used (through parentheses) as a variable initializer, a class field value or another assignment's value. Option ignoreNonDeclaration.

### `eslint-no-new-func`

Port of ESLint no-new-func. Calling or constructing the global Function, or calling its apply/bind/call (resolved with `resolves`).

### `eslint-no-obj-calls`

Port of ESLint `no-obj-calls`. Flags calling or constructing Atomics, JSON, Math, Reflect, Intl or Temporal directly, through a global object (globalThis/window/self/global), or through an effectively constant variable whose binding holds one (resolved recursively via `resolves.binding`). Names count only when the configured ecmaVersion and globals declare them (`ast.languageOptions`) and nothing shadows them.

### `eslint-no-redeclare`

Flags each definition after the first in the same scope (`binds.redefines`); with builtinGlobals, definitions of a configured or `/* global */`-declared global (`binds.builtin`). Inside `/* global */` comments it reports per item (M12a eslintDirective parse): an item naming a builtin/configured global, one an identifier declares in the global scope, or one an earlier global comment names.

### `eslint-no-self-assign`

Covers `=`, `&&=`, `||=`, `??=` for identifiers and member chains with equal tokens, or one level deep equal objects and static property names (props: true). Destructuring follows ESLint's eachSelfAssignment: i-th array elements compared by index with holes counted, nothing after a right spread, a rest against a last spread; object properties by static key after the right object's last spread. Unrolled to 3 nested containers.

### `eslint-no-unexpected-multiline`

Flags a line break before the `(` (or `<` of type arguments) of a call with arguments, the `[` of a computed member access, the template of a tagged template, or the first `/` of a division whose second `/` is directly followed by regex flags. Optional calls and members are skipped.

### `eslint-no-unsafe-optional-chaining`

Short-circuit propagates through parentheses, await, ||/??/&& operands, the last sequence element and ternary branches. Unsafe positions: callee, member object, tagged template, new, destructuring source, extends, for-of, non-object spread, in/instanceof, with. Arithmetic on optional chains only with disallowArithmeticOperators (off by default).

### `eslint-no-unused-expressions`

Unwraps parentheses and TS `as`/`<T>`/`!`/instantiation wrappers. Allowed: call, new, assignment, update, await, yield, import(), void, delete; directive prologue strings exempt. When ecmaVersion is at most 3 (no directive prologues), directive-shaped strings are reported too.

### `eslint-no-unused-vars`

Covers variables, parameters, caught errors, imports, types and type parameters. Reported where ESLint reports (the last write from the variable's own function, else its first definition) via `binds` and `resolves.write: "last"`; options filter on those. Also flags names matching an ignore pattern that are in fact used. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (first three names of a comment).

### `eslint-no-useless-computed-key`

Default enforceForClassMembers: true. Object-literal exemptions for `__proto__`/`constructor`/`prototype` by cooked value. Fix replaces with the literal unless a comment sits in the brackets.

### `eslint-no-useless-constructor`

Flags an empty constructor only in a class without `extends`; otherwise a constructor whose body is only a `super(...)` call passing its parameters through unchanged. Pass-through follows ESLint's isPassingThrough for any number of simple parameters: each parameter and super argument matched by index and name, including a spread of a rest parameter, or a single spread of `arguments`.

### `eslint-one-var`

Port of ESLint one-var with default options (`always`). var scope is the enclosing function, let/const scope the enclosing block. `using` declarations are not modelled (grammar gap).

### `eslint-prefer-destructuring`

Checks declarations and assignments, array and object. Object destructuring only where the target name equals the property name. Array: plain (non-optional) access with an integer literal index (Number.isInteger); super and #private accesses skipped. Fix (object declarations only) as ESLint: only when no comment lies outside the object.

### `eslint-prefer-exponentiation-operator`

Flags global `Math.pow` including `globalThis.Math.pow`, computed and optional-chain forms; Math/globalThis resolved via scope. Fix split per parenthesization of base, exponent and result; cases ESLint fixes using token-adjacency spacing or a leading `;` get no fix.

### `eslint-prefer-object-has-own`

Flags `Object.prototype.hasOwnProperty.call`, `Object.hasOwnProperty.call` and `({}).hasOwnProperty.call` when Object is the global. The `({})` form has no Object token to resolve and keeps an approximate scope graph.

### `eslint-require-await`

Reports an async non-generator function with a non-empty body that contains no await expression, for-await or await-using of its own.

### `eslint-strict`

Default `safe` mode: `global` for CommonJS or globalReturn, else `function`. Each check gated on the case's languageOptions as ESLint picks the mode: module code reports every directive as 'module'; impliedStrict (outside modules) as 'implied'.

### `eslint-vars-on-top`

A var must sit directly in a program, function body or static block, preceded only by declarations (after a leading run of directives and imports, except in static blocks).
