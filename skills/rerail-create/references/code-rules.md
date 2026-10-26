# Writing a code rule

A code rule is a `rerail.policy/v1` document. Its clauses check the files under its scope each time the agent edits one. Every
example on this page is a complete draft for `rerail rules test`, and Rerail's test suite runs each one through the engine.

## Anatomy

```json
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:<short-kebab-name>",
    "revision": 1,
    "mode": "steer",
    "description": "One imperative sentence: what the rule wants; a short reason if it isn't obvious.",
    "scope": { "include": ["src/**/*.ts"], "exclude": ["**/*.test.ts"] },
    "clauses": [
      {
        "id": "<clause-name>",
        "forbid": { "regex": { "pattern": "…", "source": "file.content" } },
        "violation": { "code": "UPPER_SNAKE", "anchor": "witness.span", "message": "What to change in this code, and when to leave it." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "src/a.ts", "content": "…" }],
    "passed": [{ "path": "src/a.ts", "content": "…" }]
  }
}
```

- **`scope`** holds repository-relative globs. Excludes win. Name the folders the rule is about, and exclude tests, generated
  code, fixtures and vendored code unless the rule is about them. A case outside the scope is reported as "outside the files this
  rule covers" and passes: use one to show the scope works, but never as a caught case.
- **`folder`** (optional, at the top level of the draft beside `kind`) adds the rule at one folder of the repository instead of
  at the whole of it.
