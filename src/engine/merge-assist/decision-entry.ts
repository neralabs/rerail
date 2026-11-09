/**
 * A decision ledger entry: one person's verdict on one conflict, as the file `.rerail/decisions/<id>.json` on the
 * branch whose person decided it (format `rerail.decision/v2`). It is committed and pushed with the code, so it is
 * reviewed in the pull request and final only once it merges; the ledger is a view computed from where entries sit
 * (`ledger.ts`). An entry records the question, both sides with a fingerprint of each side's position, the verdict, who
 * decided, and the decision the branch now follows. Its id is a hash of its content, so it never changes once written.
 * Entries are read from other people's branches: the schema is strict and every field bounded.
 */
import { createHash } from "node:crypto";
import { z } from "zod";
import { stableJsonText } from "../stable-json.js";
import { CANDIDATE_ID_PATTERN } from "./candidate.js";

export const DECISION_ENTRY_FORMAT = "rerail.decision/v2";
/** Where entries live, relative to the top of a working tree or Git tree. */
export const DECISIONS_DIRECTORY = ".rerail/decisions";
/** The largest entry file read, in bytes. */
export const MAX_DECISION_ENTRY_BYTES = 16 * 1024;
/** How many entries are read from one tree; more are reported, not read. */
export const MAX_DECISION_ENTRIES_PER_TREE = 256;
/** The longest decision text an entry keeps. */
export const MAX_RESULT_TEXT = 2_000;

/** a: side a (the branch that records the entry) wins; b: side b wins; new: a new decision for both. */
export const DECISION_VERDICTS = ["a", "b", "new"] as const;
export type DecisionVerdict = (typeof DECISION_VERDICTS)[number];
/** The resolver's steps that decide without a person, as the benchmark records them. */
export const POLICY_DECIDERS = ["hard-checks", "criteria", "panel"] as const;

const ENTRY_ID = /^[0-9a-f]{32}$/u;
const SHA = /^[0-9a-f]{40,64}$/u;
const DIGEST = /^[0-9a-f]{64}$/u;
const FILE_NAME = /^[0-9a-f]{32}\.json$/u;

const entryIdSchema = z.string().regex(ENTRY_ID);
const candidateIdSchema = z.string().regex(CANDIDATE_ID_PATTERN);
const pathSchema = z.string().min(1).max(1_000)
  .refine(path => !path.startsWith("/") && !path.split("/").includes("..") && !/[\\\0]/u.test(path), "a path is relative to the repository and stays inside it");

/**
 * What a side's fingerprint is taken over: its hunks in `path` at the contested base lines (an overlap), or one of its
 * candidates (a contradiction between decisions).
 */
export const positionSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("hunks"),
    path: pathSchema,
    lines: z.array(z.object({ lo: z.number().nonnegative(), hi: z.number().nonnegative() }).strict()).min(1).max(64),
  }).strict(),
  z.object({ kind: z.literal("candidate"), candidateId: candidateIdSchema }).strict(),
]);
export type Position = z.infer<typeof positionSchema>;

export const decisionSideSchema = z.object({
  /** The side's ref as the recording checkout knew it: a branch, or a pull request's `refs/pull/<n>/head`. */
  ref: z.string().min(1).max(500),
  label: z.string().min(1).max(200),
  sha: z.string().regex(SHA),
  author: z.string().min(1).max(200).nullable(),
  /** sha256 of the side's position: its normalised hunk text at the contested lines, or its candidate's content hash. */
  fingerprint: z.string().regex(DIGEST),
  at: positionSchema,
}).strict();
export type DecisionSide = z.infer<typeof decisionSideSchema>;

export const followsSchema = z.object({ ref: z.string().min(1).max(500), candidateId: candidateIdSchema }).strict();
export type Follows = z.infer<typeof followsSchema>;

const entryFields = {
  format: z.literal(DECISION_ENTRY_FORMAT),
  /** The key two conflicts share when they are the same disagreement (`questionOf`). */
  question: z.string().min(1).max(200),
  title: z.string().min(1).max(300),
  /** The commit both positions' hunks are taken against: where the recording branch left its base. */
  base: z.string().regex(SHA),
  sides: z.object({ a: decisionSideSchema, b: decisionSideSchema }).strict(),
  verdict: z.enum(DECISION_VERDICTS),
  decidedBy: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("person"), name: z.string().min(1).max(200) }).strict(),
    z.object({ kind: z.literal("precedent"), entry: entryIdSchema }).strict(),
    z.object({ kind: z.literal("policy"), step: z.enum(POLICY_DECIDERS) }).strict(),
  ]),
  /** The decision the recording branch follows: an amendment for `new`, else the winner's design. */
  result: z.object({
    candidateId: candidateIdSchema.nullable(),
    kind: z.enum(["decision", "amendment"]),
    text: z.string().min(1).max(MAX_RESULT_TEXT),
    rule: pathSchema.nullable(),
    follows: followsSchema.nullable(),
  }).strict(),
  /** An earlier entry on the same question this one replaces. */
  supersedes: entryIdSchema.nullable(),
  recordedAt: z.iso.datetime(),
};

