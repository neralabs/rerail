# e2e: dashboard browser tests

Playwright specs for the dashboard (`ui/`), run in Chromium against a real daemon over a seeded database. They live
here, not under `test/`, because vitest runs every spec file under `test/`.

```sh
npm run build:native                 # once: the daemon and the seeding need the native engine
npx playwright install chromium      # once per Playwright version
npm run test:ui:e2e                  # every spec; add `-- e2e/smoke.spec.ts` or `-- --headed` as usual
```

## How a spec runs

`dashboard-test.ts` gives every test two things:

- `rerail` (one per worker): `await rerail.dashboard(scenario, { hash?, repository? })` returns the dashboard URL of
  that scenario, scoped as the product links it (`?repository=<key>&checkout=<path>#<hash>`). The first test of a
  worker that asks for a scenario seeds it into a temp folder and starts `rerail daemon --dev` over it (free port, its
  own `RERAIL_HOME` and `HOME`, UTC); the worker stops every daemon when it ends. `await rerail.scenario(name)` gives
  the daemon's `origin` and the scenario's `manifest` (repository keys, packs, rule ids with violations, session ids).
- `page`, with its clock frozen at `SEED_NOW` (Oct 22 2026, 15:00 UTC), so Today / 7 days / 30 days and relative
  times read the same on every run. Timers keep running, so polling works. `test.use({ freezeClock: false })` turns
  it off.

Import `test` and `expect` from `./dashboard-test.js` (it is not named `fixtures`: the structure gate reserves that
name for recorded data):

```ts
import { expect, test } from "./dashboard-test.js";

test("history preselects a rule", async ({ page, rerail }) => {
  await page.goto(await rerail.dashboard("mature", { hash: "history?rule=eslint-no-console" }));
  await expect(page.locator("#content")).toContainText("eslint-no-console");
});
```

Stable hooks a spec may rely on: the title `Rerail` and the ids `status`, `content`, `panel`, `toast`, `sidenav`.

## Scenarios

Defined in `support/scenarios.ts`, written through the product's own stores. Repository `remote:github.com/acme/storefront`
is the primary one in every scenario.

| Scenario | What it holds |
| --- | --- |
| `no-packs` | Set up 30 minutes ago with nothing added; started; no sessions; no file a pack matches, so no recommendations. |
| `packs-no-session` | Set up an hour ago with `typescript` and `javascript`; started; no sessions. |
| `mature` | Set up 5 weeks ago with `typescript`, `javascript`, `complexity` and two rules created by you; 30 sessions (Claude Code, Codex, Cursor), the last still open; violations in Log, Steer and Block spread over the 5 weeks, steered sessions have steer receipts. |
| `multi-repo` | storefront (3 weeks of history, 10 sessions, one open), `acme/billing` (set up, no sessions), `acme/docs-site` (known, never set up or started). |
| `not-set-up` | Started 20 minutes ago, never set up: nothing runs. |
| `stopped` | Set up 10 days ago with `typescript` and `javascript`, two sessions, stopped 2 days ago. |

Pick the smallest scenario that shows the state under test. Add one only for a state none of these has: a new
`case` in `plansFor`, its name in `SCENARIO_NAMES` (`support/scenario-manifest.ts`) and a row here.

## Looking at a scenario

`npm run ui:scenario -- <scenario> [--port N]` (or `just ui-scenario <scenario>`) seeds it, dated back from now, runs
the daemon in the foreground and prints the dashboard URL. Edits to `ui/` show on reload.
