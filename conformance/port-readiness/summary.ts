/**
 * The summary CI writes after the gates run, and renders as the job summary (here) and the pull request comment
 * (comment.ts). It holds one verdict per rule document judged: the first step of the funnel (STEPS) the rule fails, or
 * none. The conformance steps run on every rule; the port steps only on a gated tool-pack's. Every count the summary
 * renders is derived from those verdicts, so each rule is counted exactly once.
 */
import { appendFileSync, existsSync, readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { renderComment } from "./comment.js";
import { CONFORMANCE_REPORT_FILE, CONFORMANCE_REPORT_FORMAT, type ConformanceReport, type ConformanceRuleReport } from "./conformance-gates.js";
import type { CommandRunner } from "./process.js";
import { cell, gateTitle } from "./report.js";
import { blockingFailures, gatedToolPacks, REPORT_FORMAT, unusedGapLine, type GateReport, type GatesReport, type RuleReport } from "./run.js";
import { BLOCKING_GATES, type Check } from "./types.js";
import { ungatedChecks, type UngatedResult } from "./ungated.js";

export const SUMMARY_FORMAT = "rerail.port-readiness-summary/v3" as const;

/** GitHub's step-summary limit is 1 MiB; tables stop well before it and say where the rest is. */
export const STEP_SUMMARY_LIMIT = 1024 * 1024;
const TABLE_ROWS = 200;

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/**
 * The funnel, in the order a rule meets it: the conformance gates, then what a port must also pass. Besides the gates,
 * two port steps are checks the summary reports apart: `mapping` (a document of a gated pack that no conformance.json
 * entry maps, so no port gate can run) and `tool-pack` (a failing tool-pack check, such as a stale recording, fails
 * every rule of the tool-pack), both before the port gates; and `in-scope`, `loads`'s check that compares a rule's
 * scope with the files upstream lints, which judges the port, not the document.
 */
export const STEPS = [
  { id: "loads", title: gateTitle("loads"), section: "conformance" },
  { id: "compact", title: gateTitle("compact"), section: "conformance" },
  { id: "passes-conformance", title: gateTitle("passes-conformance"), section: "conformance" },
  { id: "mapping", title: "Mapped to an upstream rule", section: "port" },
  { id: "tool-pack", title: "Tool-pack checks (the recording is current)", section: "port" },
  { id: "in-scope", title: "Applies to the files upstream lints", section: "port" },
  { id: "not-overfit", title: gateTitle("not-overfit"), section: "port" },
  { id: "matches-upstream-tests", title: gateTitle("matches-upstream-tests"), section: "port" },
] as const;

export type StepId = (typeof STEPS)[number]["id"];

export const stepTitle = (id: StepId): string => STEPS.find((step) => step.id === id)!.title;

/** One rule document's verdict. */
export interface RuleVerdict {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  pack: string;
  /**
   * Where its verdict comes from: a gated tool-pack (`python/ruff`); or, judged on the conformance gates alone, a tool
   * with no gates.yaml (`node/eslint-plugin-n`), a pack with no conformance.json (`complexity`), or a document no
   * conformance.json entry maps (`python (no conformance.json entry)`).
   */
  source: string;
  /** A gated tool-pack judged it, so the port steps ran on it. */
  gated: boolean;
  location: "shipped" | "candidate";
  /** The first step it fails, or null when it passes every step that runs on it. */
  stoppedAt: StepId | null;
  check: string;
  why: string;
}

/** A blocking failure: a shipped rule's verdict, or one no rule owns (`rule` is `-` or names what it covers). */
export interface BlockingRow {
  source: string;
  rule: string;
  stoppedAt: string;
  check: string;
  why: string;
}

/** The rule documents a change touches, from the diff against its base. */
export type ChangedRules =
  | {
      /** The verdict of every changed document that still exists and was judged. */
      verdicts: RuleVerdict[];
      /** Changed documents no report judged (their tool-pack's report is missing). */
      notJudged: string[];
      /** Changed documents that no longer exist: deleted, or moved (the new path is in `verdicts`). */
      removed: string[];
    }
  | { error: string };

export interface Summary {
  format: typeof SUMMARY_FORMAT;
  /** Every rule document judged on this run, in file order. */
  rules: RuleVerdict[];
  /** Shipped rules judged, and those that pass every step that runs on them and block nothing. */
  shipped: { rules: number; pass: number };
  blocking: BlockingRow[];
  /** Shipped rules that fail only because the engine could not read a golden (NER-1764). */
  engineGaps: { source: string; rule: string; detail: string }[];
  /** The rule documents the change touches; null when there is no base to diff against. */
  changed: ChangedRules | null;
  /** Candidates that pass every step, conformance and port: the signal to promote them (a reviewer still decides). */
  promotable: { toolPack: string; rule: string; upstreamRule: string; startsOn: boolean; startReason: string }[];
  /** Rules `passes-conformance` does not apply to: no conformance.json entry maps them. */
  notApplicable: { source: string; file: string; reason: string }[];
  ungated: UngatedResult;
  missingReports: string[];
  /** Gated tool-packs the plan left out: nothing they read changed against the base, so main's verdict stands. */
  notJudged: string[];
  onOff: { toolPack: string; rule: string; packJson: boolean; upstreamStatus: boolean; reason: string }[];
  toolPacks: { toolPack: string; failedGate: Record<string, number>; integrity: string[]; gapCandidates: number; toolPackChecks: boolean }[];
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

/** `loads` fails on its own, not only on `in-scope` (a port step). */
const loadsFails = (loads: GateReport): boolean => !loads.pass && !(loads.checks.length > 0 && loads.checks.every((check) => check.ok || check.id === "in-scope"));

/** The first conformance step a rule fails, or null. */
function conformanceStep(gates: { loads: GateReport; compact: GateReport; "passes-conformance": GateReport }): Failure | null {
  if (loadsFails(gates.loads)) return { stoppedAt: "loads", ...failing(gates.loads, "in-scope") };
  if (!gates.compact.pass) return { stoppedAt: "compact", ...failing(gates.compact) };
  if (!gates["passes-conformance"].pass) return { stoppedAt: "passes-conformance", ...failing(gates["passes-conformance"]) };
  return null;
}

/** The first step a gated tool-pack's rule fails, or null when it passes them all. */
export function gatedStep(rule: RuleReport): Failure | null {
  const conformance = conformanceStep(rule.gates);
  if (conformance !== null) return conformance;
  if (rule.blockedBy !== null) return { stoppedAt: "tool-pack", check: "tool-pack", why: rule.blockedBy };
  const inScope = rule.gates.loads.checks.find((check) => check.id === "in-scope" && !check.ok);
  if (inScope !== undefined) return { stoppedAt: "in-scope", check: inScope.id, why: inScope.detail };
  if (!rule.gates["not-overfit"].pass) return { stoppedAt: "not-overfit", ...failing(rule.gates["not-overfit"]) };
  if (!rule.gates["matches-upstream-tests"].pass) return { stoppedAt: "matches-upstream-tests", ...failing(rule.gates["matches-upstream-tests"]) };
  return null;
}

/** Ends the source label of a document no conformance.json entry maps: `python (no conformance.json entry)`. */
export const UNMAPPED_SUFFIX = " (no conformance.json entry)";

/** The source label of a rule the conformance report judged; `manifests` holds the packs with a conformance.json. */
function ungatedSource(rule: ConformanceRuleReport, manifests: ReadonlySet<string>): string {
  if (rule.upstream !== null) return `${rule.pack}/${rule.upstream.tool}`;
  return manifests.has(rule.pack) ? `${rule.pack}${UNMAPPED_SUFFIX}` : rule.pack;
}

const verdictOf = (base: Omit<RuleVerdict, "stoppedAt" | "check" | "why">, failure: Failure | null): RuleVerdict => ({ ...base, stoppedAt: failure?.stoppedAt ?? null, check: failure?.check ?? "", why: failure?.why ?? "" });

/** A rule document under `packs/`, as packDocuments reads them: `rules/*.json`, or `candidates/**` but probes and manifests. */
export function isRuleDocument(path: string): boolean {
  const match = /^packs\/[^/]+\/(rules\/[^/]+\.json|candidates\/.+\.json)$/u.exec(path);
  return match !== null && !/^packs\/[^/]+\/candidates\/probes\//u.test(path) && !path.endsWith("/conformance.json");
}

/** The changed rule documents among `paths`, against the verdicts judged; `exists` says whether a path is in the tree. */
export function changedRules(paths: string[], rules: RuleVerdict[], exists: (path: string) => boolean): ChangedRules {
  const byFile = new Map(rules.map((rule) => [rule.file, rule]));
  const documents = [...new Set(paths.filter(isRuleDocument))].sort(byText);
  return {
    verdicts: documents.flatMap((path) => byFile.get(path) ?? []),
    notJudged: documents.filter((path) => !byFile.has(path) && exists(path)),
    removed: documents.filter((path) => !byFile.has(path) && !exists(path)),
  };
}

export interface SummarizeOptions {
  /** The conformance report; null when it is missing, which blocks. Left out, the summary holds no rule it judges. */
  conformance?: ConformanceReport | null;
  /** The rule documents the change touches, or why the diff failed; left out when there is no base. */
  changed?: { paths: string[]; exists: (path: string) => boolean } | { error: string };
}

/**
 * Merge tool-pack reports with the conformance report and the not-gated and pending checks. `planned` is what CI's plan
 * judged (every gated tool-pack unless a pull request's plan left some out): each needs a report, and the gated
 * tool-packs outside it are listed as not judged, which blocks nothing.
 */
export function summarize(reports: GatesReport[], ungated: UngatedResult, gated: { pack: string; tool: string }[], planned = gated, options: SummarizeOptions = {}): Summary {
  const conformance = options.conformance === undefined ? { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs: [], rules: [] } : options.conformance;
  const have = new Set(reports.map((report) => report.toolPack));
  const gatedNames = gated.map(({ pack, tool }) => `${pack}/${tool}`);
  const plannedNames = new Set(planned.map(({ pack, tool }) => `${pack}/${tool}`));
  const missingReports = gatedNames.filter((toolPack) => plannedNames.has(toolPack) && !have.has(toolPack));
  const notJudged = gatedNames.filter((toolPack) => !plannedNames.has(toolPack));
  const gatedPacks = new Set(gated.map(({ pack }) => pack));
  const manifests = new Set((conformance?.packs ?? []).filter((entry) => entry.hasConformanceJson).map((entry) => entry.pack));

  const rules: RuleVerdict[] = [];
  for (const report of reports) {
    for (const rule of report.rules) rules.push(verdictOf({ id: rule.id, file: rule.file, pack: report.pack, source: report.toolPack, gated: true, location: rule.location }, gatedStep(rule)));
  }
  for (const rule of conformance?.rules ?? []) {
    // A document of a gated pack that no entry maps passes no port gate: none can run on it.
    const unmapped = rule.upstream === null && gatedPacks.has(rule.pack);
    const failure = conformanceStep(rule.gates) ?? (unmapped ? { stoppedAt: "mapping" as const, check: "conformance-entry", why: "no conformance.json entry maps it to an upstream rule, so no port gate can run: add its entry" } : null);
    rules.push(verdictOf({ id: rule.id, file: rule.file, pack: rule.pack, source: ungatedSource(rule, manifests), gated: false, location: rule.location }, failure));
  }
  rules.sort((a, b) => byText(a.file, b.file));

  const blocking: BlockingRow[] = rules
    .filter((rule) => rule.location === "shipped" && rule.stoppedAt !== null)
    .map((rule) => ({ source: rule.source, rule: rule.id, stoppedAt: stepTitle(rule.stoppedAt!), check: rule.check, why: rule.why }));
  for (const report of reports) {
    // A listed known gap that does not hold, or excused nothing across every shard, is a stale gates.yaml entry.
    for (const gap of report.knownGaps.filter((entry) => !entry.proven || entry.excused === 0)) {
      blocking.push({ source: report.toolPack, rule: "-", stoppedAt: "Known gaps", check: "known-gap", why: gap.proven ? unusedGapLine(gap.id) : `known gap ${gap.id} does not hold (the report's run integrity says why); remove it or correct it` });
    }
  }
  for (const entry of conformance?.packs ?? []) {
    for (const check of entry.checks.checks.filter((item) => !item.ok)) blocking.push({ source: entry.pack, rule: "-", stoppedAt: "Pack checks", check: check.id, why: check.detail });
  }
  if (conformance === null) blocking.push({ source: "(every pack)", rule: "(every rule no gated tool-pack judges)", stoppedAt: "Run", check: "report", why: `no ${CONFORMANCE_REPORT_FILE} in the directory: the conformance gates did not run` });
  for (const toolPack of missingReports) blocking.push({ source: toolPack, rule: "(every rule)", stoppedAt: "Run", check: "report", why: "the gated tool-pack has no report in the directory" });
  // The plan finds tool-packs as gatedToolPacks does, so one it planned that is not gated is a bug in the plan.
  for (const toolPack of [...plannedNames].filter((name) => !gatedNames.includes(name))) {
    blocking.push({ source: toolPack, rule: "(every rule)", stoppedAt: "Run", check: "plan", why: "planned, but not a gated tool-pack (no gates.yaml): the plan is wrong" });
  }
  for (const check of ungated.checks.filter((item) => !item.ok)) blocking.push({ source: check.id.split(":")[1] ?? "", rule: "-", stoppedAt: "Not gated / pending", check: check.id.split(":")[0]!, why: check.detail });

  const engineGaps = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && gatedStep(rule)?.stoppedAt === "matches-upstream-tests")
    .filter((rule) => rule.gates["matches-upstream-tests"].checks.filter((check) => !check.ok).every((check) => check.id === "engine-reads-goldens"))
    .map((rule) => ({ source: report.toolPack, rule: rule.id, detail: rule.gates["matches-upstream-tests"].checks.find((check) => check.id === "engine-reads-goldens")!.detail })));
  const promotable = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "candidate" && gatedStep(rule) === null)
    .map((rule) => ({ toolPack: report.toolPack, rule: rule.id, upstreamRule: rule.upstreamRule, startsOn: rule.defaultOn, startReason: rule.defaultOnReason })));
  const onOff = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && rule.on !== null && rule.on !== rule.defaultOn)
    .map((rule) => ({ toolPack: report.toolPack, rule: rule.id, packJson: rule.on!, upstreamStatus: rule.defaultOn, reason: rule.defaultOnReason })));
  const shipped = rules.filter((rule) => rule.location === "shipped");
  const changed = options.changed === undefined ? null : "error" in options.changed ? { error: options.changed.error } : changedRules(options.changed.paths, rules, options.changed.exists);
  return {
    format: SUMMARY_FORMAT,
    rules,
    shipped: { rules: shipped.length, pass: shipped.filter((rule) => rule.stoppedAt === null).length },
    blocking,
    engineGaps,
    changed,
    promotable,
    notApplicable: (conformance?.rules ?? []).filter((rule) => !rule.gates["passes-conformance"].applies)
      .map((rule) => ({ source: ungatedSource(rule, manifests), file: rule.file, reason: rule.gates["passes-conformance"].checks[0]?.detail ?? "" })),
    ungated,
    missingReports,
    notJudged,
    onOff,
    toolPacks: reports.map((report) => ({
      toolPack: report.toolPack,
      failedGate: Object.fromEntries(BLOCKING_GATES.map((gate) => [gate.id, report.summary.failedGate[gate.id]])),
      integrity: report.integrity,
      gapCandidates: report.gapCandidates.length,
      toolPackChecks: report.toolPackChecks.pass,
    })),
    pass: blocking.length === 0 && reports.every((report) => blockingFailures(report) === 0),
  };
}

