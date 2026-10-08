# Rerail development rule playground

Private Landing tooling that exercises Rerail rules against isolated OMP workers. It lives
beside `rerail/` so that the public Rerail projection (`scripts/export_rerail_repository.py`
exports only `rerail/`) carries no playground code. It is not published and never enters a
deployment build context (`/rerail-playground` in the root `.dockerignore`).

## Running it

```sh
just rerail-playground                  # attended macOS terminal only
just rerail-playground --install-bridge # also registers the portable host bridge once
```

`just rerail-playground` first runs `just rerail-dev`, so this checkout's daemon comes up in
its per-worktree container and serves the committed page from the bind-mounted `rerail/ui`.
It then prepares the host runtime (installs the repository root, `rerail/`, and
`playground/` dependencies when their lockfiles are newer than the installed tree,
probes `better-sqlite3` in both package roots under the selected host Node ABI and rebuilds
it when necessary, builds the checkout's native kernel for this platform if it is stale,
runs `npm run build:ts` in `rerail/` and `npm run build` here), waits for the container
daemon's `/healthz`, and finally supervises the on-demand isolated workers on the host as
compiled JavaScript (`dist/supervisor.js`) under Node 24. The workers stay on the host
because they need the macOS `sandbox-exec` seatbelt, the host OMP installation and login,
and a terminal PTY, none of which exist in the Linux container.

Startup prints two URLs. The first is the client: open it to pick a saved rule, generate or
revise scenarios, run in preview, scripted-trigger, or real-agent mode, drive the live OMP
terminal, and read the event trace. The second is the raw protocol socket.

`NERA_UNATTENDED=1` or a non-macOS host exit before starting the container; use
`just rerail-dev` there. `RERAIL_OMP_EXECUTABLE` may point at an absolute standalone OMP
executable for CI or local host testing; otherwise the launcher installs the release
checksum-pinned to the SDK dependency through `rerail/scripts/install-omp-host.mjs`.
Ctrl-C stops the workers; the daemon keeps serving the page. The launcher never registers
harnesses.

## The client

The supervisor serves the built client from its own loopback origin, so page, rule list,
OpenAPI document, and control socket are all same-origin; nothing is forwarded through the
daemon, and the daemon has no playground routes. `ui/` is a Vite + React shell around the
ported playground feature. It owns no design system: `@` aliases to nera-landing's
`web/src`, so shadcn components, semantic tokens, and fonts come from the enclosing
checkout, and React and friends resolve from the repository root `node_modules` (`web` is
its workspace). That makes a root `npm ci` a build prerequisite — the launcher runs it.

`npm run dev:ui` serves the same client on `http://127.0.0.1:4742` with HMR and proxies
`/__rerail_playground` to a supervisor already running from `just rerail-playground`
(`RERAIL_PLAYGROUND_PORT` selects its port; the supervisor prints it).

## Protocol and guarantees

The supervisor reaches its workspace daemon through the reserved public origin recorded
in `.nera/instance.env` (`RERAIL_PORT`). It answers `GET /` (the client), `GET
/__rerail_playground/rules` (the daemon's saved rules as playground snapshots, read
server-side so the browser stays on one origin), `GET /__rerail_playground/openapi.json`
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

Saving a file under `rerail/src`, `rerail/code-rule-dsl`, `rerail/native`, or
`playground/src` reruns the latest snapshot: TypeScript edits rebuild `rerail/`
(`build:ts`, `build:omp`) and then this package's workers; kernel edits rebuild the native
addon. Client sources are not watched — use `dev:ui` or rebuild with `build:ui`.

## Development

Rerail is consumed through `../rerail/dist/*.js`, typed by the declarations `tsc` emits
there, so `rerail/` must be built before this package type-checks or runs:

```sh
npm ci                                             # nera-landing root: the client's React and components
npm --prefix rerail run build:ts
npm ci --prefix rerail-playground
npm --prefix rerail-playground run check            # tsc --noEmit for the workers and the client
npm --prefix rerail-playground run build            # esbuild src/*.ts → dist/, then vite build → dist/ui
npm --prefix rerail-playground test                 # vitest --dir test
npm --prefix rerail-playground run test:ui          # vitest, jsdom, ui/src
npm --prefix rerail-playground run test:integration # macOS-only suites skip elsewhere; needs the host kernel
node --test playground/scripts/playground-dev.test.mjs
```

`rerail/dist` is kept external at bundle time because Rerail's model and native asset paths
are relative to `import.meta.url`; `src/` and `dist/` sit at the same depth, so the
relative specifiers survive emit unchanged. The product requirements live in
`docs/playground-prd.md`.
