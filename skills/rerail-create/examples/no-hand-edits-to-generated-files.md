# Worked example: no hand edits to generated files

An OMP session made this rule for the Rerail repository with `/rerail-create` and no words, on 2026-10-15. It is kept here as
a worked example. Rerail's test suite runs the draft below on every run.

## Suggest

`rules evidence --days 60` read 178 sessions, 167 from OMP and 11 from Claude Code. The 3 corrections were one-off task
instructions, and the risky commands were routine: `rm -rf` of caches, and hard resets to a colleague's branch. So the agent built
the shortlist from REVIEW.md's merge blockers, checked against the code:

1. No branching on a language family outside the Engine Adapters (High)
2. **Don't hand-edit generated files (High).** REVIEW.md: "Hand edits to generated files … is a blocker"; CONTRIBUTING.md: "edit
   the generator, not its output".
3. Record a vendored grammar change in its PATCHES.md (Medium)

The agent also reported two rules that ship with Rerail and fight this repo. `n-no-sync` steered 13 times, while `src/` makes 134
sync calls in 36 files. `n-no-process-env` steered 9 times, while `src/` reads `process.env` 50 times. It offered the dashboard to
set both to Log, rather than changing them.

## Write

The person picked 2. The agent wrote a command rule over every agent's edit and write tools:

- `Edit`, `Write` and `MultiEdit` in Claude Code;
- `edit` and `write` in OMP and Pi, whose `edit` names the file as `[path#hash]` in `input.input`;
- `apply_patch` in Codex.

It covers the files `scripts/generate/README.md` lists as generated, with one exception: `n-no-process-exit.json` is the
hand-written file in `packs/node/rules/`. Running the generator, or reading a generated file, is not flagged. A hand edit made
through the shell (`sed -i`) is not caught; the read-back said so.

