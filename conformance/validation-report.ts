/**
 * `rerail.pack-validation/v1`: one pack's validation report, every rule through every check, written by
 * `npm run validate:packs -- --write` to `.rerail/reports/<id>-validation.{json,md}` (gitignored). Deterministic: two
 * runs on one commit write the same bytes, so it carries no date and no timing. The committed truth is the pack's
 * scrub.json; the report says what each failure is.
 */
import { z } from "zod";
import { compareDeterministicStrings } from "../src/stable-json.js";
import {
  PACK_CHECKS,
  PACK_RULE_STATUSES,
  RULE_SUB_CHECK_IDS,
  SUB_CHECK_IDS,
  SUB_CHECK_OUTCOMES,
  SUB_CHECKS,
  type PackCheckName,
  type RuleSubCheckId,
} from "./checks.js";
import { FINDING_PARTS } from "./languages/pack.js";
import { scrubEntrySchema } from "./scrub.js";

export const VALIDATION_REPORT_FORMAT = "rerail.pack-validation/v1";

/** How check 8 classes a word a document hardcodes: explained by the language or upstream's code, or not. */
export const LITERAL_CLASSES = ["language", "implementation", "shared-implementation", "fixture-only", "scaffold", "unexplained"] as const;
/** Where in a document a hardcoded word sits: matched or emitted code, a regex, a pattern's context, a literal name. */
export const LITERAL_ROLES = ["pattern", "regex", "context", "value"] as const;

const CHECK_NAMES = PACK_CHECKS.map(check => check.name) as [PackCheckName, ...PackCheckName[]];
const nonEmpty = z.string().min(1);
const count = z.number().int().nonnegative();
const sha = (length: number) => z.string().regex(new RegExp(`^[0-9a-f]{${length}}$`, "u"));

/** A finding's position, with the parts the recording carries for its rule (1-based; `endColumn` exclusive). */
const positionSchema = z.strictObject({
  line: z.number().int().positive(),
  column: z.number().int().positive().optional(),
  endLine: z.number().int().positive().optional(),
  endColumn: z.number().int().positive().optional(),
});

/**
 * A rule's outcome on one sub-check. `details`: the failures for `failed` and `scrubbed` (at least one), the reason for
 * `n/a`, `not-judged` and `not-supported-yet` (exactly one), none for `ok`.
 */
const subCheckResultSchema = z
  .strictObject({ subCheck: z.enum(RULE_SUB_CHECK_IDS), outcome: z.enum(SUB_CHECK_OUTCOMES), details: z.array(nonEmpty) })
  .superRefine((result, context) => {
    const failing = result.outcome === "failed" || result.outcome === "scrubbed";
    const [fits, wanted] =
      result.outcome === "ok" ? [result.details.length === 0, "no details"] : failing ? [result.details.length > 0, "at least one detail"] : [result.details.length === 1, "exactly one detail"];
    if (!fits) context.addIssue({ code: "custom", message: `${result.subCheck}: ${result.outcome} takes ${wanted}` });
  });
export type SubCheckResult = z.infer<typeof subCheckResultSchema>;

/** Check 8's working (V1 Gate 3): the words the document hardcodes by class, the ones flagged, the regexes it could not run. */
const generalitySchema = z.strictObject({
  words: z.record(z.enum(LITERAL_CLASSES), count),
  flagged: z.array(z.strictObject({ token: nonEmpty, pointer: z.string(), role: z.enum(LITERAL_ROLES), text: z.string() })),
  uncheckedRegexes: z.array(nonEmpty),
});

/** Check 9's measurements (V1 Gate 4), judged only over the 15,360-byte flag. */
const sizeSchema = z.strictObject({
  rawBytes: count,
  canonicalBytes: count,
  flagged: z.boolean(),
  repeatedBytes: count,
  repeatedShare: z.number().min(0).max(1),
  nestingRun: count,
  recursiveUtils: z.array(nonEmpty),
});

/** At most this many differing fixture files per rule, in path order. */
export const DIFFERENCE_LIMIT = 50;

