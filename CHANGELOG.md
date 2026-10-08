# Changelog

## 0.1.0 (unreleased)

First release of Rerail as `@neralabs/rerail`, licensed under Apache-2.0.

### Rules

- Two rule kinds: `file` rules, `rerail.policy/v1` documents whose clauses each report a violation, and `conversation` rules, which match the live conversation semantically or structurally.
- One required `mode` on every rule: `log` records only, `steer` delivers guidance, and `block` is stored, shown, and carried on violations but delivered exactly like `steer` until block enforcement lands; no shipped rule uses `block`. The rules and diagnostics filters take `mode=log|steer|block`.

### Semantic monitor

- Native Model2Vec embeddings and cosine matching against resident compiled rule references, using the bundled, pinned INT8 `minishlab/potion-base-8M` model; runtime matching makes no network requests.
- Conversation rules over `assistant-text`, `user-input`, and `tool:<name>` sources at sentence, message, turn, and tool-call boundaries, with `once`, `always`, and `after-gap` repeat policies.
- Structural tool-call rules: a conversation rule's `match` may instead be a condition in the code-rule DSL's vocabulary (`regex`, `eq`, `contains` combined with `all`/`any`/`not`) over `text`, `tool.name`, `input`, or one string argument such as `input.command`. Structural rules never embed, and the engine validates their regexes when a rule is written.
- Generic `mark`, `select`, `project`, and `relate` primitives for custom semantic workflows, exposed in OMP and Pi as the `rerail_mark`, `rerail_select`, `rerail_project`, and `rerail_relate` tools.

### File rules

- The `rerail.policy/v1` code-rule DSL evaluated by a resident native engine over Python, JavaScript, TypeScript, PHP, and Blade sources: regex, AST (ast-grep), and relational clauses with ordered violations, witnesses, optional fixes, and bounded source, node, and witness budgets.
- Python identifiers are NFKC-normalized wherever the engine resolves or compares them, as CPython and Ruff read them; string contents, spans, and `regex` keep the source text.
- AST candidate selection through predicate-aware positional indexes that preserve matcher semantics and incremental reuse. Relations read only the nodes they relate to the roots already selected, remember value tests per node, and narrow `precedes`/`follows` comparisons to the siblings that can match, so nested relations and duplicate-key checks cost linear rather than quadratic work.
- Three audit scopes (`repository`, `touched-files`, `ai-output`) and two run modes (`steer`, `log`). AI-output scope reports only changed post-image ranges, using pre-edit source captured at the host's tool-call boundary; there are no violation baselines or acknowledgements.
- Every audit returns violations, concrete execution failures, and per-clause `coverage` (`complete`, `partial`, or `failed` with reasons).
- One automatic log-only full-repository audit per repository, explicit `repository` and `git-diff` scans, and bounded full-repository budgets (five minutes, 100,000 files or 512 MiB of source, 20 reported failures).
- Repository scans of Git checkouts use the native repository index and a machine-wide content-addressed store (`~/.cache/rerail/`) to restore clean files from previous results; checkout builds keep a private store under `<checkout>/.rerail/`. At most one main instance — the managed installation's daemon — opens the machine-wide store, guarded by an OS lock. Edits after evicting inactive files update the call graph in place instead of linking it again.
- One language registry, `languages/<id>.json` (`rerail.language/v1`, [language packs](docs/language-packs.md)): each language's family, dialects (file suffixes, parser, grammar, tape tag), per-capability support, suppressions, embedded languages and status. The engine generates its language tables from it at build time and the host reads a module generated from it. `rust`, `sql` and `vue` are `planned`: a code rule may name one and is stored, but compiling it fails with `language-not-supported`, the catalog keeps it inactive (`inactive` in the rule and activity APIs, never an audit failure), and audits route no file to it.
- An opt-in rule pack of TSLint rule ports (`packs/tslint/`, shipped inside the core package), seeded disabled, with the conformance harness in `conformance/tslint/` that scores each port against TSLint's own test fixtures.
- A language-pack conformance harness in `conformance/languages/` (`npm run conformance:languages -- --pack <pack>`): ported upstream linter rules are scored TP/FP/FN through the production workspace against the upstream rule fixtures and the linter's output on pinned corpora, with reports under the common `rerail.language-conformance/v1` header.

### Rule packs