```json draft
{
  "kind": "conversation",
  "rule": {
    "id": "local:no-hand-edits-to-generated-files",
    "mode": "steer",
    "match": {
      "all": [
        {
          "field": "tool.name",
          "regex": "^(?:Edit|Write|MultiEdit|edit|write|apply_patch)$"
        },
        {
          "any": [
            {
              "all": [
                {
                  "field": "input.file_path",
                  "regex": "^(?:[^\\s\\[\\]#]*\\.generated\\.[^\\s\\[\\]#/]*|(?:[^\\s\\[\\]#]*/)?packs/node/(?:rules|candidates)/[^\\s\\[\\]#]+|(?:[^\\s\\[\\]#]*/)?packs/typescript/candidates/rules/vitest-(?:expect-expect|no-disabled-tests|no-focused-tests)\\.json|(?:[^\\s\\[\\]#]*/)?model/(?:model\\.safetensors|tokenizer\\.json|config\\.json|manifest\\.json)|(?:[^\\s\\[\\]#]*/)?native/src/code_rule_dsl/value/names/names\\.bin)$"
                },
                {
                  "not": {
                    "field": "input.file_path",
                    "regex": "packs/node/rules/n-no-process-exit\\.json$"
                  }
                }
              ]
            },
            {
              "all": [
                {
                  "field": "input.path",
                  "regex": "^(?:[^\\s\\[\\]#]*\\.generated\\.[^\\s\\[\\]#/]*|(?:[^\\s\\[\\]#]*/)?packs/node/(?:rules|candidates)/[^\\s\\[\\]#]+|(?:[^\\s\\[\\]#]*/)?packs/typescript/candidates/rules/vitest-(?:expect-expect|no-disabled-tests|no-focused-tests)\\.json|(?:[^\\s\\[\\]#]*/)?model/(?:model\\.safetensors|tokenizer\\.json|config\\.json|manifest\\.json)|(?:[^\\s\\[\\]#]*/)?native/src/code_rule_dsl/value/names/names\\.bin)$"
                },
                {
                  "not": {
                    "field": "input.path",
                    "regex": "packs/node/rules/n-no-process-exit\\.json$"
                  }
                }
              ]
            },
            {
              "field": "input.input",
              "regex": "(?m)^\\[(?:[^\\]\\n]*/)?(?:[^\\]\\n/]*\\.generated\\.[^\\]\\n#/]*|packs/node/(?:rules/n-no-process-exi[^t]|rules/n-no-process-[^e]|rules/n-no-[^p]|rules/n-[^n]|rules/[^n]|candidates/)[^\\]\\n]*|packs/typescript/candidates/rules/vitest-(?:expect-expect|no-disabled-tests|no-focused-tests)\\.json|model/(?:tokenizer|config|manifest)\\.json)#[0-9A-Fa-f]{4}\\]"
            },
            {
              "field": "input.input",
              "regex": "(?m)^\\*\\*\\* (?:Update|Add|Delete) File: (?:\\S*/)?(?:\\S*\\.generated\\.\\S*|packs/node/candidates/\\S+|packs/typescript/candidates/rules/vitest-(?:expect-expect|no-disabled-tests|no-focused-tests)\\.json|model/(?:tokenizer|config|manifest)\\.json)\\s*$"
            }
          ]
        }
      ]
    },
    "sample": {
      "source": "tool:*",
      "boundary": "tool-call"
    },
    "action": {
      "content": "This file is generated (scripts/generate/README.md lists it with its generator). Change the generator under scripts/generate/ and rerun the command the README gives, in the same change; do not edit the output by hand. If no generator change can produce what you need, ask the person."
    },
    "repeat": {
      "mode": "always"
    }
  },
  "cases": {
    "caught": [
      {
        "source": "tool:Edit",
        "boundary": "tool-call",
        "text": "{\"file_path\": \"/Users/robin/orca/workspaces/rerail/ner-1834-node-callback-literal/packs/node/rules/n-prefer-promises-fs.json\", \"new_string\": \"  \\\"revision\\\": 2,\\n\", \"old_string\": \"  \\\"revision\\\": 1,\\n\"}",
        "input": "{\"file_path\": \"/Users/robin/orca/workspaces/rerail/ner-1834-node-callback-literal/packs/node/rules/n-prefer-promises-fs.json\", \"new_string\": \"  \\\"revision\\\": 2,\\n\", \"old_string\": \"  \\\"revision\\\": 1,\\n\"}"
      },
      {
        "source": "tool:Edit",
        "boundary": "tool-call",
        "text": "{\"file_path\": \"/Users/robin/orca/workspaces/rerail/x/src/engine/storage/schema-sql.generated.ts\", \"old_string\": \"a\", \"new_string\": \"b\"}",
        "input": "{\"file_path\": \"/Users/robin/orca/workspaces/rerail/x/src/engine/storage/schema-sql.generated.ts\", \"old_string\": \"a\", \"new_string\": \"b\"}"
      },
      {
        "source": "tool:write",
        "boundary": "tool-call",
        "text": "{\"path\": \"native/src/code_rule_dsl/scope/data/es_globals.generated.json\", \"content\": \"{}\"}",
        "input": "{\"path\": \"native/src/code_rule_dsl/scope/data/es_globals.generated.json\", \"content\": \"{}\"}"
      },
      {
        "source": "tool:edit",
        "boundary": "tool-call",
        "text": "{\"input\": \"[packs/node/rules/n-no-sync.json#1A2B]\\nPUT 3.=3:\\n+  \\\"revision\\\": 4,\\n\"}",
        "input": "{\"input\": \"[packs/node/rules/n-no-sync.json#1A2B]\\nPUT 3.=3:\\n+  \\\"revision\\\": 4,\\n\"}"
      },
      {
        "source": "tool:Write",
        "boundary": "tool-call",
        "text": "{\"file_path\": \"packs/node/candidates/rules/n-no-deprecated-api.json\", \"content\": \"{}\"}",
        "input": "{\"file_path\": \"packs/node/candidates/rules/n-no-deprecated-api.json\", \"content\": \"{}\"}"
      }
    ],
    "passed": [
      {
        "source": "tool:Edit",
        "boundary": "tool-call",
        "text": "{\"file_path\": \"packs/node/rules/n-no-process-exit.json\", \"old_string\": \"a\", \"new_string\": \"b\"}",
        "input": "{\"file_path\": \"packs/node/rules/n-no-process-exit.json\", \"old_string\": \"a\", \"new_string\": \"b\"}"
      },
      {
        "source": "tool:edit",
        "boundary": "tool-call",
        "text": "{\"input\": \"[scripts/generate/README.md#1A2B]\\nPUT 40.=40:\\n+| `src/engine/storage/schema-sql.generated.ts` | x |\\n\"}",
        "input": "{\"input\": \"[scripts/generate/README.md#1A2B]\\nPUT 40.=40:\\n+| `src/engine/storage/schema-sql.generated.ts` | x |\\n\"}"
      },
      {
        "source": "tool:Edit",
        "boundary": "tool-call",
        "text": "{\"file_path\": \"scripts/generate/schema/schema-sql.ts\", \"old_string\": \"a\", \"new_string\": \"b\"}",
        "input": "{\"file_path\": \"scripts/generate/schema/schema-sql.ts\", \"old_string\": \"a\", \"new_string\": \"b\"}"
      },
      {
        "source": "tool:read",
        "boundary": "tool-call",
        "text": "{\"path\": \"src/engine/storage/schema-sql.generated.ts\"}",
        "input": "{\"path\": \"src/engine/storage/schema-sql.generated.ts\"}"
      },
      {
        "source": "tool:bash",
        "boundary": "tool-call",
        "text": "{\"command\": \"npm run generate:schema\"}",
        "input": "{\"command\": \"npm run generate:schema\"}"
      }
    ]
  }
}
```

