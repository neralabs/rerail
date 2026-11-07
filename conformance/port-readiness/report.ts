import { comparedPositions } from "./checks/matches-upstream-tests.js";
import type { GatesReport, RuleReport } from "./run.js";
import { BLOCKING_GATES, GATES, type BlockingGateId, type Check, type GateId, type ParserVerdict } from "./types.js";

export function cell(text: string): string {
  return text.replaceAll("|", "\\|").replaceAll("\n", " ");
}

function mark(pass: boolean): string {
  return pass ? "pass" : "**fail**";
}

export function gateTitle(id: GateId): string {
  return GATES.find((gate) => gate.id === id)!.title;
}

/** The first failing blocking check of a rule, as [gate title, check id, detail]; or the failing tool-pack checks. */
export function firstFailure(rule: RuleReport): { stoppedAt: string; check: string; why: string } | null {
  if (rule.stoppedAt !== null) {
    const failing = rule.gates[rule.stoppedAt].checks.find((check: Check) => !check.ok);
    return { stoppedAt: gateTitle(rule.stoppedAt), check: failing?.id ?? "failed", why: failing?.detail ?? "failed" };
  }
  if (rule.blockedBy !== null) return { stoppedAt: "Tool-pack checks", check: "tool-pack", why: rule.blockedBy };
  return null;
}

function stoppedLabel(rule: RuleReport): string {
  if (rule.pass) return "passes";
  if (rule.stoppedAt === null) return "blocked by the tool-pack checks";
  return `stops at ${rule.stoppedAt}`;
}

function funnel(report: GatesReport, location: "shipped" | "candidate"): string[] {
  const rows = ["| Gate | Entered | Stopped here | Went on |", "| --- | ---: | ---: | ---: |"];
  let entered = report.rules.filter((rule) => rule.location === location).length;
  for (const gate of BLOCKING_GATES) {
    const stopped = report.summary.stoppedAt[location][gate.id];
    rows.push(`| ${gate.title} | ${entered} | ${stopped} | ${entered - stopped} |`);
    entered -= stopped;
  }
  return rows;
}

function matrix(report: GatesReport): string[] {
  const rows = ["| Gate | Fail | Pass |", "| --- | ---: | ---: |"];
  for (const gate of BLOCKING_GATES) {
    const failed = report.summary.failedGate[gate.id];
    rows.push(`| ${gate.title} | ${failed} | ${report.rules.length - failed} |`);
  }
  return rows;
}

function failuresFor(report: GatesReport, gate: BlockingGateId): string[] {
  const failing = report.rules.filter((rule) => !rule.gates[gate].pass);
  if (failing.length === 0) return ["No rule fails this gate."];
  const lines: string[] = [];
  for (const rule of failing) {
    const failure = rule.gates[gate].checks.find((check) => !check.ok);
    lines.push(`- \`${rule.id}\` (${rule.location}): ${failure === undefined ? "failed" : `${failure.id}: ${cell(failure.detail)}`}`);
    if (gate === "not-overfit") {
      for (const literal of rule.gates["not-overfit"].flagged.slice(0, 5)) lines.push(`  - \`${literal.token}\` at \`${literal.pointer}\` in \`${cell(literal.text.slice(0, 80))}\``);
    }
    if (gate === "matches-upstream-tests") {
      const failures = rule.gates["matches-upstream-tests"].checks.filter((check) => !check.ok);
      for (const check of failures.slice(1)) lines.push(`  - ${check.id}: ${cell(check.detail)}`);
      for (const difference of rule.gates["matches-upstream-tests"].differences.slice(0, 4)) lines.push(`  - ${difference.status} \`${difference.fixture}\`: ${cell(difference.reason)}`);
      const goldens = rule.gates["matches-upstream-tests"].goldens;
      for (const golden of goldens.differing.first) lines.push(`  - golden \`${golden.task}/${golden.path}\`: upstream ${golden.expected}, rule ${golden.actual}: ${cell(golden.reason)}`);
      for (const golden of goldens.engineGaps.first) lines.push(`  - engine gap \`${golden.task}/${golden.path}\`: upstream ${golden.expected}, rule ${golden.actual}: ${cell(golden.reason)}`);
    }
  }
  return lines;
}

