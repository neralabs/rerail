# Rerail for Claude Code

[Rerail](https://github.com/neralabs/rerail) checks the code your coding agent writes against your repository's rules while it works, and steers it back in the same session when it breaks one.

This plugin installs Rerail. It holds one skill, `install`, and no hooks, MCP server, scripts or binaries.

## Install

In Claude Code, in the repository you want Rerail in, run `/rerail:install` or ask Claude to install Rerail. You need Node.js 24 or later and Git. The skill runs Rerail's own installer, pinned to one version:

```sh
npx --yes @neralabs/rerail@0.1.3 setup --relay
```

Set-up asks you each of its questions in the session and changes nothing in a repository before you say yes.

[Installing Rerail](https://github.com/neralabs/rerail/blob/main/docs/installation.md) lists what set-up writes and how to update or remove Rerail, [Privacy](https://github.com/neralabs/rerail/blob/main/docs/privacy.md) every network request, and [Troubleshooting](https://github.com/neralabs/rerail/blob/main/docs/troubleshooting.md) what to do when set-up fails. Removing this plugin does not uninstall Rerail; `rerail install --remove` does.

## License

Apache-2.0. Source, issues and security reports: [neralabs/rerail](https://github.com/neralabs/rerail).
