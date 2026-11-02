# Complexity opt-in rule pack

Cyclomatic complexity rules for Python, TypeScript, JavaScript, PHP, Blade, Rust and Vue, with the decision points of
[lizard](https://github.com/terryyin/lizard) 1.24.0 and without its tokenizer defects. A `switch` or `match` counts once
however many cases it has, as slapkit scores a function with the lower of lizard's standard count and its modified count
(`-m`), so a flat dispatch table is not a reason to split a function. The rule decides per switch, so a default-only
switch beside a cased one counts 0 where slapkit's whole-function minimum gives it 1 (see "Deviations"). One document per language, rule id
`complexity-cyclomatic-<language>`. They form the `complexity` rule pack, which set-up offers for a repository with
Python, TypeScript, JavaScript, PHP, Blade, Rust or Vue source outside tests and build output; once a repository adds
it, its rules are on (see `docs/rule-packs.md`). SQL has no rule (see "SQL").
Every document has `mode: "steer"`: a rule that runs steers the agent and never blocks.

| Rule | Files |
|---|---|
| `complexity-cyclomatic-python` | `.py` |
| `complexity-cyclomatic-typescript` | `.ts`, `.tsx`, `.mts`, `.cts` |
| `complexity-cyclomatic-javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` |
| `complexity-cyclomatic-php` | `.php`, not `.blade.php` |
| `complexity-cyclomatic-blade` | `.blade.php` |
| `complexity-cyclomatic-rust` | `.rs` |
| `complexity-cyclomatic-vue` | `.vue` |

A function's cyclomatic complexity is 1 plus its decision points. A function fails when that exceeds 6, so 7 or more.
The finding spans the whole function, so under the `ai-output` scope only functions an edit touched are reported. The
message names the function and its number, as slapkit's advisory does: `` `grade` has cyclomatic complexity 7, over the
ceiling of 6. Decompose it into smaller functions. `` (code `CYCLOMATIC_COMPLEXITY`). A TypeScript, JavaScript or Vue function with no name of its own takes the variable, class field or object key it is assigned
to, and a PHP or Blade closure the variable it is assigned to (`` `$format` ``); a callback with none of those is "This
function" (the `too-complex-anonymous` clause). Every Python and Rust function has a name. The number comes from the
count's `as`/`offset` binding: the rule counts decision points with `count.has` (`stopBy` the function boundary, so a
nested function is not entered), binds the count plus 1 as `$CCN`, and fires at `min: 6` decisions.

Test files are out of scope: Python `**/test_*.py`, `**/*_test.py`, `**/tests/**`, `**/conftest.py`; TypeScript,
JavaScript and Vue `**/*.test.*`, `**/*.spec.*`, `**/__tests__/**`; PHP `**/test/**`, `**/tests/**`, `**/Test/**`,
`**/Tests/**`, `**/spec/**`, `**/*Test.php`, `**/*TestCase.php`; Blade `**/tests/**`, `**/Tests/**`; Rust `**/tests/**`,
`**/tests.rs`, `**/*_test.rs`, `**/*_tests.rs` (a `#[cfg(test)]` module inside a source file is scored). So are
dependency, virtual-environment, build-output and generated directories (`node_modules`, `vendor`, `.venv`, `target`,
`dist`, `build`, `.nuxt`, `generated`, ...).

## What counts

Python, per `def` (async, decorated and methods included):

- `if`, `elif`, `for` and `async for`, `while`, `except`, `finally`, each `and` and each `or`, and each `match` once,
  however many `case`s it has (the wildcard included);
- the `if` of a conditional expression, and every `for` and `if` clause of a comprehension; a `case` guard is an `if`
  too;
- not `else`, `with`, `assert`, `try` or `return`.

A nested `def` is scored on its own and adds nothing to its parent. A `lambda` is not a function here: its conditions
count toward the enclosing `def`, as lizard counts them.

TypeScript and JavaScript, per function declaration, function expression, arrow function, method, getter, setter,
constructor and generator function:

- `if` (`else if` included), `for`, `for...in`, `for...of`, `while`, `do...while`, `catch`, each `switch` once however
  many `case`s it has (a `switch` with only `default` counts 0), `&&`, `||`, `||=`, `&&=` and the `?:` conditional;