/** Check 4's working for a port: the fixtures compared part by part, and the runner's row for each scored source. */
const portsSchema = z.strictObject({
  /** The parts the recording carries for the rule, which ports:span and ports:evidence compare. */
  parts: z.array(z.enum(["line", ...FINDING_PARTS] as const)).min(1),
  units: count,
  /** Upstream's findings on the fixtures, and Rerail's. */
  expected: count,
  actual: count,
  /** Pairs at the port's declared match (ports:points), and at every carried part. */
  pointsMatched: count,
  matched: count,
  missing: count,
  extra: count,
  /** Points a divergence explains. */
  explained: count,
  /** Upstream findings reproduced in fixture units where the port agrees completely. */
  evidence: count,
  differences: z
    .array(z.strictObject({ path: nonEmpty, missing: z.array(positionSchema), extra: z.array(positionSchema), failures: z.array(nonEmpty) }))
    .max(DIFFERENCE_LIMIT),
  sources: z.array(
    z.strictObject({
      /** `fixtures` or `corpus:<name>`. */
      source: nonEmpty,
      tp: count,
      fp: count,
      fn: count,
      explained: count,
      engineFailures: count,
    }),
  ),
});

const STOPS = new Set<string>(["failed", "scrubbed"]);
const checkNumberOf = (subCheck: string): number => PACK_CHECKS.find(check => check.name === subCheck.split(":")[0])!.number;

const ruleSchema = z
  .strictObject({
    /** As pack.json lists it. */
    id: nonEmpty,
    document: nonEmpty,
    /** Of the document's bytes; null when the document does not exist. */
    sha256: sha(64).nullable(),
    bytes: count.nullable(),
    /** The conformance.json entry that ports the rule; null for a rule proven by examples. */
    port: z.strictObject({ id: nonEmpty, tool: nonEmpty, rule: nonEmpty }).nullable(),
    status: z.enum(PACK_RULE_STATUSES),
    /** The lowest check with a failed or scrubbed sub-check; null when there is none. */
    stoppedAt: z.number().int().min(1).max(PACK_CHECKS.length).nullable(),
    /** One per rule-level sub-check, in `RULE_SUB_CHECK_IDS` order. */
    results: z.array(subCheckResultSchema),
    /** Null when check 8, 9 or 4 did not run for the rule (n/a, not judged, not supported yet, or not compiled). */
    generality: generalitySchema.nullable(),
    size: sizeSchema.nullable(),
    ports: portsSchema.nullable(),
  })
  .superRefine((rule, context) => {
    const issue = (message: string): void => context.addIssue({ code: "custom", message: `${rule.id}: ${message}` });
    if (rule.results.map(result => result.subCheck).join(" ") !== RULE_SUB_CHECK_IDS.join(" ")) issue("results are not one per rule-level sub-check, in order");
    const outcomes = new Set(rule.results.map(result => result.outcome));
    const status = outcomes.has("failed") ? "failed" : outcomes.has("scrubbed") ? "scrubbed" : outcomes.has("not-supported-yet") ? "not-supported-yet" : "ok";
    if (rule.status !== status) issue(`status ${rule.status}, but its results make it ${status}`);
    const stops = rule.results.filter(result => STOPS.has(result.outcome)).map(result => checkNumberOf(result.subCheck));
    const stoppedAt = stops.length === 0 ? null : Math.min(...stops);
    if (rule.stoppedAt !== stoppedAt) issue(`stoppedAt ${rule.stoppedAt}, but its results stop at ${stoppedAt}`);
  });
export type RuleReport = z.infer<typeof ruleSchema>;

