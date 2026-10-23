/**
 * The summary CI writes after the gates run: one verdict per rule document, the first step of STEPS it fails, or none.
 * Every rule meets the conformance stage, which starts with its declared origin (conformance/origins.ts). A port, a rule
 * its conformance.json entry or origins.json declares comes from an upstream linter's rule, then meets the port
 * readiness stage; an original, Rerail's own rule (complexity's), is done after conformance. comment.ts renders the
 * summary as the pull request comment and the job summary.
 */
import { appendFileSync, existsSync, readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { UNDECLARED } from "../origins.js";
import { renderComment, renderJobSummary } from "./comment.js";
import { CONFORMANCE_REPORT_FILE, CONFORMANCE_REPORT_FORMAT, type ConformanceReport } from "./conformance-gates.js";
import { LEFT_TO_CONFORMANCE_JOB } from "./checks/passes-conformance.js";
import { gateTitle } from "./report.js";
import { gatedToolPacks, REPORT_FORMAT, unusedGapLine, type GateReport, type GatesReport, type RuleReport } from "./run.js";
import type { Check } from "./types.js";

export const SUMMARY_FORMAT = "rerail.port-readiness-summary/v4" as const;

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/**
 * The steps in the order a rule meets them. Besides the gates, the conformance stage starts with `origin` (the rule
 * declares where it comes from: a port or an original), and the port readiness stage has three steps the summary
 * reports apart: `mapping` (a port origins.json declares, which no conformance.json entry maps yet), `set-up` (its
 * upstream linter has no gates.yaml, so no port readiness gate can run) and `linter-checks` (a failing tool-pack check,
 * such as a stale recording, fails every port of that linter); and `in-scope`, `loads`'s check that compares a rule's
 * scope with the files upstream lints, which judges the port, not the document.
 */
export const STEPS = [
  { id: "origin", title: "Declares its origin: a port or an original", stage: "conformance" },
  { id: "loads", title: gateTitle("loads"), stage: "conformance" },
  { id: "compact", title: gateTitle("compact"), stage: "conformance" },
  { id: "passes-conformance", title: gateTitle("passes-conformance"), stage: "conformance" },
  { id: "mapping", title: "Mapped to an upstream rule", stage: "port" },
  { id: "set-up", title: "Set up for its upstream linter", stage: "port" },
  { id: "linter-checks", title: "Upstream linter checks (the recording is current)", stage: "port" },
  { id: "in-scope", title: "Applies to the files upstream lints", stage: "port" },
  { id: "not-overfit", title: gateTitle("not-overfit"), stage: "port" },
  { id: "matches-upstream-tests", title: gateTitle("matches-upstream-tests"), stage: "port" },
] as const;

export type StepId = (typeof STEPS)[number]["id"];

export const STAGE_TITLES = { conformance: "Conformance", port: "Port readiness" } as const;

/** A step with its stage: `Conformance > Compact (no written-out expansion)`. */
export function stepLabel(id: StepId): string {
  const step = STEPS.find((entry) => entry.id === id)!;
  return `${STAGE_TITLES[step.stage]} > ${step.title}`;
}

/** One rule document's verdict. */
export interface RuleVerdict {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  pack: string;
  location: "shipped" | "candidate";
  /** What its pack declares it: a port of an upstream linter's rule, or an original, Rerail's own; null when nothing does. */
  origin: "port" | "original" | null;
  /** The upstream rule a port comes from, as its declaration names it; null for an original or an undeclared document. */
  upstream: { tool: string; rule: string } | null;
  /** The first step it fails, or null when it passes every step it must. */
  stoppedAt: StepId | null;
  check: string;
  why: string;
  /** A port's upstream-status call for pack.json `on`: on when upstream recommends it; absent for any other rule. */
  onByDefault?: boolean;
}

/** A blocking failure no one rule owns: a stale known gap, a failing pack check, a missing report. */
export interface OtherFailure {
  /** What fails: a pack (`python`), an upstream linter's run (`python/ruff`), or `(every pack)`. */
  what: string;
  check: string;
  why: string;
}

export interface Summary {
  format: typeof SUMMARY_FORMAT;
  /** Every rule document judged on this run, in file order. */
  rules: RuleVerdict[];
  /** Shipped rules judged, and those that pass every step they must. */
  shipped: { rules: number; pass: number };
  /** Shipped rules that fail a step: each blocks. */
  failing: RuleVerdict[];
  otherFailures: OtherFailure[];
  /** Candidates that pass every step they must: the signal to promote them (a reviewer still decides). */
  promotable: RuleVerdict[];
  /** Upstream linters the plan left out: nothing their ports read changed against the base, so main's results stand. */
  notJudged: string[];
  pass: boolean;
}

/** Every `<pack>--<tool>.json` tool-pack report in `directory`. */
export function readReports(directory: string): GatesReport[] {
  return readdirSync(directory)
    .filter((name) => /^[a-z0-9-]+--[A-Za-z0-9._-]+\.json$/u.test(name))
    .sort(byText)
    .map((name) => JSON.parse(readFileSync(join(directory, name), "utf8")) as GatesReport)
    .filter((report) => report.format === REPORT_FORMAT);
}

/** The conformance report in `directory`, or null when there is none (the summary then fails closed). */
export function readConformanceReport(directory: string): ConformanceReport | null {
  const path = join(directory, CONFORMANCE_REPORT_FILE);
  if (!existsSync(path)) return null;
  const report = JSON.parse(readFileSync(path, "utf8")) as ConformanceReport;
  return report.format === CONFORMANCE_REPORT_FORMAT ? report : null;
}

/**
 * `--planned`'s value: a JSON array of `<pack>/<tool>` strings, the tool-packs CI's plan judged
 * (conformance/port-readiness/plan.mjs).
 */
export function parsePlanned(text: string): { pack: string; tool: string }[] {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    value = undefined;
  }
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string" && /^[^/]+\/[^/]+$/u.test(entry))) {
    throw new Error(`--planned takes a JSON array of "<pack>/<tool>" strings (got ${JSON.stringify(text)})`);
  }
  return (value as string[]).map((entry) => {
    const [pack, tool] = entry.split("/") as [string, string];
    return { pack, tool };
  });
}