- not optional parameters or properties (`a?: T`), optional chaining (`?.`), default parameters, `??` or `??=`.

Each function is scored on its own: decisions inside a nested function do not count for the function around it. A
class is not scored itself.

PHP, per function, method and closure (`function () use (...) {}`), in classes, traits, enums and at the top level:

- `if`, `elseif` and `else if`, `for`, `foreach`, `while`, `do...while`, `catch`, each `switch` once however many
  `case`s it has (a `switch` with only `default` counts 0), each `match` once however many arms it has, `&&`, `||`,
  `and`, `or` and the `?:` conditional, in brace and alternative (`if: ... endif;`) syntax alike;
- not `else`, `finally`, `xor`, nullable types (`?int`), the short `?:` (`$a ?: $b`), `??`, `??=` or `?->`.

A closure is scored on its own, as a TypeScript function is. An arrow function (`fn ($x) => ...`) is not a function
here: its conditions count toward the function around it, as lizard counts them, the way a Python `lambda`'s do.

Blade: the same, over the PHP its template compiles to. A template's own directives (`@if`, `@foreach`, `@switch`, an
echo's `?:`) run in no function, so they are never scored; a function or closure the template defines (in `<?php ?>`,
in `@php ... @endphp` or in an echo) is.

Rust, per `fn` (methods, trait default methods, nested `fn`s, `async`, `const` and `unsafe` included):

- `if` (`else if` and `if let` included), `for`, `while` and `while let`, each `match` once however many arms it has, a
  match arm's `if` guard, the `?` operator, `&&` and `||`, and each `&&` of a let chain;
- not `loop`, `else`, `let ... else`, `where` or `?Sized`.

A nested `fn` is scored on its own. A closure is not a function here: its conditions count toward the `fn` around it,
as lizard counts them. A trait's required method (`fn f(&self);`) has no body and is not scored. Rerail reads Rust's
syntax before macro expansion, so nothing inside a macro invocation (`assert!(a && b)`) counts.

Vue: the functions of a component's `<script>` and `<script setup>` blocks, counted as TypeScript and JavaScript
functions are. The template's expressions and directives (`v-if`, `v-for`, `@click` handlers) run in no function of
the scripts, so they are never scored.

Tree-sitter node kinds, from the grammars' `node-types.json` (tree-sitter-python, tree-sitter-typescript and
tree-sitter-javascript as vendored under `native/grammars/`; tree-sitter-php 0.24.2 and tree-sitter-rust 0.24.2
through ast-grep-language 0.50.0):

| Language | Function boundary | Decision |
|---|---|---|
| Python | `function_definition` | `if_statement`, `elif_clause`, `for_statement`, `while_statement`, `except_clause`, `finally_clause`, `boolean_operator` (one per operator), `conditional_expression`, `for_in_clause`, `if_clause`, `match_statement` |
| TypeScript, JavaScript | `function_declaration`, `function_expression`, `arrow_function`, `method_definition`, `generator_function`, `generator_function_declaration` | `if_statement`, `for_statement`, `for_in_statement` (in and of), `while_statement`, `do_statement`, `catch_clause`, `switch_statement` with a `switch_case` in its `switch_body`, `ternary_expression`, `binary_expression` with `&&` or `\|\|`, `augmented_assignment_expression` with `&&=` or `\|\|=` |
| PHP, Blade (`ast.embedded: compiled-view`) | `function_definition`, `method_declaration`, `anonymous_function` | `if_statement`, `else_if_clause`, `for_statement`, `foreach_statement`, `while_statement`, `do_statement`, `catch_clause`, `switch_statement` with a `case_statement` in its `switch_block`, `match_expression`, `conditional_expression` with a `body`, `binary_expression` whose `operator` is `&&`, `\|\|`, `and` or `or` |
| Rust | `function_item` | `if_expression`, `for_expression`, `while_expression`, `match_expression`, `match_pattern` with a `condition`, `try_expression`, `binary_expression` whose `operator` is `&&` or `\|\|`, each `&&` of a `let_chain` |
| Vue (`ast.embedded: script-block`) | TypeScript's and JavaScript's, outside the template's `statement_block` | TypeScript's and JavaScript's |