export const validationReportSchema = z
  .strictObject({
    format: z.literal(VALIDATION_REPORT_FORMAT),
    pack: nonEmpty,
    /** The Rerail checkout whose engine ran: its HEAD, and whether its working tree had changes. */
    engine: z.strictObject({ commit: sha(40), dirty: z.boolean() }),
    /**
     * conformance.json's tools. `recommended` null: the tool declares no source facts, so checks 7-9 are not judged
     * for it. `source` null: no valid source.json (a provenance:pins failure when `recommended` is declared).
     */
    tools: z.array(
      z.strictObject({
        name: nonEmpty,
        version: nonEmpty,
        recommended: z.array(nonEmpty).min(1).nullable(),
        source: z.strictObject({ repository: nonEmpty, commit: sha(40), tag: nonEmpty }).nullable(),
      }),
    ),
    /** Checks that apply but are not judged for some of the pack's rules, and why (one entry per reason). */
    notJudged: z.array(z.strictObject({ check: z.enum(CHECK_NAMES), detail: nonEmpty })),
    /** Tool-level and pack-level failures: they fail their check, are never scrubbed and set no rule's status. */
    packFailures: z.array(z.strictObject({ subCheck: z.enum(SUB_CHECK_IDS), tool: nonEmpty.nullable(), detail: nonEmpty })),
    /** The scrub.json entries that cover a failure, grouped: defects of a port, and upstream's own position on a rule. */
    scrub: z.strictObject({ fidelity: z.array(scrubEntrySchema), "upstream-policy": z.array(scrubEntrySchema) }),
    summary: z.strictObject({
      rules: count,
      ok: count,
      failed: count,
      scrubbed: count,
      notSupportedYet: count,
      /** Rules with a not-judged sub-check: whatever their status, not every check vouches for them. */
      notJudged: count,
      /** Rules by the check they stopped at (V1's funnel). */
      stoppedAt: z.record(z.enum(CHECK_NAMES), count),
      /** Rules with a failed, and with a scrubbed, result of each rule-level sub-check. */
      failedBy: z.record(z.enum(RULE_SUB_CHECK_IDS), count),
      scrubbedBy: z.record(z.enum(RULE_SUB_CHECK_IDS), count),
    }),
    rules: z.array(ruleSchema),
    /** No pack failure and no failed rule. */
    pass: z.boolean(),
  })
  .superRefine((report, context) => {
    const issue = (message: string): void => context.addIssue({ code: "custom", message });
    const { summary, rules } = report;
    const having = (outcome: string, subCheck?: RuleSubCheckId): number =>
      rules.filter(rule => rule.results.some(result => result.outcome === outcome && (subCheck === undefined || result.subCheck === subCheck))).length;
    const statuses = { ok: "ok", failed: "failed", scrubbed: "scrubbed", notSupportedYet: "not-supported-yet" } as const;
    if (summary.rules !== rules.length) issue(`summary.rules is ${summary.rules}; there are ${rules.length} rules`);
    for (const [key, status] of Object.entries(statuses)) {
      const actual = rules.filter(rule => rule.status === status).length;
      if (summary[key as keyof typeof statuses] !== actual) issue(`summary.${key} is ${summary[key as keyof typeof statuses]}; ${actual} rules are ${status}`);
    }
    if (summary.notJudged !== having("not-judged")) issue(`summary.notJudged is ${summary.notJudged}; ${having("not-judged")} rules have a not-judged result`);
    for (const check of PACK_CHECKS) {
      const actual = rules.filter(rule => rule.stoppedAt === check.number).length;
      if (summary.stoppedAt[check.name] !== actual) issue(`summary.stoppedAt.${check.name} is ${summary.stoppedAt[check.name]}; ${actual} rules stop there`);
    }
    for (const subCheck of RULE_SUB_CHECK_IDS) {
      if (summary.failedBy[subCheck] !== having("failed", subCheck)) issue(`summary.failedBy.${subCheck} disagrees with the rules`);
      if (summary.scrubbedBy[subCheck] !== having("scrubbed", subCheck)) issue(`summary.scrubbedBy.${subCheck} disagrees with the rules`);
    }
    const pass = report.packFailures.length === 0 && summary.failed === 0;
    if (report.pass !== pass) issue(`pass is ${report.pass}, but the failures make it ${pass}`);
  });
export type ValidationReport = z.infer<typeof validationReportSchema>;

/** What the report is built from: one pack's validation (conformance/pack-validation.ts). */
export interface ValidatedPack {
  id: string;
  tools: ValidationReport["tools"];
  notJudged: Partial<Record<PackCheckName, string[]>>;
  packFailures: ValidationReport["packFailures"];
  rules: RuleReport[];
  pass: boolean;
}

const SCRUB_GROUP = new Map(SUB_CHECKS.map(subCheck => [subCheck.id as string, subCheck.scrub]));

