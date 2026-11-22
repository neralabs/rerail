import { afterEach, beforeEach } from "vitest";
import { experimentVariable, type ExperimentalFeature } from "../../../src/engine/index.js";

/**
 * Turns `features` on for every test of the file (or `describe`) that calls it, as `RERAIL_EXPERIMENTAL_<FEATURE>=1`,
 * and gives each variable back the value it had before after every test, so a `describe` that calls it leaves its
 * siblings as they were. Every experimental feature starts off in a test (`setup-rerail-home.ts`); a process a test
 * spawns inherits the variable. Set in the environment, not stubbed: a test that stubs variables of its own and puts
 * them back (`vi.unstubAllEnvs()`) keeps its features on, and one that stubs a feature's variable off gets it back on.
 */
export function withExperiments(...features: readonly ExperimentalFeature[]): void {
  const before = new Map<string, string | undefined>();
  beforeEach(() => {
    for (const feature of features) {
      const variable = experimentVariable(feature);
      before.set(variable, process.env[variable]);
      process.env[variable] = "1";
    }
  });
  afterEach(() => {
    for (const [variable, value] of before) {
      if (value === undefined) delete process.env[variable];
      else process.env[variable] = value;
    }
    before.clear();
  });
}