## SQL

SQL has no rule. The one reference, lizard's PL/SQL reader, scores procedural routines: `CREATE PROCEDURE`,
`FUNCTION` and `TRIGGER` bodies with `IF`, `ELSIF`, `CASE WHEN`, `LOOP`, `WHILE`, `FOR` and `EXCEPTION WHEN`. Rerail
reads every `.sql` file with tree-sitter-sql (`native/grammars/tree-sitter-sql`), which reads no such body: a PL/pgSQL
body is one dollar-quoted string literal to it, and an Oracle PL/SQL block does not parse. What it does read, a query
or a SQL-standard function body (`RETURN ...`, `BEGIN ATOMIC ... END`), has no control flow statement, and counting a
query's `CASE`, `AND` and `OR` would score `WHERE` clauses, which lizard does not score and the limit of 6 was not set
for. A routine's complexity can be measured once the grammar reads procedural bodies.

## Deviations from lizard 1.24.0

Each is a lizard defect, found on real code (see "Checked on real code") or in its readers' source, except the last,
which is a choice in how a switch counts. Each in bold has a corpus case.

- **`??` and `??=` count 0.** lizard's tokenizer reads each as two `?` and counts 2.
- **A `?` inside a regex literal counts 0**, and so does **`?.` right after `)` or `]`** (`f()?.x`, `a[0]?.x`). lizard
  reads each as a conditional.
- **An arrow function whose expression body starts on a later line is scored on its own.** lizard adds its conditions
  to the enclosing function and lists a one-line arrow, scored 1, on the line after.