/** The steps that run on a gated tool-pack's rules (all of them) or on the rest (the conformance steps and `mapping`). */
export const stepsFor = (gated: boolean) => STEPS.filter((step) => gated || step.section === "conformance" || step.id === "mapping");

/** Each side's candidate count at each step and at the end: ready to promote (gated) or passing the conformance gates. */
export function funnel(summary: Summary): { gated: Record<StepId | "passes", number>; rest: Record<StepId | "passes", number>; gatedTotal: number; restTotal: number } {
  const zero = () => Object.fromEntries([...STEPS.map((step) => step.id), "passes"].map((id) => [id, 0])) as Record<StepId | "passes", number>;
  const gated = zero();
  const rest = zero();
  for (const rule of summary.rules.filter((item) => item.location === "candidate")) (rule.gated ? gated : rest)[rule.stoppedAt ?? "passes"] += 1;
  const total = (counts: Record<string, number>) => Object.values(counts).reduce((sum, count) => sum + count, 0);
  return { gated, rest, gatedTotal: total(gated), restTotal: total(rest) };
}

/** Per source, its shipped rules passing and its candidates at each step, gated tool-packs first. */
export function bySource(summary: Summary): { source: string; gated: boolean; shipped: { rules: number; pass: number }; candidates: Record<StepId | "passes", number>; total: number }[] {
  const rows = new Map<string, ReturnType<typeof bySource>[number]>();
  for (const rule of summary.rules) {
    const row = rows.get(rule.source) ?? { source: rule.source, gated: rule.gated, shipped: { rules: 0, pass: 0 }, candidates: Object.fromEntries([...STEPS.map((step) => step.id), "passes"].map((id) => [id, 0])) as Record<StepId | "passes", number>, total: 0 };
    rows.set(rule.source, row);
    if (rule.location === "shipped") {
      row.shipped.rules += 1;
      if (rule.stoppedAt === null) row.shipped.pass += 1;
    } else {
      row.candidates[rule.stoppedAt ?? "passes"] += 1;
      row.total += 1;
    }
  }
  return [...rows.values()].sort((a, b) => Number(b.gated) - Number(a.gated) || byText(a.source, b.source));
}

