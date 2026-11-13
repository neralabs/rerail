import { describe, expect, it } from "vitest";
import {
  firstRepositoryAuditEnabled,
  mergeAssistExperimentEnabled,
  missedSteerDigestEnabled,
  steerStrategiesEnabled,
} from "../../src/engine/experimental-features.js";

/** Every experimental feature, by its own variable. */
const FEATURES = {
  RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST: missedSteerDigestEnabled,
  RERAIL_EXPERIMENTAL_STEER_STRATEGIES: steerStrategiesEnabled,
  RERAIL_EXPERIMENTAL_FIRST_REPOSITORY_AUDIT: firstRepositoryAuditEnabled,
  RERAIL_EXPERIMENTAL_MERGE_ASSIST: mergeAssistExperimentEnabled,
} satisfies Record<string, (env: NodeJS.ProcessEnv) => boolean>;

/** Which features are on in `env`, by their variable. */
const on = (env: NodeJS.ProcessEnv): string[] => Object.entries(FEATURES).filter(([, enabled]) => enabled(env)).map(([name]) => name);

describe("experimental features", () => {
  it("are all off with nothing set", () => {
    expect(on({})).toEqual([]);
  });

  it("are all on with the blanket switch, but for one whose own variable is 0", () => {
    expect(on({ RERAIL_EXPERIMENTAL: "1" })).toEqual(Object.keys(FEATURES));
    expect(on({ RERAIL_EXPERIMENTAL: "1", RERAIL_EXPERIMENTAL_MERGE_ASSIST: "0" })).toEqual(Object.keys(FEATURES).filter(name => name !== "RERAIL_EXPERIMENTAL_MERGE_ASSIST"));
  });

  it("turn on one at a time by their own variable, without the blanket switch", () => {
    for (const name of Object.keys(FEATURES)) expect(on({ [name]: "1" })).toEqual([name]);
    expect(on({ RERAIL_EXPERIMENTAL: "0", RERAIL_EXPERIMENTAL_STEER_STRATEGIES: "1" })).toEqual(["RERAIL_EXPERIMENTAL_STEER_STRATEGIES"]);
  });

  it("read any other value as unset, of a feature's own variable and of the blanket switch", () => {
    for (const value of ["", "true", "yes", "on", " 1"]) {
      expect(on({ RERAIL_EXPERIMENTAL: value }), JSON.stringify(value)).toEqual([]);
      expect(on({ RERAIL_EXPERIMENTAL_MERGE_ASSIST: value }), JSON.stringify(value)).toEqual([]);
      expect(on({ RERAIL_EXPERIMENTAL: "1", RERAIL_EXPERIMENTAL_MERGE_ASSIST: value }), JSON.stringify(value)).toEqual(Object.keys(FEATURES));
    }
  });
});