type Failure = { stoppedAt: StepId; check: string; why: string };

function failing(gate: GateReport, skip?: string): { check: string; why: string } {
  const check = gate.checks.find((entry: Check) => !entry.ok && entry.id !== skip);
  return { check: check?.id ?? "failed", why: check?.detail ?? "failed" };
}

/** `loads` fails on its own, not only on `in-scope` (a port readiness step). */
const loadsFails = (loads: GateReport): boolean => !loads.pass && !(loads.checks.length > 0 && loads.checks.every((check) => check.ok || check.id === "in-scope"));

/** The first conformance step a rule fails, or null. */
function conformanceStep(gates: { loads: GateReport; compact: GateReport; "passes-conformance": GateReport }): Failure | null {
  if (loadsFails(gates.loads)) return { stoppedAt: "loads", ...failing(gates.loads, "in-scope") };
  if (!gates.compact.pass) return { stoppedAt: "compact", ...failing(gates.compact) };
  if (!gates["passes-conformance"].pass) return { stoppedAt: "passes-conformance", ...failing(gates["passes-conformance"]) };
  return null;
}

/** The first step a port of a gated upstream linter fails, or null when it passes them all. */
export function gatedStep(rule: RuleReport): Failure | null {
  const conformance = conformanceStep(rule.gates);
  if (conformance !== null) return conformance;
  if (rule.blockedBy !== null) return { stoppedAt: "linter-checks", check: "tool-pack", why: rule.blockedBy };
  const inScope = rule.gates.loads.checks.find((check) => check.id === "in-scope" && !check.ok);
  if (inScope !== undefined) return { stoppedAt: "in-scope", check: inScope.id, why: inScope.detail };
  if (!rule.gates["not-overfit"].pass) return { stoppedAt: "not-overfit", ...failing(rule.gates["not-overfit"]) };
  if (!rule.gates["matches-upstream-tests"].pass) return { stoppedAt: "matches-upstream-tests", ...failing(rule.gates["matches-upstream-tests"]) };
  return null;
}

