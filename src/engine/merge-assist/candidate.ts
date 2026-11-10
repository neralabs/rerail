/**
 * Merge Assist's candidate rule: one decision a developer made while steering their coding agent, written to
 * `.rerail/candidates/<name>.json` on their branch so that it travels with the branch's commits. Candidates are read
 * from other people's branches, so everything here treats a candidate as untrusted input: a strict schema, size and
 * count limits, a fixed id shape, relative scope globs, and decision text cleaned of control characters and links.
 * See docs/merge-assist-v1-build.md §6–7.
 */
import { createHash, randomBytes } from "node:crypto";
import { z } from "zod";
import { globPattern } from "../glob-pattern.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { stableJsonText } from "../stable-json.js";
import { MAX_SIGNALS, signalSchema } from "./signals.js";

export const CANDIDATE_FORMAT = "rerail.candidate/v1";
/** Where candidates live, relative to the top of a working tree or Git tree. */
export const CANDIDATE_DIRECTORY = ".rerail/candidates";
/** How many candidates are read from one branch; more are reported, not read. */
export const MAX_CANDIDATES_PER_BRANCH = 64;
/** The largest candidate file read, in bytes. */
export const MAX_CANDIDATE_BYTES = 16 * 1024;
/** The longest decision, in characters, after cleaning. */
export const MAX_DECISION_LENGTH = 280;
/** The longest message a candidate's rule steers with. */
export const MAX_RULE_MESSAGE_LENGTH = 200;
/** The most scope globs a candidate holds. */
export const MAX_SCOPE_PATTERNS = 16;
const MAX_PATTERN_LENGTH = 200;
/** The most `*` and `**` tokens one scope pattern holds. */
const MAX_PATTERN_WILDCARDS = 4;

export const CANDIDATE_ID_PATTERN = /^candidate:[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const FILE_NAME_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?\.json$/;

export const CANDIDATE_GENERALITIES = ["branch", "repository"] as const;
export type CandidateGenerality = (typeof CANDIDATE_GENERALITIES)[number];
export const CANDIDATE_STATES = ["active", "rejected"] as const;
export type CandidateState = (typeof CANDIDATE_STATES)[number];

/**
 * Decision text as it may be shown to a person or an agent: control and format characters removed, Markdown links
 * reduced to their label, bare URLs dropped, whitespace collapsed. Cleaning is repeated until nothing changes, since
 * removing one link can leave another (`[[a](b)](c)`), so clean text cleans to itself.
 */
export function cleanDecision(text: string): string {
  let cleaned = text;
  for (;;) {
    const next = cleaned
      .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi, "")
      .replace(/\s+/g, " ")
      .trim();
    // After the first pass every change shortens the text, so this ends.
    if (next === cleaned) return next;
    cleaned = next;
  }
}

const decisionSchema = z.string()
  .transform(cleanDecision)
  .pipe(z.string().min(1, "the decision is empty").max(MAX_DECISION_LENGTH, `the decision is longer than ${MAX_DECISION_LENGTH} characters`));

/** Whether Rerail's glob matcher compiles `pattern`: braces closed and not nested. */
function compiles(pattern: string): boolean {
  try {
    globPattern([pattern]);
    return true;
  } catch {
    return false;
  }
}

/**
 * A repository-relative glob: no absolute path, no `..` segment, no backslash or NUL, at most
 * {@link MAX_PATTERN_WILDCARDS} `*` and `**` tokens, and one the glob matcher compiles. Peer scopes are matched on every
 * scan, so a pattern that could not compile, or one built to be slow, is refused when the candidate is read.
 */