/** A rule's verdict in words, as the tables print it. */
export function verdictText(rule: RuleVerdict): string {
  if (rule.stoppedAt === null) {
    if (rule.gated) return rule.location === "shipped" ? "✅ passes every gate" : "🟢 passes every gate: ready to promote";
    return "✅ passes the conformance gates; no port gate runs on it";
  }
  return `${rule.location === "shipped" ? "❌ blocks" : "stops"} at ${stepTitle(rule.stoppedAt)}: ${rule.check}: ${rule.why}`;
}

/** The step a source's candidates end at when they pass every step: promotion for a gated tool-pack. */
export const passesLabel = (gated: boolean): string => (gated ? "Ready to promote" : "Passes the conformance gates");

/** Rules first by location (shipped, then candidates), then by how far they got (blocked first), then by file. */
export function byVerdict(a: RuleVerdict, b: RuleVerdict): number {
  const rank = (rule: RuleVerdict) => (rule.stoppedAt === null ? STEPS.length : STEPS.findIndex((step) => step.id === rule.stoppedAt));
  return Number(a.location === "candidate") - Number(b.location === "candidate") || rank(a) - rank(b) || byText(a.file, b.file);
}

function table(header: string[], rows: string[][], limit = TABLE_ROWS): string[] {
  const lines = [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.slice(0, limit).map((row) => `| ${row.map(cell).join(" | ")} |`)];
  if (rows.length > limit) lines.push("", `${rows.length - limit} more row(s) in summary.json and the uploaded report.`);
  return lines;
}