/**
 * A gated tool-pack's rule with its `passes-conformance` from the conformance job, which runs the pack's conformance
 * check once for every tool-pack (the CI legs leave it to that job). A rule the leg left to the job and the job did not
 * report fails closed.
 */
function withPackConformance(rule: RuleReport, fromJob: ReadonlyMap<string, GateReport>): RuleReport {
  const gate = fromJob.get(rule.file);
  if (gate !== undefined) return { ...rule, gates: { ...rule.gates, "passes-conformance": gate } };
  if (!rule.gates["passes-conformance"].checks.some((check) => check.id === LEFT_TO_CONFORMANCE_JOB)) return rule;
  return { ...rule, gates: { ...rule.gates, "passes-conformance": { pass: false, checks: [{ id: "conformance-job", ok: false, detail: "the conformance job's report has no result for it, and its tool-pack run left the pack's conformance check to that job" }] } } };
}

const verdictOf = (base: Omit<RuleVerdict, "stoppedAt" | "check" | "why">, failure: Failure | null): RuleVerdict => ({ ...base, stoppedAt: failure?.stoppedAt ?? null, check: failure?.check ?? "", why: failure?.why ?? "" });

/**
 * Merge the gated upstream linters' reports with the conformance report. `planned` is what CI's plan judged (every
 * gated linter unless a pull request's plan left some out): each needs a report, and the gated linters outside it are
 * listed as not judged, which blocks nothing. A missing conformance report blocks.
 */
