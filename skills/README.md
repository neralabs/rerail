# Skills

Instructions a coding agent follows for a Rerail command. They ship in the package (`files` in `package.json`).
Each command file points the agent here, with the installed package's path.

| Skill | Command | What it does |
|---|---|---|
| [rerail-create](rerail-create/SKILL.md) | `/rerail-create` (`$rerail-create` in Codex) | Suggests rules from the repo's sessions and standards. Writes, tests and tunes a rule, and adds it only after the person says yes. |

A skill is Markdown for the agent: `SKILL.md`, plus `references/` that it reads when a step names one, and `examples/` (for
`rerail-create`, [a rule an OMP session made for this repository](rerail-create/examples/no-hand-edits-to-generated-files.md),
with the replay that proves it). It holds no rule
documents (`docs/repository-structure.md`, metrics 2 and 3). Every rule draft in a skill is a fenced `json draft` block, and
`test/cli/rule-writing-skill.test.ts` runs each one through the engine.