/** The report of one validated pack, checked against its own schema; `engine` is the Rerail checkout that ran. */
export function validationReport(validation: ValidatedPack, engine: ValidationReport["engine"]): ValidationReport {
  const { rules } = validation;
  const counted = (keep: (rule: RuleReport) => boolean): number => rules.filter(keep).length;
  const having = (subCheck: RuleSubCheckId, outcome: string) => counted(rule => rule.results.some(result => result.subCheck === subCheck && result.outcome === outcome));
  const scrubbed = rules
    .flatMap(rule => rule.results.filter(result => result.outcome === "scrubbed").map(result => ({ rule: rule.id, check: result.subCheck })))
    .sort((left, right) => compareDeterministicStrings(left.rule, right.rule) || compareDeterministicStrings(left.check, right.check));
  return validationReportSchema.parse({
    format: VALIDATION_REPORT_FORMAT,
    pack: validation.id,
    engine,
    tools: validation.tools,
    notJudged: PACK_CHECKS.flatMap(check => (validation.notJudged[check.name] ?? []).map(detail => ({ check: check.name, detail }))),
    packFailures: validation.packFailures,
    scrub: {
      fidelity: scrubbed.filter(entry => SCRUB_GROUP.get(entry.check) === "fidelity"),
      "upstream-policy": scrubbed.filter(entry => SCRUB_GROUP.get(entry.check) === "upstream-policy"),
    },
    summary: {
      rules: rules.length,
      ok: counted(rule => rule.status === "ok"),
      failed: counted(rule => rule.status === "failed"),
      scrubbed: counted(rule => rule.status === "scrubbed"),
      notSupportedYet: counted(rule => rule.status === "not-supported-yet"),
      notJudged: counted(rule => rule.results.some(result => result.outcome === "not-judged")),
      stoppedAt: Object.fromEntries(PACK_CHECKS.map(check => [check.name, counted(rule => rule.stoppedAt === check.number)])),
      failedBy: Object.fromEntries(RULE_SUB_CHECK_IDS.map(subCheck => [subCheck, having(subCheck, "failed")])),
      scrubbedBy: Object.fromEntries(RULE_SUB_CHECK_IDS.map(subCheck => [subCheck, having(subCheck, "scrubbed")])),
    },
    rules,
    pass: validation.pass,
  });
}

const cell = (text: string): string => text.replaceAll("|", "\\|").replaceAll("\n", " ");
const code = (text: string): string => `\`${text.replaceAll("`", "'")}\``;
const list = (items: readonly string[], empty: string): string[] => (items.length === 0 ? [empty] : items.map(item => `- ${item}`));