## Proof: the incident it is for

PR #71 (NER-1834) was blocked in review because its first commit, `afb8e9abfd`, edited `packs/node/rules/*.json` by hand. Its
second commit, `1ac84f9eae`, moved the change into `scripts/generate/packs/node_policies.py` and regenerated. Each file the two
commits touched was replayed as the edit call an agent makes, through the same `ConversationRuleRegistry.match` the daemon runs:

| Tool | File | Commit | Rule | Right |
|---|---|---|---|---|
| `Edit` | .changeset/node-callback-literal-fs-watch.md | afb8e9abfd | silent | ✓ |
| `edit` | .changeset/node-callback-literal-fs-watch.md | afb8e9abfd | silent | ✓ |
| `Edit` | packs/node/README.md | afb8e9abfd | silent | ✓ |
| `edit` | packs/node/README.md | afb8e9abfd | silent | ✓ |
| `Edit` | packs/node/divergences.json | afb8e9abfd | silent | ✓ |
| `edit` | packs/node/divergences.json | afb8e9abfd | silent | ✓ |
| `Edit` | packs/node/rules/n-no-callback-literal.json | afb8e9abfd | **steered** | ✓ |
| `edit` | packs/node/rules/n-no-callback-literal.json | afb8e9abfd | **steered** | ✓ |
| `Edit` | packs/node/rules/n-prefer-promises-fs.json | afb8e9abfd | **steered** | ✓ |
| `edit` | packs/node/rules/n-prefer-promises-fs.json | afb8e9abfd | **steered** | ✓ |
| `Edit` | test/engine/code-rule/node-callback-literal-fs-watch.test.ts | afb8e9abfd | silent | ✓ |
| `edit` | test/engine/code-rule/node-callback-literal-fs-watch.test.ts | afb8e9abfd | silent | ✓ |
| `Edit` | scripts/generate/packs/node_policies.py | 1ac84f9eae | silent | ✓ |
| `edit` | scripts/generate/packs/node_policies.py | 1ac84f9eae | silent | ✓ |
| `bash` | regenerate | 1ac84f9eae | silent | ✓ |

Both hand edits of generated rules are steered, in Claude Code's and OMP's edit formats. Every other file in the commit is left
alone, and so is the fix through the generator.

**No false positives in history.** Replayed over the tool calls Rerail recorded for this repository (7,638 calls, 874 of them
edits or writes, from OMP and Claude Code), the rule fired 0 times. Replayed over the Claude Code transcripts of 12 Rerail
worktrees (1,425 tool calls), it also fired 0 times. No agent hand-edited a generated file in those sessions, and the rule fired on
nothing else.
