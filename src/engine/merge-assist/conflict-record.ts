/**
 * A conflict as the resolver reads it, and the vocabulary policy steps decide it in: the two sides, the steps,
 * a step's result, the record (repository, base, question, both sides' refs, commits, sanitised intents and anchors),
 * the question two sides disagree on, and how a side's description is made fit for a resolver. No YAML and no Git: the
 * policy file itself is `policy.ts`.
 */
import { createHash } from "node:crypto";
import { z } from "zod";

/** The steps a policy can run, in the only order they run in; escalation to a person always follows them. */
export const POLICY_STEPS = ["hard-checks", "criteria", "panel"] as const;
export type PolicyStep = (typeof POLICY_STEPS)[number];
/** What settled a conflict: one of the steps, or escalation to a person. */
export type DecidingStep = PolicyStep | "escalate";

/** The two sides of a conflict. Neither name says which came first. */
export const SIDES = ["a", "b"] as const;
export type SideId = (typeof SIDES)[number];
export type Verdict = SideId | "escalated";

export function otherSide(side: SideId): SideId {
  return side === "a" ? "b" : "a";
}

/** One step's outcome: the side it picked, or null with why it could not. */
export interface StepResult {
  readonly step: DecidingStep;
  readonly winner: SideId | null;
  readonly reason: string;
}

const SHA = /^[0-9a-f]{40,64}$/u;

/** One side of a conflict as the resolver reads it (the status shows a side as `ConflictSide`). */
export const recordSideSchema = z.object({
  /** The side's ref: a branch, or a pull request's `refs/pull/<n>/head`. */
  ref: z.string().min(1).max(500),
  sha: z.string().regex(SHA),
  /** What the side says it does: its title and description, without cross-references (`sanitizeIntent`). */
  intent: z.string().max(20_000),
  /** The files and symbols the side's half of the conflict is in. */
  anchors: z.object({ files: z.array(z.string().max(1_000)).max(500), symbols: z.array(z.string().max(300)).max(500) }).strict(),
}).strict();
export type RecordSide = z.infer<typeof recordSideSchema>;

export const conflictRecordSchema = z.object({
  repository: z.string().min(1).max(500),
  /** The commit both sides are judged against: the base the later side started from. */
  base: z.string().regex(SHA),
  /** What the sides disagree on (`questionOf`). */
  question: z.string().min(1).max(200),
  sides: z.object({ a: recordSideSchema, b: recordSideSchema }).strict(),
}).strict();
export type ConflictRecord = z.infer<typeof conflictRecordSchema>;

/**
 * The question two sides disagree on, as a stable key: the files both change (every file either changes when they
 * share none), with the symbols both name. The same overlap is the same question, whichever side is `a`.
 */
export function questionOf(a: RecordSide["anchors"], b: RecordSide["anchors"]): string {
  const shared = (left: readonly string[], right: readonly string[]) => {
    const other = new Set(right);
    const both = left.filter(item => other.has(item));
    return [...new Set(both.length > 0 ? both : [...left, ...right])].sort();
  };
  const files = shared(a.files, b.files);
  const symbols = a.symbols.filter(symbol => b.symbols.includes(symbol));
  const digest = createHash("sha256").update(JSON.stringify({ files, symbols: [...new Set(symbols)].sort() })).digest("hex");
  return `overlap:${digest.slice(0, 24)}`;
}

/** Pull request and issue links, `owner/name#12`, `GH-12` and `#12`: references from one side to another. */
const CROSS_REFERENCE = /https?:\/\/(?:www\.)?github\.com\/[\w.-]+\/[\w.-]+\/(?:pull|issues|discussions)\/\d+[^\s)>\]]*|\b[\w.-]+\/[\w.-]+#\d+\b|\bGH-\d+\b|(?<![\w&])#\d+\b/giu;
/** References in parentheses, `(#12)` or `(#12, #14)`, removed with their parentheses so none are left empty. */
const PARENTHESISED_REFERENCES = new RegExp(String.raw`\(\s*(?:${CROSS_REFERENCE.source})(?:\s*[,;]\s*(?:${CROSS_REFERENCE.source}))*\s*\)`, "giu");
/** Wording that says how the two relate or how one ended: supersedes, in favour of, duplicate of, closing, merged. */
const OUTCOME_WORDING = /\b(?:supersed\w*|in\s+favou?r\s+of|duplicat\w*|replac(?:e|es|ed|ing)\s+(?:the\s+)?(?:earlier|previous|other|older|existing)\s+(?:pr|pull|attempt|approach|implementation)|alternative\s+to|instead\s+of\s+(?:the\s+)?(?:earlier|previous|other)|competing|clos(?:e|es|ed|ing)\s+(?:this|it|the|in|as|because|for)|merged|obsolete[sd]?|follow[- ]?up\s+(?:to|of|on)|continu\w+\s+(?:of|from)|based\s+on\s+(?:the\s+)?(?:earlier|previous|other|work\s+in))\b/iu;
const HTML_COMMENT = /<!--[\s\S]*?-->/gu;
const MAX_INTENT = 8_000;

/** `text` without references to pull requests and issues (links, `owner/name#12`, `GH-12`, `#12`): for diffs, whose wording is code. */
export function stripCrossReferences(text: string): string {
  return text.replace(PARENTHESISED_REFERENCES, "").replace(CROSS_REFERENCE, "");
}

/**
 * A side's description fit for the resolver: no HTML comments (templates' instructions), no sentence that says how it
 * relates to other work or how it ended, and no cross-reference to a pull request or issue. The resolver never sees
 * outcomes, so the description must not carry one.
 */
export function sanitizeIntent(text: string): string {
  const lines = text.replace(HTML_COMMENT, "").replace(/\r\n?/gu, "\n").split("\n");
  const kept: string[] = [];
  for (const line of lines) {
    const sentences = line.split(/(?<=[.!?])\s+/u).filter(sentence => !OUTCOME_WORDING.test(sentence));
    const cleaned = stripCrossReferences(sentences.join(" ")).replace(/[ \t]{2,}/gu, " ").trimEnd();
    // A line reduced to a keyword and punctuation ("Fixes", "Closes:", "- Related:") said only what was removed.
    if (cleaned.trim() !== "" && /^[\s>*_-]*(?:fix(?:es|ed)?|clos(?:es|ed)?|resolv(?:es|ed)?|related(?:\s+to)?|refs?|see(?:\s+also)?|part\s+of)?[\s:,.*_-]*$/iu.test(cleaned)) continue;
    kept.push(line.trim() === "" ? "" : cleaned);
  }
  return kept.join("\n").replace(/\n{3,}/gu, "\n\n").trim().slice(0, MAX_INTENT);
}
