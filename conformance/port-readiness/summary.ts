import { appendFileSync, existsSync, readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

import { notJudgedNote, renderComment, unmappedCandidatesLine } from "./comment.js";
import type { CommandRunner } from "./process.js";
import { cell, firstFailure, gateTitle } from "./report.js";
import { blockingFailures, gatedToolPacks, REPORT_FORMAT, unusedGapLine, type GatesReport } from "./run.js";
import { BLOCKING_GATES } from "./types.js";
import { ungatedChecks, type UngatedResult } from "./ungated.js";

export const SUMMARY_FORMAT = "rerail.port-readiness-summary/v1" as const;

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

export interface Summary {
  format: typeof SUMMARY_FORMAT;
  shipped: { rules: number; pass: number };
  candidates: { rules: number; pass: number };
  blocking: BlockingRow[];
  engineGaps: { pack: string; rule: string; detail: string }[];
  ungated: UngatedResult;
  missingReports: string[];
  /** Gated tool-packs the plan left out: nothing they read changed against the base, so main's verdict stands. */
  notJudged: string[];
  candidateFunnel: Record<string, Record<string, number>>;
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
 * Merge tool-pack reports with the not-gated and pending checks. `planned` is what CI's plan judged (every gated
 * tool-pack unless a pull request's plan left some out): each needs a report, and the gated tool-packs outside it are
 * listed as not judged, which blocks nothing.
 */
export function summarize(reports: GatesReport[], ungated: UngatedResult, gated: { pack: string; tool: string }[], planned = gated): Summary {
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
    for (const rule of report.rules.filter((item) => item.location === "shipped" && !item.pass)) {
      const failure = firstFailure(rule)!;
      blocking.push({ pack: report.toolPack, rule: rule.id, ...failure });
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
  // A refresh (the nightly) found a fresh verdict that differs from the cached one under the same key: the key leaves
  // out something the verdict depends on. Every difference blocks, a candidate's too; the cache holds the fresh result.
  for (const report of reports) {
    for (const drift of report.cache.differing ?? []) {
      blocking.push({ pack: report.toolPack, rule: drift.id, stoppedAt: "Cache", check: drift.what, why: `${drift.file}: cached ${drift.cached}, fresh ${drift.fresh} under the same key, which leaves out something this verdict depends on; the cache now holds the fresh result` });
    }
  }

  const engineGaps = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && rule.stoppedAt === "matches-upstream-tests")
    .filter((rule) => rule.gates["matches-upstream-tests"].checks.filter((check) => !check.ok).every((check) => check.id === "engine-reads-goldens"))
    .map((rule) => ({ pack: report.toolPack, rule: rule.id, detail: rule.gates["matches-upstream-tests"].checks.find((check) => check.id === "engine-reads-goldens")!.detail })));

  const candidateFunnel: Summary["candidateFunnel"] = {};
  const candidateRows: Summary["candidateRows"] = [];
  const promotable: Summary["promotable"] = [];
  for (const report of reports) {
    candidateFunnel[report.toolPack] = Object.fromEntries([...BLOCKING_GATES.map((gate) => [gate.title, report.summary.stoppedAt.candidate[gate.id]] as const), ["passes every gate", report.summary.candidates.pass] as const]);
    for (const rule of report.rules.filter((item) => item.location === "candidate")) {
      const failure = firstFailure(rule);
      if (failure === null) promotable.push({ toolPack: report.toolPack, rule: rule.id, upstreamRule: rule.upstreamRule, startsOn: rule.defaultOn, startReason: rule.defaultOnReason });
      candidateRows.push({ toolPack: report.toolPack, pack: report.pack, rule: rule.id, ...(failure ?? { stoppedAt: "passes", check: "-", why: "-" }) });
    }
  }
  const onOff = reports.flatMap((report) => report.rules
    .filter((rule) => rule.location === "shipped" && rule.on !== null && rule.on !== rule.defaultOn)
    .map((rule) => ({ pack: report.toolPack, rule: rule.id, packJson: rule.on!, upstreamStatus: rule.defaultOn, reason: rule.defaultOnReason })));
  const shipped = reports.reduce((sum, report) => ({ rules: sum.rules + report.summary.shipped.rules, pass: sum.pass + report.summary.shipped.pass }), { rules: 0, pass: 0 });
  const candidates = reports.reduce((sum, report) => ({ rules: sum.rules + report.summary.candidates.rules, pass: sum.pass + report.summary.candidates.pass }), { rules: 0, pass: 0 });
  return {
    format: SUMMARY_FORMAT,
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
      shipped: `${report.summary.shipped.pass}/${report.summary.shipped.rules}`,
      candidates: `${report.summary.candidates.pass}/${report.summary.candidates.rules}`,
      failedGate: Object.fromEntries(BLOCKING_GATES.map((gate) => [gate.id, report.summary.failedGate[gate.id]])),
      integrity: report.integrity,
      gapCandidates: report.gapCandidates.length,
      toolPackChecks: report.toolPackChecks.pass,
    })),
    pass: blocking.length === 0 && reports.every((report) => blockingFailures(report) === 0),
  };
}

function table(header: string[], rows: string[][], limit = TABLE_ROWS): string[] {
  const lines = [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.slice(0, limit).map((row) => `| ${row.map(cell).join(" | ")} |`)];
  if (rows.length > limit) lines.push("", `${rows.length - limit} more row(s) in summary.json and the uploaded report.`);
  return lines;
}

/** The summary as Markdown, under GitHub's step-summary limit. */
export function renderSummary(summary: Summary): string {
  const notJudged = notJudgedNote(summary);
  const lines = [
    "# Port readiness",
    "",
    `**${notJudged === undefined ? "" : `In ${notJudged.scope}: `}${summary.shipped.pass} of ${summary.shipped.rules} shipped rules pass; ${summary.blocking.length} blocking failure(s); ${summary.promotable.length} candidate(s) ready to promote (${summary.candidates.pass} of ${summary.candidates.rules} pass every gate).**`,
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
    "## Not gated",
    "",
    ...(summary.ungated.notGated.length === 0 ? ["None."] : summary.ungated.notGated.map((entry) => `- ${entry.pack}: ${entry.rules} shipped rule(s), frozen (no upstream adapter)`)),
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
    ...table(["Tool-pack", "Rule", "First failing gate", "Check", "Why"], summary.candidateRows.map((row) => [row.toolPack, row.rule, row.stoppedAt, row.check, row.why])),
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
 * (comment.md), append the summary to the step summary, return whether it passed. Without `planned`, every gated
 * tool-pack needs a report. The comment names the commit in PORT_READINESS_COMMIT and links the run in
 * PORT_READINESS_RUN_URL when CI sets them.
 */
export async function writeSummary(root: string, directory: string, base: string | undefined, runner: CommandRunner, planned?: { pack: string; tool: string }[]): Promise<Summary> {
  if (!existsSync(directory)) throw new Error(`${directory} does not exist`);
  const ungated = await ungatedChecks(root, { ...(base === undefined ? {} : { base }), runner });
  const gated = gatedToolPacks(root);
  const summary = summarize(readReports(directory), ungated, gated, planned ?? gated);
  const markdown = renderSummary(summary);
  await writeFile(join(directory, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
  await writeFile(join(directory, "summary.md"), markdown);
  const context = { commit: process.env.PORT_READINESS_COMMIT ?? "", runUrl: process.env.PORT_READINESS_RUN_URL ?? "" };
  await writeFile(join(directory, "comment.md"), renderComment(summary, context));
  const step = process.env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
  return summary;
}
