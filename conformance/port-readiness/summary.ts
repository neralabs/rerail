import { appendFileSync, existsSync, readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { notJudgedNote, renderComment, unmappedCandidatesLine } from "./comment.js";
import { CONFORMANCE_REPORT_FILE, CONFORMANCE_REPORT_FORMAT, type ConformanceReport } from "./conformance-gates.js";
import type { CommandRunner } from "./process.js";
import { cell, gateTitle } from "./report.js";
import { blockingFailures, gatedToolPacks, REPORT_FORMAT, unusedGapLine, type GateReport, type GatesReport, type RuleReport } from "./run.js";
import { BLOCKING_GATES, CONFORMANCE_GATES, PORT_GATES, type Check, type ConformanceGateId } from "./types.js";
import { ungatedChecks, type UngatedResult } from "./ungated.js";

export const SUMMARY_FORMAT = "rerail.port-readiness-summary/v2" as const;

/** GitHub's step-summary limit is 1 MiB; tables stop well before it and say where the rest is. */
export const STEP_SUMMARY_LIMIT = 1024 * 1024;
const TABLE_ROWS = 200;

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

export interface BlockingRow {
  pack: string;
  rule: string;
  stoppedAt: string;
  check: string;
  why: string;
}

/**
 * The conformance section: the conformance gates (types.ts CONFORMANCE_GATES) over every rule document of every pack,
 * from the tool-pack reports for the rules a gated tool-pack judges and from the conformance report for the rest.
 */
export interface ConformanceSummary {
  shipped: { rules: number; pass: number };
  candidates: { rules: number; pass: number };
  /** Per pack, `<pass>/<rules>`. */
  packs: { pack: string; shipped: string; candidates: string }[];
  /** Shipped rules that fail a conformance gate, failing pack checks, and a missing conformance report. */
  blocking: BlockingRow[];
  /** Per pack, how many candidates stop first at each conformance gate, and how many pass them all. */
  funnel: Record<string, Record<string, number>>;
  /** The candidates that fail a conformance gate, at the first one they fail. */
  candidateRows: BlockingRow[];
  /** Rules `passes-conformance` does not apply to: conformance.json does not map them. */
  notApplicable: { pack: string; file: string; reason: string }[];
}

/**
 * The summary CI posts. `conformance` is the conformance section; every other field is the port readiness section,
 * which reads only the gated tool-packs' reports and judges their rules on the port gates (PORT_GATES) and the
 * tool-pack checks. A rule that does not load is the conformance section's: no port gate can judge it.
 */
export interface Summary {
  format: typeof SUMMARY_FORMAT;
  conformance: ConformanceSummary;
  /** Shipped ported rules, and those that pass the port gates and their tool-pack's checks. */
  shipped: { rules: number; pass: number };
  /** Candidates of gated tool-packs, and those that pass every gate (ready to promote). */
  candidates: { rules: number; pass: number };
  blocking: BlockingRow[];
  engineGaps: { pack: string; rule: string; detail: string }[];
  ungated: UngatedResult;
  missingReports: string[];
  /** Gated tool-packs the plan left out: nothing they read changed against the base, so main's verdict stands. */
  notJudged: string[];
  /** Per tool-pack, how many candidates stop at each port gate, at the tool-pack checks, at a conformance gate, or are ready. */
  candidateFunnel: Record<string, Record<string, number>>;
  /** Every candidate of a gated tool-pack, at what keeps it from promotion (`passes` when nothing does). */
  candidateRows: (BlockingRow & { toolPack: string })[];
  /** Candidates with no conformance.json entry: no gate judged them, so they are neither ready nor stopped at a gate. */
  unmappedCandidates: { pack: string; file: string }[];
  /** Candidates that pass every gate: the signal to promote them into rules/ (a reviewer still decides). */
  promotable: { toolPack: string; rule: string; upstreamRule: string; startsOn: boolean; startReason: string }[];
  onOff: { pack: string; rule: string; packJson: boolean; upstreamStatus: boolean; reason: string }[];
  toolPacks: { toolPack: string; shipped: string; candidates: string; failedGate: Record<string, number>; integrity: string[]; gapCandidates: number; toolPackChecks: boolean }[];
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

/** The first conformance gate a rule fails, or undefined. */
function conformanceFailure(gates: Record<ConformanceGateId, GateReport>): { stoppedAt: string; check: string; why: string } | undefined {
  const stopped = CONFORMANCE_GATES.find((gate) => !gates[gate.id].pass);
  if (stopped === undefined) return undefined;
  const failing = gates[stopped.id].checks.find((check: Check) => !check.ok);
  return { stoppedAt: stopped.title, check: failing?.id ?? "failed", why: failing?.detail ?? "failed" };
}

/**
 * What keeps a ported rule from passing the port readiness section, or null: the first port gate it fails, then its
 * tool-pack's checks. Undefined for a rule that does not load, which no port gate can judge.
 */
export function portFailure(rule: RuleReport): { stoppedAt: string; check: string; why: string } | null | undefined {
  if (!rule.gates.loads.pass) return undefined;
  const stopped = PORT_GATES.find((gate) => !rule.gates[gate.id].pass);
  if (stopped !== undefined) {
    const failing = rule.gates[stopped.id].checks.find((check: Check) => !check.ok);
    return { stoppedAt: stopped.title, check: failing?.id ?? "failed", why: failing?.detail ?? "failed" };
  }
  if (rule.blockedBy !== null) return { stoppedAt: "Tool-pack checks", check: "tool-pack", why: rule.blockedBy };
  return null;
}

/** The candidate funnel's column for a candidate that passes the port gates but fails a conformance gate (or does not load). */
export const CONFORMANCE_COLUMN = "Fails a conformance gate";
export const READY_COLUMN = "Ready to promote";

/**
 * The conformance section, from the tool-pack reports' rules and the conformance report's (null: it is missing, which
 * blocks). Each document is counted once: a tool-pack report holds only its own tool's rules.
 */
export function summarizeConformance(reports: GatesReport[], conformance: ConformanceReport | null): ConformanceSummary {
  const rows: { pack: string; id: string; file: string; location: "shipped" | "candidate"; gates: Record<ConformanceGateId, GateReport & { applies?: boolean }> }[] = [
    ...reports.flatMap((report) => report.rules.map((rule) => ({ pack: report.pack, id: rule.id, file: rule.file, location: rule.location, gates: rule.gates }))),
    ...(conformance?.rules ?? []).map((rule) => ({ pack: rule.pack, id: rule.id, file: rule.file, location: rule.location, gates: rule.gates })),
  ];
  const blocking: BlockingRow[] = [];
  const candidateRows: BlockingRow[] = [];
  const funnel: ConformanceSummary["funnel"] = {};
  const counts = new Map<string, { shipped: number; shippedPass: number; candidates: number; candidatesPass: number }>();
  for (const row of [...rows].sort((a, b) => byText(a.file, b.file))) {
    const failure = conformanceFailure(row.gates);
    const count = counts.get(row.pack) ?? { shipped: 0, shippedPass: 0, candidates: 0, candidatesPass: 0 };
    counts.set(row.pack, count);
    if (row.location === "shipped") {
      count.shipped += 1;
      if (failure === undefined) count.shippedPass += 1;
      else blocking.push({ pack: row.pack, rule: row.id, ...failure });
      continue;
    }
    count.candidates += 1;
    funnel[row.pack] ??= Object.fromEntries([...CONFORMANCE_GATES.map((gate) => [gate.title, 0] as const), ["passes every conformance gate", 0] as const]);
    funnel[row.pack]![failure?.stoppedAt ?? "passes every conformance gate"]! += 1;
    if (failure === undefined) count.candidatesPass += 1;
    else candidateRows.push({ pack: row.pack, rule: row.id, ...failure });
  }
  for (const entry of conformance?.packs ?? []) {
    for (const check of entry.checks.checks.filter((item) => !item.ok)) blocking.push({ pack: entry.pack, rule: "-", stoppedAt: "Pack checks", check: check.id, why: check.detail });
  }
  if (conformance === null) blocking.push({ pack: "(every pack)", rule: "(every rule no tool-pack judges)", stoppedAt: "Run", check: "report", why: `no ${CONFORMANCE_REPORT_FILE} in the directory: the conformance gates did not run` });
  const packs = [...counts.entries()].sort(([a], [b]) => byText(a, b));
  const total = (pick: (count: { shipped: number; shippedPass: number; candidates: number; candidatesPass: number }) => number) => packs.reduce((sum, [, count]) => sum + pick(count), 0);
  return {
    shipped: { rules: total((count) => count.shipped), pass: total((count) => count.shippedPass) },
    candidates: { rules: total((count) => count.candidates), pass: total((count) => count.candidatesPass) },
    packs: packs.map(([pack, count]) => ({ pack, shipped: `${count.shippedPass}/${count.shipped}`, candidates: `${count.candidatesPass}/${count.candidates}` })),
    blocking,
    funnel: Object.fromEntries(Object.entries(funnel).sort(([a], [b]) => byText(a, b))),
    candidateRows,
    notApplicable: (conformance?.rules ?? []).filter((rule) => !rule.gates["passes-conformance"].applies)
      .map((rule) => ({ pack: rule.pack, file: rule.file, reason: rule.gates["passes-conformance"].checks[0]?.detail ?? "" })),
  };
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

/**
 * Merge tool-pack reports with the conformance report and the not-gated and pending checks. `planned` is what CI's plan
 * judged (every gated tool-pack unless a pull request's plan left some out): each needs a report, and the gated
 * tool-packs outside it are listed as not judged, which blocks nothing. `conformance` is null when its report is
 * missing, which blocks; tests that judge only the port readiness section leave it out.
 */
export function summarize(reports: GatesReport[], ungated: UngatedResult, gated: { pack: string; tool: string }[], planned = gated, conformance: ConformanceReport | null = { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: "" }, packs: [], rules: [] }): Summary {
  const have = new Set(reports.map((report) => report.toolPack));
  const gatedNames = gated.map(({ pack, tool }) => `${pack}/${tool}`);
  const plannedNames = new Set(planned.map(({ pack, tool }) => `${pack}/${tool}`));
  const missingReports = gatedNames.filter((toolPack) => plannedNames.has(toolPack) && !have.has(toolPack));
  const notJudged = gatedNames.filter((toolPack) => !plannedNames.has(toolPack));
  const blocking: BlockingRow[] = [];
  const unmappedCandidates: Summary["unmappedCandidates"] = [];
  // Every tool-pack report of a pack lists the pack's unmapped documents, so each is taken once.
  const seenUnmapped = new Set<string>();
  for (const report of reports) {
    for (const entry of report.unmapped) {
      if (seenUnmapped.has(entry.file)) continue;
      seenUnmapped.add(entry.file);
      if (entry.location === "candidate") unmappedCandidates.push({ pack: report.pack, file: entry.file });
      else blocking.push({ pack: report.pack, rule: entry.file, stoppedAt: "Mapping", check: "conformance-entry", why: "not mapped to an upstream rule: add its conformance.json entry" });
    }
    // A shipped rule that fails only conformance gates (or does not load) is the conformance section's blocker.
    for (const rule of report.rules.filter((item) => item.location === "shipped")) {
      const failure = portFailure(rule);
      if (failure !== undefined && failure !== null) blocking.push({ pack: report.toolPack, rule: rule.id, ...failure });
    }
    // A listed known gap that does not hold, or excused nothing across every shard, is a stale gates.yaml entry.
    for (const gap of report.knownGaps.filter((entry) => !entry.proven || entry.excused === 0)) {
      blocking.push({ pack: report.toolPack, rule: "-", stoppedAt: "Known gaps", check: "known-gap", why: gap.proven ? unusedGapLine(gap.id) : `known gap ${gap.id} does not hold (the report's run integrity says why); remove it or correct it` });
    }
  }
  for (const toolPack of missingReports) blocking.push({ pack: toolPack, rule: "(every rule)", stoppedAt: "Run", check: "report", why: "the gated tool-pack has no report in the directory" });
  // The plan finds tool-packs as gatedToolPacks does, so one it planned that is not gated is a bug in the plan.
  for (const toolPack of [...plannedNames].filter((name) => !gatedNames.includes(name))) {
    blocking.push({ pack: toolPack, rule: "(every rule)", stoppedAt: "Run", check: "plan", why: "planned, but not a gated tool-pack (no gates.yaml): the plan is wrong" });
  }
  for (const check of ungated.checks.filter((item) => !item.ok)) blocking.push({ pack: check.id.split(":")[1] ?? "", rule: "-", stoppedAt: "Not gated / pending", check: check.id.split(":")[0]!, why: check.detail });

  const engineGaps = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && portFailure(rule)?.stoppedAt === gateTitle("matches-upstream-tests"))
    .filter((rule) => rule.gates["matches-upstream-tests"].checks.filter((check) => !check.ok).every((check) => check.id === "engine-reads-goldens"))
    .map((rule) => ({ pack: report.toolPack, rule: rule.id, detail: rule.gates["matches-upstream-tests"].checks.find((check) => check.id === "engine-reads-goldens")!.detail })));

  const candidateFunnel: Summary["candidateFunnel"] = {};
  const candidateRows: Summary["candidateRows"] = [];
  const promotable: Summary["promotable"] = [];
  for (const report of reports) {
    const funnel: Record<string, number> = Object.fromEntries([...PORT_GATES.map((gate) => gate.title), "Tool-pack checks", CONFORMANCE_COLUMN, READY_COLUMN].map((column) => [column, 0]));
    candidateFunnel[report.toolPack] = funnel;
    for (const rule of report.rules.filter((item) => item.location === "candidate")) {
      const port = portFailure(rule);
      const conformanceFails = conformanceFailure(rule.gates);
      // Ready to promote: every gate, conformance and port, and the tool-pack checks.
      const failure = port ?? (conformanceFails === undefined ? null : { stoppedAt: CONFORMANCE_COLUMN, check: conformanceFails.check, why: `${conformanceFails.stoppedAt}: ${conformanceFails.why}` });
      funnel[failure === null ? READY_COLUMN : port === undefined ? CONFORMANCE_COLUMN : failure.stoppedAt]! += 1;
      if (failure === null) promotable.push({ toolPack: report.toolPack, rule: rule.id, upstreamRule: rule.upstreamRule, startsOn: rule.defaultOn, startReason: rule.defaultOnReason });
      candidateRows.push({ toolPack: report.toolPack, pack: report.pack, rule: rule.id, ...(failure ?? { stoppedAt: "passes", check: "-", why: "-" }) });
    }
  }
  const onOff = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && rule.on !== null && rule.on !== rule.defaultOn)
    .map((rule) => ({ pack: report.toolPack, rule: rule.id, packJson: rule.on!, upstreamStatus: rule.defaultOn, reason: rule.defaultOnReason })));
  const portCounts = (report: GatesReport) => {
    const shippedRules = report.rules.filter((rule) => rule.location === "shipped");
    return { rules: shippedRules.length, pass: shippedRules.filter((rule) => portFailure(rule) === null).length };
  };
  const shipped = reports.reduce((sum, report) => ({ rules: sum.rules + portCounts(report).rules, pass: sum.pass + portCounts(report).pass }), { rules: 0, pass: 0 });
  const candidates = { rules: candidateRows.length, pass: promotable.length };
  const conformanceSummary = summarizeConformance(reports, conformance);
  return {
    format: SUMMARY_FORMAT,
    conformance: conformanceSummary,
    shipped,
    candidates,
    blocking,
    engineGaps,
    ungated,
    missingReports,
    notJudged,
    candidateFunnel,
    candidateRows,
    unmappedCandidates: unmappedCandidates.sort((a, b) => byText(a.file, b.file)),
    promotable,
    onOff,
    toolPacks: reports.map((report) => ({
      toolPack: report.toolPack,
      shipped: `${portCounts(report).pass}/${portCounts(report).rules}`,
      candidates: `${promotable.filter((entry) => entry.toolPack === report.toolPack).length}/${report.summary.candidates.rules}`,
      failedGate: Object.fromEntries(BLOCKING_GATES.map((gate) => [gate.id, report.summary.failedGate[gate.id]])),
      integrity: report.integrity,
      gapCandidates: report.gapCandidates.length,
      toolPackChecks: report.toolPackChecks.pass,
    })),
    pass: blocking.length === 0 && conformanceSummary.blocking.length === 0 && reports.every((report) => blockingFailures(report) === 0),
  };
}

