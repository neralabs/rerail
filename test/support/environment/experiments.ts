import { beforeEach, vi } from "vitest";
import { experimentVariable, type ExperimentalFeature } from "../../../src/engine/index.js";

/**
 * Turns `features` on for every test of the file (or `describe`) that calls it, as `RERAIL_EXPERIMENTAL_<FEATURE>=1`.
 * Every experimental feature starts off in a test (`setup-rerail-home.ts`); a process a test spawns inherits the stub.
 */
export function withExperiments(...features: readonly ExperimentalFeature[]): void {
  beforeEach(() => {
    for (const feature of features) vi.stubEnv(experimentVariable(feature), "1");
  });
}
