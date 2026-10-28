/**
 * The dashboard's Playwright fixtures. `scenarios` (worker-scoped) seeds a scenario the first time a test of the worker
 * asks for it, runs the daemon over it and stops every daemon when the worker ends; `rerail` (test-scoped) hands the
 * test's browser each daemon's access capability as the `rerail-access` cookie, as the printed link would; `page` runs
 * at the seed clock, so Today / 7 days / 30 days read the same on every run. Import `test` and `expect` from here, not
 * from `@playwright/test`.
 */
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test as base } from "@playwright/test";
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
}

interface TestOptions {
  /** Freeze the page's clock at {@link SEED_NOW} (timers keep running). On by default. */
  readonly freezeClock: boolean;
}

export const test = base.extend<TestOptions & { rerail: RerailScenarios }, { scenarios: RerailScenarios }>({
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
    // Every daemon requires its capability; the browser (and `page.request`, which shares the context) holds it as
    // the cookie the printed link sets, so a test navigates and reads as a person who opened that link once.
    const admitted = async (name: ScenarioName): Promise<RunningScenario> => {
      const started = await scenarios.scenario(name);
      await context.addCookies([{ name: "rerail-access", value: started.accessToken, url: started.origin }]);
      return started;
    };
    await use({
      scenario: admitted,
      async dashboard(name, options) {
        await admitted(name);
        return scenarios.dashboard(name, options);
      },
    });
  },

  freezeClock: [true, { option: true }],

  page: async ({ page, freezeClock }, use) => {
    if (freezeClock) await page.clock.setFixedTime(SEED_NOW);
    await use(page);
  },
});