function table(header: string[], rows: string[][], limit = TABLE_ROWS): string[] {
  const lines = [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.slice(0, limit).map((row) => `| ${row.map(cell).join(" | ")} |`)];
  if (rows.length > limit) lines.push("", `${rows.length - limit} more row(s) in summary.json and the uploaded report.`);
  return lines;
}

/** The summary as Markdown, under GitHub's step-summary limit: the conformance section, then port readiness. */
export function renderSummary(summary: Summary): string {
  const notJudged = notJudgedNote(summary);
  const section = summary.conformance;
  const lines = [
    "# Conformance",
    "",
    `Every rule document of every pack, ported or not: ${CONFORMANCE_GATES.map((gate) => gate.title).join("; ")}.`,
    "",
    `**${section.shipped.pass} of ${section.shipped.rules} shipped rules pass; ${section.blocking.length} blocking failure(s); ${section.candidates.pass} of ${section.candidates.rules} candidates pass.**`,
    "",
    ...(summary.notJudged.length === 0 ? [] : [`The rules of the tool-packs not judged on this change (${summary.notJudged.join(", ")}) keep main's verdict here too.`, ""]),
    ...table(["Pack", "Shipped rules passing", "Candidates passing"], section.packs.map((entry) => [entry.pack, entry.shipped, entry.candidates])),
    "",
    "## Conformance blocking failures",
    "",
    ...(section.blocking.length === 0 ? ["None."] : table(["Pack", "Rule", "Stopped at", "Check", "Why"], section.blocking.map((row) => [row.pack, row.rule, row.stoppedAt, row.check, row.why]))),
    "",
    "## Conformance: candidates",
    "",
    ...Object.entries(section.funnel).map(([pack, counts]) => `- ${pack}: ${Object.entries(counts).map(([gate, count]) => `${gate} ${count}`).join(", ")}`),
    "",
    ...(section.candidateRows.length === 0 ? [] : ["<details><summary>Every candidate that fails a conformance gate</summary>", "", ...table(["Pack", "Rule", "First failing gate", "Check", "Why"], section.candidateRows.map((row) => [row.pack, row.rule, row.stoppedAt, row.check, row.why])), "", "</details>", ""]),
    ...(section.notApplicable.length === 0 ? [] : [`Passes the pack's conformance check does not apply to ${section.notApplicable.length} rule(s):`, "", ...section.notApplicable.map((entry) => `- ${entry.pack}: \`${entry.file}\`: ${entry.reason}`), ""]),
    "# Port readiness",
    "",
    `The rules a gated tool-pack ports, against the upstream linter's recorded answers: ${PORT_GATES.map((gate) => gate.title).join("; ")}, and the tool-pack's checks. A rule that does not load is listed under Conformance only.`,
    "",
    `**${notJudged === undefined ? "" : `In ${notJudged.scope}: `}${summary.shipped.pass} of ${summary.shipped.rules} shipped ported rules pass; ${summary.blocking.length} blocking failure(s); ${summary.promotable.length} candidate(s) ready to promote (they pass every gate, conformance and port).**`,
    "",
    ...(notJudged === undefined ? [] : [notJudged.line, ""]),
    "## Blocking failures",
    "",
    ...(summary.blocking.length === 0 ? ["None."] : table(["Pack", "Rule", "Stopped at", "Check", "Why"], summary.blocking.map((row) => [row.pack, row.rule, row.stoppedAt, row.check, row.why]))),
    "",
    "## Ready to promote",
    "",
    ...(summary.promotable.length === 0
      ? ["No candidate passes every gate yet."]
      : [
          `${summary.promotable.length} candidate(s) pass every gate: the signal to promote them into \`rules/\` and \`pack.json\`. Passing the gates is necessary, not sufficient; a reviewer still decides. "Starts" is what upstream-status sets for pack.json \`on\`.`,
          "",
          ...table(["Tool-pack", "Rule", "Upstream rule", "Starts", "Why"], summary.promotable.map((row) => [row.toolPack, row.rule, row.upstreamRule, row.startsOn ? "on" : "off", row.startReason])),
        ]),
    "",
    "## Blocked by engine gaps",
    "",
    ...(summary.engineGaps.length === 0 ? ["None."] : table(["Pack", "Rule", "Files"], summary.engineGaps.map((row) => [row.pack, row.rule, row.detail]))),
    "",
    "## No port readiness gates",
    "",
    ...(summary.ungated.notGated.length === 0 ? ["None."] : summary.ungated.notGated.map((entry) => `- ${entry.pack}: ${entry.rules} shipped rule(s), frozen (no upstream adapter); the conformance gates still judge them`)),
    "",
    "## Pending",
    "",
    ...(summary.ungated.pending.length === 0 ? ["None."] : summary.ungated.pending.map((entry) => `- ${entry.toolPack}: pending, ${entry.reason}`)),
    "",
    ...summary.ungated.checks.filter((check) => !check.ok).map((check) => `- **${check.id}**: ${check.detail}`),
    "",
    "## Candidates",
    "",
    ...Object.entries(summary.candidateFunnel).flatMap(([toolPack, counts]) => [`- ${toolPack}: ${Object.entries(counts).map(([gate, count]) => `${gate} ${count}`).join(", ")}`]),
    "",
    `Ready to promote: ${summary.promotable.length} (listed above).`,
    "",
    "<details><summary>Every candidate</summary>",
    "",
    ...table(["Tool-pack", "Rule", "Stopped at", "Check", "Why"], summary.candidateRows.map((row) => [row.toolPack, row.rule, row.stoppedAt, row.check, row.why])),
    "",
    "</details>",
    "",
    ...(summary.unmappedCandidates.length === 0 ? [] : [unmappedCandidatesLine(summary), "", ...summary.unmappedCandidates.map((entry) => `- ${entry.pack}: \`${entry.file}\``), ""]),
    "## On or off",
    "",
    ...(summary.onOff.length === 0 ? ["Every shipped rule's pack.json on agrees with upstream-status."] : table(["Pack", "Rule", "pack.json", "upstream-status", "Reason"], summary.onOff.map((row) => [row.pack, row.rule, row.packJson ? "on" : "off", row.upstreamStatus ? "on" : "off", row.reason]))),
    "",
    "## Per tool-pack",
    "",
    ...summary.toolPacks.flatMap((entry) => [
      `<details><summary>${entry.toolPack}: shipped ${entry.shipped}, candidates ${entry.candidates}</summary>`,
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

/**
 * `gates summary <dir> [--base <sha>] [--planned <json>]`: write summary.{md,json} and the pull request comment
 * (comment.md) from the tool-pack reports and the conformance report (CONFORMANCE_REPORT_FILE) in `dir`, append the
 * summary to the step summary, return whether it passed. Without `planned`, every gated tool-pack needs a report; the
 * conformance report is always needed. The comment names the commit in PORT_READINESS_COMMIT and links the run in
 * PORT_READINESS_RUN_URL when CI sets them.
 */
export async function writeSummary(root: string, directory: string, base: string | undefined, runner: CommandRunner, planned?: { pack: string; tool: string }[]): Promise<Summary> {
  if (!existsSync(directory)) throw new Error(`${directory} does not exist`);
  const ungated = await ungatedChecks(root, { ...(base === undefined ? {} : { base }), runner });
  const gated = gatedToolPacks(root);
  const summary = summarize(readReports(directory), ungated, gated, planned ?? gated, readConformanceReport(directory));
  const markdown = renderSummary(summary);
  await writeFile(join(directory, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
  await writeFile(join(directory, "summary.md"), markdown);
  const context = { commit: process.env.PORT_READINESS_COMMIT ?? "", runUrl: process.env.PORT_READINESS_RUN_URL ?? "" };
  await writeFile(join(directory, "comment.md"), renderComment(summary, context));
  const step = process.env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
  return summary;
}