/** The funnel as a table, steps as rows: what the comment and the job summary both show. */
export function funnelTable(summary: Summary): string[] {
  const counts = funnel(summary);
  const shown = STEPS.filter((step) => !(["mapping", "tool-pack"] as StepId[]).includes(step.id) || counts.gated[step.id] + counts.rest[step.id] > 0);
  return table(
    ["Gate", "Section", `Gated tool-packs (${counts.gatedTotal})`, `Not judged by a gated tool-pack (${counts.restTotal})`],
    [
      ...shown.map((step) => [step.title, step.section, String(counts.gated[step.id]), stepsFor(false).some((entry) => entry.id === step.id) ? String(counts.rest[step.id]) : "—"]),
      ["**Passes every gate that runs**", "", `**${counts.gated.passes}** ready to promote`, `**${counts.rest.passes}** pass the conformance gates`],
    ],
  );
}

/** The per-source table of one side, with only the steps some candidate of that side stops at. */
export function sourceTable(summary: Summary, gated: boolean): string[] {
  const rows = bySource(summary).filter((row) => row.gated === gated);
  if (rows.length === 0) return [];
  const steps = stepsFor(gated).filter((step) => rows.some((row) => row.candidates[step.id] > 0));
  return table(
    [gated ? "Tool-pack" : "Rules from", "Shipped passing", "Candidates", ...steps.map((step) => step.title), passesLabel(gated)],
    rows.map((row) => [row.source, `${row.shipped.pass}/${row.shipped.rules}`, String(row.total), ...steps.map((step) => String(row.candidates[step.id])), String(row.candidates.passes)]),
  );
}

