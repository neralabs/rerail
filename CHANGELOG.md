# Changelog

## 0.1.0 (unreleased)

First release of Rerail as `@neralabs/rerail`, licensed under Apache-2.0.

### Semantic monitor

- Native Model2Vec embeddings and cosine matching against resident compiled rule references, using the bundled, pinned INT8 `minishlab/potion-base-8M` model; runtime matching makes no network requests.
- Cosine rules over `assistant-text`, `user-input`, and `tool:<name>` sources at sentence, message, turn, and tool-call boundaries, with `once`, `always`, and `after-gap` repeat policies and `observe`, `steer`, `followUp`, and `nextTurn` delivery.
- Structural tool-call rules: a cosine-kind rule's `match` may instead be a condition in the policy DSL's vocabulary (`regex`, `eq`, `contains` combined with `all`/`any`/`not`) over `text`, `tool.name`, `input`, or one string argument such as `input.command`. Structural rules never embed, and the kernel validates their regexes when a rule is written.
- Generic `mark`, `select`, `project`, and `relate` primitives for custom semantic workflows, exposed in OMP and Pi as the `rerail_mark`, `rerail_select`, `rerail_project`, and `rerail_relate` tools.

### Executable policies

- The `rerail.policy/v1` policy DSL evaluated by a resident native kernel over Python, JavaScript, TypeScript, PHP, and Blade sources: regex, AST (ast-grep), and relational checks with ordered findings, witnesses, optional fixes, and bounded source, node, and witness budgets.
- AST candidate selection through predicate-aware positional indexes that preserve matcher semantics and incremental reuse.
- Three evaluation scopes (`repository`, `touched-files`, `ai-output`) and two independent modes (`resteer`, `log`). AI-output scope reports only changed post-image ranges, using pre-edit source captured at the host's tool-call boundary; there are no finding baselines or acknowledgements.
- Every evaluation returns findings, concrete execution failures, and per-check `coverage` (`complete`, `partial`, or `failed` with reasons).
- One automatic log-only full-repository diagnostic per repository, explicit `repository` and `git-diff` scans, and bounded full-repository budgets (five minutes, 100,000 files or 512 MiB of source, 20 reported failures).
- Repository scans of Git checkouts use the native repository index and a machine-wide content-addressed store (`~/.cache/rerail/`) to restore clean files from previous results; checkout builds keep a private store under `<checkout>/.rerail/`. At most one main instance — the managed installation's daemon — opens the machine-wide store, guarded by an OS lock.
- An opt-in rule pack of TSLint rule ports (`policies/executable-policy-dsl/rules/opt-in/tslint/`), seeded disabled, with the conformance harness in `conformance/tslint/` that scores each port against TSLint's own test fixtures.

### Rule catalog and database

- One SQLite database per Rerail home (`$RERAIL_HOME/rerail.sqlite`) holds the scoped rule catalog and every repository's sessions, events, work links, policy runs, scan requests, and diagnostics; the schema is defined as Drizzle tables.
- Rules apply globally or to a project, an explicit set of repositories identified by a stable repository ID shared across worktrees; edits made through the daemon, the UI, MCP, or plain SQL apply to the next sample or tool call in every open session.
- Bundled rule documents are seeded without overwriting rules you edited or created. `rerail rules export` / `rerail rules import` move rules by hand; `rerail reset` deletes the current repository's history and keeps rules; `rerail seed` writes mock data.
- JSON `cosine-rules.json` files per harness remain a secondary source; a database row with the same id wins.

### Daemon, API, and UI

- `rerail start`, `stop`, `status`, `ui`, and `session`: one background daemon that owns the resident kernel, serves the rules UI, an HTTP API under `/api`, its OpenAPI 3.1 document, and a stateless MCP Streamable HTTP endpoint at `/mcp`, all generated from one operation registry.
- Loopback-only by default; DNS-rebinding protection through `Host` and `Origin` checks, `--allow-remote` for any other bind address, and framing protection on the served page.
- A static rules UI (plain HTML, CSS, and ES modules, no toolchain) with Rules, Sessions, Settings, and Diagnostics pages, conditional GETs, and paginated, server-filtered diagnostics.

### Harness integration

- OMP and Pi extensions, and Claude Code, Codex, and Cursor lifecycle hooks, all sharing one transport-neutral event runtime; command hooks reach the daemon's resident kernel over an owner-only Unix socket or Windows named pipe, and the daemon autostarts on first use (`RERAIL_AUTOSTART=0` opts out).
- Per-session event lanes, tool-call correlation by native call ID, preimages and pending calls that survive daemon restarts, and shell-write discovery at turn checkpoints.
- The persistent `RERAIL-EVENTS/1` stream (`rerail events`) for out-of-process harnesses.
- Slash commands: `/rerail-settings`, `/rerail-scan`, `/rerail-disable`, `/rerail-enable`, `/rerail-ui`, `/rerail-session`, `/rerail-status`, `/rerail-check`, `/rerail-update`, `/rerail-rollback`, and `/rerail-reset`; `/cosine-rules` and the `cosine_rules` tool in OMP and Pi. A checkout-local `.rerail/disabled` marker (`rerail disable` / `rerail enable`) switches Rerail off for one checkout.
- In-session presence: an OMP and Claude Code status line with the session's rule count and steering count linking to the session page, and a Codex `rerail` skill for session detail.

### Installation and distribution

- `npx @neralabs/rerail setup` installs a managed version under `~/.rerail/versions`, writes the `rerail` launcher, registers every detected harness with backed-up, idempotent edits, and starts the daemon; `rerail install --remove` takes the registrations out again.
- Explicit updates only: `rerail update` resolves an npm dist-tag (`latest`, `beta`, `next`), validates the native kernel and a real event-stream round-trip before activating, copies the database to `rerail.sqlite.pre-update` first, and keeps the previous version for `rerail update rollback`.
- Signed customer activation profiles (`rerail activate`, online or offline) with device keys in macOS Keychain, Linux Secret Service, or Windows Credential Manager.
- Prebuilt native kernels for macOS, Linux glibc, and Windows MSVC on arm64 and x64; the Windows kernels link the C runtime statically.
