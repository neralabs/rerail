/**
 * Merge Assist's conflict policy: `.rerail/policy.yaml`, versioned and reviewed like code. When two pieces of in-flight
 * work conflict, its steps run in order and the first that decides wins: precedent (a decision in force, or an earlier
 * verdict in the log on the same question), hard checks (a side whose head fails the tests or type check, or breaks a
 * public contract it did not declare, loses), the measurable criteria (compatibility with existing callers, then blast
 * radius, then benchmarks), and the judge panel. What nothing decides goes to a person, and so does a conflict in a
 * class the policy reserves (public API, security, data model) once the first three steps have not decided it.
 *
 * This module parses the file and holds each step's rule as a pure function of the evidence; the resolver gathers the
 * evidence (Git, the checks, the judges). The policy that governs a conflict is the one at the base both sides start
 * from, never either side's own.
 */
import { createHash } from "node:crypto";
import { parse as parseYaml } from "yaml";
import { z } from "zod";
import { runGit } from "../git.js";

export const POLICY_PATH = ".rerail/policy.yaml";

/** The steps a policy can run, in the only order they run in; escalation to a person always follows them. */
export const POLICY_STEPS = ["precedent", "hard-checks", "criteria", "panel"] as const;
export type PolicyStep = (typeof POLICY_STEPS)[number];
/** What settled a conflict: one of the steps, or escalation to a person. */
export type DecidingStep = PolicyStep | "escalate";

/** Classes of conflict a policy can keep for a person once precedent, hard checks and criteria have not decided. */
export const RESERVED_CLASSES = ["public-api", "security", "data-model"] as const;
export type ReservedClass = (typeof RESERVED_CLASSES)[number];

/** What each panel judge scores both sides on. */
export const RUBRIC = ["conventions", "simplicity", "compatibility", "goal"] as const;
export type RubricItem = (typeof RUBRIC)[number];

export const CHECK_KINDS = ["tests", "typecheck"] as const;
export type CheckKind = (typeof CHECK_KINDS)[number];

/** The two sides of a conflict. Neither name says which came first. */
export const SIDES = ["a", "b"] as const;
export type SideId = (typeof SIDES)[number];
export type Verdict = SideId | "escalated";

export function otherSide(side: SideId): SideId {
  return side === "a" ? "b" : "a";
}

const CHECK_NAME = /^[a-z0-9][a-z0-9-]{0,39}$/u;
const MAX_COMMAND = 2_000;

const checkSchema = z.object({
  name: z.string().regex(CHECK_NAME),
  kind: z.enum(CHECK_KINDS),
  run: z.string().min(1).max(MAX_COMMAND),
}).strict();

const benchmarkSchema = z.object({
  name: z.string().regex(CHECK_NAME),
  /** A command whose output's last number is the measurement. */
  run: z.string().min(1).max(MAX_COMMAND),
  better: z.enum(["lower", "higher"]),
  /** The relative difference that decides, as a fraction of the larger measurement. */
  minChange: z.number().positive().max(10).default(0.05),
}).strict();

const panelSchema = z.object({
  judges: z.number().int().min(1).max(9).default(3),
  /** Votes for one side a verdict needs; every judge's by default. */
  agreement: z.number().int().min(1).max(9).optional(),
}).strict()
  .refine(panel => panel.agreement === undefined || panel.agreement <= panel.judges, "panel.agreement is at most panel.judges")
  .transform(panel => ({ judges: panel.judges, agreement: panel.agreement ?? panel.judges }));