export const scopePatternSchema = z.string()
  .min(1)
  .max(MAX_PATTERN_LENGTH)
  .refine(pattern => !pattern.startsWith("/") && !/^[A-Za-z]:/.test(pattern), "a scope pattern is relative to the repository")
  .refine(pattern => !pattern.split("/").includes(".."), "a scope pattern stays inside the repository")
  .refine(pattern => !/[\\\0]/.test(pattern), "a scope pattern uses / and no NUL")
  .refine(pattern => (pattern.match(/\*\*|\*/g)?.length ?? 0) <= MAX_PATTERN_WILDCARDS, `a scope pattern has at most ${MAX_PATTERN_WILDCARDS} * or ** wildcards`)
  .refine(compiles, "a scope pattern closes each { and does not nest them");

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
  /** Code fragments whose presence in new code means the decision is at stake. */
  signals: z.array(signalSchema).max(MAX_SIGNALS, `a candidate has at most ${MAX_SIGNALS} signals`).optional(),
  supersedes: candidateIdSchema.nullable(),
}).strict().superRefine((candidate, context) => {
  const rule = candidate.rule as CodeRule | undefined;
  if (rule === undefined) return;
  const issue = (path: Array<string | number>, message: string): void => context.addIssue({ code: "custom", path: ["rule", ...path], message });
  if (rule.id !== candidate.id) issue(["id"], "the rule's id is the candidate's id");
  if (rule.mode !== "steer") issue(["mode"], "a candidate's rule steers; it never logs only or blocks");
  if (rule.description !== undefined && rule.description !== candidate.decision) issue(["description"], "the rule's description is the candidate's decision");
  // The rule steers wherever its own scope says; a peer's rule is checked only within the candidate's scope.
  const include = new Set(rule.scope?.include ?? []);
  if (rule.scope === undefined || include.size !== new Set(candidate.scope).size || !candidate.scope.every(pattern => include.has(pattern))) {
    issue(["scope", "include"], "the rule's scope includes exactly the candidate's scope");
  }
  if ((rule.scope?.exclude.length ?? 0) > 0) issue(["scope", "exclude"], "the rule's scope excludes nothing");
  if (rule.clauses.length !== 1) issue(["clauses"], "a candidate's rule has exactly one clause");
  rule.clauses.forEach((clause, index) => {
    const forbid = clause.forbid;
    if (forbid === undefined || !("regex" in forbid || "ast" in forbid)) issue(["clauses", index], "a candidate's rule forbids a regex or an ast pattern");
    const violation = clause.violation;
    if (violation === undefined) return;
    if (violation.message !== cleanDecision(violation.message) || violation.message.length > MAX_RULE_MESSAGE_LENGTH) {
      issue(["clauses", index, "violation", "message"], `the rule's message is clean text of at most ${MAX_RULE_MESSAGE_LENGTH} characters, without links or control characters`);
    }
    if (violation.fix !== undefined) issue(["clauses", index, "violation", "fix"], "a candidate's rule suggests no fix");
  });
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

/**
 * The candidates in force among one branch's: the active ones that no other active candidate there amends
 * (`supersedes` names it). An amendment of a candidate that is missing or rejected is an ordinary decision, and a
 * rejected amendment puts the decision it amended back in force. Amendments that lead back to themselves amend
 * nothing, so a cycle never takes its decisions out of force. Order is kept.
 */
export function candidatesInForce(candidates: readonly Candidate[]): Candidate[] {
  const active = new Map(candidates.filter(candidate => candidate.state === "active").map(candidate => [candidate.id, candidate]));
  /** Whether following `supersedes` from `from` through active candidates comes to `to`. */
  const leadsTo = (from: Candidate, to: string): boolean => {
    const seen = new Set<string>();
    for (let next = from.supersedes; next !== null && !seen.has(next); next = active.get(next)?.supersedes ?? null) {
      if (next === to) return true;
      seen.add(next);
    }
    return false;
  };
  const amended = new Set<string>();
  for (const candidate of active.values()) {
    const target = candidate.supersedes === null ? undefined : active.get(candidate.supersedes);
    if (target !== undefined && target.id !== candidate.id && !leadsTo(target, candidate.id)) amended.add(target.id);
  }
  return candidates.filter(candidate => active.get(candidate.id) === candidate && !amended.has(candidate.id));
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

/**
 * A new id from the decision's first words and six random hex digits. Ids are not checked for collisions here; a
 * caller writing a candidate uses {@link unusedCandidateId}.
 */
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

/** How many fresh ids {@link unusedCandidateId} tries before giving up. */
const ID_ATTEMPTS = 16;

/**
 * A new id for `decision` that `taken` does not report as used, so writing the candidate never replaces another one's
 * file. Throws after {@link ID_ATTEMPTS} taken ids, which with six random hex digits means `taken` answers wrongly.
 */
export function unusedCandidateId(decision: string, taken: (id: string) => boolean, random?: () => string): string {
  for (let attempt = 0; attempt < ID_ATTEMPTS; attempt += 1) {
    const id = newCandidateId(decision, random);
    if (!taken(id)) return id;
  }
  throw new Error(`no unused candidate id after ${ID_ATTEMPTS} attempts`);
}