- A rule pack (`packs/<id>/pack.json`) lists its rule documents, with a title and a group for each rule and a match definition (file globs or `package.json` dependencies) that explains which repositories it fits. One pack ships inside core: TSLint, the 74 golden TSLint ports.
- `recommendRulePacks` matches a repository's files and manifests against each pack locally, with no model and no network, and returns each fitting pack with its rule count and the reason it matched. Read-only daemon operations and MCP tools `rule_packs_list` and `rule_packs_recommend`; listed rules carry `packId`. Packs are not attached to repositories yet. See `docs/rule-packs.md`.
- Packs in their own packages under `packs/<id>/` ([pack format](docs/pack-format.md)): `pack.json` names each rule's `document`, and `package.json` declares core as a peer. `test/packs.test.ts` holds every pack to the validation contract: its rules are listed, match their documents and compile; each is proven by examples or a port; examples are caught or pass through the engine; ports agree with the tools' recordings outside declared divergences; the package publishes no validation material and its licence agrees with `licenses/`; every copied file is recorded. A rule for a language core does not read yet is reported `not supported yet` and does not fail. The Python pack (22 Ruff and Bandit ports) is the first to move there; it is not published yet.

### Rule catalog and database

- One SQLite database per Rerail home (`$RERAIL_HOME/rerail.sqlite`) holds the scoped rule catalog and every repository's sessions, events, work links, audits, audit requests, and start state; the schema is defined as Drizzle tables.
- Rules apply globally or to a project, an explicit set of repositories identified by a stable repository ID shared across worktrees; edits made through the daemon, the UI, MCP, or plain SQL apply to the next sample or tool call in every open session. Catalog origins are `user` and `rerail`.
- Bundled rule documents are seeded without overwriting rules you edited or created. `rerail rules export` / `rerail rules import` move rules by hand; `rerail reset` deletes the current repository's history and keeps rules; `rerail seed` writes mock data.
- JSON `.<harness>/conversation-rules.json` files remain a secondary source; a database row with the same id wins.

### Daemon, API, and UI

- `rerail daemon start`, `rerail daemon stop`, `status`, `ui`, and `session`: one background daemon that owns the resident engine, serves the rules UI, an HTTP API under `/api`, its OpenAPI 3.1 document, and a stateless MCP Streamable HTTP endpoint at `/mcp`, all generated from one operation registry.
- Audits, violations, repository audits, audit settings, and audit requests are exposed as `audits_list`, `violations_list`, `repository_audits_list`, `repository_audit_entries_list`, `audit_settings_get`/`audit_settings_update`, and `audit_requests_create`/`audit_requests_list`.
- Loopback-only by default; DNS-rebinding protection through `Host` and `Origin` checks, `--allow-remote` for any other bind address, and framing protection on the served page.
- A static rules UI (plain HTML, CSS, and ES modules, no toolchain) with Rules, Sessions, Settings, and Diagnostics pages, conditional GETs, and paginated, server-filtered diagnostics.

### Harness integration

- OMP and Pi extensions, and Claude Code, Codex, and Cursor lifecycle hooks, all sharing one transport-neutral event runtime; command hooks reach the daemon's resident engine over an owner-only Unix socket or Windows named pipe, and the daemon autostarts on first use (`RERAIL_AUTOSTART=0` opts out).
- Rerail is off in every repository until it is started there. `/rerail-start` and `/rerail-stop` (OMP, Pi, Claude Code, Cursor) or `rerail start` / `rerail stop` (terminal and Codex) turn it on or off in the current checkout and the worktrees that follow it. Set-up and rules belong to the repository; on and off belong to each checkout. Starting a repository that was never set up also seeds the default rules.
- Per-session event lanes, tool-call correlation by native call ID, preimages and pending calls that survive daemon restarts, and shell-write discovery at turn checkpoints.
- The persistent `RERAIL-EVENTS/1` stream (`rerail events`) for out-of-process harnesses.
- Slash commands: `/rerail-start`, `/rerail-stop`, `/rerail-settings`, `/rerail-scan`, `/rerail-ui`, `/rerail-session`, `/rerail-status`, `/rerail-check`, `/rerail-update`, `/rerail-rollback`, and `/rerail-reset`; `/cosine-rules` and the `conversation_rules` tool in OMP and Pi.
- In-session presence: an OMP and Claude Code status line with the session's rule count and steering count linking to the session page, and a Codex `rerail` skill for session detail.

### Installation and distribution

- `npx @neralabs/rerail setup` installs a managed version under `~/.rerail/versions`, writes the `rerail` launcher, registers every detected harness with backed-up, idempotent edits, and starts the daemon; `rerail install --remove` takes the registrations out again, stops the daemon, and moves `rerail.sqlite` into a dated backup.
- Explicit updates only: `rerail update` resolves an npm dist-tag (`latest`, `beta`, `next`), validates the native engine and a real event-stream round-trip before activating, copies the database to `rerail.sqlite.pre-update` first, and keeps the previous version for `rerail update rollback`.
- Signed customer activation profiles (`rerail activate`, online or offline) with device keys in macOS Keychain, Linux Secret Service, or Windows Credential Manager.
- Prebuilt native engines for macOS, Linux glibc, and Windows MSVC on arm64 and x64; the Windows engines link the C runtime statically.
