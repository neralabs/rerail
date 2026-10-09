# Writing Command-Line Rules

> A practical guide to catching the commands an agent should never run — `git push --force`,
> `rm -rf`, `--no-verify` — with structural tool-call rules. Every example below was run through the
> engine; **FLAG** means the rule fired, **PASS** means it stayed quiet.

- [Why a regex and not similarity](#why-a-regex-and-not-similarity)
- [Anatomy of a rule](#anatomy-of-a-rule)
- [The regex dialect](#the-regex-dialect)
- [Seven steps to a good command regex](#seven-steps-to-a-good-command-regex)
- [When not to use a regex](#when-not-to-use-a-regex)
- [Beyond the command line](#beyond-the-command-line)
- [Test it, then install it](#test-it-then-install-it)
- [Errors you will meet](#errors-you-will-meet)
- [Checklist](#checklist)

---

## Why a regex and not similarity

"Never run `git push --force`" is an exact rule. A similarity rule expresses it as a similarity threshold,
which is too loose (it fires on neighbouring phrasings) and too tight (it misses the one spelling that
matters) at the same time. A structural rule states the command itself.

|                      | Similarity rule             | Structural rule                         |
| -------------------- | --------------------------- | --------------------------------------- |
| Matches              | meaning, by embedding       | text, by `regex` / `eq` / `contains`    |
| Needs a `reference`  | yes                         | no, and rejects one                     |
| Cost per sample      | tokenize + embed (~80 µs)   | no model at all (~5 µs for 32 rules)    |
| Sees tool arguments  | as one flattened JSON string | field by field, e.g. `input.command`   |

## Anatomy of a rule

```json
{
  "id": "oss:no-force-push",
  "mode": "steer",
  "match": { "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s--force(?:\\s|=|$)" },
  "sample": { "source": "tool:Bash", "boundary": "tool-call" },
  "action": { "content": "Do not force-push: it rewrites published history. Ask the user first." },
  "repeat": { "mode": "always" }
}
```

| Part | What it does |
| --- | --- |
| `mode` | `log` only records a match; `steer` tells the agent; `block` is stored as the intent to stop the call and, until block enforcement lands, is delivered exactly like `steer`. |
| `match` | A condition: one `field` plus exactly one of `regex`, `eq`, `contains` — or `all` / `any` / `not` over conditions. |
| `sample.source` | Which tool's calls to read: `tool:Bash` for one tool, `tool:*` for every tool. |
| `sample.boundary` | Always `tool-call` for a rule that reads `tool.name` or `input`. |
| `action` | What the agent is told when the rule fires. |
| `repeat` | `always` suits command rules: every bad call deserves a steer. The default is `once`. |

### Fields

| Field | Reads | Example value |
| --- | --- | --- |
| `input.command` | one string argument, by key | `git push --force origin main` |
| `input.<a>.<b>` | a nested string argument | `input.options.branch` → `main` |
| `input` | all arguments as stable JSON (keys sorted) | `{"command":"git push","description":"push"}` |
| `tool.name` | the tool's name | `Bash`, `shell`, `run_terminal_cmd` |
| `text` | the sample text, for any source | an assistant sentence |

A missing argument, or one that is not a string, never matches. A rule that reads `tool.name` or
`input` only ever sees **tool calls** — never tool results — so output that merely *mentions*
`git push --force` cannot trip it.

## The regex dialect

Patterns use the **Rust `regex` crate** — the same dialect, and the same 16,384-byte
`maxRegexPatternBytes` limit, as the [code-rule DSL](../code-rule-dsl/capabilities.json).
It runs in linear time, so no command an agent writes can make a rule hang.

| You may use | You may not use |
| --- | --- |
| `\b`, `\s`, `\w`, `[^\n]`, `(?:…)`, `{n,m}` | look-ahead / look-behind: `(?=…)`, `(?!…)`, `(?<=…)` |
| inline flags: `(?i)` ignore case, `(?m)` line anchors, `(?s)` dot matches newline | backreferences: `\1` |
| named groups `(?<NAME>…)` | |

Two defaults matter for command lines:

- **Unanchored.** `git push` matches anywhere in the argument. Add `^` / `$` deliberately.
- **`.` stops at a newline.** Without `(?s)`, `.*` and `[^\n]*` behave the same.

In JSON every backslash is doubled: the regex `\bgit\s+push\b` is written `"\\bgit\\s+push\\b"`.

---

## Seven steps to a good command regex

We build the force-push rule up from the obvious first attempt, one failure at a time.

### 1. Start from the naive literal, and watch it fail

```json
{ "field": "input.command", "regex": "git push --force" }
```

| Command | Result | |
| --- | --- | --- |
| `git push --force` | FLAG | |
| `git  push --force` | PASS | missed: two spaces |
| `git push origin main --force` | PASS | missed: flag after the arguments |
| `git push -f` | PASS | missed: short flag |
| `echo 'legit push --force'` | FLAG | false positive: `legit` |

### 2. Bound the words and allow anything in between

`\b` stops `legit` matching `git`; `\s+` absorbs any run of spaces; `[^\n]*` lets arguments sit
between the subcommand and the flag, but only on the same line.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*--force\\b" }
```

| Command | Result | |
| --- | --- | --- |
| `git  push origin main --force` | FLAG | |
| `legit push --force` | PASS | fixed |
| `git pushx --force` | PASS | `\b` after `push` |
| `git push --forced` | PASS | `\b` after `--force` |
| `git push --force-with-lease` | **FLAG** | surprise — see step 3 |

### 3. End a flag with a delimiter, not `\b`

`\b` sits between a word character and a non-word one — and `-` is not a word character, so
`--force\b` also matches the start of `--force-with-lease`. End a flag with what may actually follow
it: whitespace, `=`, or the end of the command. Require whitespace before it, too.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s--force(?:\\s|=|$)" }
```

| Command | Result |
| --- | --- |
| `git push --force` | FLAG |
| `git push --force origin main` | FLAG |
| `git push --force=true` | FLAG |
| `git push --force-with-lease` | PASS |

Whether `--force-with-lease` *should* pass is a policy question. The corpus rule lists it
explicitly: `(?:--force\b|\s-f\b|--force-with-lease\b)`. Decide, then make the regex say so.

### 4. Cover short and bundled flags

`-f` can arrive alone or bundled: `-uf`, `-fu`. Match a single-dash cluster that contains the letter.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s-[A-Za-z]*f[A-Za-z]*\\b" }
```

| Command | Result | |
| --- | --- | --- |
| `git push -f` | FLAG | |
| `git push -uf origin x` | FLAG | bundled |
| `git push --follow-tags` | PASS | `\s-` needs exactly one dash before the letters |
| `git push origin feature-fix` | PASS | `\s-` needs whitespace before the dash |

Case matters for flags: `git branch -D` force-deletes, `git branch -d` does not. Leave `(?i)` off
unless the tool itself ignores case.

| Regex `\bgit\s+branch\b[^\n]*\s-D\b` | Result |
| --- | --- |
| `git branch -D old` | FLAG |
| `git branch -d old` | PASS |

### 5. Think about chained and multi-line commands

Agents chain commands with `&&`, `;`, and `|`, and sometimes send several lines. `^` anchors to the
start of the **whole argument**, so it misses everything after the first command.

| Regex | Command | Result |
| --- | --- | --- |
| `^rm\s` | `rm -rf build` | FLAG |
| `^rm\s` | `cd x && rm -rf build` | **PASS** — missed |

To mean "at the start of any command in the chain", anchor on a separator instead:

```json
{ "field": "input.command", "regex": "(?:^|[;&|]\\s*)rm\\s+-[A-Za-z]*r" }
```

| Command | Result |
| --- | --- |
| `rm -rf build` | FLAG |
| `cd x && rm -rf build` | FLAG |
| `ls \| rm -r y` | FLAG |
| `echo rm -rf build` | PASS |

`[^\n]*` keeps the match on one line, so a flag on a *different* line does not count:

| Regex `\bgit\s+push\b.*--force\b` | Result |
| --- | --- |
| `git push origin main` ⏎ `echo --force` | PASS |
| `git status` ⏎ `git push --force` | FLAG |

Use `(?m)` when `^` and `$` should mean start and end of each line.

### 6. Use `not` where you would reach for look-around

There is no `(?!…)`. Express exceptions as a second condition instead — it reads better anyway.

```json
{
  "all": [
    { "field": "input.command", "regex": "\\brm\\s+-[A-Za-z]*r[A-Za-z]*f|\\brm\\s+-[A-Za-z]*f[A-Za-z]*r" },
    { "not": { "field": "input.command", "regex": "\\brm\\s+-\\w+\\s+(?:\\./)?(?:tmp|build|dist|node_modules)(?:/|\\s|$)" } }
  ]
}
```

| Command | Result | |
| --- | --- | --- |
| `rm -rf src` | FLAG | |
| `rm -rf ~` | FLAG | |
| `rm -rf tmp/cache` | PASS | scratch directory |
| `rm -fr ./build` | PASS | either flag order, optional `./` |

### 7. Know the limit: a regex does not parse shell

A regex sees characters, not shell syntax. It cannot tell a flag from the same text inside a quoted
string:

| Regex `\bgit\s+push\b[^\n]*--force\b` | Result |
| --- | --- |
| `git commit -m 'never git push --force'` | FLAG — false positive |

This is rare in practice, and the cost is one unnecessary steer. If a rule keeps tripping on it,
tighten the prefix (step 5) rather than trying to model quoting.

---

## When not to use a regex

Reach for the simplest test that states the rule.

| Test | Use it when | Example | Result |
| --- | --- | --- | --- |
| `contains` | one literal is the whole signal | `"contains": "--no-verify"` on `git commit --no-verify -m x` | FLAG |
| `eq` | exactly one full command is forbidden | `"eq": "npm publish"` on `npm publish` | FLAG |
| | | `"eq": "npm publish"` on `npm publish --dry-run` | PASS |
| | | `"eq": "npm publish"` on ` npm publish` (leading space) | PASS |

`eq` compares the whole value byte for byte, so it is brittle for commands agents compose freely.
Prefer it for tool names and paths.

## Beyond the command line

**Every host names its shell differently.** Bind the rule to `tool:*` and name the shells:

```json
{
  "match": { "all": [
    { "field": "tool.name", "regex": "^(?:Bash|bash|shell|run_terminal_cmd)$" },
    { "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*--force\\b" }
  ] },
  "sample": { "source": "tool:*", "boundary": "tool-call" }
}
```

| Tool | Command | Result |
| --- | --- | --- |
| `Bash` / `shell` / `run_terminal_cmd` | `git push --force` | FLAG |
| `Write` | (an argument named `command`) | PASS |

**Paths, whatever the argument is called.** Write tools use `file_path` or `path`; cover both with `any`:

```json
{ "any": [
  { "field": "input.file_path", "regex": "(?:^|/)\\.git/" },
  { "field": "input.path", "regex": "(?:^|/)\\.git/" }
] }
```

| Path | Result | |
| --- | --- | --- |
| `.git/hooks/pre-commit` | FLAG | |
| `repo/.git/config` | FLAG | |
| `.github/workflows/ci.yml` | PASS | |
| `my.git/x` | PASS | `(?:^\|/)` requires a path boundary |

**Secrets in any argument.** `input` is every argument as one JSON string, so a single rule covers
commands, file contents, and URLs alike:

```json
{ "field": "input", "regex": "ghp_[A-Za-z0-9]{36}" }
```

Matching `input` sees JSON escaping (`\"`, `\n` as two characters), so keep patterns on it free of
quotes and line anchors. For anything command-shaped, read `input.command`.

---

## Test it, then install it

**1. Write the cases first.** A rule is only as good as the commands it has been tried against.
For each rule, list at least one command it must flag, one near-miss it must pass, and — if it reads
the command line — a tool *result* that echoes the command and must pass.

**2. Run them through the engine.** Add the rule to `test/fixtures/tool-call-rules.json` and its
cases to `test/engine/catalog/tool-call-rules.test.ts`, or try it ad hoc:

```ts
import { resolve } from "node:path";
import { ConversationRuleRegistry, stableJsonText } from "./src/engine/index.js";

const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
registry.replace([rule]);

const input = stableJsonText({ command: "git push --force origin main" })!;
registry.match({ source: "tool:Bash", boundary: "tool-call", text: input, input }, { respectRepeat: false });
// → [{ ruleId: "oss:no-force-push", kind: "structural", action: { … } }]
```

```sh
npx vitest run test/engine/catalog/tool-call-rules.test.ts
```

**3. Measure noise before blocking.** `candidate-rules/oss-tool-rules/` checks each pattern against
the commands your own repository already sanctions (scripts, CI, docs). A rule that matches
sanctioned commands — `rm -rf` in a cleanup script — should use `mode: "steer"`, not `"block"`, until it is narrowed.

**4. Install it** with the daemon running (`rerail daemon start`):

```sh
curl -X PUT http://127.0.0.1:4737/api/rules/oss:no-force-push \
  -H 'content-type: application/json' \
  -d "{\"kind\": \"conversation\", \"enabled\": true, \"rule\": $(cat no-force-push.json)}"
```

The body is validated before it is stored, regex included; an invalid rule is rejected with
`400 invalid-request`. (Structural rules are stored under the `conversation` kind for now.)

## Errors you will meet

| Message (shortened) | Cause and fix |
| --- | --- |
| `rule.match.regex is not a valid regex: regex parse error: … look-around, including look-ahead and look-behind, is not supported` | `(?!…)` or `(?<=…)`. Use `not` (step 6). |
| `rule.match.regex is not a valid regex: regex parse error: … backreferences are not supported` | `\1`. Spell the repeated text out. |
| `rule.match.regex is 16385 bytes; maxRegexPatternBytes allows at most 16384` | Split the pattern across `any`. |
| `rule.match must contain exactly one of regex, eq, or contains` | Two tests in one condition. Wrap them in `all`. |
| `rule.match.field must be text, tool.name, input, or input.<path>` | A typo such as `input..command` or `command`. |
| `rule.sample.source must be tool:<name> or tool:* when the match reads tool.name or input` | The rule reads tool fields but samples a conversation source. |
| `rule.sample.boundary must be tool-call when the match reads tool.name or input` | Set `"boundary": "tool-call"`. |
| `rule.reference must be omitted: a structural match computes no similarity` | Delete `reference`; it only belongs on similarity rules. |
| `rule.sample.minimumTokens must be omitted: a structural match does not tokenize` | Delete `minimumTokens`. |

## Checklist

- [ ] The field is as narrow as possible: `input.command`, not `input`.
- [ ] Commands and subcommands are wrapped in `\b…\b`; separators are `\s+`, not a single space.
- [ ] Flags end in `(?:\s|=|$)`, not `\b` — unless you mean to catch longer flags too.
- [ ] Short flags cover bundles: `-[A-Za-z]*f[A-Za-z]*`.
- [ ] `[^\n]*` between parts, so the match stays on one line.
- [ ] Anchored with `(?:^|[;&|]\s*)` if position matters — never a bare `^` on a command.
- [ ] Exceptions are a `not` condition, not look-around.
- [ ] `(?i)` only where the tool ignores case (SQL, not git flags).
- [ ] Shells from every host named in `tool.name`, or the rule bound to the one tool it targets.
- [ ] At least one FLAG case, one near-miss PASS case, and one tool-result PASS case, run through the engine.
- [ ] Noise checked against the repository's own sanctioned commands before `mode` is set to `block`.
- [ ] `repeat` set to `always`, and an `action.content` that says what to do instead.
