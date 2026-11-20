import { describe, expect, it } from "vitest";
import {
  activationProfilesEnabled,
  blockModeEnabled,
  conversationRulesEnabled,
  enabledExperiments,
  EXPERIMENTAL_FEATURES,
  experimentEnabled,
  experimentOffMessage,
  ExperimentOffError,
  firstRepositoryAuditEnabled,
  mergeAssistExperimentEnabled,
  missedSteerDigestEnabled,
  remoteSinkEnabled,
  requireExperiment,
  semanticToolsEnabled,
  steerStrategiesEnabled,
  type ExperimentalFeature,
} from "../../src/engine/experimental-features.js";

const ALL = Object.keys(EXPERIMENTAL_FEATURES) as ExperimentalFeature[];
const variable = (feature: ExperimentalFeature): string => `RERAIL_EXPERIMENTAL_${feature}`;

/** Which features are on in `env`, by their variable. */
const on = (env: NodeJS.ProcessEnv): string[] => enabledExperiments(env).map(variable);

/** The named switches callers use, each the generic one for its feature. */
const NAMED = {
  MISSED_STEER_DIGEST: missedSteerDigestEnabled,
  STEER_STRATEGIES: steerStrategiesEnabled,
  FIRST_REPOSITORY_AUDIT: firstRepositoryAuditEnabled,
  MERGE_ASSIST: mergeAssistExperimentEnabled,
  CONVERSATION_RULES: conversationRulesEnabled,
  SEMANTIC_TOOLS: semanticToolsEnabled,
  BLOCK_MODE: blockModeEnabled,
  REMOTE_SINK: remoteSinkEnabled,
  ACTIVATION_PROFILES: activationProfilesEnabled,
} satisfies Partial<Record<ExperimentalFeature, (env: NodeJS.ProcessEnv) => boolean>>;

describe("experimental features", () => {
  it("are all off with nothing set", () => {
    expect(on({})).toEqual([]);
  });

  it("are all on with the blanket switch, but for one whose own variable is 0", () => {
    expect(on({ RERAIL_EXPERIMENTAL: "1" })).toEqual(ALL.map(variable));
    expect(on({ RERAIL_EXPERIMENTAL: "1", RERAIL_EXPERIMENTAL_MERGE_ASSIST: "0" })).toEqual(ALL.filter(feature => feature !== "MERGE_ASSIST").map(variable));
  });

  it("turn on one at a time by their own variable, without the blanket switch", () => {
    for (const feature of ALL) expect(on({ [variable(feature)]: "1" })).toEqual([variable(feature)]);
    expect(on({ RERAIL_EXPERIMENTAL: "0", RERAIL_EXPERIMENTAL_STEER_STRATEGIES: "1" })).toEqual(["RERAIL_EXPERIMENTAL_STEER_STRATEGIES"]);
  });

  it("read any other value as unset, of a feature's own variable and of the blanket switch", () => {
    for (const value of ["", "true", "yes", "on", " 1"]) {
      expect(on({ RERAIL_EXPERIMENTAL: value }), JSON.stringify(value)).toEqual([]);
      expect(on({ RERAIL_EXPERIMENTAL_MERGE_ASSIST: value }), JSON.stringify(value)).toEqual([]);
      expect(on({ RERAIL_EXPERIMENTAL: "1", RERAIL_EXPERIMENTAL_MERGE_ASSIST: value }), JSON.stringify(value)).toEqual(ALL.map(variable));
    }
  });

  it("answer each named switch as the generic one does for its feature", () => {
    for (const [feature, enabled] of Object.entries(NAMED) as Array<[ExperimentalFeature, (env: NodeJS.ProcessEnv) => boolean]>) {
      for (const env of [{}, { RERAIL_EXPERIMENTAL: "1" }, { [variable(feature)]: "1" }, { RERAIL_EXPERIMENTAL: "1", [variable(feature)]: "0" }]) {
        expect(enabled(env), `${feature} ${JSON.stringify(env)}`).toBe(experimentEnabled(feature, env));
      }
    }
  });

  it("name the variable that turns a feature on when it is reached while off, and where to set it", () => {
    expect(experimentOffMessage("MERGE_ASSIST", "start the Rerail daemon with")).toBe(
      "Merge Assist is experimental and off: start the Rerail daemon with RERAIL_EXPERIMENTAL_MERGE_ASSIST=1 (or RERAIL_EXPERIMENTAL=1 for every experimental feature) to use it.");
    expect(experimentOffMessage("CONVERSATION_RULES")).toMatch(/^Conversation rules are experimental and off: set RERAIL_EXPERIMENTAL_CONVERSATION_RULES=1 /u);
    expect(() => requireExperiment("EVENT_STREAM", undefined, {})).toThrow(ExperimentOffError);
    expect(() => requireExperiment("EVENT_STREAM", undefined, {})).toThrow("The event stream is experimental and off: set RERAIL_EXPERIMENTAL_EVENT_STREAM=1");
    expect(() => requireExperiment("EVENT_STREAM", undefined, { RERAIL_EXPERIMENTAL: "1" })).not.toThrow();
  });
});
