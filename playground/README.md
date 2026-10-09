# Rerail development rule playground

Development tooling that exercises Rerail rules against isolated OMP workers. It is a private
package (`@neralabs/rerail-playground`, never published), outside the root npm workspaces and
outside the published package's `files` allowlist, so none of it ships with `@neralabs/rerail`.

## Running it

```sh
just rerail-playground                  # attended macOS terminal only
just rerail-playground --install-bridge # also registers this checkout's build with every coding agent
```

`just rerail-playground` first runs `node scripts/dev/rerail-dev.mjs up` (what `just rerail-dev`
runs), which installs the checkout if needed, builds its native engine and `dist/`, and starts
this checkout's dev daemon on the host with its own home (`.rerail/dev/home`) and port. It then
reads that instance from `.rerail/dev/instance.json`, prepares the playground (installs its
dependencies when `package-lock.json` is newer than the installed tree, probes `better-sqlite3`
in the checkout and here under the running Node ABI and rebuilds it when necessary, runs
`npm run build` here), waits for the daemon's `/healthz`, and finally supervises the on-demand
isolated workers as compiled JavaScript (`dist/supervisor.js`) under Node 24 or newer. The
workers need the macOS `sandbox-exec` seatbelt, the host OMP installation and login, and a
terminal PTY.

Startup prints two URLs. The first is the client: open it to pick a saved rule, generate or
revise scenarios, run in preview, scripted-trigger, or real-agent mode, drive the live OMP
terminal, and read the event trace. The second is the raw protocol socket.

`RERAIL_UNATTENDED=1` or a non-macOS host exits before anything starts; use `just rerail-dev`
there. `RERAIL_OMP_EXECUTABLE` may point at an absolute standalone OMP executable for CI or local
host testing; otherwise the launcher installs the release checksum-pinned to the SDK dependency
through `scripts/host/install-omp-host.mjs`. Ctrl-C stops the workers; the dev daemon keeps serving
(`just rerail-down` stops it). The launcher never registers harnesses unless asked with
`--install-bridge`.

## The client

The supervisor serves the built client from its own loopback origin, so page, rule list,
OpenAPI document, and control socket are all same-origin; nothing is forwarded through the
daemon, and the daemon has no playground routes. `ui/` is a Vite + React shell around the
playground feature. Its shadcn-style primitives live in `ui/src/components/ui` (`@` aliases to
`ui/src`), and its colour, radius, shadow, and font tokens are the daemon page's
`ui/tokens.generated.css` at the checkout root, so the two pages cannot drift. Light and dark
follow the operating system.

`npm run dev:ui` serves the same client on `http://127.0.0.1:4742` with HMR and proxies
`/__rerail_playground` to a supervisor already running from `just rerail-playground`
(set `RERAIL_PLAYGROUND_PORT` to the port in the client URL the supervisor printed).

## Protocol and guarantees

The supervisor reaches this checkout's dev daemon at the port recorded in
`.rerail/dev/instance.json` (passed as `RERAIL_PLAYGROUND_DAEMON_PORT`). It answers `GET /` (the
client), `GET /__rerail_playground/rules` (the daemon's saved rules as playground snapshots,
read server-side so the browser stays on one origin), `GET /__rerail_playground/openapi.json`
(the wire schema), and the `/__rerail_playground` WebSocket. Only its own origin and the
daemon's are accepted on that socket; every other `Origin`, including a missing one, is
refused, because a connection can spawn OMP workers.

One tab owns the experiment at a time, because a connection drives real workers. The newest
connection takes ownership and the previous tab is closed with a message saying so, so a
reloaded, frozen, or crashed tab can never lock you out — reload the tab you want to use.

Every worker requires `sandbox-exec`, has a three-minute lifetime, gets private
configuration, session, database, and filesystem state, and copies only an access token,
account ID, and expiry from an existing OMP Codex login with at least 210 seconds
remaining; the original auth database is read-only and refresh tokens are never copied.
Rules are read from the daemon over HTTP and activated in a sandboxed catalog seeded from
Rerail's bundled rules; the live catalog is never touched. Experiments persist under
checkout-local `.rerail/playground/`, separate from live rules.

Saving a file under `src`, `code-rule-dsl`, `native`, or `playground/src` reruns the latest
snapshot: TypeScript edits rebuild the checkout (`build:ts`, `build:omp`) and then this
package's workers; kernel edits rebuild the native addon. Client sources are not watched — use
`dev:ui` or rebuild with `build:ui`.

## Development

Rerail is consumed through the checkout's `dist/*.js`, typed by the declarations `tsc` emits
there, so the checkout must be built before this package type-checks or runs:

```sh
npm ci && npm run build:ts                         # checkout root
npm ci --prefix playground
npm --prefix playground run check                  # tsc --noEmit for the workers and the client
npm --prefix playground run build                  # esbuild src/*.ts → dist/, then vite build → dist/ui
npm --prefix playground test                       # vitest: test/ and scripts/
npm --prefix playground run test:ui                # vitest, jsdom, ui/src
npm --prefix playground run test:integration       # macOS-only suites skip elsewhere; needs the host kernel
```

The checkout's `dist` is kept external at bundle time because Rerail's model and native asset
paths are relative to `import.meta.url`; `src/` and `dist/` sit at the same depth, so the
relative specifiers survive emit unchanged. The product requirements live in
`docs/playground-prd.md`.