/**
 * The rules matches-upstream-tests compared without an end part, by the positions compared. A part is left out for the
 * whole rule when upstream does not report it on every finding: for an ESLint rule, one finding on its own tests without
 * an end is enough, so upstream may report ends on the rule's other findings and they go uncompared.
 */
function partialPositions(report: GatesReport): string[] {
  const byPositions = new Map<string, string[]>();
  for (const rule of report.rules) {
    const compared = rule.gates["matches-upstream-tests"].compared;
    if (compared === undefined || (compared.endLine && compared.endCol)) continue;
    const positions = comparedPositions(compared);
    byPositions.set(positions, [...(byPositions.get(positions) ?? []), `\`${rule.id}\` (${rule.location})`]);
  }
  if (byPositions.size === 0) return [];
  return ["", "Rules compared without an end part, left out for every finding because upstream does not report it on every finding (for an ESLint rule, one finding on its own tests without an end is enough):", "", ...[...byPositions].map(([positions, rules]) => `- ${positions}: ${rules.join(", ")}`)];
}

function list(items: string[], empty: string): string[] {
  return items.length === 0 ? [empty] : items.map((item) => `- ${item}`);
}

function parserCell(verdict: ParserVerdict): string {
  if (verdict.syntax.length > 0) return cell(`syntax error: ${verdict.syntax[0]!.slice(0, 70)}`);
  if ((verdict.grammar?.length ?? 0) > 0) return cell(`grammar error: ${verdict.grammar![0]!.slice(0, 70)}`);
  return verdict.grammar === null ? "accepts (no grammar checks)" : "accepts";
}

function gapCandidateTable(report: GatesReport): string[] {
  if (report.gapCandidates.length === 0) return ["None."];
  return [
    "| Gate | File | Verdict | Parsers |",
    "| --- | --- | --- | --- |",
    ...report.gapCandidates.map((candidate) => `| ${candidate.gate} | \`${cell(candidate.file)}\` | ${candidate.verdict} | ${candidate.parsers.length === 0 ? "none recorded" : candidate.parsers.map((verdict) => `${verdict.parser}: ${parserCell(verdict)}`).join("; ")} |`),
  ];
}

function goldensSection(report: GatesReport): string[] {
  const { goldens } = report.summary;
  return [
    `matches-upstream-tests judges each rule on its goldens: real-code files a real-code run found, listed in goldens.tsv with that run named on its source line, whose expected answer is upstream's on the pinned corpus. Rerail's CI does not run the rules over the corpus. The rules have ${goldens.count} golden(s), differ on ${goldens.differing} the engine read fully, and ${goldens.engineGaps} are engine gaps.`,
    "",
    report.corpus === null ? "The corpus was not read on this run." : `The corpus is ${report.corpus.files} file(s) from ${report.corpus.tasks.filter((task) => task.status === "applied").length} applied task(s) and corpora; ${report.corpus.files - report.corpus.excluded} have upstream's answer.`,
  ];
}

function ruleTable(report: GatesReport): string[] {
  const rows = [
    "| Rule | Location | Verdict | Starts | pack.json on | Bytes | Upstream tests matched | Goldens |",
    "| --- | --- | --- | --- | --- | ---: | --- | --- |",
  ];
  for (const rule of report.rules) {
    const totals = rule.gates["matches-upstream-tests"].totals;
    const upstream = totals === null ? "not run" : `${totals.matched}/${totals.expected}, ${totals.extra} extra`;
    const goldens = rule.gates["matches-upstream-tests"].goldens;
    const agreeing = goldens.count - goldens.differing.count - goldens.engineGaps.count;
    const goldenCell = goldens.count === 0 ? "none" : `${agreeing}/${goldens.count} agree${goldens.engineGaps.count === 0 ? "" : `, ${goldens.engineGaps.count} engine gap(s)`}`;
    rows.push(`| \`${rule.id}\` | ${rule.location} | ${stoppedLabel(rule)} | ${rule.defaultOn ? "on" : "off"} | ${rule.on === null ? "-" : rule.on ? "on" : "off"} | ${rule.bytes} | ${upstream} | ${goldenCell} |`);
  }
  return rows;
}