export const policySchema = z.object({
  version: z.literal(1),
  steps: z.array(z.enum(POLICY_STEPS))
    .refine(steps => steps.every((step, index) => index === 0 || POLICY_STEPS.indexOf(step) > POLICY_STEPS.indexOf(steps[index - 1]!)), "steps are distinct and in the order precedent, hard-checks, criteria, panel")
    .default([...POLICY_STEPS]),
  checks: z.object({
    /** Run once in each side's tree before its checks (an install); a failing setup makes every check unknown. */
    setup: z.string().min(1).max(MAX_COMMAND).nullable().default(null),
    run: z.array(checkSchema).max(8).default([])
      .refine(checks => new Set(checks.map(check => check.name)).size === checks.length, "check names are distinct"),
    timeoutMinutes: z.number().int().min(1).max(240).default(30),
  }).strict().prefault({}),
  contracts: z.object({
    /** Whether an undeclared break of a public contract loses at the hard checks. */
    enabled: z.boolean().default(true),
    /** Names whose break the repository has declared, beyond what a side's description and changelog declare. */
    declared: z.array(z.string().min(1).max(200)).max(64).default([]),
  }).strict().prefault({}),
  criteria: z.object({
    /** The side whose changed declarations leave fewer existing callers to adapt wins by this many or more. */
    compatibility: z.object({
      enabled: z.boolean().default(true),
      minDifference: z.number().int().min(1).default(1),
    }).strict().prefault({}),
    /** The side that changes fewer source lines wins when the other changes `ratio` times as many and at least `minLines`. */
    blastRadius: z.object({
      enabled: z.boolean().default(true),
      ratio: z.number().min(1).default(3),
      minLines: z.number().int().min(0).default(100),
    }).strict().prefault({}),
    benchmarks: z.array(benchmarkSchema).max(8).default([]),
  }).strict().prefault({}),
  panel: panelSchema.prefault({}),
  reserve: z.array(z.enum(RESERVED_CLASSES)).default([...RESERVED_CLASSES])
    .refine(classes => new Set(classes).size === classes.length, "reserved classes are distinct"),
}).strict();

export type Policy = z.output<typeof policySchema>;
export type PolicyCheck = Policy["checks"]["run"][number];
export type PolicyBenchmark = Policy["criteria"]["benchmarks"][number];

/** The policy of a repository with no `.rerail/policy.yaml`: every step, a unanimous panel of 3, every class reserved. */
export function defaultPolicy(): Policy {
  return policySchema.parse({ version: 1 });
}

export type PolicyParse = { readonly ok: true; readonly policy: Policy } | { readonly ok: false; readonly problems: string[] };

/** `text` as a policy: YAML (duplicate keys refused) that matches the schema; every problem otherwise. */
export function parsePolicy(text: string): PolicyParse {
  let value: unknown;
  try {
    value = parseYaml(text, { uniqueKeys: true, maxAliasCount: 0 });
  } catch (error) {
    return { ok: false, problems: [`not YAML: ${error instanceof Error ? error.message.split("\n")[0] : String(error)}`] };
  }
  const parsed = policySchema.safeParse(value ?? {});
  if (parsed.success) return { ok: true, policy: parsed.data };
  return { ok: false, problems: parsed.error.issues.map(issue => `${issue.path.length === 0 ? "policy" : issue.path.join(".")}: ${issue.message}`) };
}

export type LoadedPolicy =
  | { readonly source: "file" | "default"; readonly policy: Policy }
  | { readonly source: "invalid"; readonly problems: string[] };

/** The policy at `revision` (the base a conflict is judged from): its file, the default when it has none, or why it is invalid. */
export function loadPolicy(root: string, revision: string): LoadedPolicy {
  if (revision.startsWith("-") || /[\0\n:]/u.test(revision)) return { source: "invalid", problems: [`not a revision: ${revision}`] };
  const result = runGit(root, ["cat-file", "blob", `${revision}:${POLICY_PATH}`], { maxBuffer: 1024 * 1024 });
  if (result.status !== 0) return { source: "default", policy: defaultPolicy() };
  const parsed = parsePolicy(result.stdout);
  return parsed.ok ? { source: "file", policy: parsed.policy } : { source: "invalid", problems: parsed.problems };
}

// ------------------------------------------------------------------------------------------------- conflict record

const SHA = /^[0-9a-f]{40,64}$/u;

export const conflictSideSchema = z.object({
  /** The side's ref: a branch, or a pull request's `refs/pull/<n>/head`. */
  ref: z.string().min(1).max(500),
  sha: z.string().regex(SHA),
  /** What the side says it does: its title and description, without cross-references (`sanitizeIntent`). */
  intent: z.string().max(20_000),
  /** The files and symbols the side's half of the conflict is in. */
  anchors: z.object({ files: z.array(z.string().max(1_000)).max(500), symbols: z.array(z.string().max(300)).max(500) }).strict(),
}).strict();
export type ConflictSide = z.infer<typeof conflictSideSchema>;