/** The human-readable report: the same facts as the JSON, in reading order. */
export function renderValidationReport(report: ValidationReport): string {
  const { summary, rules } = report;
  const judgedAll = summary.rules - summary.notJudged;
  const lines: string[] = [
    `# Pack validation: ${report.pack}`,
    "",
    `**${report.pass ? "Passes" : "Fails"}.** Of ${summary.rules} rules, ${summary.ok} are ok, ${summary.scrubbed} scrubbed (every failure is on scrub.json), ${summary.failed} failed and ${summary.notSupportedYet} not supported yet. ${judgedAll} of ${summary.rules} are judged by every check that applies to them; ${summary.notJudged} have a check this layer does not judge. ${report.packFailures.length} pack-level failure(s).`,
    "",
    `- Engine: Rerail ${code(report.engine.commit)}${report.engine.dirty ? ", with uncommitted changes" : ""}`,
    ...report.tools.map(
      tool =>
        `- ${tool.name} ${tool.version}: ${
          tool.recommended === null
            ? "no source facts recorded, so checks 7 to 9 are not judged for its ports"
            : `approved configurations ${tool.recommended.map(code).join(", ")}; source facts from ${tool.source === null ? "no valid source.json" : `${tool.source.repository} at ${code(tool.source.commit)} (${tool.source.tag})`}`
        }`,
    ),
    "",
    "## Not judged",
    "",
    "A check that is not judged for a rule is never a pass.",
    "",
    ...list(
      report.notJudged.map(entry => `check ${PACK_CHECKS.find(check => check.name === entry.check)!.number} (${entry.check}): ${entry.detail}`),
      "Every check is judged for every rule it applies to.",
    ),
    "",
    "## Where rules stop",
    "",
    "A rule stops at the first check with a failed or scrubbed sub-check; every check still ran on every rule.",
    "",
    "| Check | Entered | Stopped here | Went on |",
    "| --- | ---: | ---: | ---: |",
  ];
  let entered = summary.rules;
  for (const check of PACK_CHECKS) {
    const stopped = summary.stoppedAt[check.name];
    lines.push(`| ${check.number}. ${check.name} | ${entered} | ${stopped} | ${entered - stopped} |`);
    entered -= stopped;
  }
  lines.push("", "## Each sub-check on its own", "", "| Sub-check | Failed | Scrubbed | OK | n/a | Not judged | Not supported yet |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
  for (const subCheck of RULE_SUB_CHECK_IDS) {
    const outcomes = rules.map(rule => rule.results.find(result => result.subCheck === subCheck)!.outcome);
    const n = (outcome: string): number => outcomes.filter(entry => entry === outcome).length;
    lines.push(`| ${subCheck} | ${n("failed")} | ${n("scrubbed")} | ${n("ok")} | ${n("n/a")} | ${n("not-judged")} | ${n("not-supported-yet")} |`);
  }
  lines.push(
    "",
    "## Pack failures",
    "",
    "Tool-level and pack-level failures fail their check and are never scrubbed.",
    "",
    ...(report.packFailures.length === 0
      ? ["None."]
      : ["| Sub-check | Tool | Failure |", "| --- | --- | --- |", ...report.packFailures.map(failure => `| ${failure.subCheck} | ${failure.tool ?? ""} | ${cell(failure.detail)} |`)]),
    "",
    "## Scrub list",
    "",
    "The scrub.json entries that cover a failure, to fix or remove before the pack goes live.",
    "",
    "### Fidelity defects",
    "",
    ...list(report.scrub.fidelity.map(entry => `${code(entry.rule)}: ${entry.check}`), "None."),
    "",
    "### Upstream-policy exclusions",
    "",
    ...list(report.scrub["upstream-policy"].map(entry => `${code(entry.rule)}: ${entry.check}`), "None."),
    "",
    "## Failed and scrubbed results",
    "",
  );
  const failing = RULE_SUB_CHECK_IDS.flatMap(subCheck =>
    rules.flatMap(rule => {
      const result = rule.results.find(entry => entry.subCheck === subCheck)!;
      return result.outcome === "failed" || result.outcome === "scrubbed" ? [{ subCheck, rule, result }] : [];
    }),
  );
  if (failing.length === 0) lines.push("None.");
  for (const subCheck of RULE_SUB_CHECK_IDS) {
    const entries = failing.filter(entry => entry.subCheck === subCheck);
    if (entries.length === 0) continue;
    lines.push(`### ${subCheck}`, "");
    for (const { rule, result } of entries) {
      lines.push(`- ${code(rule.id)} (${result.outcome}): ${cell(result.details[0]!)}${result.details.length > 1 ? ` (and ${result.details.length - 1} more)` : ""}`);
      if (subCheck === "generality:names") for (const literal of rule.generality?.flagged.slice(0, 5) ?? []) lines.push(`  - ${code(literal.token)} at ${code(literal.pointer)} (${literal.role})`);
      if (subCheck === "ports:span" || subCheck === "ports:evidence")
        for (const difference of rule.ports?.differences.slice(0, 3) ?? [])
          lines.push(`  - ${code(difference.path)}: ${difference.missing.length} upstream finding(s) unmatched, ${difference.extra.length} Rerail finding(s) unmatched`);
    }
    lines.push("");
  }
  const flagged = rules.filter(rule => rule.size?.flagged === true);
  lines.push(
    "## Size",
    "",
    ...(flagged.length === 0
      ? ["No judged rule is over the 15,360-byte flag."]
      : [
          "| Rule | Bytes | Canonical bytes | Repeated structure | Longest self-nesting | Recursive utilities |",
          "| --- | ---: | ---: | ---: | ---: | --- |",
          ...flagged.map(
            rule =>
              `| ${code(rule.id)} | ${rule.size!.rawBytes} | ${rule.size!.canonicalBytes} | ${(rule.size!.repeatedShare * 100).toFixed(1)}% | ${rule.size!.nestingRun} | ${rule.size!.recursiveUtils.join(", ") || "none"} |`,
          ),
        ]),
    "",
    "## Every rule",
    "",
    "Fixtures: upstream findings Rerail reproduces at every part the recording carries, and at the port's declared match, of upstream's; then Rerail's unmatched findings.",
    "",
    "| Rule | Status | Stopped at | Port | Parts | Fixtures | Bytes |",
    "| --- | --- | --- | --- | --- | --- | ---: |",
    ...rules.map(rule => {
      const stopped = rule.stoppedAt === null ? "" : `${rule.stoppedAt} ${PACK_CHECKS[rule.stoppedAt - 1]!.name}`;
      const port = rule.port === null ? "" : `${rule.port.tool} ${code(rule.port.rule)}`;
      const fixtures = rule.ports === null ? "" : `${rule.ports.matched} (${rule.ports.pointsMatched}) of ${rule.ports.expected}, ${rule.ports.extra} extra`;
      return `| ${code(rule.id)} | ${rule.status} | ${stopped} | ${port} | ${rule.ports?.parts.join(", ") ?? ""} | ${fixtures} | ${rule.bytes ?? ""} |`;
    }),
    "",
  );
  return lines.join("\n");
}