/** The human-readable tool-pack report: the same facts as its JSON, in reading order. Deterministic. */
export function renderMarkdown(report: GatesReport): string {
  const { summary } = report;
  const lines = [
    `# Port readiness: ${report.toolPack}`,
    "",
    `**${summary.shipped.pass} of ${summary.shipped.rules} shipped rules pass every blocking gate; ${summary.candidates.pass} of ${summary.candidates.rules} candidates do.**`,
    "",
    `- Upstream: ${report.upstream.name}, ${report.upstream.repository} at \`${report.upstream.commit}\` (${report.upstream.version}, ${report.upstream.license}); runtime ${report.upstream.runtime}`,
    `- Engine: this checkout's, identity \`${report.engine.sha256.slice(0, 16)}\``,
    `- Cache: ${report.cache.hits} of ${report.cache.rules} rule result(s) from the cache`,
    `- Documents not mapped to an upstream rule: ${report.unmapped.length === 0 ? "none" : report.unmapped.map((entry) => `\`${entry.file}\` (${entry.location})`).join(", ")}`,
    `- conformance.json entries of ${report.tool} listed, no document yet: ${report.listedWithoutDocument}`,
    "",
    "## Tool-pack checks",
    "",
    report.toolPackChecks.pass ? "Every tool-pack check passes." : "A tool-pack check fails, so every shipped rule of this tool-pack fails: the recording is stale.",
    "",
    "| Check | Result | Detail |",
    "| --- | --- | --- |",
    ...report.toolPackChecks.checks.map((check) => `| ${check.id} | ${mark(check.ok)} | ${cell(check.detail)} |`),
    "",
    "## How this run judged",
    "",
    ...Object.entries(report.method).flatMap(([id, entries]) => [`- ${id}:`, ...Object.entries(entries).map(([key, text]) => `  - ${key}: ${text}`)]),
    "",
    "## Where shipped rules stop",
    "",
    ...funnel(report, "shipped"),
    "",
    "## Where candidates stop",
    "",
    ...funnel(report, "candidate"),
    "",
    "## Each gate on its own",
    "",
    ...matrix(report),
    "",
    ...BLOCKING_GATES.flatMap((gate) => [`## ${gate.title}`, "", ...failuresFor(report, gate.id), ...(gate.id === "matches-upstream-tests" ? partialPositions(report) : []), ""]),
    `## ${gateTitle("upstream-status")}`,
    "",
    ...list(report.rules.filter((rule) => !rule.defaultOn).map((rule) => `\`${rule.id}\` (${rule.location}): ${cell(rule.defaultOnReason)}`), "Every rule exists upstream, is stable and is recommended."),
    "",
    "## Goldens",
    "",
    ...goldensSection(report),
    "",
    "## Run integrity",
    "",
    ...list(report.integrity.map(cell), "No problems with the run's inputs."),
    "",
    "## Parser gaps",
    "",
    ...list(report.knownGaps.map((gap) => `${gap.id}: ${gap.proven ? `holds (${gap.kind})` : `does not hold${gap.kind === null ? "" : ` (${gap.kind})`}`}, excused ${gap.excused} file(s)`), "No known gaps are listed."),
    "",
    ...gapCandidateTable(report),
    "",
    "## Every rule",
    "",
    ...ruleTable(report),
    "",
  ];
  return lines.join("\n");
}