- **A Python def whose return annotation spans several lines, or whose parameter list breaks after a default holding
  brackets, is scored whole.** lizard ends the def inside its signature and scores it 1 (fixed in crapkit #72). A
  parameter list that only spans several lines is read correctly by both.
- **`?.` counts 0 in `.jsx` files too.** lizard's JSX reader counts each `?.` as a conditional; its `.js` and `.ts`
  readers do not, and neither does Rerail anywhere.
- lizard does not read `.mts` or `.cts` files at all; the TypeScript rule covers them with the same grammar.
- **PHP: a nullable type's `?` counts 0** (`?int $count`, `): ?string`). lizard reads each as a conditional.
- **PHP: `and` and `or` count 1 each**, as `&&` and `||` do. lizard's PHP reader counts neither.
- **PHP: a function or method named `match` counts 0** (`$router->match(...)`). lizard counts the name as a `match`.
- **PHP: a closure inside a function is scored on its own.** lizard does not read a closure inside a function body as
  a function, and adds its decisions to the function around it; it does read one at the top level.
- lizard's PHP reader loses a function's range after an interface or abstract method declared with a return type
  (`public function f(): int;`), running it on into the next function, and ends no method of an `enum` at its own
  closing brace. The corpus has neither; Rerail scores a method without a body 1.
- **Blade: a function in a `@php` block or an echo is scored.** lizard reads a `.blade.php` file as PHP, where both are
  template text; it reads only the template's `<?php ?>` blocks.
- **Rust: `where` and `?Sized` count 0.** lizard counts each `where` clause, and reads `?Sized` as the `?` operator.
- **Rust: a closure's empty parameter list (`|| x`) counts 0.** lizard reads it as an `||`.
- **Rust: nothing inside a macro invocation counts.** lizard counts the `&&`, `||`, `if` and `?` in
  `assert!(a && b)` or `vec![if a { 1 } else { 2 }]`.
- **Rust: a char or byte literal is read as one** (`'a'`, `b'0'`). lizard reads its quote as a lifetime's and misreads
  what follows: it loses decisions after it, and, after several, whole functions.
- **Rust: a trait's required method (`fn f(&self);`) is no function.** lizard runs it on into the next function and
  scores that one under the required method's name and line.
- **Vue: a `<` directly before a letter is a comparison** (`a<b`). lizard's Vue reader reads it as the start of a tag
  and skips everything up to the next `>`, even lines later.
- **A default-only switch counts 0 beside a cased one.** The recorded number is the lower of lizard's standard and `-m`
  counts over the whole function, and there `-m`'s 1 for a switch with no case survives; the rule counts each switch on
  its own (`defaultOnlyBesideCasedSwitch` in the TypeScript and JavaScript corpus).

## Checked on real code

The rules were compared with lizard 1.24.0 function by function on released sources (2026-10-07; PHP, Rust and Vue
2026-10-29), pairing each lizard function with the Rerail function that starts within two lines of it:

| Repository | Rule | Paired | Same number | The rest |
|---|---|--:|--:|---|
| jinja 3.1.4 `src` | Python | 747 | 743 | 4 defs with a multi-line return annotation (crapkit #72); collapsing each signature onto one line makes lizard agree with Rerail |
| click 8.1.7 `src` | Python | 512 | 512 | |
| marshmallow 3.21.3 `src` | Python | 254 | 254 | |
| express 4.21.0 `lib` | JavaScript | 153 | 152 | 1 function lizard runs on past its end into the next one |
| axios 1.7.7 `lib` | JavaScript | 285 | 270 | regex `?`, multi-line arrow bodies, lizard ranges |
| zod 3.23.8 `src` | TypeScript | 440 | 413 | `??`, `)?.`, multi-line arrow bodies, lizard ranges |
| hono 4.6.3 `src` | TypeScript | 728 | 610 | the same, plus conditional types lizard reads as functions |
| monolog 3.7.0 `src` | PHP | 639 | 587 | nullable types' `?`, closures lizard adds to their function, lizard ranges |
| serde_json 1.0.128 `src` | Rust | 920 | 610 | `where` (on most generic functions), `?Sized`, byte literals, lizard ranges |
| ripgrep 14.1.0 `crates` | Rust | 1,969 | 1,866 | closures' `\|\|`, `where`, char and byte literals, lizard ranges |
| element-plus 2.8.4 `packages/components` | Vue | 1,100 | 905 | `??`, multi-line arrow bodies, lizard ranges |

Python agrees on every function lizard reads correctly. lizard also leaves out one-line stubs (`def f(x) -> T: ...`),
which Rerail scores 1. Every TypeScript, JavaScript, PHP, Rust and Vue difference inspected traced to one of the
deviations above, to lizard getting a function's range wrong (running past its end, ending it early, or missing a
function inside an array literal), or to lizard reading a type as a function; none was a Rerail miscount. Not every
difference was inspected one by one. On element-plus's components, the Vue rule gives every function of a
component's script the number the TypeScript or JavaScript rule gives the script on its own (1,562 functions in 159
files). laravel/framework 11.31.0's 52 templates (`src`) define no function: lizard and the Blade rule score none, and
the Blade rule reads them all without a failure.

Every file of hono 4.6.3's `src` parses. Where a file has syntax errors, the functions outside the error regions are
scored and the rest are left unknown, so they never report a guessed number.

## Parity corpus

`upstream/lizard/fixtures/{python,typescript,javascript,php,blade,rust,vue}/` holds small source files that cover every
counted decision, a function at exactly 6 and one at 7 for several kinds, nested functions, Python lambdas and
comprehensions, PHP and Rust closures, `match`/`case` and `switch`/`case`, TypeScript optional parameters and
properties, PHP alternative syntax, Rust let chains and `?`, every deviation above in bold, a Blade template's
directives and a Vue component's template, files with only simple functions, and test files that lizard scores over the
limit. Rerail wrote them; nothing is copied from lizard (`upstream/lizard/UPSTREAM.md`).
`upstream/lizard/lizard-1.24.0.tsv` records lizard's score for every function (path relative to the corpus, function
name as lizard prints it, start line, CCN), and `upstream/lizard/fixtures.expected.json` lizard's findings: one
`CYCLOMATIC_COMPLEXITY` per function scored above 6, on the corpus files some rule's scope covers. lizard numbers a
`.vue` file's lines from its first `<script>` block and leaves out the lines between blocks, so each Vue file has one
script block, before its template, where lizard's line is the file's. The Rust corpus has no char or byte literal
outside its deviation case, as lizard misreads them.

`test/engine/code-rule/complexity-rules.test.ts` evaluates the seven rules over the corpus through the native runtime and checks
that:

