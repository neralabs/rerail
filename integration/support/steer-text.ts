import { expect } from "vitest";
import { headlineText, linkSteers, steerBlock, steerHeadlines } from "../../src/engine/index.js";
import { steerDashboard } from "../../src/presentation/index.js";

/**
 * The steer block a rule delivers. Runtime/kernel deliveries carry the relative `Why?` link; pass the checkout
 * `cwd` to get the text an adapter hands the agent, with the link made absolute for that checkout. `origin` is the
 * rule's source sentence (`steerOrigin`), absent for rules whose source is unknown.
 */
export async function steerText(ruleId: string, guidance: string, cwd?: string, origin?: string): Promise<string> {
  const block = steerBlock(ruleId, guidance, { mode: "steer", origin });
  return cwd === undefined ? block : linkSteers(block, await steerDashboard(cwd));
}

const escapeRegExp = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/gu, String.raw`\$&`);

/**
 * Cursor context carrying `content`: the steer itself first, then the request that the model relay each
 * steer's line to the person, ending with those lines, each with a markdown `why` link.
 */
export function cursorRelayed(content: string): unknown {
  const relayRequest = String.raw`\n\n[^\n]+\n`;
  const lines = steerHeadlines(content).map(headline => headlineText(headline, "markdown")).join("\n");
  return expect.stringMatching(new RegExp(`^${escapeRegExp(content)}${relayRequest}${escapeRegExp(lines)}$`, "u"));
}
