# Writing a command rule

Internal testing: conversation rules are not part of Rerail 0.1.0, which releases code rules only. Tell the person so in the
read-back (SKILL.md, Write step 6).

A conversation rule with a structural `match` checks each tool call the agent makes, before Rerail records it. Use one for a
command the agent should never run, or should run only with care: force pushes, skipped hooks, publishing, deleting outside
scratch folders. Rerail's test suite runs every example on this page through the engine.

## Anatomy

```json
{
  "kind": "conversation",
  "rule": {
    "id": "local:<short-kebab-name>",
    "mode": "steer",
    "match": { "field": "input.command", "regex": "…" },
    "sample": { "source": "tool:*", "boundary": "tool-call" },
    "action": { "content": "What not to do, and what to do instead." },
    "repeat": { "mode": "always" }
  },
  "cases": {
    "caught": [{ "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"…\"}", "input": "{\"command\":\"…\"}" }],
    "passed": []
  }
}
```

| Part | What to put there |
|---|---|
| `match` | One condition: a `field` and exactly one of `regex`, `eq` or `contains`. Combine conditions with `all`, `any` and `not`. |
| `field` | `input.command` for a shell command. `tool.name` for the tool. `input.<key>` for another argument, such as `input.file_path`. |
| `sample.source` | `tool:*` reads every agent's shell: `Bash` in Claude Code, `bash` in OMP and Pi, and the shell tools of Codex and Cursor. Use `tool:<name>` to read one tool. |
| `action.content` | The steer. Say what to do instead, and when it is fine to ask the person. |
| `repeat` | `always` for commands: every bad call deserves a steer. |

A case's `input` is the call's arguments as JSON. Its `text` is the same JSON string.

## Building the regex

1. **Bound the words and allow arguments in between, on one command.** Write `\bgit\s+push\b[^\n;&|]*`. `[^\n;&|]*` stops at the
   next command in a chain, so `git status; ls -f` cannot match a push rule.
2. **End a flag with what may follow it:** `\s--force(?:\s|=|$)`. Written as `--force\b`, it also matches `--force-with-lease`.
3. **Cover bundled short flags:** `\s-[A-Za-z]*f[A-Za-z]*\b` matches `-f`, `-uf` and `-fu`.
4. **Know what the flag means for this subcommand.** `-n` skips hooks for `git commit`. For `git push` it is `--dry-run`, and for
   `git merge` it is `--no-stat`.
5. **Anchor on a separator, not `^`,** when position matters: `(?:^|[;&|]\s*)rm\s`. A bare `^` sees only the first command of a
   chain.
6. **Express exceptions with `not`.** There is no look-around.
7. **Leave case alone.** `git branch -D` and `-d` differ. Use `(?i)` only for tools that ignore case.

A regex reads characters, not shell. It cannot tell a flag from the same text inside a quoted string, so write a passed case for
that, and accept the rare miss.

## Example

```json draft
{
  "kind": "conversation",
  "rule": {
    "id": "local:no-skipping-git-hooks",
    "mode": "steer",
    "match": {
      "any": [
        { "field": "input.command", "regex": "\\bgit\\s+commit\\b[^\\n;&|]*\\s-[A-Za-z]*n[A-Za-z]*\\b" },
        { "field": "input.command", "regex": "\\bgit\\s+(?:commit|push|merge)\\b[^\\n;&|]*\\s--no-verify(?:\\s|=|$)" }
      ]
    },
    "sample": { "source": "tool:*", "boundary": "tool-call" },
    "action": { "content": "Do not skip the git hooks: they run this repo's checks. Fix what the hook reports, or ask the person before skipping it." },
    "repeat": { "mode": "always" }
  },
  "cases": {
    "caught": [
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git commit --no-verify -m wip\"}", "input": "{\"command\":\"git commit --no-verify -m wip\"}" },
      { "source": "tool:bash", "boundary": "tool-call", "text": "{\"command\":\"cd app && git commit -nm wip\"}", "input": "{\"command\":\"cd app && git commit -nm wip\"}" }
    ],
    "passed": [
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git push -n origin main\"}", "input": "{\"command\":\"git push -n origin main\"}" },
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git merge --no-commit main | tail -n 3\"}", "input": "{\"command\":\"git merge --no-commit main | tail -n 3\"}" },
      { "source": "tool:Bash", "boundary": "tool-call", "text": "{\"command\":\"git commit -m 'no-verify removed'\"}", "input": "{\"command\":\"git commit -m 'no-verify removed'\"}" }
    ]
  }
}
```

## Rules about what is said

A conversation rule can also match what the person or the agent says, by similarity to a `reference` sentence. Its source is
`user-input` or `assistant-text`, it has a `match` of `{ "operator": "gte", "threshold": 0.8 }`, and it has no structural
condition. These cosine rules are in internal testing too. Similarity is loose: it fires on neighbouring phrasings and misses
the one that matters. Prefer a structural rule. When nothing structural fits, set such a rule to Log, and say "Not sure" in the
read-back.
