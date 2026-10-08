import { expect } from "vitest";
import { linkSteers, steerBlock, steerHeadlines } from "../src/rerail-steer.js";
import { dashboardOrigin } from "../src/session-presentation.js";

/**
 * The steer block a rule delivers. Runtime/kernel deliveries carry the relative `Why?` link; pass the checkout
 * `cwd` to get the text an adapter hands the agent, with the link made absolute for that checkout.
 */
export async function steerText(ruleId: string, guidance: string, cwd?: string): Promise<string> {
  const block = steerBlock(ruleId, guidance);
  return cwd === undefined ? block : linkSteers(block, await dashboardOrigin(cwd));
}

const escapeRegExp = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/gu, String.raw`\$&`);

/**
 * Cursor context carrying `content`: the steer itself first, then the request that the model relay each
 * steer's headline to the person, ending with those headlines.
 */
export function cursorRelayed(content: string): unknown {
  const relayRequest = String.raw`\n\n[^\n]+\n`;
  return expect.stringMatching(new RegExp(`^${escapeRegExp(content)}${relayRequest}${escapeRegExp(steerHeadlines(content).join("\n"))}$`, "u"));
}
