# Rerail for Codex

[Rerail](https://github.com/neralabs/rerail) checks the code your coding agent writes against your repository's rules while it works, and steers it back in the same session when it breaks one. Rules run on your machine, by regex, syntax tree and relations, in a native engine; ready-made rule packs port ESLint, typescript-eslint, Ruff, Bandit, Clippy and other linters, and you can add rules in your own words.

This plugin installs Rerail. It holds one skill, `install`, and nothing else: no hooks, no MCP server, no scripts and no binaries.

## Use

In Codex (the CLI or the desktop app), in the repository you want Rerail in, ask Codex to install Rerail; the plugin also offers its `install` skill as its setup step. You need Node.js 24 or later and Git, on macOS, Linux with glibc 2.28 or later, or Windows.

## What it runs

The skill has Codex run Rerail's own installer, pinned to one version:

```sh
npx --yes @neralabs/rerail@0.1.0 setup --relay
```

`npx` downloads `@neralabs/rerail` from npm. Set-up then asks you each of its questions in the session (Codex relays them as written), and changes nothing in a repository before you say yes. Once you answer, it:

- installs Rerail under `~/.rerail` and puts `~/.rerail/bin` on your `PATH`;
- connects the coding agents you choose: for Codex, hooks in `~/.codex/hooks.json` (Codex asks you to trust them once, with `/hooks`) and `$rerail-*` skills in `~/.codex/skills/`;
- starts a local daemon that listens on loopback only;
- installs the rule packs you confirm, from npm.

Rerail has no telemetry. Its only network traffic goes through npm, to the registry your npm configuration names, and carries package names and versions only: the install itself, and fetching rule packs to match them against your repository on your machine. Rule checking makes no network requests. [Privacy](https://github.com/neralabs/rerail/blob/main/docs/privacy.md) lists every request, and [Installing Rerail](https://github.com/neralabs/rerail/blob/main/docs/installation.md) everything set-up writes.

In ChatGPT chat the skill can't run a terminal, so it gives you the command to run yourself.

## Updates and removal

Nothing updates on its own: `rerail update` installs a newer Rerail when you run it. `rerail install --remove` takes Rerail out of every coding agent, stops the daemon and deletes the install, keeping your rules and history in a backup. Removing this plugin does not uninstall Rerail.

## License

Apache-2.0. Source, issues and security reports: [neralabs/rerail](https://github.com/neralabs/rerail).
