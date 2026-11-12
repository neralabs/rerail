import { z } from "zod/v4";
import { STEER_STRATEGIES, type AdapterDelivery } from "../engine/adapter-contract.js";
import { RULE_SOURCE_KINDS } from "../engine/catalog/repository-model.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { SECRET_FILE_FINDING_MESSAGE, type CodeRuleViolation } from "../engine/code-rule/code-rule.js";
import type { SteerRuleSource } from "../engine/code-rule/steer-origin.js";
import { scrubSecrets } from "../engine/secret-scrub.js";
import { SteerRuleSchema } from "./event-protocol.js";

/*
 * What a session owes its agent while it is closed. A session that closes without its end (quiet past the idle limit,
 * or open when its runtime closes, as at a daemon stop or restart) writes the steers and held findings it has not yet
 * delivered to its ledger as one `owed` entry; the session takes them back when its next event opens it again, and
 * writes the entry again, emptied, once they have gone out.
 */

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

/** A steer the session had not delivered when it closed, and the sources whose newer revision makes it stale. */
export interface OwedSteer {
  readonly delivery: AdapterDelivery;
  readonly scope: readonly string[];
}

/** A finding the session held for its yield digest when it closed, with where its rule comes from. */
export interface OwedFinding {
  readonly violation: OwedViolation;
  readonly source: SteerRuleSource | null;
  readonly scope: readonly string[];
}

export interface OwedEntry {
  readonly type: "owed";
  readonly steers: readonly OwedSteer[];
  readonly findings: readonly OwedFinding[];
}

export const OwedEntrySchema = z.object({
  type: z.literal("owed"),
  steers: z.array(z.object({ delivery: LedgerDeliverySchema, scope: z.array(z.string()) })),
  findings: z.array(z.object({
    violation: OwedViolationSchema,
    source: z.object({ kind: z.enum(RULE_SOURCE_KINDS), packId: z.string().nullable() }).nullable(),
    scope: z.array(z.string()),
  })),
});

/**
 * A held finding as the ledger keeps it. In a file `sensitive` judges secret-like, its message and fix are withheld, as
 * the steer withholds them (`SECRET_FILE_FINDING_MESSAGE`), so the ledger never holds what the agent would not be told.
 */
export function owedFinding(
  violation: CodeRuleViolation,
  source: SteerRuleSource | undefined,
  scope: readonly string[],
  sensitive: (path: string) => boolean,
): OwedFinding {
  const { captures: _captures, variables: _variables, ...said } = violation;
  const secret = said.path !== undefined && sensitive(said.path);
  return { violation: secret ? withheld(said) : said, source: source ?? null, scope: [...scope] };
}

function withheld(violation: OwedViolation): OwedViolation {
  const { fix: _fix, ...rest } = violation;
  return { ...rest, message: SECRET_FILE_FINDING_MESSAGE };
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