const amendsWhenNew = (entry: { verdict: DecisionVerdict; result: { kind: "decision" | "amendment" } }): boolean => (entry.verdict === "new") === (entry.result.kind === "amendment");
const AMENDS_WHEN_NEW = "a new decision is an amendment, and a side's verdict a decision";

/** An entry before its id is set: what the id is hashed over. */
export const decisionDraftSchema = z.object(entryFields).strict().refine(amendsWhenNew, AMENDS_WHEN_NEW);
export type DecisionDraft = z.infer<typeof decisionDraftSchema>;

/** The first 32 hex digits of sha256 over the canonical JSON (keys sorted) of the entry without its id. */
export function decisionEntryId(entry: DecisionDraft & { readonly id?: string }): string {
  const { id: _id, ...draft } = entry;
  const canonical = stableJsonText(draft);
  if (canonical === undefined) throw new TypeError("a decision entry is not JSON representable");
  return createHash("sha256").update(canonical).digest("hex").slice(0, 32);
}

export const decisionEntrySchema = z.object({ ...entryFields, id: entryIdSchema }).strict()
  .refine(amendsWhenNew, AMENDS_WHEN_NEW)
  .refine(entry => entry.id === decisionEntryId(entry), "the id is the hash of the entry's content");
export type DecisionEntry = z.infer<typeof decisionEntrySchema>;

/** `draft` with its id; throws when it is not a valid entry. */
export function sealDecisionEntry(draft: DecisionDraft): DecisionEntry {
  return decisionEntrySchema.parse({ ...draft, id: decisionEntryId(draft) });
}

/** `<id>.json`. */
export function decisionFileName(id: string): string {
  if (!ENTRY_ID.test(id)) throw new Error(`not a decision entry id: ${id}`);
  return `${id}.json`;
}

/** Whether a name under the decisions folder is one Rerail reads. */
export function isDecisionFileName(name: string): boolean {
  return FILE_NAME.test(name);
}

/** The file text Rerail writes: stable key order, two-space indent, a trailing newline. */
export function serializeDecisionEntry(entry: DecisionEntry): string {
  const stable = stableJsonText(entry);
  if (stable === undefined) throw new TypeError(`decision entry ${entry.id} is not JSON representable`);
  return `${JSON.stringify(JSON.parse(stable), null, 2)}\n`;
}

/** One file's text as an entry, or the problem with it; the file's name must be the one its id gives. Never throws. */
export function decisionEntryFromText(name: string, text: string): { entry: DecisionEntry } | { problem: string } {
  if (Buffer.byteLength(text, "utf8") > MAX_DECISION_ENTRY_BYTES) return { problem: `${name}: larger than ${MAX_DECISION_ENTRY_BYTES} bytes` };
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { problem: `${name}: not JSON` };
  }
  const parsed = decisionEntrySchema.safeParse(value);
  if (!parsed.success) return { problem: `${name}: ${parsed.error.issues.map(issue => (issue.path.length === 0 ? issue.message : `${issue.path.join(".")}: ${issue.message}`)).join("; ")}` };
  if (decisionFileName(parsed.data.id) !== name) return { problem: `${name}: its id ${parsed.data.id} belongs in ${decisionFileName(parsed.data.id)}` };
  return { entry: parsed.data };
}

/** The side that won, or null for a new decision. */
export function winnerOf(entry: Pick<DecisionEntry, "verdict">): "a" | "b" | null {
  return entry.verdict === "new" ? null : entry.verdict;
}

/**
 * Who won, as two entries compare: the winner's position (its fingerprint), or for a new decision its text. Two
 * entries on one question with different winners disagree.
 */
export function winnerKey(entry: Pick<DecisionEntry, "verdict" | "sides" | "result">): string {
  const winner = winnerOf(entry);
  return winner === null ? `new:${createHash("sha256").update(entry.result.text).digest("hex")}` : entry.sides[winner].fingerprint;
}

/** Who decided, as a person reads it: their name, `precedent`, or the policy step. */
export function deciderName(entry: Pick<DecisionEntry, "decidedBy">): string {
  const by = entry.decidedBy;
  return by.kind === "person" ? by.name : by.kind === "precedent" ? "precedent" : by.step;
}
