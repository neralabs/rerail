/**
 * How Rerail recognises a person correcting their coding agent, and how it names what the agent had just done. Shared
 * by `rerail rules evidence` and Merge Assist's capture, which both read stored session events.
 */
import { callNamesDeniedPath } from "../transcript-boundary.js";

/** Longer prompts are task statements, not corrections. */
const CORRECTION_MAX_WORDS = 120;
const CALL_TEXT_LENGTH = 120;

/**
 * Phrases a person uses when correcting an agent, from the Tempo study of 59,690 prompts. A prompt matching
 * none of them is not reported. `standing` marks a phrase that states a rule rather than reacting to one turn.
 */
export const CORRECTION_FAMILIES: ReadonlyArray<{ name: string; standing: boolean; pattern: RegExp }> = [
  { name: "standing instruction", standing: true, pattern: /\b(?:never|always|do not|don'?t|dont|stop|instead of|rather than|should not|shouldn'?t|no more|avoid|must not|we use|use \S+ not)\b/i },
  { name: "hard stop", standing: false, pattern: /^\s*(?:(?:stop|wait|hold on|nope|wrong|hang on)\b|no[.,!])/i },
  { name: "not listening", standing: false, pattern: /\b(?:i (?:told|asked) you|as i (?:said|asked)|not what i asked|that(?:'s| is) not what|read (?:my|the) (?:prompt|instructions?))/i },
  { name: "quality complaint", standing: false, pattern: /\b(?:this is (?:wrong|bad|a mess|broken|not right)|sloppy|hacky|band[- ]?aid|workaround|over[- ]?engineer|unnecessary)/i },
  { name: "repeat failure", standing: false, pattern: /\b(?:still (?:broken|failing|fails|not working|wrong)|same (?:error|problem|issue|bug)|yet again|once again|not fixed|didn'?t fix)/i },
  { name: "false completion", standing: false, pattern: /\b(?:you (?:said|claimed) (?:it|that|this)? ?(?:was|is)? ?(?:done|fixed|working|passing)|didn'?t actually|not actually|doesn'?t (?:actually )?work)/i },
];

/** The families a follow-up prompt matches; none for prompts that are not corrections (tool notices, long briefs). */
export function correctionFamilies(text: string): string[] {
  // Typographic apostrophes (macOS, phones) read as plain ones: "Don’t" is "Don't".
  const trimmed = text.trim().replace(/[‘’]/gu, "'");
  if (trimmed.length === 0 || trimmed.startsWith("<") || trimmed.startsWith("#") || trimmed.startsWith("---")) return [];
  if (trimmed.split(/\s+/).length > CORRECTION_MAX_WORDS) return [];
  return CORRECTION_FAMILIES.filter(family => family.pattern.test(trimmed)).map(family => family.name);
}

function clip(text: string, length: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > length ? `${flat.slice(0, length - 1)}…` : flat;
}

/**
 * `<tool> <command or path>`: what a call did, in one line, from a stored `call` event's payload. A call that names a
 * hidden or secret-like path (`cat .env`) is named by its tool alone: the summary goes to a coding agent, and that
 * path stays out of its conversation.
 */
export function callSummary(payload: Record<string, unknown>): string {
  const tool = typeof payload.toolName === "string" ? payload.toolName : "tool";
  if (callNamesDeniedPath(payload.input)) return `${tool} (a hidden or secret-like path, withheld)`;
  const input = typeof payload.input === "object" && payload.input !== null ? payload.input as Record<string, unknown> : {};
  const target = [input.command, input.file_path, input.path, input.filePath, input.pattern].find(value => typeof value === "string");
  return clip(typeof target === "string" ? `${tool} ${target}` : tool, CALL_TEXT_LENGTH);
}
