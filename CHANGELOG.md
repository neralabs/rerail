# Changelog

Every notable change to Rerail's core package, `@neralabs/rerail`, latest first. From 0.1.1 the notes follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/): each release lists its changes under Added, Changed, Deprecated, Removed, Fixed and Security. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are in [.changelog](.changelog/README.md), so `## Unreleased` holds only that release's opening paragraph.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. A release is a patch unless it names a later version, and a patch can change or remove what you use, so read the notes before you update.

## Unreleased

Rerail and its rule packs are public on npm, so installing Rerail, and adding a pack, needs no npm account. The daemon now answers only a request that carries its access capability, so another user of your machine, or a sandboxed process allowed localhost, gets nothing from it: open the dashboard from `rerail manage`, since an old bookmark no longer gets in. A coding agent's Rerail tools reach only the repository it works in, and a file named like one holding secrets reaches it by rule and place only. The dashboard says less, by rule. A daemon reachable from other machines now serves HTTPS with `--tls-cert` and `--tls-key`, or needs `--insecure-http` to stay on plain HTTP, so check that before updating a remote set-up. Block is still not in this release.

## 0.1.0 (2026-10-27)

First release of Rerail, licensed under Apache-2.0. Rerail keeps coding agents on your rules, with checks that live outside the agent: it audits what Claude Code, Codex, Cursor, OMP or Pi writes, steers the agent in the same session, and shows every session, rule and violation in a dashboard.

To start, run this in a repository:

```bash
npx @neralabs/rerail setup
```

### Rules

- Code rules check what an agent writes in Python, JavaScript, TypeScript, PHP, Blade, SQL, Rust and Vue. A native engine runs them on your computer.
- Each rule either logs a violation or steers the agent to fix it. Turn a rule or pack on or off, or change its mode, for a repository, a checkout or a folder.
- An edit hears only about the lines it changed, not problems already in the file.
- A file or rule Rerail could not fully check is reported as not checked, never as passing.
- Comments that silence a linter also silence the rules ported from it, and `rerail-ignore` silences any rule by its id.

### Rule packs

- Rules come in packs, each its own npm package. Set-up recommends the ones that fit your repository, says why, and installs nothing until you answer.
- Packs with rules in this release: complexity, JavaScript, TypeScript, Next.js, Python, PHP, Blade, SQL, Rust and Vue.
- Set-up reads your ESLint, Oxlint and Ruff configs without running them, and offers to start the rules they turn off as off.
- Manage packs with `rerail packs list`, `add`, `remove` and `recommend`.

### Your own rules

- `/rerail-create` writes a rule from your own words: the agent drafts it, tests it on your repository, reads it back, and adds it once you confirm. With no words, it suggests rules from your sessions.
- `/rerail-show-me` shows Rerail catching a problem in your own repository in under a minute, then cleans up.
- Rules belong to the repository, so every clone and worktree shares them. Add one for the whole repository or a single folder.
- Move rules between computers with `rerail rules export` and `rerail rules import`.

### Coding agents

- Every agent gets the same commands: `/rerail`, `/rerail-manage`, `/rerail-create`, `/rerail-show-me`, `/rerail-start`, `/rerail-stop`, `/rerail-update`, `/rerail-uninstall` and `/rerail-help`. Codex writes them as `$rerail`.
- Rerail is off in every repository until you start it there.
- A steer is one line and a link for you, and a block for the agent, which may decline it by telling you why.
- Claude Code, OMP and Pi show a status line with how many rules are on and how often Rerail steered.
- A stopped daemon never stalls a tool call, and a busy one holds it for 25 seconds at most.

### Dashboard

- See rule packs, sessions and violations, with history by rule and by session. Open it with `rerail manage`.
- Files Git ignores are not checked, unless you turn off Follow .gitignore for the repository.
- The daemon listens on this computer only unless you pass `--allow-remote`.
- The daemon also serves an HTTP API and MCP tools.

### Install and update

- Needs Node 24 or newer and Git. Runs on macOS, Linux (glibc 2.28 or newer) and Windows, on arm64 and x64.
- Set-up asks before it reads your coding agent sessions, and again before it reads a repository.
- Updates happen only when you ask: `rerail update` backs up the database first, and `rerail update rollback` goes back.
- `rerail install --remove`, or `/rerail-uninstall`, takes Rerail off the computer and keeps your database in a dated backup.

### For tool builders

- `@neralabs/rerail/rules` validates a code rule as Rerail's own writes do, and `@neralabs/rerail/schemas` exports the rule schemas.

### Upgrading from a preview

- Core no longer ships the complexity pack. If a repository uses it, run `rerail packs add complexity`.
- `/rerail-ui` is now `/rerail-manage`, `rerail ui` is `rerail manage`, and `/rerail-rage-quit` is now `/rerail-uninstall`.
- Rules come only from the repository's configuration: Rerail no longer reads conversation rule files, and `rerail install` no longer takes `--project`.
- On Windows, Codex asks you to trust its hook once more with `/hooks`.

[Every detail of this release](https://github.com/neralabs/rerail/blob/v0.1.0/CHANGELOG.md#010-2026-10-27) is in the changelog at its tag.