- each rule reports exactly the in-scope functions lizard scores above 6, with the recorded number in the message;
- the same counting without the threshold gives every in-scope function the recorded number, apart from the deviations,
  which the test lists with lizard's number and Rerail's;
- test files produce no finding, even though lizard scores them over the limit.

lizard is also a gated tool-pack (`upstream/lizard/gates.yaml`, `conformance.json`): the conformance check
(`npm run test:conformance`, [conformance/README.md](../../conformance/README.md#the-gate)) judges each rule on
`fixtures.expected.json` like any other port, one port per language (its `fixtures` and `lints`). A deviation that puts
one side over the limit and the other not is a disagreement there, declared with its evidence in `divergences.json`:
`??`, the Python signatures, the PHP closure inside a function and the Blade `@php` block.

lizard is never a runtime dependency; it runs only to derive the scores. The corpus and what is derived from it are pack
test data, not in git ([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)): the corpus is authored and the rest
derived, and all of it is the tool-pack's archive in S3, which `upstream/lizard/test-data.lock` names and
`npm run fixtures:pull` fetches. The tests fail saying so without them.

## Regenerating the scores

After changing the corpus, derive the scores again (needs [uv](https://docs.astral.sh/uv/) on `PATH`):

```sh
npm run fixtures:derive -- --pack complexity
```

Its recipe (`scripts/pack-data/recipes/complexity.ts`) installs the lizard `scripts/pack-data/runtimes/lizard-1.24.0/`
pins and runs `upstream/lizard/generate.mjs` with it, which runs `lizard --csv` with and without `-m` on
`upstream/lizard/fixtures/`, keeps the lower of the two counts for each function, and rewrites
`upstream/lizard/lizard-1.24.0.tsv` and `upstream/lizard/fixtures.expected.json`, which are never committed. Stage the
changed corpus files and the derived files into the tool-pack's publish request
(`npm run fixtures:stage -- --pack complexity`, which copies them into `upstream/lizard/publish/`); a new corpus file
needs its row in `upstream/lizard/UPSTREAM.md`, which is what makes it authored, and removing a corpus file is removing
its row. A maintainer then runs the publish workflow for the pull request
([docs/upstream.md](../../docs/upstream.md#publishing-pack-data)), which uploads them once a code owner approves and
commits the new `test-data.lock`.
A new case lizard mis-scores needs an entry in the test's `DEVIATIONS` list and a line under "Deviations" above, and a
`divergences.json` entry when it puts one side over the limit and the other not.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `complexity-cyclomatic-typescript`

Conditional types (`T extends U ? X : Y`) do not count as decision points. The README's "What counts" list for
TypeScript names optional parameters/properties, optional chaining, default parameters, `??` and `??=` as not counting,
but does not say conditional types are excluded too (it only mentions lizard reading conditional types as functions
in the hono comparison).

### `complexity-cyclomatic-php`

`and` and `or` count as `&&` and `||` do (a `binary_expression` whose `operator` matches them in any case); `xor`, the
short `?:` (a `conditional_expression` without a `body`), `??`, `??=` and `?->` do not. A closure
(`anonymous_function`) is a function of its own; an arrow function (`fn`) is not. `.blade.php` files are the Blade
rule's.

### `complexity-cyclomatic-blade`

Reads the PHP program Blade compiles the template to (`ast.embedded: "compiled-view"`), with the PHP rule's boundaries
and decisions. The template's control directives become top-level PHP statements there, in no function, so only a
function or closure the template itself defines is scored; the finding is placed at the template bytes it came from.

### `complexity-cyclomatic-rust`

The `?` operator counts, as lizard counts it: each is an early return on an `Err` or `None`. A let chain is one
`let_chain` node, so the rule counts each `&&` token in it rather than the node. `loop` does not count; the `if` that
breaks out of it does.

### `complexity-cyclomatic-vue`

Reads the program a component's scripts and template make (`ast.embedded: "script-block"`). The template is a
`statement_block` whose host is the `template_element`, and its `v-for` and slot bodies and inline handlers are
arrow functions there; the boundary excludes every function inside that block, so only the scripts' functions are
scored. The program is TypeScript for `lang="ts"`, TSX for `lang="tsx"` and JavaScript otherwise.