- **`description`** is for people reading the rule later in the dashboard. See [Writing the description](#writing-the-description).
- **`violation.message`** is the steer the agent reads. See [Writing the message](#writing-the-message).
- **Raise `revision`** every time you change a rule that has been added.

## Regex or syntax

| Use | When | Cost |
|---|---|---|
| `regex` over `file.content` | the thing is a token, an import line, a path, or a comment marker; any file type | cheapest; it can match inside strings and comments |
| `ast` with `pattern` | a call, an expression or a statement in Python, JavaScript, TypeScript, PHP or Blade | needs the file to parse; ignores strings and comments |
| `ast` with `inside` / `has` / `not` | the thing is wrong only in some context: inside an `async def`, a loop, or a component | as above; the clearest way to state an exception |

Rerail parses Python, JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`), TypeScript (`.ts`, `.tsx`), PHP and Blade. It reads other text
files for `regex` clauses only.

### Regex

Patterns use the Rust `regex` dialect. In JSON, write every backslash twice.

- **No look-around and no backreferences.** Express an exception as an `ast` `not`, or as a narrower pattern.
- **`(?m)`** makes `^` and `$` match at each line. Without it they match the start and end of the file.
- **`(?s)`** lets `.` match a newline. Without it, `.*` stays on one line.
- **Bound both ends of a word.** `\bfoo\b`. `(?:QuerySet|Manager)\b` also matches `AbstractContextManager`.
- A flag ends in `(?:\s|=|$)`, not `\b`: `-` is not a word character, so `--force\b` also matches `--force-with-lease`.

### Syntax (ast)

The `ast` query is an [ast-grep](https://ast-grep.github.io) rule with `language` set to `python`, `javascript`, `typescript`,
`php` or `blade`.

- **`pattern`** is code with metavariables. `$X` matches one node and `$$$ARGS` matches any number of them:
  `console.log($$$A)`, `requests.get($URL, $$$REST)`.
- **`kind`** is a tree-sitter node kind: `function_definition`, `call_expression`, `import_statement`.
- **`regex`** inside a rule tests the node's source text: `{ "kind": "function_definition", "regex": "^async\\s" }`.
- **`inside`, `has`, `not`, `all`, `any`** combine rules. `inside` and `has` look only at the parent or direct children unless
  you add `"stopBy": "end"`.
- **`utils`** with `matches`: name a sub-rule once and reuse it (see `packs/complexity/rules/*.json` in the Rerail package).
- **A metavariable in the message:** `"message": "Use the logger, not console.$M."` fills in what `$M` matched.

When a case does not parse, `rules test` reports it as `not checked`, never as passed. Fix the case's code or switch to `regex`.

## Writing the description

The description is for people browsing the rules in the dashboard. The agent never reads it.

1. Readable at a glance by a developer, in plain English.
2. States the intent: what the rule wants the developer to do or avoid.
3. No code or source material: no snippets, regexes, AST node kinds, globs, JSON, config keys or backticks. Naming the API
   or construct the rule is about is fine when the intent needs it ("console.log", "time.sleep", "useEffect").
4. No origin: no linter or rule names it was taken from, no versions.
5. Clear and concise.
6. Accurate: it matches what the clauses and scope catch, no more and no less.
7. One imperative sentence that starts with a verb (Avoid, Use, Prefer, Keep, Remove, Add, Don't…) and ends with a
   period, optionally followed by `; <short reason>`.
8. At most 200 characters.
9. A reason only when it isn't obvious.
10. Name the language or file type when a sibling rule covers another one.
11. Keep thresholds that matter ("more than 5 parameters"); drop implementation trivia.
12. Don't just repeat the message.

What it deliberately lets through can go after the semicolon when it fits and matters to a reader: `Log through logger in
application code, not console.log; scripts and tests may print.`

## Writing the message

The message is what the agent acts on. Rerail shows it as one line in a block that already gives the location, the rule
id and "fix it, or tell the user why you won't":

```
Rerail found 1 problem in your last edit:
- src/api/orders.ts:2:3 — <message> [local:use-the-logger]
Fix it, or tell the user in one line why you won't.
```

So the message tells the agent what to change and, when the change could be wrong, when not to.

1. **Name the change:** a replacement or what to remove. A fact or a ban alone isn't advice. Where a helper exists,
   point to it rather than saying "remove" or "delete": a steer to delete is the most common harmful steer.
2. **Say when to leave it alone** if following it would be wrong for some code it matches, or what the change trades
   off.
3. **State a risk only as far as the rule can confirm it,** without softening ("consider") or shouting (NEVER, MUST).
4. **Be right for every match you don't exempt,** agree with the clause's `fix`, and name captures (`$NAME`) where that
   shows which code you mean.
5. **Leave out the rule id, location, "Rerail" and "fix it or explain",** and name no linter, rule code, tool version
   or matcher term outside quoted code.
6. **At most 200 characters for each text it can show** (metavariables as written), in any number of sentences.
7. **Give matches that need different advice different messages** (`x == None` and `x != None`).

| Instead of | Write |
|---|---|
| `Bandit B113 (request-without-timeout)` | `Pass a finite timeout to this HTTP call; without one a slow server can hang the program.` |
| `zip() without an explicit strict= parameter` | ``Pass `strict=` explicitly: `strict=True` when lengths must match, `strict=False` when truncation is intended.`` |
| `` Comparison to `None` should be `cond is None` ``, one clause for `==` and `!=` | One clause each: ``Use `is None` instead of `== None` for this comparison.`` and ``Use `is not None` instead of `!= None` for this comparison.`` |

Also:

- **Do not word it around one literal variant.** Write "with or without `override`", not "`override get`".
- **Do not claim what the rule did not check.** Write "this rule found no write to `$V` in the loop", not "`$V` never changes".

## Examples

### An import boundary (regex)

UI components must not reach into the database layer.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-db-imports-in-components",
    "revision": 1,
    "mode": "steer",
    "description": "Read data in components under src/components through the hooks in src/data, not by importing src/db; type-only imports are fine.",
    "scope": { "include": ["src/components/**/*.ts", "src/components/**/*.tsx"], "exclude": ["**/*.test.*"] },
    "clauses": [
      {
        "id": "db-import",
        "forbid": { "regex": { "pattern": "(?m)^import\\s+(?:\\{[^}]*\\}|\\*\\s+as\\s+[\\w$]+|[A-Za-z_$][\\w$]*(?:\\s*,\\s*\\{[^}]*\\})?)\\s+from\\s+['\"](?:@/db|(?:\\.\\./)+db)(?:/[^'\"]*)?['\"]", "source": "file.content" } },
        "violation": { "code": "DB_IMPORT_IN_COMPONENT", "anchor": "witness.span", "message": "Read this data through a hook in src/data instead of importing src/db, adding a hook if none fits." }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/components/UserCard.tsx", "content": "import { getUser } from '@/db/users';\nexport function UserCard() { return null; }\n" },
      { "path": "src/components/admin/Table.tsx", "content": "import db, { tx } from '../../db';\n" },
      { "path": "src/components/Orders.tsx", "content": "import * as orders from '@/db/orders';\n" }
    ],
    "passed": [
      { "path": "src/components/UserCard.tsx", "content": "import { useUser } from '@/data/users';\n" },
      { "path": "src/components/UserCard.tsx", "content": "import type { User } from '@/db/users';\n" },
      { "path": "src/data/users.ts", "content": "import { getUser } from '@/db/users';\n" }
    ]
  }
}
```

Adapt the paths to the repo: this one has a `@/db` alias and relative imports. It does not cover the inline form
`import { type User }`, `export … from` or `import()`; extend it if the repo writes those. The pattern spells out the three forms
of a value import: `{ … }`, `* as name`, and a default name with optional `, { … }`. A
type-only import passes because `import type {` fits none of them. A first draft that allowed any word after `import` caught it,
and the passed case is what showed that. The file in `src/data` passes because it is outside the scope.

### A repo convention (ast pattern)

The repo logs through its logger; agents reach for `console.log`.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:use-the-logger",
    "revision": 1,
    "mode": "steer",
    "description": "Log through logger in application code, not console.log; scripts and tests may print.",
    "scope": { "include": ["src/**/*.ts", "src/**/*.tsx"], "exclude": ["**/*.test.*", "src/scripts/**"] },
    "clauses": [
      {
        "id": "console-log",
        "forbid": { "ast": { "language": "typescript", "rule": { "pattern": "console.log($$$ARGS)" } } },
        "violation": { "code": "CONSOLE_LOG", "anchor": "witness.span", "message": "Log through `logger` from src/log.ts (`logger.info`, `logger.debug`) instead of console.log." }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "src/api/orders.ts", "content": "export function place() {\n  console.log('placing');\n}\n" }
    ],
    "passed": [
      { "path": "src/api/orders.ts", "content": "import { logger } from '../log';\nexport function place() {\n  logger.info('placing');\n}\n" },
      { "path": "src/api/orders.ts", "content": "const help = 'call console.log(x) to debug';\n" },
      { "path": "src/scripts/seed.ts", "content": "console.log('seeded');\n" }
    ]
  }
}
```

The string mentioning `console.log(x)` passes. A regex would have caught it.

### A defect only in context (ast inside / not)

`time.sleep` is fine in sync code and a bug in `async def`.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-blocking-sleep-in-async",
    "revision": 1,
    "mode": "steer",
    "description": "Use await asyncio.sleep instead of time.sleep inside async functions; time.sleep blocks the event loop.",
    "scope": { "include": ["**/*.py"], "exclude": ["**/.venv/**"] },
    "clauses": [
      {
        "id": "blocking-sleep",
        "forbid": {
          "ast": {
            "language": "python",
            "rule": {
              "pattern": "time.sleep($$$ARGS)",
              "inside": { "kind": "function_definition", "regex": "^async\\s", "stopBy": "end" },
              "not": { "inside": { "kind": "function_definition", "not": { "regex": "^async\\s" }, "stopBy": "end" } }
            }
          }
        },
        "violation": { "code": "BLOCKING_SLEEP_IN_ASYNC", "anchor": "witness.span", "message": "Replace time.sleep with `await asyncio.sleep(...)`; it blocks the event loop inside async code." }
      }
    ]
  },
  "cases": {
    "caught": [
      { "path": "app/jobs.py", "content": "import time\n\nasync def poll():\n    time.sleep(1)\n" },
      { "path": "app/jobs.py", "content": "import time\n\nclass W:\n    async def run(self):\n        if True:\n            time.sleep(0.5)\n" }
    ],
    "passed": [
      { "path": "app/jobs.py", "content": "import asyncio\n\nasync def poll():\n    await asyncio.sleep(1)\n" },
      { "path": "app/jobs.py", "content": "import time\n\ndef poll():\n    time.sleep(1)\n" },
      { "path": "app/jobs.py", "content": "import time\n\nasync def outer():\n    def worker():\n        time.sleep(1)\n    return worker\n" }
    ]
  }
}
```

The last passed case is the near miss: a sync worker defined inside an async function may sleep.

## More

The Rerail package holds the full grammar in `native/src/code_rule_dsl/capabilities.json`, and shipped rules in `packs/*/rules/`.
The complexity rules use `utils` and `count`. For every construct, see `docs/code-rule-dsl-reference.html` in the Rerail
repository.
