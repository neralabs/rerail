# Internal testing

> [!WARNING]
> **These features are in the code and you can find and run them, but they are not ready for public use.** We are
> still testing them internally. They are not supported, they may change or be removed in any release without notice,
> and nothing in Rerail's documentation promises they work. Do not build on them or rely on them in your workflow.

Each is tagged with an `Internal testing:` comment where it is defined, and every description a person or a coding agent reads for one (CLI usage, MCP tools, OMP and Pi tools and commands) starts with `[Internal testing]`, through `internalTesting()` in `src/engine/internal-testing.ts`.

| Feature | Where it surfaces |
| --- | --- |
| Full-repository scanning | `rerail scan`; the `audit_requests_create`, `audit_requests_list`, `repository_audits_list` and `repository_audit_entries_list` operations; the automatic first repository audit at session start |
| Conversation rules, cosine (similarity) and command (tool-call) alike | `kind: "conversation"` rules, shown in the dashboard with an `Internal testing` badge; `/cosine-rules` and the `conversation_rules` tool in OMP and Pi; `/rerail-create`'s command rules ([Tool-call rules](tool-call-rules.md)) |
| Semantic tools | `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate` in OMP and Pi |
| Block mode | `mode: "block"`, stored and delivered exactly like `steer`; the dashboard shows it as not in this version |
| Activation profiles | `rerail activate`, `rerail activation request`, `rerail profile` and `--profile` |
| Event stream | `rerail events` |
| Steer strategies `rewind` and `subagent`, experimental: off unless `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` | `steerStrategy` in the audit settings (`rerail settings --steer-strategy`, `audit_settings_update`); Pi's `rerail_fix` tool; `benchmark/steer-strategy.ts` |

`rerail settings` and the `audit_settings_*` operations are not tagged yet: their scope is still being decided. The daemon's HTTP API, OpenAPI document and MCP endpoint are described in the [reference](reference.md#run); the dashboard uses them internally. Found something in this list? Feedback is welcome in an issue, labelled as internal testing.