export const conflictRecordSchema = z.object({
  repository: z.string().min(1).max(500),
  /** The commit both sides are judged against: the base the later side started from. */
  base: z.string().regex(SHA),
  /** What the sides disagree on (`questionOf`): earlier verdicts on the same question are precedent. */
  question: z.string().min(1).max(200),
  sides: z.object({ a: conflictSideSchema, b: conflictSideSchema }).strict(),
}).strict();
export type ConflictRecord = z.infer<typeof conflictRecordSchema>;

/**
 * The question two sides disagree on, as a stable key: the files both change (every file either changes when they
 * share none), with the symbols both name. The same overlap is the same question, whichever side is `a`.
 */
export function questionOf(a: ConflictSide["anchors"], b: ConflictSide["anchors"]): string {
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

// ---------------------------------------------------------------------------------------------------- the steps

/** One step's outcome: the side it picked, or null with why it could not. */
export interface StepResult {
  readonly step: DecidingStep;
  readonly winner: SideId | null;
  readonly reason: string;
}

export const CHECK_STATUSES = ["pass", "fail", "unknown"] as const;
export type CheckStatus = (typeof CHECK_STATUSES)[number];

/** One check run on one side's head: pass, fail, or unknown (could not run, timed out, setup failed). */
export interface CheckResult {
  readonly name: string;
  readonly kind: CheckKind;
  readonly status: CheckStatus;
  /** Why the status is what it is: the exit code, the timeout, the setup's failure. */
  readonly detail: string;
  /** The end of the output. */
  readonly log: string;
}

/** An exported declaration a side removed or changed in a way existing callers notice. */
export interface ContractBreak {
  readonly path: string;
  readonly name: string;
  readonly change: "removed" | "changed";
  readonly before: string;
  readonly after: string | null;
  /** Visible outside the package: exported, and not under a test, internal or example path. */
  readonly public: boolean;
  /** Files at the base, other than the declaring one, that refer to it: its existing callers and tests. */
  readonly dependents: readonly string[];
  /** The side's description or changelog, or the policy, declares the break. */
  readonly declared: boolean;
}

/** A break the hard checks hold against a side: public, depended on, and not declared. */
export function undeclaredBreak(change: ContractBreak): boolean {
  return change.public && change.dependents.length > 0 && !change.declared;
}

/**
 * Hard checks: a side loses when it fails a check the other side passes, or when it alone breaks a public contract
 * without declaring it. Decided only when exactly one side has something against it; a check either side could not
 * run counts for neither.
 */
export function decideHardChecks(
  checks: Readonly<Record<SideId, readonly CheckResult[]>> | null,
  contracts: Readonly<Record<SideId, readonly ContractBreak[]>> | null,
): StepResult {
  const against: Record<SideId, string[]> = { a: [], b: [] };
  if (checks !== null) {
    for (const side of SIDES) {
      for (const result of checks[side]) {
        const theirs = checks[otherSide(side)].find(other => other.name === result.name);
        if (result.status === "fail" && theirs?.status === "pass") against[side].push(`fails ${result.name} (${result.kind}), which the other side passes`);
      }
    }
  }
  if (contracts !== null) {
    for (const side of SIDES) {
      const breaks = contracts[side].filter(undeclaredBreak);
      if (breaks.length > 0) {
        const names = breaks.slice(0, 3).map(change => `${change.name} (${change.change}, ${change.dependents.length} dependent files)`).join(", ");
        against[side].push(`breaks the public contract of ${names}${breaks.length > 3 ? ` and ${breaks.length - 3} more` : ""} without declaring it`);
      }
    }
  }
  const losers = SIDES.filter(side => against[side].length > 0);
  if (losers.length === 1) {
    const loser = losers[0]!;
    return { step: "hard-checks", winner: otherSide(loser), reason: `Side ${loser} ${against[loser].join("; ")}.` };
  }
  if (losers.length === 2) return { step: "hard-checks", winner: null, reason: `Both sides fail: a ${against.a.join("; ")}; b ${against.b.join("; ")}.` };
  const unknown = checks === null ? ["no checks ran"] : SIDES.flatMap(side => checks[side].filter(result => result.status === "unknown").map(result => `${result.name} unknown on ${side}`));
  return { step: "hard-checks", winner: null, reason: `Neither side fails a check the other passes or breaks a public contract undeclared${unknown.length > 0 ? ` (${unknown.join(", ")})` : ""}.` };
}

/** What the criteria measure on one side. */
export interface SideMeasures {
  /** Existing callers its changed or removed declarations leave to adapt: (declaration, dependent file) pairs. */
  readonly callersAffected: number;
  /** Source lines it adds and removes, tests and lockfiles left out. */
  readonly sourceLines: number;
  readonly sourceFiles: number;
  /** Each configured benchmark's measurement, null when it could not be measured. */
  readonly benchmarks: Readonly<Record<string, number | null>>;
}

/** Measurable criteria in order: compatibility with existing callers, blast radius, then each benchmark. The first that separates the sides decides. */
export function decideCriteria(criteria: Policy["criteria"], measures: Readonly<Record<SideId, SideMeasures>>): StepResult {
  const { a, b } = measures;
  const notes: string[] = [];
  if (criteria.compatibility.enabled) {
    const difference = Math.abs(a.callersAffected - b.callersAffected);
    if (difference >= criteria.compatibility.minDifference) {
      const winner: SideId = a.callersAffected < b.callersAffected ? "a" : "b";
      return { step: "criteria", winner, reason: `Compatibility: side ${winner} leaves ${measures[winner].callersAffected} existing callers to adapt, side ${otherSide(winner)} ${measures[otherSide(winner)].callersAffected}.` };
    }
    notes.push(`callers to adapt ${a.callersAffected} against ${b.callersAffected}`);
  }
  if (criteria.blastRadius.enabled) {
    const larger = Math.max(a.sourceLines, b.sourceLines);
    const smaller = Math.min(a.sourceLines, b.sourceLines);
    if (larger >= criteria.blastRadius.minLines && larger >= criteria.blastRadius.ratio * Math.max(smaller, 1)) {
      const winner: SideId = a.sourceLines < b.sourceLines ? "a" : "b";
      return { step: "criteria", winner, reason: `Blast radius: side ${winner} changes ${measures[winner].sourceLines} source lines in ${measures[winner].sourceFiles} files, side ${otherSide(winner)} ${larger} in ${measures[otherSide(winner)].sourceFiles}.` };
    }
    notes.push(`source lines ${a.sourceLines} against ${b.sourceLines}`);
  }
  for (const benchmark of criteria.benchmarks) {
    const left = a.benchmarks[benchmark.name] ?? null;
    const right = b.benchmarks[benchmark.name] ?? null;
    if (left === null || right === null) {
      notes.push(`${benchmark.name} not measured on both`);
      continue;
    }
    const scale = Math.max(Math.abs(left), Math.abs(right));
    if (scale > 0 && Math.abs(left - right) / scale >= benchmark.minChange) {
      const winner: SideId = (benchmark.better === "lower") === (left < right) ? "a" : "b";
      return { step: "criteria", winner, reason: `Benchmark ${benchmark.name} (${benchmark.better} is better): side a ${left}, side b ${right}.` };
    }
    notes.push(`${benchmark.name} ${left} against ${right}`);
  }
  return { step: "criteria", winner: null, reason: notes.length === 0 ? "No criterion is configured." : `No criterion separates the sides: ${notes.join("; ")}.` };
}

/** A panel's verdict: the side with at least `agreement` votes, if any. Abstentions and failed judges count for neither. */
export function decidePanel(votes: ReadonlyArray<SideId | null>, agreement: number): StepResult {
  const count = (side: SideId) => votes.filter(vote => vote === side).length;
  const tally = `${count("a")} for a, ${count("b")} for b, ${votes.length - count("a") - count("b")} for neither`;
  for (const side of SIDES) {
    if (count(side) >= agreement) return { step: "panel", winner: side, reason: `The panel agrees on side ${side} (${tally}; ${agreement} needed).` };
  }
  return { step: "panel", winner: null, reason: `The panel does not agree (${tally}; ${agreement} needed).` };
}

/** The reserved classes either side's change falls in, of those the policy reserves. */
export function reservedHits(reserve: readonly ReservedClass[], classes: Readonly<Record<SideId, readonly ReservedClass[]>>): ReservedClass[] {
  return reserve.filter(reserved => classes.a.includes(reserved) || classes.b.includes(reserved));
}
