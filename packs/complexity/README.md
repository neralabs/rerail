# Complexity opt-in rule pack

Cyclomatic complexity rules for Python, TypeScript and JavaScript, with the decision points of
[lizard](https://github.com/terryyin/lizard) 1.24.0 and without its tokenizer defects. One document per language, rule id
`complexity:cyclomatic-<language>`. They form the `complexity` rule pack, which is never recommended and runs only in
repositories that add it (`repository_attachments_set`; see `docs/rule-packs.md`).
Every document has `mode: "steer"`: a rule that runs steers the agent and never blocks.

| Rule | Files |
|---|---|
| `complexity:cyclomatic-python` | `.py` |
| `complexity:cyclomatic-typescript` | `.ts`, `.tsx`, `.mts`, `.cts` |
| `complexity:cyclomatic-javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` |

A function's cyclomatic complexity is 1 plus its decision points. A function fails when that exceeds 6, so 7 or more.
The finding spans the whole function, so under the `ai-output` scope only functions an edit touched are reported, and
the message carries the number: `Cyclomatic complexity 7 (limit 6). Split this function into smaller ones or simplify
its branching.` (code `CYCLOMATIC_COMPLEXITY`). The number comes from the count's `as`/`offset` binding: the rule
counts decision points with `count.has` (`stopBy` the function boundary, so a nested function is not entered), binds
the count plus 1 as `$CCN`, and fires at `min: 6` decisions.

Test files are out of scope: Python `**/test_*.py`, `**/*_test.py`, `**/tests/**`, `**/conftest.py`; TypeScript and
JavaScript `**/*.test.*`, `**/*.spec.*`, `**/__tests__/**`. So are dependency, virtual-environment, build-output and
generated directories (`node_modules`, `.venv`, `dist`, `build`, `generated`, ...), as in the TSLint pack.

## What counts

Python, per `def` (async, decorated and methods included):

- `if`, `elif`, `for` and `async for`, `while`, `except`, `finally`, each `and` and each `or`, and each `case` of a
  `match` (the wildcard included);
- the `if` of a conditional expression, and every `for` and `if` clause of a comprehension; a `case` guard is an `if`
  too;
- not `else`, `with`, `assert`, `try` or `return`.

A nested `def` is scored on its own and adds nothing to its parent. A `lambda` is not a function here: its conditions
count toward the enclosing `def`, as lizard counts them.

TypeScript and JavaScript, per function declaration, function expression, arrow function, method, getter, setter,
constructor and generator function:

- `if` (`else if` included), `for`, `for...in`, `for...of`, `while`, `do...while`, `catch`, each `case` of a `switch`
  (not `default`), `&&`, `||`, `||=`, `&&=` and the `?:` conditional;
- not optional parameters or properties (`a?: T`), optional chaining (`?.`), default parameters, `??` or `??=`.

Each function is scored on its own: decisions inside a nested function do not count for the function around it. A
class is not scored itself.

Tree-sitter node kinds, from the grammars' `node-types.json` (tree-sitter-python and tree-sitter-typescript as vendored
under `native/grammars/`, tree-sitter-javascript 0.25.0):

| Language | Function boundary | Decision |
|---|---|---|
| Python | `function_definition` | `if_statement`, `elif_clause`, `for_statement`, `while_statement`, `except_clause`, `finally_clause`, `boolean_operator` (one per operator), `conditional_expression`, `for_in_clause`, `if_clause`, `case_clause` |
| TypeScript, JavaScript | `function_declaration`, `function_expression`, `arrow_function`, `method_definition`, `generator_function`, `generator_function_declaration` | `if_statement`, `for_statement`, `for_in_statement` (in and of), `while_statement`, `do_statement`, `catch_clause`, `switch_case`, `ternary_expression`, `binary_expression` with `&&` or `\|\|`, `augmented_assignment_expression` with `&&=` or `\|\|=` |

## Deviations from lizard 1.24.0

Each is a lizard defect found on real code (see "Checked on real code"), and each has a corpus case.

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

## Checked on real code

The rules were compared with lizard 1.24.0 function by function on released sources (2026-10-07), pairing each lizard
function with the Rerail function that starts within two lines of it:

| Repository | Rule | Paired | Same number | The rest |
|---|---|--:|--:|---|
| jinja 3.1.4 `src` | Python | 747 | 743 | 4 defs with a multi-line return annotation (crapkit #72); collapsing each signature onto one line makes lizard agree with Rerail |
| click 8.1.7 `src` | Python | 512 | 512 | |
| marshmallow 3.21.3 `src` | Python | 254 | 254 | |
| express 4.21.0 `lib` | JavaScript | 153 | 152 | 1 function lizard runs on past its end into the next one |
| axios 1.7.7 `lib` | JavaScript | 285 | 270 | regex `?`, multi-line arrow bodies, lizard ranges |
| zod 3.23.8 `src` | TypeScript | 440 | 413 | `??`, `)?.`, multi-line arrow bodies, lizard ranges |
| hono 4.6.3 `src` | TypeScript | 728 | 610 | the same, plus conditional types lizard reads as functions |

Python agrees on every function lizard reads correctly. lizard also leaves out one-line stubs (`def f(x) -> T: ...`),
which Rerail scores 1. Every TypeScript and JavaScript difference inspected traced to one of the deviations above, to
lizard getting a function's range wrong (running past its end, ending it early, or missing a function inside an array
literal), or to lizard reading a type as a function; none was a Rerail miscount. Not every difference was inspected
one by one.

The TypeScript grammar does not parse a few newer forms, such as `export type * from` and call signatures with type
parameters in an interface. In such a file the functions outside the error regions are scored and the rest are left
unknown, so they never report a guessed number (8 of hono's 165 files).

## Parity corpus

`test/fixtures/complexity/{python,typescript,javascript}/` holds small source files that cover every counted
decision, a function at exactly 6 and one at 7 for several kinds, nested functions, Python lambdas and comprehensions,
`match`/`case` and `switch`/`case`, TypeScript optional parameters and properties, every deviation above, files with
only simple functions, and test files that lizard scores over the limit. `lizard-1.24.0.tsv` beside it records lizard's
score for every function (path relative to the corpus, function name as lizard prints it, start line, CCN).

`test/complexity-rules.test.ts` evaluates the three rules over the corpus through the native runtime and checks
that:

- each rule reports exactly the in-scope functions lizard scores above 6, with lizard's number in the message;
- the same counting without the threshold gives every in-scope function lizard's number, apart from the deviations,
  which the test lists with lizard's number and Rerail's;
- test files produce no finding, even though lizard scores them over the limit.

lizard is never a runtime or CI dependency: the TSV is checked in.

## Regenerating the scores

After changing the corpus, rerun lizard (needs [uv](https://docs.astral.sh/uv/) on `PATH`) and commit the TSV:

```sh
node scripts/record-lizard-complexity.mjs
```

It runs `uvx --from lizard==1.24.0 lizard --csv test/fixtures/complexity/` and rewrites
`test/fixtures/complexity/lizard-1.24.0.tsv`. A new case lizard mis-scores needs an entry in the test's `DEVIATIONS`
list and a line under "Deviations" above.
