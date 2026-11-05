# Writing command-line rules

> Structural tool-call rules catch commands an agent should never run: `git push --force`, `rm -rf`,
> `--no-verify`. Every example ran through the engine. **FLAG**: the rule fired. **PASS**: it stayed quiet.

Internal testing: Rerail's latest release ships code rules only, not tool-call rules. See
[Internal testing](../README.md#internal-testing).

---

## Why a regex and not similarity

A structural rule states the exact command; a similarity threshold cannot.

<details>
<summary>Similarity and structural rules compared</summary>

As a similarity threshold, "never run `git push --force`" fires on neighbouring phrasings and misses the
one spelling that matters.

|                      | Similarity rule             | Structural rule                         |
| -------------------- | --------------------------- | --------------------------------------- |
| Matches              | meaning, by embedding       | text, by `regex` / `eq` / `contains`    |
| Needs a `reference`  | yes                         | no, and rejects one                     |
| Cost per sample      | tokenize + embed (~80 µs)   | no model at all (~5 µs for 32 rules)    |
| Sees tool arguments  | as one flattened JSON string | field by field, e.g. `input.command`   |

</details>

## Anatomy of a rule

A rule pairs a match condition with the tool calls it reads and what to tell the agent.

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

- **`match`**: the test, and the field it reads.
- **`sample`**: which tool calls the rule reads.
- **`mode`**, **`action`**, **`repeat`**: what happens, and how often, when it fires.

<details>
<summary>What each part does</summary>

| Part | What it does |
| --- | --- |
| `mode` | `log` records a match; `steer` tells the agent; `block` is stored as the intent to stop the call and, until block enforcement lands, is delivered exactly like `steer`. |
| `match` | One `field` plus exactly one of `regex`, `eq`, `contains` — or `all` / `any` / `not` over conditions. |
| `sample.source` | Which tool's calls to read: `tool:Bash` for one tool, `tool:*` for every tool. |
| `sample.boundary` | Always `tool-call` for a rule that reads `tool.name` or `input`. |
| `action` | What the agent is told when the rule fires. |
| `repeat` | `always` suits command rules: every bad call deserves a steer. Default: `once`. |

</details>

### Fields

Command rules usually read `input.command`, one string argument. A missing or non-string argument never matches.

<details>
<summary>All fields</summary>

| Field | Reads | Example value |
| --- | --- | --- |
| `input.command` | one string argument, by key | `git push --force origin main` |
| `input.<a>.<b>` | a nested string argument | `input.options.branch` → `main` |
| `input` | all arguments as stable JSON (keys sorted) | `{"command":"git push","description":"push"}` |
| `tool.name` | the tool's name | `Bash`, `shell`, `run_terminal_cmd` |
| `text` | the sample text, for any source | an assistant sentence |

Rules on `tool.name` or `input` see only **tool calls**, never tool results, so output that merely *mentions*
`git push --force` cannot trip them.

</details>

## The regex dialect

Patterns use the Rust `regex` crate, which runs in linear time: no command can make a rule hang.

- **Unanchored.** `git push` matches anywhere in the argument. Add `^` / `$` deliberately.
- **`.` stops at a newline.** Without `(?s)`, `.*` and `[^\n]*` behave the same.
- **No look-around or backreferences.** Use a `not` condition instead (step 6).

In JSON, double every backslash: the regex `\bgit\s+push\b` is written `"\\bgit\\s+push\\b"`.

<details>
<summary>Allowed syntax and limits</summary>

Dialect and 16,384-byte `maxRegexPatternBytes` limit match the
[code-rule DSL](../native/src/code_rule_dsl/capabilities.json).

| You may use | You may not use |
| --- | --- |
| `\b`, `\s`, `\w`, `[^\n]`, `(?:…)`, `{n,m}` | look-ahead / look-behind: `(?=…)`, `(?!…)`, `(?<=…)` |
| inline flags: `(?i)` ignore case, `(?m)` line anchors, `(?s)` dot matches newline | backreferences: `\1` |
| named groups `(?<NAME>…)` | |

</details>

---

## Seven steps to a good command regex

Build the force-push rule from a naive first attempt, fixing one failure at a time.

### 1. Start from the naive literal, and watch it fail

The literal misses extra spaces, later flags and `-f`, and fires on `legit`.

```json
{ "field": "input.command", "regex": "git push --force" }
```

<details>
<summary>Results</summary>

| Command | Result | |
| --- | --- | --- |
| `git push --force` | FLAG | |
| `git  push --force` | PASS | missed: two spaces |
| `git push origin main --force` | PASS | missed: flag after the arguments |
| `git push -f` | PASS | missed: short flag |
| `echo 'legit push --force'` | FLAG | false positive: `legit` |

</details>

### 2. Bound the words and allow anything in between

`\b` stops `legit` matching `git`. `\s+` absorbs any run of spaces. `[^\n]*` lets arguments sit between subcommand
and flag, on the same line only.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*--force\\b" }
```

<details>
<summary>Results</summary>

| Command | Result | |
| --- | --- | --- |
| `git  push origin main --force` | FLAG | |
| `legit push --force` | PASS | fixed |
| `git pushx --force` | PASS | `\b` after `push` |
| `git push --forced` | PASS | `\b` after `--force` |
| `git push --force-with-lease` | **FLAG** | surprise — see step 3 |

</details>

### 3. End a flag with a delimiter, not `\b`

`--force\b` also matches `--force-with-lease`. End a flag with whitespace, `=` or end of command, and require whitespace before it.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s--force(?:\\s|=|$)" }
```

<details>
<summary>Results and why</summary>

`-` is not a word character, so `\b` matches between `--force` and `-with-lease`.

| Command | Result |
| --- | --- |
| `git push --force` | FLAG |
| `git push --force origin main` | FLAG |
| `git push --force=true` | FLAG |
| `git push --force-with-lease` | PASS |

Whether `--force-with-lease` *should* pass is policy; the corpus rule flags it:
`(?:--force\b|\s-f\b|--force-with-lease\b)`. Decide, then make the regex say so.

</details>

### 4. Cover short and bundled flags

Match any single-dash cluster containing the letter, so `-f`, `-uf` and `-fu` all count.

```json
{ "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s-[A-Za-z]*f[A-Za-z]*\\b" }
```

Flags are case-sensitive: leave `(?i)` off unless the tool ignores case.

<details>
<summary>Results</summary>

| Command | Result | |
| --- | --- | --- |
| `git push -f` | FLAG | |
| `git push -uf origin x` | FLAG | bundled |
| `git push --follow-tags` | PASS | `\s-` needs exactly one dash before the letters |
| `git push origin feature-fix` | PASS | `\s-` needs whitespace before the dash |

`git branch -D` force-deletes, `git branch -d` does not:

| Regex `\bgit\s+branch\b[^\n]*\s-D\b` | Result |
| --- | --- |
| `git branch -D old` | FLAG |
| `git branch -d old` | PASS |

</details>

### 5. Think about chained and multi-line commands

Agents chain commands with `&&`, `;` and `|`. `^` anchors to the start of the **whole argument**, so anchor on a separator instead:

```json
{ "field": "input.command", "regex": "(?:^|[;&|]\\s*)rm\\s+-[A-Za-z]*r" }
```

<details>
<summary>Results, and multi-line commands</summary>

A bare `^` misses every command after the first:

| Regex | Command | Result |
| --- | --- | --- |
| `^rm\s` | `rm -rf build` | FLAG |
| `^rm\s` | `cd x && rm -rf build` | **PASS** — missed |

Anchored on a separator:

| Command | Result |
| --- | --- |
| `rm -rf build` | FLAG |
| `cd x && rm -rf build` | FLAG |
| `ls \| rm -r y` | FLAG |
| `echo rm -rf build` | PASS |

`[^\n]*` keeps the match on one line; a flag on a *different* line does not count:

| Regex `\bgit\s+push\b.*--force\b` | Result |
| --- | --- |
| `git push origin main` ⏎ `echo --force` | PASS |
| `git status` ⏎ `git push --force` | FLAG |

Use `(?m)` to make `^` and `$` match at each line.

</details>

### 6. Use `not` where you would reach for look-around

There is no `(?!…)`. Write the exception as a second, `not` condition.

```json
{
  "all": [
    { "field": "input.command", "regex": "\\brm\\s+-[A-Za-z]*r[A-Za-z]*f|\\brm\\s+-[A-Za-z]*f[A-Za-z]*r" },
    { "not": { "field": "input.command", "regex": "\\brm\\s+-\\w+\\s+(?:\\./)?(?:tmp|build|dist|node_modules)(?:/|\\s|$)" } }
  ]
}
```

<details>
<summary>Results</summary>

| Command | Result | |
| --- | --- | --- |
| `rm -rf src` | FLAG | |
| `rm -rf ~` | FLAG | |
| `rm -rf tmp/cache` | PASS | scratch directory |
| `rm -fr ./build` | PASS | either flag order, optional `./` |

</details>

### 7. Know the limit: a regex does not parse shell

A regex sees characters, not shell syntax, so it cannot tell a flag from the same text in a quoted string:

| Regex `\bgit\s+push\b[^\n]*--force\b` | Result |
| --- | --- |
| `git commit -m 'never git push --force'` | FLAG — false positive |

This is rare and costs one unneeded steer. If it recurs, tighten the prefix (step 5); don't model quoting.

---

## When not to use a regex

Use the simplest test that states the rule; often that is `contains` or `eq`.

- **`contains`**: one literal is the whole signal, such as `--no-verify`.
- **`eq`**: exactly one full command is forbidden. It compares byte for byte, so prefer it for tool names and paths.

<details>
<summary>Examples</summary>

| Test | Use it when | Example | Result |
| --- | --- | --- | --- |
| `contains` | one literal is the whole signal | `"contains": "--no-verify"` on `git commit --no-verify -m x` | FLAG |
| `eq` | exactly one full command is forbidden | `"eq": "npm publish"` on `npm publish` | FLAG |
| | | `"eq": "npm publish"` on `npm publish --dry-run` | PASS |
| | | `"eq": "npm publish"` on ` npm publish` (leading space) | PASS |

`eq` is brittle on free-form commands: an extra flag or a leading space makes it pass.

</details>

## Beyond the command line

Match shells across hosts, paths under any argument name, and secrets in any argument.

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

**Paths, whatever the argument is called.** Write tools use `file_path` or `path`; cover both with `any`:

```json
{ "any": [
  { "field": "input.file_path", "regex": "(?:^|/)\\.git/" },
  { "field": "input.path", "regex": "(?:^|/)\\.git/" }
] }
```

**Secrets in any argument.** `input` is all arguments as one JSON string, so one rule covers commands, file
contents, and URLs:

```json
{ "field": "input", "regex": "ghp_[A-Za-z0-9]{36}" }
```

<details>
<summary>Results and JSON escaping</summary>

Shell names:

| Tool | Command | Result |
| --- | --- | --- |
| `Bash` / `shell` / `run_terminal_cmd` | `git push --force` | FLAG |
| `Write` | (an argument named `command`) | PASS |

Paths:

| Path | Result | |
| --- | --- | --- |
| `.git/hooks/pre-commit` | FLAG | |
| `repo/.git/config` | FLAG | |
| `.github/workflows/ci.yml` | PASS | |
| `my.git/x` | PASS | `(?:^\|/)` requires a path boundary |

`input` is JSON-escaped (`\"`, and `\n` as two characters): keep its patterns free of quotes and line
anchors. For commands, read `input.command`.

</details>

---

## Test it, then install it

Test each rule against real cases, measure its noise, then install it.

**1. Write the cases first.** For each rule, list at least:

- one command it must flag;
- one near-miss it must pass;
- if it reads the command line, a tool *result* that echoes the command and must pass.

**2. Run them through the engine.** Put the rule and its cases in one `conversation` draft file, then test it:

```json
{
  "kind": "conversation",
  "rule": {
    "id": "oss:no-force-push",
    "mode": "steer",
    "match": { "field": "input.command", "regex": "\\bgit\\s+push\\b[^\\n]*\\s--force(?:\\s|=|$)" },
    "sample": { "source": "tool:Bash", "boundary": "tool-call" },
    "action": { "content": "Do not force-push: it rewrites published history. Ask the user first." },
    "repeat": { "mode": "always" }
  },
  "cases": {
    "caught": [
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git push --force origin main\"}", "input": "{\"command\":\"git push --force origin main\"}" }
    ],
    "passed": [
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git push --force-with-lease origin main\"}", "input": "{\"command\":\"git push --force-with-lease origin main\"}" },
      { "source": "tool:Bash", "boundary": "tool-call", "text": "hint: use 'git push --force' to overwrite" }
    ]
  }
}
```

```sh
rerail rules test no-force-push.draft.json
```

It exits non-zero when a case fails.

<details>
<summary>Draft format and drafting help</summary>

A tool-call case carries its arguments as stable JSON in both `text` and `input`; a tool-result case has
`text` only. `rerail rules test` validates the rule, regex included, and runs every case without storing
anything. `rerail rules test --help` prints the draft format. `/rerail-create` has your coding agent draft
the rule and its cases.

</details>

**3. Measure noise before blocking.** Search your sanctioned commands (scripts, CI, docs) with the same
pattern. `rg` uses the same Rust dialect, with single backslashes:

```sh
rg '\bgit\s+push\b[^\n]*\s--force(?:\s|=|$)' scripts .github docs
```

Keep `mode: "steer"`, not `"block"`, while it still matches sanctioned commands, such as `rm -rf` in a cleanup
script.

**4. Install it.** `rerail rules add` stores the rule in your library and adds it to the current checkout's
repository:

```sh
rerail rules add no-force-push.draft.json
```

It runs in Log until a person approves it: `rerail rules approve oss:no-force-push` or the dashboard
([approval](rule-scoping.md#approval)).

<details>
<summary>Install options and Rerail's own rules</summary>

`--repo <dir>` adds the rule to another repository. It applies at the draft's `folder`, or repository-wide
without one. To change the tool-call rules in Rerail's own tests (`fixtures/host/tool-call-rules/`), follow
[CONTRIBUTING.md](../CONTRIBUTING.md).

</details>

## Errors you will meet

Each validation error names the part of the rule at fault; the table gives the fix.

<details>
<summary>Error messages and fixes</summary>

| Message (shortened) | Cause and fix |
| --- | --- |
| `rule.match.regex is not a valid regex: regex parse error: … look-around, including look-ahead and look-behind, is not supported` | `(?!…)` or `(?<=…)`. Use `not` (step 6). |
| `rule.match.regex is not a valid regex: regex parse error: … backreferences are not supported` | `\1`. Spell the repeated text out. |
| `rule.match.regex is 16385 bytes; maxRegexPatternBytes allows at most 16384` | Split the pattern across `any`. |
| `rule.match must contain exactly one of regex, eq, or contains` | Two tests in one condition. Wrap them in `all`. |
| `rule.match.field must be text, tool.name, input, or input.<path>` | A typo such as `input..command` or `command`. |
| `rule.sample.source must be tool:<name> or tool:* when the match reads tool.name or input` | The rule reads tool fields but samples a conversation source. |
| `rule.sample.boundary must be tool-call when the match reads tool.name or input` | Set `"boundary": "tool-call"`. |
| `rule.reference must be omitted: a structural match computes no similarity` | Delete `reference`; it belongs only on similarity rules. |
| `rule.sample.minimumTokens must be omitted: a structural match does not tokenize` | Delete `minimumTokens`. |

</details>

## Checklist

Check every rule against this list before installing it.

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
