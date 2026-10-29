# plugins

Plugins that coding agents' plugin directories list. Each one only installs Rerail: it runs Rerail's own set-up,
`npx @neralabs/rerail@<version> setup --relay`, pinned to the core version the Release pull request last released, and
holds no hooks, MCP servers, scripts or binaries. Set-up stays the installer and writes the coding agent's hooks itself.

- `claude/`: the Claude Code plugin the Claude plugin directory lists, submitted from the developer portal with this
  repository and the plugin path `plugins/claude` ([docs/release.md](../docs/release.md)).

The plugin manifests at the repository root (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`) are a different
thing: they ship in the npm package and point each agent's hooks at the installed package.
