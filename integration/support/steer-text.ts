import { expect } from "vitest";
import { headlineText, steerBlock, steerHeadlines, type SteerTrigger } from "../../src/engine/index.js";
import { steerDashboard } from "../../src/presentation/index.js";

/**
 * The steer block a rule delivers for one finding, as the coding agent reads it: a code rule's after an edit (pass its
 * `location`), or a conversation rule's guidance, led for the tool call it matched (`{ kind: "tool", tool }`) or for
 * the step (`{ kind: "tool" }`, the default without a location).
 */
export function steerText(ruleId: string, message: string, location?: string, trigger?: SteerTrigger): string {
  const led = trigger ?? (location === undefined ? { kind: "tool" } as const : { kind: "edit" } as const);
  return steerBlock(led, [{ ruleId, mode: "steer", message, location }]);
}

/** The person's line for `ruleId` as the adapter for checkout `cwd` draws it, its `why?` link in `style`. */
export async function steerLine(ruleId: string, cwd: string, style: "markdown" | "plain"): Promise<string> {
  const dashboard = await steerDashboard(cwd);
  const url = dashboard === undefined ? undefined : `${dashboard}#rule/${encodeURIComponent(ruleId)}`;
  return headlineText(steerHeadlines([{ id: ruleId, state: "steered", url }])[0]!, style);
}

export const escapeRegExp = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/gu, String.raw`\$&`);

/**
 * Cursor context carrying `content`: the steer itself first, then the request that the model relay each
 * steer's line to the person, ending with the line for `ruleId`, with its markdown `why` link for checkout `cwd`.
 */
export async function cursorRelayed(content: string, ruleId: string, cwd: string): Promise<unknown> {
  const relayRequest = String.raw`\n\n[^\n]+\n`;
  const line = await steerLine(ruleId, cwd, "markdown");
  return expect.stringMatching(new RegExp(`^${escapeRegExp(content)}${relayRequest}${escapeRegExp(line)}$`, "u"));
}
