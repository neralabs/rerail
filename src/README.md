# src

The TypeScript side of Rerail, one directory per layer. A layer imports only the layers below it;
`npm run check` enforces this with [`scripts/check/check-boundaries.mjs`](../scripts/check/check-boundaries.mjs),
which prints `file:line: [rule] message` for every import that crosses a boundary.

| Layer | What it is | May import |
|---|---|---|
| `engine/` | The rule model and schemas (`catalog/`, `code-rule/`), code-rule compile and enforce, the native bridge and semantic primitives (`native/`), audits (`audit/`), the database they persist to (`storage/`), languages and packs | nothing else |
| `runtime/` | One session's events through the engine (`runtime.ts`), the resident engine service, the session and presentation stores, repository start state, and the client side of the daemon (`daemon-control.ts`) | engine |
| `api/` | The daemon's HTTP and MCP operations and contracts, and the stdio MCP bridge | engine, runtime |
| `presentation/` | The session footer, dashboard links and the OMP-family terminal status | engine, runtime, api |
| `daemon/` | The resident daemon: the HTTP/MCP server, the page, the native session server | engine, runtime, api |
| `activation/` | Signed customer profiles and the device secret store | engine |
| `harness/` | Host plumbing every harness shares: hook adapter and launcher, event stream, native session client, semantic tools, `/rerail` commands | engine, runtime, presentation |
| `setup/` | Harness registration (what `rerail install` writes into each coding agent's config) and the managed updater | engine, runtime, harness, activation |
| `hosts/<harness>/` | One coding agent's adapter, hook or extension: `omp`, `pi`, `claude`, `codex`, `cursor`. No host imports another | engine, runtime, presentation, harness, setup |
| `cli/` | The `rerail` command line | any layer |

`index.ts` at the root is the package's public API (`@neralabs/rerail`), and `engine/schemas.ts` is its
browser-safe `@neralabs/rerail/schemas` entry. Each layer's own `index.ts` is its public surface for code outside
`src/`: tests of other layers, `integration/`, `conformance/`, `scripts/` and `benchmark/` import a layer only through
it. Unit tests under `test/<layer>/` may import their own layer's modules, and a vitest mock seam (`vi.mock`,
`vi.importActual`) names the module it replaces. The playground imports only the built package's `exports`.

`omp-semantic.ts` (runtime) and `omp-presentation.ts` (presentation) are named for OMP's session-entry and terminal
formats, which Pi shares; they serve both hosts and the daemon's native sessions, so they sit below the hosts.

Built files mirror this tree under `dist/`. Module code finds the package root with `engine/package-root.ts` rather
than counting `..`, and `ENTRY_POINTS` there names every built file the package starts (the `bin`, hooks and harness
registrations). Files named `*.generated.ts` are written by the generators listed in
[`scripts/generate/README.md`](../scripts/generate/README.md).
