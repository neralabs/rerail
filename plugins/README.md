# plugins

Plugins that coding agents' plugin directories list. Each one only installs Rerail: it runs Rerail's own set-up,
`npx @neralabs/rerail@<version> setup --relay`, pinned to the core version the Release pull request last released, and
holds no hooks, MCP servers, scripts or binaries. Set-up stays the installer and writes the coding agent's hooks itself.

- `claude/`: the Claude Code plugin the Claude plugin directory lists, submitted from the developer portal at
  claude.ai/directory/manage with this repository and the plugin path `plugins/claude`.
- `codex/`: the plugin for OpenAI's plugin directory (ChatGPT and Codex), in the portable Agent Plugins format with
  OpenAI's listing under `extensions.com.openai`. It is submitted as a ZIP of this folder at
  platform.openai.com/plugins; its `install` skill is also its onboarding skill.
- `cursor/`: the Cursor plugin for the Cursor Marketplace, submitted at cursor.com/marketplace/publish with the link to
  this folder.

The Release pull request moves every plugin's pins with core's version (`PLUGIN_PINS` in `scripts/release/core.mjs`);
publish each new plugin version only once that release is on npm ([docs/release.md](../docs/release.md)).

The plugin manifests at the repository root (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`) are a different
thing: they ship in the npm package and point each agent's hooks at the installed package.