/** The summary as Markdown, under GitHub's step-summary limit: the comment's order, with every list in full. */
export function renderSummary(summary: Summary): string {
  const candidates = summary.rules.filter((rule) => rule.location === "candidate").sort(byVerdict);
  const lines = [
    "# Rule gates",
    "",
    `**${summary.shipped.pass} of ${summary.shipped.rules} shipped rules pass; ${summary.blocking.length} blocking failure(s); ${summary.promotable.length} candidate(s) ready to promote.**`,
    "",
    "Each rule is counted once, at the first gate it fails: the conformance gates, which run on every rule, then the port gates, which run on a gated tool-pack's rules.",
    "",
    ...(summary.notJudged.length === 0 ? [] : [`Not judged on this change: ${summary.notJudged.join(", ")} (nothing they read changed against the base, so main's verdict stands).`, ""]),
    "## Blocking failures",
    "",
    ...(summary.blocking.length === 0 ? ["None."] : table(["Rules from", "Rule", "Stopped at", "Check", "Why"], summary.blocking.map((row) => [row.source, row.rule, row.stoppedAt, row.check, row.why]))),
    "",
    ...(summary.engineGaps.length === 0 ? [] : ["Blocked only by engine gaps (the fix is the engine's, NER-1764):", "", ...table(["Tool-pack", "Rule", "Files"], summary.engineGaps.map((row) => [row.source, row.rule, row.detail])), ""]),
    "## This change's rules",
    "",
    ...(summary.changed === null
      ? ["No base to diff against."]
      : "error" in summary.changed
        ? [`Could not diff against the base: ${summary.changed.error}`]
        : summary.changed.verdicts.length + summary.changed.notJudged.length + summary.changed.removed.length === 0
          ? ["The change touches no rule document."]
          : [
              ...table(["Rule", "Rules from", "Where", "Verdict"], [...summary.changed.verdicts].sort(byVerdict).map((rule) => [rule.id, rule.source, rule.location, verdictText(rule)])),
              ...summary.changed.notJudged.map((path) => `- \`${path}\`: not judged (its tool-pack has no report)`),
              ...summary.changed.removed.map((path) => `- \`${path}\`: removed or moved`),
            ]),
    "",
    "## Ready to promote",
    "",
    ...(summary.promotable.length === 0
      ? ["No candidate passes every gate yet."]
      : [
          `${summary.promotable.length} candidate(s) pass every gate, conformance and port: the signal to promote them into \`rules/\` and \`pack.json\`. Passing the gates is necessary, not sufficient; a reviewer still decides. "Starts" is what upstream-status sets for pack.json \`on\`.`,
          "",
          ...table(["Tool-pack", "Rule", "Upstream rule", "Starts", "Why"], summary.promotable.map((row) => [row.toolPack, row.rule, row.upstreamRule, row.startsOn ? "on" : "off", row.startReason])),
        ]),
    "",
    "## Where every candidate stands",
    "",
    ...funnelTable(summary),
    "",
    ...sourceTable(summary, true),
    "",
    ...sourceTable(summary, false),
    "",
    ...(summary.notApplicable.length === 0 ? [] : [`${gateTitle("passes-conformance")} does not apply to ${summary.notApplicable.length} rule(s), which no conformance.json entry maps:`, "", ...summary.notApplicable.map((entry) => `- ${entry.source}: \`${entry.file}\``), ""]),
    "<details><summary>Every candidate</summary>",
    "",
    ...table(["Rules from", "Rule", "Verdict"], candidates.map((rule) => [rule.source, rule.id, verdictText(rule)]), 1000),
    "",
    "</details>",
    "",
    "## Not gated and pending",
    "",
    ...(summary.ungated.notGated.length === 0 ? ["No pack is without a gated or pending tool-pack."] : summary.ungated.notGated.map((entry) => `- ${entry.pack}: ${entry.rules} shipped rule(s), frozen (no port gate can judge them; the conformance gates do)`)),
    ...summary.ungated.pending.map((entry) => `- ${entry.toolPack}: pending, ${entry.reason}`),
    ...summary.ungated.checks.filter((check) => !check.ok).map((check) => `- **${check.id}**: ${check.detail}`),
    "",
    "## On or off",
    "",
    ...(summary.onOff.length === 0 ? ["Every shipped rule's pack.json on agrees with upstream-status."] : table(["Tool-pack", "Rule", "pack.json", "upstream-status", "Reason"], summary.onOff.map((row) => [row.toolPack, row.rule, row.packJson ? "on" : "off", row.upstreamStatus ? "on" : "off", row.reason]))),
    "",
    "## Per tool-pack",
    "",
    ...summary.toolPacks.flatMap((entry) => [
      `<details><summary>${entry.toolPack}</summary>`,
      "",
      `- Tool-pack checks: ${entry.toolPackChecks ? "pass" : "**fail**: every rule of it is blocked (see the report's tool-pack checks)"}`,
      ...BLOCKING_GATES.map((gate) => `- ${gateTitle(gate.id)}: ${entry.failedGate[gate.id]} fail`),
      `- Gap candidates: ${entry.gapCandidates}`,
      `- Run integrity: ${entry.integrity.length === 0 ? "no problems" : entry.integrity.slice(0, 10).map(cell).join("; ")}`,
      "",
      "</details>",
      "",
    ]),
  ];
  let text = lines.join("\n");
  if (Buffer.byteLength(text) > STEP_SUMMARY_LIMIT - 1024) text = `${text.slice(0, STEP_SUMMARY_LIMIT / 2)}\n\nTruncated: the full summary is in summary.json and the uploaded report.\n`;
  return text;
}

