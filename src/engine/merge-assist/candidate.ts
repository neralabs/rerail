/**
 * Merge Assist's candidate rule: one decision a developer made while steering their coding agent, written to
 * `.rerail/candidates/<name>.json` on their branch so that it travels with the branch's commits. Candidates are read
 * from other people's branches, so everything here treats a candidate as untrusted input: a strict schema, size and
 * count limits, a fixed id shape, relative scope globs, and decision text cleaned of control characters and links.
 * See docs/merge-assist-prd.md §6–7.
 */
import { createHash, randomBytes } from "node:crypto";
import { z } from "zod";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { stableJsonText } from "../stable-json.js";

export const CANDIDATE_FORMAT = "rerail.candidate/v1";
/** Where candidates live, relative to the top of a working tree or Git tree. */
export const CANDIDATE_DIRECTORY = ".rerail/candidates";
/** How many candidates are read from one branch; more are reported, not read. */
export const MAX_CANDIDATES_PER_BRANCH = 64;
/** The largest candidate file read, in bytes. */
export const MAX_CANDIDATE_BYTES = 16 * 1024;
/** The longest decision, in characters, after cleaning. */
export const MAX_DECISION_LENGTH = 280;
const MAX_SCOPE_PATTERNS = 16;
const MAX_PATTERN_LENGTH = 200;

export const CANDIDATE_ID_PATTERN = /^candidate:[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const FILE_NAME_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?\.json$/;

export const CANDIDATE_GENERALITIES = ["branch", "repository"] as const;
export type CandidateGenerality = (typeof CANDIDATE_GENERALITIES)[number];
export const CANDIDATE_STATES = ["active", "rejected"] as const;
export type CandidateState = (typeof CANDIDATE_STATES)[number];

/**
 * Decision text as it may be shown to a person or an agent: control and format characters removed, Markdown links
 * reduced to their label, bare URLs dropped, whitespace collapsed.
 */
export function cleanDecision(text: string): string {
  return text
    .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi, "")
    .replace(/\s+/g, " ")
    .trim();
}

const decisionSchema = z.string()
  .transform(cleanDecision)
  .pipe(z.string().min(1, "the decision is empty").max(MAX_DECISION_LENGTH, `the decision is longer than ${MAX_DECISION_LENGTH} characters`));

/** A repository-relative glob: no absolute path, no `..` segment, no backslash or NUL. */
export const scopePatternSchema = z.string()
  .min(1)
  .max(MAX_PATTERN_LENGTH)
  .refine(pattern => !pattern.startsWith("/") && !/^[A-Za-z]:/.test(pattern), "a scope pattern is relative to the repository")
  .refine(pattern => !pattern.split("/").includes(".."), "a scope pattern stays inside the repository")
  .refine(pattern => !/[\\\0]/.test(pattern), "a scope pattern uses / and no NUL");

const authorSchema = z.string().regex(/^[\p{L}\p{N}][\p{L}\p{N} ._@-]{0,63}$/u, "the author is a login or name of up to 64 letters, digits, spaces and . _ @ -");

const candidateIdSchema = z.string().regex(CANDIDATE_ID_PATTERN, "a candidate id is candidate:<lowercase letters, digits and dashes>");

export const candidateSchema = z.object({
  format: z.literal(CANDIDATE_FORMAT),
  id: candidateIdSchema,
  decision: decisionSchema,
  generality: z.enum(CANDIDATE_GENERALITIES),
  scope: z.array(scopePatternSchema).min(1).max(MAX_SCOPE_PATTERNS),
  author: authorSchema,
  createdAt: z.iso.datetime(),
  state: z.enum(CANDIDATE_STATES),
  rule: codeRuleSchema.optional(),
  supersedes: candidateIdSchema.nullable(),
}).strict().superRefine((candidate, context) => {
  const rule = candidate.rule as CodeRule | undefined;
  if (rule === undefined) return;
  if (rule.id !== candidate.id) context.addIssue({ code: "custom", path: ["rule", "id"], message: "the rule's id is the candidate's id" });
  if (rule.mode !== "steer") context.addIssue({ code: "custom", path: ["rule", "mode"], message: "a candidate's rule steers; it never logs only or blocks" });
});

export type Candidate = z.infer<typeof candidateSchema>;

export type CandidateParse =
  | { readonly ok: true; readonly candidate: Candidate }
  | { readonly ok: false; readonly problem: string };

function issuesText(error: z.ZodError): string {
  return error.issues.map(issue => (issue.path.length === 0 ? issue.message : `${issue.path.map(String).join(".")}: ${issue.message}`)).join("; ");
}

/** Parse one candidate file's text; never throws. */
export function parseCandidateText(text: string): CandidateParse {
  if (Buffer.byteLength(text, "utf8") > MAX_CANDIDATE_BYTES) return { ok: false, problem: `larger than ${MAX_CANDIDATE_BYTES} bytes` };
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { ok: false, problem: "not JSON" };
  }
  const parsed = candidateSchema.safeParse(value);
  return parsed.success ? { ok: true, candidate: parsed.data } : { ok: false, problem: issuesText(parsed.error) };
}

/** The file text Rerail writes: stable key order, two-space indent, a trailing newline. */
export function serializeCandidate(candidate: Candidate): string {
  const stable = stableJsonText(candidate);
  if (stable === undefined) throw new Error(`candidate ${candidate.id} is not JSON representable`);
  return `${JSON.stringify(JSON.parse(stable), null, 2)}\n`;
}

/** The content hash that identifies one version of a candidate. */
export function candidateHash(candidate: Candidate): string {
  return createHash("sha256").update(serializeCandidate(candidate)).digest("hex");
}

/** `candidate:retries-3f9a1c` → `retries-3f9a1c.json`. */
export function candidateFileName(id: string): string {
  if (!CANDIDATE_ID_PATTERN.test(id)) throw new Error(`not a candidate id: ${id}`);
  return `${id.slice("candidate:".length)}.json`;
}

/** Whether a name under the candidate folder is one Rerail reads. */
export function isCandidateFileName(name: string): boolean {
  return FILE_NAME_PATTERN.test(name);
}

/** A new id from the decision's first words and six random hex digits. */
export function newCandidateId(decision: string, random: () => string = () => randomBytes(3).toString("hex")): string {
  const slug = cleanDecision(decision)
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .split(" ")
    .filter(word => word.length > 0)
    .slice(0, 6)
    .join("-")
    .slice(0, 48)
    .replace(/-+$/, "");
  const suffix = random().toLowerCase().replace(/[^a-z0-9]/g, "").slice(0, 8) || "0";
  return `candidate:${slug === "" ? "decision" : slug}-${suffix}`;
}
