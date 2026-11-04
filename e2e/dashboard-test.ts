/**
 * The dashboard's Playwright fixtures. `scenarios` (worker-scoped) seeds a scenario the first time a test of the worker
 * asks for it, runs the daemon over it and stops every daemon when the worker ends; `rerail` (test-scoped) hands the
 * test's browser each daemon's access capability in that origin's storage, as opening the printed link once would, and
 * reads the daemon's API with it (`rerail.get`); `page` runs at the seed clock, so Today / 7 days / 30 days read the same
 * on every run. Import `test` and `expect` from here, not from `@playwright/test`.
 */
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test as base, type APIResponse } from "@playwright/test";
import { type RunningScenario, scenarioDashboardUrl, seedScenarioFolder, startScenarioDaemon } from "./support/scenario-daemon.js";
import { SEED_NOW, type ScenarioName } from "./support/scenario-manifest.js";

export { expect } from "@playwright/test";
export { DAY, HOUR, MINUTE, SCENARIO_NAMES, SEED_NOW, type ScenarioManifest, type ScenarioName } from "./support/scenario-manifest.js";

export interface DashboardOptions {
  /** The route, without `#` (`overview`, `pack/typescript`, `history?rule=eslint-no-console`); none opens the default. */
  readonly hash?: string;
  /** Scope to another repository of the scenario, by key; the primary one by default. */
  readonly repository?: string;
}

export interface RerailScenarios {
  /** The running scenario: its manifest (repository keys, packs, sessions, ...) and daemon origin. */
  scenario(name: ScenarioName): Promise<RunningScenario>;
  /** The dashboard URL of `name`, scoped as the product links it: `?repository=<key>&checkout=<path>#<hash>`. */
  dashboard(name: ScenarioName, options?: DashboardOptions): Promise<string>;
  /** `GET url` on a scenario's daemon with its capability, as a Rerail process reads the API. */
  get(url: string): Promise<APIResponse>;
}

interface TestOptions {
  /** Freeze the page's clock at {@link SEED_NOW} (timers keep running). On by default. */
  readonly freezeClock: boolean;
}

export const test = base.extend<TestOptions & { rerail: RerailScenarios }, { scenarios: Omit<RerailScenarios, "get"> }>({
  scenarios: [async ({}, use) => {
    const root = mkdtempSync(join(tmpdir(), "rerail-e2e-"));
    const running = new Map<ScenarioName, Promise<RunningScenario>>();
    const scenario = (name: ScenarioName): Promise<RunningScenario> => {
      let started = running.get(name);
      if (started === undefined) {
        started = seedScenarioFolder(name, join(root, name)).then(manifest => startScenarioDaemon(manifest));
        running.set(name, started);
      }
      return started;
    };
    try {
      await use({
        scenario,
        async dashboard(name, options = {}) {
          const { manifest, origin } = await scenario(name);
          const repository = options.repository === undefined
            ? manifest.primary
            : manifest.repositories.find(entry => entry.repositoryKey === options.repository);
          if (repository === undefined) throw new Error(`scenario ${name} has no repository ${options.repository ?? ""}`);
          return scenarioDashboardUrl(origin, repository, options.hash);
        },
      });
    } finally {
      const stops = [...running.values()].map(async started => (await started).stop());
      await Promise.allSettled(stops);
      rmSync(root, { recursive: true, force: true });
    }
  }, { scope: "worker", timeout: 180_000 }],

  rerail: async ({ scenarios, context }, use) => {
    // Every daemon requires its capability. The page keeps it in its origin's storage, from the link that opened it;
    // the browser gets it there before any page loads, so a test navigates as a person who opened that link once, and
    // the page's own requests must send it themselves. A test's own reads carry it explicitly (`get`).
    const tokens = new Map<string, string>();
    const admitted = async (name: ScenarioName): Promise<RunningScenario> => {
      const started = await scenarios.scenario(name);
      if (!tokens.has(started.origin)) {
        tokens.set(started.origin, started.accessToken);
        await context.addInitScript(([origin, token]) => {
          if (location.origin === origin) localStorage.setItem("rerail-access", token);
        }, [started.origin, started.accessToken] as const);
      }
      return started;
    };
    await use({
      scenario: admitted,
      async dashboard(name, options) {
        await admitted(name);
        return scenarios.dashboard(name, options);
      },
      get(url) {
        const token = tokens.get(new URL(url).origin);
        if (token === undefined) throw new Error(`no scenario daemon was started at ${new URL(url).origin}`);
        return context.request.get(url, { headers: { "x-rerail-access": token } });
      },
    });
  },

  freezeClock: [true, { option: true }],

  page: async ({ page, freezeClock }, use) => {
    if (freezeClock) await page.clock.setFixedTime(SEED_NOW);
    await use(page);
  },
});
