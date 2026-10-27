# Rerail

Rerail checks the code your coding agent writes against your rules while it works, and steers it back in the same session.

[![CI](https://github.com/neralabs/rerail/actions/workflows/ci.yml/badge.svg)](https://github.com/neralabs/rerail/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

[Docs](docs/README.md) · [Changelog](CHANGELOG.md)

## What is Rerail

Coding agents drift. They cross architectural seams, repeat approaches you already rejected, and ignore repository conventions they never saw. Writing more rules into the prompt doesn't fix this: it uses up context, goes stale, and still relies on the model noticing when a rule applies.

Rerail takes enforcement out of the model loop. Hooks or extensions in your coding agent send each edit the agent makes to a local daemon. A native Rust engine checks the changed code against your repository's rules, by regex, syntax tree and relations. When the agent breaks a rule, Rerail steers it back during the session and records what fired, where and why.

Rule checking runs on your machine and makes no network requests. No account or API key is needed.

## Supported coding agents

Rerail works with five coding agents: Claude Code, Codex, Cursor, OMP (Oh My Pi) and Pi. Set-up connects the ones it finds, or the ones you name (`--agents claude-code,codex,cursor,omp,pi`). It backs up each config before changing it, and `rerail install --remove` takes everything out again.

| Agent | Integration | In-session commands | Status line |
| --- | --- | --- | --- |
| Claude Code | `.claude-plugin`, `hooks/claude.json`; commands in `~/.claude/commands/` | `/rerail-*` | Yes |
| Codex | `.codex-plugin`, `hooks/codex.json`; skills in `~/.codex/skills/` | `$rerail-*` skills | No |
| Cursor | `.cursor-plugin`, `hooks/cursor.json`; commands in `~/.cursor/commands/` | `/rerail-*` | No |
| OMP, Pi | Native extensions (`src/hosts/omp`, `src/hosts/pi`) | `/rerail-*` | Yes |

Every agent gets the same commands:

| Command | What it does |
| --- | --- |
| `/rerail` | What Rerail is doing in this repository, and every command (`/rerail-help` is the same) |
| `/rerail-start`, `/rerail-stop` | Start or stop Rerail in this checkout and the worktrees inside it; rules and history are kept |
| `/rerail-create` | Create a rule in your own words |
| `/rerail-show-me` | See Rerail catch a broken rule in this repository and steer the fix, then clean up |
| `/rerail-manage` | Open the dashboard |
| `/rerail-update` | Update Rerail to the newest version |
| `/rerail-uninstall` | Remove Rerail from this computer; rules and history are kept in a backup |

Everything else is a terminal command: `rerail status`, `rerail version`, `rerail update check`, `rerail update rollback`, `rerail reset`. Run `rerail --help` for the full list. In Claude Code, OMP and Pi the status line shows `R E R A I L ● <n> rules`, and how many times Rerail steered, while Rerail is on; see [In-session surfaces](docs/in-session-surfaces.md).

## Quickstart

You need Node.js 24 or later on macOS, Linux (glibc) or Windows, on arm64 or x64. Prebuilt native engines ship in the package, so nothing is compiled on your machine.

```sh
cd your-repo
npx @neralabs/rerail setup   # install under ~/.rerail, connect your coding agents, start Rerail here
rerail manage                # open the dashboard (a new shell picks up the rerail launcher)
```

Set-up asks before it reads the repository, shows the rule packs that fit it with the reason for each, and installs only the ones you confirm. It connects the coding agents it finds (or only the ones you name with `--agents`), puts `~/.rerail/bin` on your `PATH`, and starts Rerail in this repository only. Everywhere else Rerail stays off until you run `/rerail-start` (or `rerail start`) there. To have a coding agent install it for you, ask it to; it runs `setup --relay` and asks you each question in the session. [Installing Rerail](docs/installation.md) lists everything set-up writes and how to remove it.

Then start a new Claude Code, Codex, Cursor, OMP or Pi session in the repository. To watch Rerail catch a broken rule and steer the fix, run:

```text
/rerail-show-me
```

To add a rule of your own, type `/rerail-create` followed by what you want:

```text
/rerail-create <what you want>
/rerail-create never import from src/legacy in new code
```

The agent drafts the rule from your description, tests it against the repository without saving it, and adds it after you confirm. [Writing rules](docs/writing-rules.md) covers which rules are worth having.

## How it works

```mermaid
flowchart LR
  agent([Coding agent]) -- edit --> engine[Rerail engine]
  rules[(Repository rules)] --> engine
  engine -. steer .-> agent
  engine --> history[(Sessions and dashboard)]
```

1. **Capture.** Hooks (Claude Code, Codex, Cursor) or native extensions (OMP, Pi) observe the session automatically, so the agent never has to remember to call Rerail. Before an edit, Rerail records the file as it was, so it can tell which lines the agent changed.
2. **Evaluate.** One background daemon keeps the engine and every repository's compiled rules in memory, checks the files the agent touched, and reports only the lines it changed.
3. **Steer.** Each rule has a mode. `log` records the violation. `steer` sends the agent the rule, the location, what is wrong and, where the rule has one, a suggested fix.
4. **Record.** Sessions and violations are stored in one local SQLite database (`~/.rerail/rerail.sqlite`) and shown in the dashboard, so you can see what fired, where and why.

![An OMP session: the agent writes isGuest(user) with user == null, Rerail steers it on the eslint-eqeqeq rule, and the agent rewrites the check with === null and === undefined and says why. Below, the Rerail footer reads "R E R A I L ● 25 rules · steered 1×"; each part links to Rerail's dashboard.](docs/images/session-steer.png)

A real session in OMP: Rerail steers the agent on `eslint-eqeqeq` from the `javascript` pack, the agent fixes its edit in the same turn, and the footer counts the steer. `R E R A I L`, the rule count and the steer count link to the session and the rules on your local dashboard.

## Capabilities

- **Code rules.** Rules are `rerail.policy/v1` documents. Each clause is `forbid`, `require_each` or `budget`, built from regex, syntax-tree and relational queries. The native engine runs them the same way every time, within set limits on files, nodes and time. The [code-rule DSL reference](docs/code-rule-dsl-reference.html) documents every construct.
- **Only the agent's new work is judged.** Code rules report only the lines the agent changed in the session, so existing code does not bury the signal.
- **Runtime steering.** Violations go back into the live session while the change is still being made, not later in review.
- **Rules belong to the repository.** Every clone and worktree of a remote shares its rules. Turn a rule or pack on, off or to log-only per repository, folder or checkout, and choose whether linters' suppression comments (`eslint-disable`, `noqa`, ...) silence it; changes apply on the next tool call. See [Rule scoping](docs/rule-scoping.md).
- **Rules in your own words.** `/rerail-create` has your coding agent draft a rule, test it with `rerail rules test`, and add it with `rerail rules add` after you confirm. `rerail rules evidence` shows what recent sessions suggest a rule for.
- **Local and multi-agent.** One daemon serves Claude Code, Codex, Cursor, OMP and Pi, with a dashboard on your own machine.
- **Explicit updates.** Nothing updates on its own. `rerail update` backs up the database and validates the new version before switching to it; `rerail update rollback` goes back.

## Rule packs

Core ships no rules. A rule pack is a set of rules published as its own npm package, `@neralabs/rerail-pack-<id>`. Set-up recommends the packs that fit a repository, by its files or its dependencies, and installs the ones you confirm into `~/.rerail/packs`. A pack runs only where you add it. Manage packs from the dashboard's **Rule packs** page or with `rerail packs list|add|remove|recommend`.

| Pack | Rules (on when added) | Ported from |
| --- | --- | --- |
| `python` | 136 (112) | Ruff 0.16.9, Bandit 1.9.4 |
| `javascript` | 71 (24) | ESLint 10.11.0, eslint-plugin-unicorn 76.0.0 |
| `typescript` | 24 (13) | typescript-eslint 8.70.1, eslint-plugin-vitest 1.6.27 |
| `next` | 21 (21) | @next/eslint-plugin-next 16.3.8 |
| `sql` | 19 (17) | SQLFluff 4.4.0 |
| `vue` | 19 (18) | eslint-plugin-vue 10.11.1 |
| `rust` | 18 (13) | Clippy (Rust 1.99.0) |
| `blade` | 10 (8) | Bladestan, tlint, stillat/blade-parser |
| `php` | 7 (3) | PHP_CodeSniffer 4.0.4, PHPStan 2.2.16, Psalm 6.19.1 |
| `complexity` | 3 (3) | lizard 1.24.0: cyclomatic complexity above 6, for Python, TypeScript and JavaScript |
| `node` | none yet | eslint-plugin-n 18.4.1 (24 candidates) |
| `react` | none yet | eslint-plugin-react 7.37.5, react-hooks (21 candidates) |

Every shipped ported rule is scored against the original tool's recorded findings and must pass the [port readiness gates](docs/port-readiness.md). Ports that do not pass yet sit in each pack's `candidates/`, scored but never shipped. Each pack is versioned with changesets and released separately from core. See [Rule packs](docs/rule-packs.md) and the [pack format](docs/pack-format.md).

Rules can target Python, TypeScript, JavaScript, PHP, Blade, Rust, SQL and Vue; regex clauses work on any file. Move your own rules between machines with `rerail rules export` and `rerail rules import`.

## Internal testing

> [!WARNING]
> **These features are in the code and you can find and run them, but they are not ready for public use.** We are
> still testing them internally. They are not supported, they may change or be removed in any release without notice,
> and nothing in this README promises they work. Do not build on them or rely on them in your workflow.

Each is tagged with an `Internal testing:` comment where it is defined, and every description a person or a coding agent reads for one (CLI usage, MCP tools, OMP and Pi tools and commands) starts with `[Internal testing]`, through `internalTesting()` in `src/engine/internal-testing.ts`.

| Feature | Where it surfaces |
| --- | --- |
| Full-repository scanning | `rerail scan`; the `audit_requests_create`, `audit_requests_list`, `repository_audits_list` and `repository_audit_entries_list` operations; the automatic first repository audit at session start |
| Conversation rules, cosine (similarity) and command (tool-call) alike | `kind: "conversation"` rules, shown in the dashboard with an `Internal testing` badge; `/cosine-rules` and the `conversation_rules` tool in OMP and Pi; `/rerail-create`'s command rules ([Tool-call rules](docs/tool-call-rules.md)) |
| Semantic tools | `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate` in OMP and Pi |
| Block mode | `mode: "block"`, stored and delivered exactly like `steer`; the dashboard shows it as not in this version |
| Activation profiles | `rerail activate`, `rerail activation request`, `rerail profile` and `--profile` |
| Event stream | `rerail events` |
| Steer strategies `rewind` and `subagent`, experimental: off unless `RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1` | `steerStrategy` in the audit settings (`rerail settings --steer-strategy`, `audit_settings_update`); Pi's `rerail_fix` tool; `benchmark/steer-strategy.ts` |

`rerail settings` and the `audit_settings_*` operations are not tagged yet: their scope is still being decided. The daemon's HTTP API, OpenAPI document and MCP endpoint are described in the [reference](docs/reference.md#run); the dashboard uses them internally. Found something in this list? Feedback is welcome in an issue, labelled as internal testing.

## Repository structure

```text
.
├── src/              # TypeScript, one folder per layer (see src/README.md)
│   ├── engine/       # rule model, schemas, code-rule compile, native bridge, audits, database, pack catalog
│   ├── runtime/      # session events through the engine; the daemon client
│   ├── api/          # HTTP and MCP operations served by the daemon
│   ├── presentation/ # the session footer, dashboard links, terminal status
│   ├── daemon/       # the resident daemon: HTTP/MCP server, page, native session server
│   ├── harness/      # shared hook plumbing, event stream, /rerail commands
│   ├── hosts/        # one adapter per agent: claude, codex, cursor, omp, pi
│   ├── setup/        # agent registration and the managed updater
│   ├── activation/   # signed customer profiles and device keys
│   └── cli/          # the rerail command line
├── native/           # Rust engine (napi): embeddings, code-rule DSL, content store, git workspace
├── languages/        # language registry: one JSON file per language
├── packs/            # rule packs, one npm package each
├── model/            # bundled, pinned potion-base-8M model (Git LFS)
├── hooks/            # hook manifests for Claude Code, Codex and Cursor
├── skills/           # agent skills shipped with Rerail (rerail-create)
├── .claude-plugin/ .codex-plugin/ .cursor-plugin/  # agent plugin manifests
├── ui/               # dashboard: plain HTML, CSS and ES modules, no build step
├── conformance/      # scores ported rules against upstream tools' recorded output
├── fixtures/         # upstream test fixtures and corpora per language
├── benchmark/        # query parity and steer benchmarks
├── playground/       # tool for developing rules; never published
├── test/             # unit tests (Vitest), mirroring src/
├── integration/      # end-to-end tests against real agent hosts
├── e2e/              # dashboard tests (Playwright)
├── scripts/          # build, check, generate, release
├── docs/             # guides, reference and design notes
└── CONTEXT.md        # domain glossary: rules, clauses, violations, oracles
```

### Good places to start

- [CONTEXT.md](CONTEXT.md): the vocabulary used across the codebase. Read it first.
- [docs/installation.md](docs/installation.md): what set-up writes to your machine and how to remove it.
- [docs/reference.md](docs/reference.md): rules and their configuration, the database, updates, the daemon and its API.
- [docs/code-rule-dsl-reference.html](docs/code-rule-dsl-reference.html): every construct of the `rerail.policy/v1` rule language.
- [docs/rule-packs.md](docs/rule-packs.md) and [docs/pack-format.md](docs/pack-format.md): how packs work and what each must prove.
- [packs/complexity/rules/](packs/complexity/rules/): real rules to copy when writing your own.
- [src/README.md](src/README.md): the layers and which may import which.
- [docs/vision.md](docs/vision.md): where Rerail is going beyond code rules, and the principles behind it.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to start work and what a pull request needs. Contributions are licensed under Apache-2.0, with no CLA, and third-party material must use a permissive licence and be recorded in `UPSTREAM.md` and `REUSE.toml`. How core is released is in [docs/release.md](docs/release.md).

To build from a checkout you need Node.js 24+, npm, Rust (pinned in `rust-toolchain.toml`), [`just`](https://github.com/casey/just) and [Git LFS](https://git-lfs.com). The model weights and an engine data table are stored in LFS, so install it before you clone.

```sh
git lfs install
npm ci && npm run build
npm run check:all
npm run test:all
```

## Feedback

Found a false positive, a missed violation or a confusing steer? [Open an issue](https://github.com/neralabs/rerail/issues/new/choose) with the rule id and the session link. To report a vulnerability, follow [SECURITY.md](SECURITY.md) instead.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Third-party material bundled with Rerail, including the `minishlab/potion-base-8M` model (MIT), the Blade parser ported from Forte 1.1.0 (MIT), the vendored grammars and the ported rules, is listed with its licence in [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES) and `LICENSES/`.
