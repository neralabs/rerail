/**
 * The conformance check's summary over every pack: the blocking failures (the pack reports' failures, and each shipped
 * rule of a gated tool-pack that fails a gate), the candidates ready to promote, and the shipped rules whose pack.json
 * `on` disagrees with upstream-status. Written as summary.{json,md} and the pull request comment (comment.ts).
 */
import { appendFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { cell, renderComment } from "./comment.js";
import type { Check } from "./checks/check.js";
import type { UngatedResult } from "./checks/ungated.js";
import { BLOCKING_GATES, type RuleVerdict, type ToolPackVerdict } from "./gates.js";

export const SUMMARY_FORMAT = "rerail.conformance-summary/v1" as const;

/** GitHub's step-summary limit is 1 MiB; tables stop well before it and say where the rest is. */
export const STEP_SUMMARY_LIMIT = 1024 * 1024;
const TABLE_ROWS = 200;

export interface BlockingRow {
  pack: string;
  rule: string;
  stoppedAt: string;
  check: string;
  why: string;
}

/** One pack's conformance report as the summary reads it: `reportFailures`' lines. */
export interface PackFailures {
  pack: string;
  failures: string[];
}

export interface Summary {
  format: typeof SUMMARY_FORMAT;
  /** The rules of gated tool-packs, shipped and candidate. */
  shipped: { rules: number; pass: number };
  candidates: { rules: number; pass: number };
  blocking: BlockingRow[];
  ungated: UngatedResult;
  candidateFunnel: Record<string, Record<string, number>>;
  candidateRows: (BlockingRow & { toolPack: string })[];
  /** Candidates with no conformance.json entry: no gate judged them, so they are neither ready nor stopped at a gate. */
  unmappedCandidates: { pack: string; file: string }[];
  /** Candidates that pass every gate: the signal to promote them into rules/ (a reviewer still decides). */
  promotable: { toolPack: string; rule: string; upstreamRule: string; startsOn: boolean; startReason: string }[];
  onOff: { pack: string; rule: string; packJson: boolean; upstreamStatus: boolean; reason: string }[];
  /** Shipped rules that pass a check with no evidence behind it (`Check.vacuous`): reported, not blocking. */
  vacuous: { toolPack: string; rule: string; check: string; why: string }[];
  toolPacks: { toolPack: string; shipped: string; candidates: string; failedGate: Record<string, number>; checks: { id: string; ok: boolean; detail: string }[] }[];
  pass: boolean;
}

const GATE_TITLES: Record<NonNullable<RuleVerdict["stoppedAt"]>, string> = {
  ...Object.fromEntries(BLOCKING_GATES.map((gate) => [gate.id, gate.title])) as Record<(typeof BLOCKING_GATES)[number]["id"], string>,
  "tool-pack": "Tool-pack checks",
};

/** A line of `reportFailures` that names rule `id` (`<id> (<source>): …`, `<id>: stale divergence …`). */
const namesRule = (line: string, id: string): boolean => line.startsWith(`${id} `) || line.startsWith(`${id}:`);

function failureRow(toolPack: string, rule: RuleVerdict): BlockingRow {
  return { pack: toolPack, rule: rule.id, stoppedAt: GATE_TITLES[rule.stoppedAt!], check: rule.failure!.check, why: rule.failure!.why };
}

/**
 * A failing not-gated or pending check as a blocking row. Its id is `<kind>:<subject>`: the subject names a pack
 * (`not-gated:`, `frozen-list:`), a tool-pack (`pending:`), or, for `shrinks:`, a list's path, which goes in the why.
 */
export function ungatedRow(check: Check): BlockingRow {
  const separator = check.id.indexOf(":");
  const kind = separator < 0 ? check.id : check.id.slice(0, separator);
  const subject = separator < 0 ? "" : check.id.slice(separator + 1);
  if (kind === "shrinks") return { pack: "-", rule: "-", stoppedAt: "Not gated / pending", check: kind, why: `${subject}: ${check.detail}` };
  return { pack: subject, rule: "-", stoppedAt: "Not gated / pending", check: kind, why: check.detail };
}

/**
 * Merge the pack reports' failures, the gated tool-packs' verdicts and the not-gated checks. A failure line of a shipped
 * rule the gate failed is that rule's gate failure, so it is listed once, as the gate's. A failure line of a rule the
 * gate passed stays: the report fails the job on it, so the summary must not read green.
 */
export function summarize(packs: readonly PackFailures[], toolPacks: readonly ToolPackVerdict[], ungated: UngatedResult): Summary {
  const blocking: BlockingRow[] = [];
  const gateFailed = new Map<string, Set<string>>();
  for (const verdict of toolPacks) {
    const ids = gateFailed.get(verdict.pack) ?? new Set<string>();
    for (const rule of verdict.rules) if (rule.location === "shipped" && !rule.pass) ids.add(rule.id);
    gateFailed.set(verdict.pack, ids);
  }
  for (const { pack, failures } of packs) {
    const failed = [...(gateFailed.get(pack) ?? [])];
    for (const line of failures.filter((entry) => !failed.some((id) => namesRule(entry, id)))) {
      blocking.push({ pack, rule: "-", stoppedAt: "Conformance", check: "report", why: line });
    }
  }
  const unmappedCandidates: Summary["unmappedCandidates"] = [];
  const seenUnmapped = new Set<string>();
  const candidateFunnel: Summary["candidateFunnel"] = {};
  const candidateRows: Summary["candidateRows"] = [];
  const promotable: Summary["promotable"] = [];
  for (const verdict of toolPacks) {
    // Every gated tool-pack of a pack lists the pack's unmapped documents, so each is taken once.
    for (const entry of verdict.unmapped.filter((item) => !seenUnmapped.has(item.file))) {
      seenUnmapped.add(entry.file);
      if (entry.location === "candidate") unmappedCandidates.push({ pack: verdict.pack, file: entry.file });
      else blocking.push({ pack: verdict.pack, rule: entry.file, stoppedAt: "Mapping", check: "conformance-entry", why: "not mapped to an upstream rule: add its conformance.json entry" });
    }
    for (const rule of verdict.rules.filter((item) => item.location === "shipped" && !item.pass)) blocking.push(failureRow(verdict.toolPack, rule));
    const candidates = verdict.rules.filter((item) => item.location === "candidate");
    candidateFunnel[verdict.toolPack] = Object.fromEntries([
      ...[...BLOCKING_GATES.map((gate) => gate.id), "tool-pack" as const].map((id) => [GATE_TITLES[id], candidates.filter((rule) => rule.stoppedAt === id).length] as const),
      ["passes every gate", candidates.filter((rule) => rule.pass).length] as const,
    ]);
    for (const rule of candidates) {
      if (rule.pass) promotable.push({ toolPack: verdict.toolPack, rule: rule.id, upstreamRule: rule.upstreamRule, startsOn: rule.defaultOn, startReason: rule.defaultOnReason });
      candidateRows.push({ toolPack: verdict.toolPack, ...(rule.pass ? { pack: verdict.pack, rule: rule.id, stoppedAt: "passes", check: "-", why: "-" } : failureRow(verdict.toolPack, rule)) });
    }
  }
  for (const check of ungated.checks.filter((item) => !item.ok)) blocking.push(ungatedRow(check));

  const rules = toolPacks.flatMap((verdict) => verdict.rules);
  const count = (location: RuleVerdict["location"]) => ({ rules: rules.filter((rule) => rule.location === location).length, pass: rules.filter((rule) => rule.location === location && rule.pass).length });
  return {
    format: SUMMARY_FORMAT,
    shipped: count("shipped"),
    candidates: count("candidate"),
    blocking,
    ungated,
    candidateFunnel,
    candidateRows,
    unmappedCandidates: unmappedCandidates.sort((a, b) => (a.file < b.file ? -1 : a.file > b.file ? 1 : 0)),
    promotable,
    onOff: toolPacks.flatMap((verdict) => verdict.rules
      .filter((rule) => rule.location === "shipped" && rule.on !== null && rule.on !== rule.defaultOn)
      .map((rule) => ({ pack: verdict.toolPack, rule: rule.id, packJson: rule.on!, upstreamStatus: rule.defaultOn, reason: rule.defaultOnReason }))),
    vacuous: toolPacks.flatMap((verdict) => verdict.rules
      .filter((rule) => rule.location === "shipped")
      .flatMap((rule) => rule.gates.conformance.filter((check) => check.vacuous === true).map((check) => ({ toolPack: verdict.toolPack, rule: rule.id, check: check.id, why: check.detail })))),
    toolPacks: toolPacks.map((verdict) => {
      const of = (location: RuleVerdict["location"]) => verdict.rules.filter((rule) => rule.location === location);
      return {
        toolPack: verdict.toolPack,
        shipped: `${of("shipped").filter((rule) => rule.pass).length}/${of("shipped").length}`,
        candidates: `${of("candidate").filter((rule) => rule.pass).length}/${of("candidate").length}`,
        failedGate: Object.fromEntries(BLOCKING_GATES.map((gate) => [gate.id, verdict.rules.filter((rule) => rule.stoppedAt === gate.id).length])),
        checks: verdict.checks,
      };
    }),
    pass: blocking.length === 0,
  };
}

function table(header: string[], rows: string[][], limit = TABLE_ROWS): string[] {
  const lines = [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.slice(0, limit).map((row) => `| ${row.map(cell).join(" | ")} |`)];
  if (rows.length > limit) lines.push("", `${rows.length - limit} more row(s) in summary.json.`);
  return lines;
}

/** The summary as Markdown, under GitHub's step-summary limit. */
export function renderSummary(summary: Summary): string {
  const lines = [
    "# Conformance",
    "",
    `**${summary.blocking.length} blocking failure(s); ${summary.shipped.pass} of ${summary.shipped.rules} shipped rules of gated tool-packs pass; ${summary.promotable.length} candidate(s) ready to promote (of ${summary.candidates.rules}).**`,
    "",
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
    "## On or off",
    "",
    ...(summary.onOff.length === 0 ? ["Every shipped rule's pack.json on agrees with upstream-status."] : table(["Tool-pack", "Rule", "pack.json", "upstream-status", "Reason"], summary.onOff.map((row) => [row.pack, row.rule, row.packJson ? "on" : "off", row.upstreamStatus ? "on" : "off", row.reason]))),
    "",
    "## Passing without evidence",
    "",
    ...(summary.vacuous.length === 0 ? ["None."] : [
      `${summary.vacuous.length} shipped rule check(s) pass with no evidence behind them: reported, not blocking.`,
      "",
      ...table(["Tool-pack", "Rule", "Check", "Why"], summary.vacuous.map((row) => [row.toolPack, row.rule, row.check, row.why])),
    ]),
    "",
    "## Not gated",
    "",
    ...(summary.ungated.notGated.length === 0 ? ["None."] : summary.ungated.notGated.map((entry) => `- ${entry.pack}: ${entry.rules} shipped rule(s), frozen (no gates.yaml)`)),
    ...summary.ungated.pending.map((entry) => `- ${entry.toolPack}: pending, ${entry.reason}`),
    "",
    "## Candidates",
    "",
    ...Object.entries(summary.candidateFunnel).map(([toolPack, counts]) => `- ${toolPack}: ${Object.entries(counts).map(([gate, count]) => `${gate} ${count}`).join(", ")}`),
    "",
    "<details><summary>Every candidate</summary>",
    "",
    ...table(["Tool-pack", "Rule", "First failing gate", "Check", "Why"], summary.candidateRows.map((row) => [row.toolPack, row.rule, row.stoppedAt, row.check, row.why])),
    "",
    "</details>",
    "",
    ...(summary.unmappedCandidates.length === 0 ? [] : ["## Candidates without a conformance.json entry", "", ...summary.unmappedCandidates.map((entry) => `- ${entry.pack}: \`${entry.file}\``), ""]),
    "## Per tool-pack",
    "",
    ...summary.toolPacks.flatMap((entry) => [
      `<details><summary>${entry.toolPack}: shipped ${entry.shipped}, candidates ${entry.candidates}</summary>`,
      "",
      ...entry.checks.map((check) => `- ${check.id}: ${check.ok ? "pass" : "**fail**"}: ${cell(check.detail)}`),
      ...BLOCKING_GATES.map((gate) => `- ${gate.title}: ${entry.failedGate[gate.id]} fail`),
      "",
      "</details>",
      "",
    ]),
  ];
  const text = lines.join("\n");
  return Buffer.byteLength(text) > STEP_SUMMARY_LIMIT - 1024 ? `${text.slice(0, STEP_SUMMARY_LIMIT / 2)}\n\nTruncated: the full summary is in summary.json.\n` : text;
}

/**
 * Write summary.{json,md} and the pull request comment (comment.md) into `directory`, and append the summary to the
 * step summary. The comment names the commit in CONFORMANCE_COMMIT and links the run in CONFORMANCE_RUN_URL when CI
 * sets them.
 */
export async function writeSummary(directory: string, summary: Summary): Promise<void> {
  const markdown = renderSummary(summary);
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);
  await writeFile(join(directory, "summary.md"), markdown);
  await writeFile(join(directory, "comment.md"), renderComment(summary, { commit: process.env.CONFORMANCE_COMMIT ?? "", runUrl: process.env.CONFORMANCE_RUN_URL ?? "" }));
  const step = process.env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
}