/** The paths that changed from `base` to HEAD, or why git could not say. */
async function changedPaths(root: string, base: string, runner: CommandRunner): Promise<{ paths: string[]; exists: (path: string) => boolean } | { error: string }> {
  const diff = await runner.run(["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD"], { cwd: root });
  if (diff.exitCode !== 0) return { error: diff.stderr.trim().split("\n")[0] ?? `git diff exited ${diff.exitCode}` };
  return { paths: diff.stdout.split("\0").filter((path) => path !== ""), exists: (path) => existsSync(join(root, path)) };
}

/**
 * `gates summary <dir> [--base <sha>] [--planned <json>]`: write summary.{md,json} and the pull request comment
 * (comment.md) from the tool-pack reports and the conformance report (CONFORMANCE_REPORT_FILE) in `dir`, append the
 * summary to the step summary, return whether it passed. Without `planned`, every gated tool-pack needs a report; the
 * conformance report is always needed. With `base`, the rule documents changed from it to HEAD are listed with their
 * verdicts. The comment names the commit in PORT_READINESS_COMMIT and links the run in PORT_READINESS_RUN_URL when CI
 * sets them.
 */
export async function writeSummary(root: string, directory: string, base: string | undefined, runner: CommandRunner, planned?: { pack: string; tool: string }[]): Promise<Summary> {
  if (!existsSync(directory)) throw new Error(`${directory} does not exist`);
  const ungated = await ungatedChecks(root, { ...(base === undefined ? {} : { base }), runner });
  const gated = gatedToolPacks(root);
  const changed = base === undefined ? undefined : await changedPaths(root, base, runner);
  const summary = summarize(readReports(directory), ungated, gated, planned ?? gated, { conformance: readConformanceReport(directory), ...(changed === undefined ? {} : { changed }) });
  const markdown = renderSummary(summary);
  await writeFile(join(directory, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
  await writeFile(join(directory, "summary.md"), markdown);
  const context = { commit: process.env.PORT_READINESS_COMMIT ?? "", runUrl: process.env.PORT_READINESS_RUN_URL ?? "" };
  await writeFile(join(directory, "comment.md"), renderComment(summary, context));
  const step = process.env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
  return summary;
}
