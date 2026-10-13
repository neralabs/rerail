import { linkSteers, steerBlock, type AdapterDelivery } from "../../src/engine/index.js";

/**
 * The steers of the session enforcement design (`PLAN-NER-1870.md`, scenes N4 and N4b), as each coding agent's
 * adapter receives them: linked to a loopback dashboard. Every agent's golden output renders these same deliveries.
 */
export const SCENE_DASHBOARD = "http://127.0.0.1:4737/";
export const SCENE_SESSION = "5f2c";

function sceneSteer(ruleId: string, guidance: string): AdapterDelivery {
  const block = steerBlock(ruleId, guidance, { mode: "steer", origin: "From the TypeScript rule pack." });
  return { sessionId: SCENE_SESSION, requested: "steer", mechanism: "additional-context", content: linkSteers(block, SCENE_DASHBOARD), ruleIds: [ruleId] };
}

export const NO_EXPLICIT_ANY = sceneSteer(
  "typescript-eslint:no-explicit-any",
  "Use unknown or a real type in exported signatures. (web/src/features/orders/useOrders.ts:1:33)",
);

export const NO_FLOATING_PROMISES = sceneSteer(
  "typescript-eslint:no-floating-promises",
  "Await the call or hand its promise on; a dropped promise swallows its errors. (web/src/features/orders/useOrders.ts:9:5)",
);

/** Hook output as a golden file: the exact JSON the coding agent reads, pretty-printed. */
export const goldenJson = (output: unknown): string => `${JSON.stringify(output, null, 2)}\n`;

/** What a terminal draws for `text`: each OSC 8 link reduced to its label, as the person reads it. */
export const drawn = (text: string): string => `${text.replace(/\u001b\]8;;[^\u0007]*\u0007(.*?)\u001b\]8;;\u0007/gu, "$1")}\n`;