export function summarize(reports: GatesReport[], gated: { pack: string; tool: string }[], planned = gated, conformance: ConformanceReport | null = { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs: [], rules: [], passesConformance: [] }): Summary {
  const have = new Set(reports.map((report) => report.toolPack));
  const gatedNames = gated.map(({ pack, tool }) => `${pack}/${tool}`);
  const plannedNames = new Set(planned.map(({ pack, tool }) => `${pack}/${tool}`));
  const missingReports = gatedNames.filter((toolPack) => plannedNames.has(toolPack) && !have.has(toolPack));
  const notJudged = gatedNames.filter((toolPack) => !plannedNames.has(toolPack));
  const fromJob = new Map((conformance?.passesConformance ?? []).map((entry) => [entry.file, entry.gate]));

  const rules: RuleVerdict[] = [];
  for (const report of reports) {
    for (const rule of report.rules) {
      // A gated linter judges the documents its conformance.json entries map, which declares each a port.
      rules.push(verdictOf({ id: rule.id, file: rule.file, pack: report.pack, location: rule.location, origin: "port", upstream: { tool: report.tool, rule: rule.upstreamRule }, onByDefault: rule.defaultOn }, gatedStep(withPackConformance(rule, fromJob))));
    }
  }
  // The conformance report judges every rule no gated linter does: an original, a document that declares no origin, a
  // port origins.json declares that no conformance.json entry maps, and a port of a linter with no gates.yaml, which
  // stops at set-up since no port readiness gate can run on it.
  for (const rule of conformance?.rules ?? []) {
    const { origin } = rule;
    const portStep = (): Failure | null => {
      if (origin?.kind !== "port") return null;
      const { tool, rule: name } = origin.upstream;
      if (origin.unmapped !== undefined) return { stoppedAt: "mapping", check: "conformance-entry", why: `no conformance.json entry maps it to ${tool}'s ${name}, so no port readiness gate can run: ${origin.unmapped}` };
      return { stoppedAt: "set-up", check: "gates-yaml", why: `${tool} has no packs/${rule.pack}/upstream/${tool}/gates.yaml yet, so no port readiness gate can run on its ports` };
    };
    const originStep: Failure | null = origin === null ? { stoppedAt: "origin", check: "origin", why: UNDECLARED } : null;
    rules.push(verdictOf({ id: rule.id, file: rule.file, pack: rule.pack, location: rule.location, origin: origin?.kind ?? null, upstream: origin?.kind === "port" ? origin.upstream : null }, originStep ?? conformanceStep(rule.gates) ?? portStep()));
  }
  rules.sort((a, b) => byText(a.file, b.file));

  const otherFailures: OtherFailure[] = [];
  for (const report of reports) {
    // A listed known gap that does not hold, or excused nothing across every shard, is a stale gates.yaml entry.
    for (const gap of report.knownGaps.filter((entry) => !entry.proven || entry.excused === 0)) {
      otherFailures.push({ what: report.toolPack, check: "known-gap", why: gap.proven ? unusedGapLine(gap.id) : `known gap ${gap.id} does not hold (the report's run integrity says why); remove it or correct it` });
    }
  }
  for (const entry of conformance?.packs ?? []) {
    for (const check of entry.checks.checks.filter((item) => !item.ok)) otherFailures.push({ what: entry.pack, check: check.id, why: check.detail });
  }
  if (conformance === null) otherFailures.push({ what: "(every pack)", check: "report", why: `no ${CONFORMANCE_REPORT_FILE} in the directory: the conformance gates did not run` });
  for (const toolPack of missingReports) otherFailures.push({ what: toolPack, check: "report", why: "its port readiness run left no report" });
  // The plan finds tool-packs as gatedToolPacks does, so one it planned that is not gated is a bug in the plan.
  for (const toolPack of [...plannedNames].filter((name) => !gatedNames.includes(name))) {
    otherFailures.push({ what: toolPack, check: "plan", why: "planned, but it has no gates.yaml: the plan is wrong" });
  }

  const shipped = rules.filter((rule) => rule.location === "shipped");
  const failingRules = shipped.filter((rule) => rule.stoppedAt !== null);
  return {
    format: SUMMARY_FORMAT,
    rules,
    shipped: { rules: shipped.length, pass: shipped.length - failingRules.length },
    failing: failingRules,
    otherFailures,
    promotable: rules.filter((rule) => rule.location === "candidate" && rule.stoppedAt === null),
    notJudged,
    // Every rule's verdict is the summary's own: a tool-pack run's pass leaves out what it left to the conformance job.
    pass: failingRules.length === 0 && otherFailures.length === 0,
  };
}

/**
 * `gates summary <dir> [--planned <json>]`: write summary.{md,json} and the pull request comment (comment.md) from the
 * tool-pack reports and the conformance report (CONFORMANCE_REPORT_FILE) in `dir`, append the job summary to the step
 * summary, return the summary. Without `planned`, every gated upstream linter needs a report; the conformance report is
 * always needed. The comment names the commit in PORT_READINESS_COMMIT and links the run in PORT_READINESS_RUN_URL when
 * CI sets them.
 */
export async function writeSummary(root: string, directory: string, planned?: { pack: string; tool: string }[]): Promise<Summary> {
  if (!existsSync(directory)) throw new Error(`${directory} does not exist`);
  const gated = gatedToolPacks(root);
  const summary = summarize(readReports(directory), gated, planned ?? gated, readConformanceReport(directory));
  const context = { commit: process.env.PORT_READINESS_COMMIT ?? "", runUrl: process.env.PORT_READINESS_RUN_URL ?? "" };
  const markdown = renderJobSummary(summary, context);
  await writeFile(join(directory, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
  await writeFile(join(directory, "summary.md"), markdown);
  await writeFile(join(directory, "comment.md"), renderComment(summary, context));
  const step = process.env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
  return summary;
}
