import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { join } from "node:path";
import { z } from "zod/v4";
import { STEER_STRATEGIES, type AdapterDelivery } from "../engine/adapter-contract.js";
import { RULE_SOURCE_KINDS } from "../engine/catalog/repository-model.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { SECRET_FILE_FINDING_MESSAGE, type CodeRuleViolation, type SteerSourceOf } from "../engine/code-rule/code-rule.js";
import type { SteerRuleSource } from "../engine/code-rule/steer-origin.js";
import { scrubSecrets } from "../engine/secret-scrub.js";
import { SteerRuleSchema } from "./event-protocol.js";

/*
 * What a session owes its agent while it is closed. A session that closes without its end (quiet past the idle limit,
 * or open when its runtime closes, as at a daemon stop or restart) writes the steers and held findings it has not yet
 * delivered to its ledger as one `owed` entry; the session takes them back when its next event opens it again, and
 * writes the entry again, emptied, once they have gone out. Each keeps what the files it names held as the session last
 * saw them, so one whose file changed while the session was closed is not sent (`changedFiles`).
 */

/**
 * What each file a steer or held finding names (relative to the session's folder) held as the session last saw it: the
 * SHA-256 of its bytes, or null when there was no file to read.
 */
export type FileDigests = Readonly<Record<string, string | null>>;

const FileDigestsSchema = z.record(z.string(), z.string().nullable());

/** A delivery as a session ledger records it. */
export const LedgerDeliverySchema = z.object({
  sessionId: z.string(),
  requested: z.enum(RULE_MODES),
  mechanism: z.enum(["observe", "native-queue", "additional-context", "deferred-context"]),
  content: z.string(),
  ruleIds: z.array(z.string()),
  // Absent in a ledger written before deliveries carried their rules.
  rules: z.array(SteerRuleSchema).default([]),
  strategy: z.enum(STEER_STRATEGIES).optional(),
  toolCallId: z.string().optional(),
  rewound: z.array(z.string()).optional(),
});

const SpanSchema = z.object({
  startByte: z.number().int(),
  endByte: z.number().int(),
  startLine: z.number().int(),
  startColumn: z.number().int(),
  endLine: z.number().int(),
  endColumn: z.number().int(),
});

/** A held finding's violation as the ledger keeps it: what its steer says, not the text a rule captured. */
export type OwedViolation = Omit<CodeRuleViolation, "captures" | "variables">;

const OwedViolationSchema = z.object({
  id: z.string(),
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().optional(),
  span: SpanSchema.optional(),
  fix: z.object({ span: SpanSchema, text: z.string() }).optional(),
});

/**
 * Where a steer or held finding speaks: the sources whose newer revision makes it stale, and what each held as the
 * session last saw it.
 */
export interface OwedPlace {
  readonly scope: readonly string[];
  readonly digests: FileDigests;
}

/** A steer the session had not delivered when it closed. */
export interface OwedSteer extends OwedPlace {
  readonly delivery: AdapterDelivery;
}

/** A finding the session held for its yield digest when it closed, with where its rule comes from. */
export interface OwedFinding extends OwedPlace {
  readonly violation: OwedViolation;
  readonly source: SteerRuleSource | null;
}

export interface OwedEntry {
  readonly type: "owed";
  readonly steers: readonly OwedSteer[];
  readonly findings: readonly OwedFinding[];
}

export const OwedEntrySchema = z.object({
  type: z.literal("owed"),
  steers: z.array(z.object({ delivery: LedgerDeliverySchema, scope: z.array(z.string()), digests: FileDigestsSchema })),
  findings: z.array(z.object({
    violation: OwedViolationSchema,
    source: z.object({ kind: z.enum(RULE_SOURCE_KINDS), packId: z.string().nullable() }).nullable(),
    scope: z.array(z.string()),
    digests: FileDigestsSchema,
  })),
});

/**
 * A held finding as the ledger keeps it. In a file `sensitive` judges secret-like, its message and fix are withheld, as
 * the steer withholds them (`SECRET_FILE_FINDING_MESSAGE`), so the ledger never holds what the agent would not be told.
 */
export function owedFinding(
  violation: CodeRuleViolation,
  source: SteerRuleSource | undefined,
  place: OwedPlace,
  sensitive: (path: string) => boolean,
): OwedFinding {
  const { captures: _captures, variables: _variables, ...said } = violation;
  const secret = said.path !== undefined && sensitive(said.path);
  return { violation: secret ? withheld(said) : said, source: source ?? null, scope: [...place.scope], digests: { ...place.digests } };
}

function withheld(violation: OwedViolation): OwedViolation {
  const { fix: _fix, ...rest } = violation;
  return { ...rest, message: SECRET_FILE_FINDING_MESSAGE };
}

/**
 * A held finding the ledger kept (`owedFinding`), as the session holds it again: with no text captured, which the
 * ledger never keeps, and the source of its rule as the ledger recorded it.
 */
export function restoredFinding(owed: OwedFinding): { violation: CodeRuleViolation; sourceOf: SteerSourceOf; scope: string[]; digests: FileDigests } {
  return {
    violation: { ...owed.violation, captures: {}, variables: {} },
    sourceOf: () => owed.source ?? undefined,
    scope: [...owed.scope],
    digests: { ...owed.digests },
  };
}

/** The SHA-256 of what the file at `path` holds now, read in chunks; null when there is no file to read there. */
export async function fileDigest(path: string): Promise<string | null> {
  const hash = createHash("sha256");
  try {
    // Without an encoding, a file's stream gives its bytes.
    const bytes: AsyncIterable<Buffer> = createReadStream(path);
    for await (const chunk of bytes) hash.update(chunk);
  } catch {
    return null;
  }
  return hash.digest("hex");
}

/**
 * The files `digests` names, relative to the session's folder `root`, that no longer hold what it says the session last
 * saw in them: changed, made or removed since, by the person or another session. One read per file.
 */
export async function changedFiles(root: string, digests: FileDigests): Promise<string[]> {
  const entries = Object.entries(digests);
  const now = await Promise.all(entries.map(async ([path]) => await fileDigest(join(root, path))));
  return entries.filter(([, digest], index) => now[index] !== digest).map(([path]) => path);
}

/**
 * The entry as it is written to disk: each steer's content and each finding's message and fix scrubbed of secrets
 * (`scrubSecrets`), as the ledger stores a delivery; `entry` itself when nothing changes.
 */
export function owedForDisk(entry: OwedEntry): OwedEntry {
  const stored: OwedEntry = {
    type: "owed",
    steers: entry.steers.map(steer => ({ ...steer, delivery: { ...steer.delivery, content: scrubSecrets(steer.delivery.content) } })),
    findings: entry.findings.map(finding => ({ ...finding, violation: scrubbedViolation(finding.violation) })),
  };
  return JSON.stringify(stored) === JSON.stringify(entry) ? entry : stored;
}

function scrubbedViolation(violation: OwedViolation): OwedViolation {
  const message = scrubSecrets(violation.message);
  if (violation.fix === undefined) return { ...violation, message };
  return { ...violation, message, fix: { ...violation.fix, text: scrubSecrets(violation.fix.text) } };
}
