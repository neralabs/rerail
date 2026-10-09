# Hooks

The hook registrations the Claude Code, Codex and Cursor plugins declare. One file per harness:

- `claude.json`: the `.claude-plugin` manifest
- `codex.json`: the `.codex-plugin` manifest
- `cursor.json`: the `.cursor-plugin` manifest

Each file maps the harness's lifecycle events to the built hook entry point under `dist/hosts/<harness>/`. Those entry
points send normalized events to the daemon's resident kernel. The same registrations are written into user configs by
`src/setup/harness-registration.ts`. `test/setup/plugin-package-contract.test.ts` and the per-host plugin contract
tests check that every path they name is built and shipped.
