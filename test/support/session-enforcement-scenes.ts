import { steerBlock, steerRules, type AdapterDelivery, type SteerFinding } from "../../src/engine/index.js";

/**
 * The steers of the session enforcement design (NER-1870, scenes N4 and N4b; docs/in-session-surfaces.md), as each
 * coding agent's adapter receives them: their rules linked to a loopback dashboard. Every agent's golden output
 * renders these same deliveries.
 */
export const SCENE_DASHBOARD = "http://127.0.0.1:4737/";
export const SCENE_SESSION = "5f2c";
/** The coloured `R E R A I L` a coding agent that draws colour shows for Rerail, tagged so a golden reads its place. */
export const SCENE_MARK = "<mark>R E R A I L</mark>";

const ORIGIN = "From the TypeScript rule pack.";

const NO_EXPLICIT_ANY_FINDING: SteerFinding = {
  ruleId: "typescript-eslint-no-explicit-any",
  mode: "steer",
  message: "Use unknown or a real type in exported signatures.",
  location: "web/src/features/orders/useOrders.ts:1:33",
};

const NO_FLOATING_PROMISES_FINDING: SteerFinding = {
  ruleId: "typescript-eslint-no-floating-promises",
  mode: "steer",
  message: "Await the call or hand its promise on; a dropped promise swallows its errors.",
  location: "web/src/features/orders/useOrders.ts:9:5",
};

/** An edit's steer for `findings`, its rules linked to the scene's dashboard. */
function sceneSteer(...findings: SteerFinding[]): AdapterDelivery {
  const rules = steerRules(findings, () => ORIGIN).map(rule => ({ ...rule, url: `${SCENE_DASHBOARD}#rule/${encodeURIComponent(rule.id)}` }));
  return {
    sessionId: SCENE_SESSION,
    requested: "steer",
    mechanism: "additional-context",
    content: steerBlock({ kind: "edit" }, findings),
    ruleIds: rules.map(rule => rule.id),
    rules,
  };
}

export const NO_EXPLICIT_ANY = sceneSteer(NO_EXPLICIT_ANY_FINDING);

/** One edit that broke two rules: one block, one line per rule for the person. */
export const TWO_RULES = sceneSteer(NO_EXPLICIT_ANY_FINDING, NO_FLOATING_PROMISES_FINDING);

/** Hook output as a golden file: the exact JSON the coding agent reads, pretty-printed. */
export const goldenJson = (output: unknown): string => `${JSON.stringify(output, null, 2)}\n`;

/** What a terminal draws for `text`: each OSC 8 link reduced to its label, as the person reads it. */
export const drawn = (text: string): string => `${text.replace(/\u001b\]8;;[^\u0007]*\u0007(.*?)\u001b\]8;;\u0007/gu, "$1")}\n`;
