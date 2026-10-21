/**
 * The pull request comment CI posts after the gates run, rendered from the merged summary in two sections: conformance
 * (every rule of every pack, on the gates every rule must pass) and port readiness (the ported rules of the gated
 * tool-packs, on the gates that judge a port against upstream), each with the verdicts and blockers for the shipped
 * rules (`rules/`) and for the candidates (`candidates/`). CI minimizes the earlier gate
 * comments as outdated before posting a new one, so the pull request keeps its history collapsed.
 *
 * A comment is limited to 65,536 characters, and a pack may hold hundreds of candidates, so the comment leads with
 * counts, keeps every list collapsed, groups candidates by the gate they stop at, and drops detail (longest lists
 * first) until it fits. The job summary and the uploaded reports always have everything.
 */
import { cell, gateTitle } from "./report.js";
import { CONFORMANCE_COLUMN, type BlockingRow, type Summary } from "./summary.js";
import { CONFORMANCE_GATES, GATES, PORT_GATES } from "./types.js";

/** Starts every port readiness gates comment, so CI finds the earlier ones to minimize. */
export const COMMENT_MARKER = "<!-- rerail-port-readiness -->";

/** GitHub refuses a comment over 65,536 characters; stay well under it. */
export const COMMENT_LIMIT = 60_000;

/** At most this many characters of a check's detail in a table cell. */
const WHY_LENGTH = 160;

const trim = (text: string, length = WHY_LENGTH): string => (text.length <= length ? text : `${text.slice(0, length - 1)}…`);

function table(header: string[], rows: string[][]): string[] {
  return [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.map((row) => `| ${row.map(cell).join(" | ")} |`)];
}

function details(summary: string, body: string[]): string[] {
  return ["<details>", `<summary>${summary}</summary>`, "", ...body, "", "</details>", ""];
}

export interface CommentContext {
  /** The commit the gates judged (the pull request head), shown in the heading. */
  commit?: string;
  /** The workflow run, linked for the full job summary and reports. */
  runUrl?: string;
}

/** The line over the candidates no port gate judged because conformance.json has no entry for them. */
export function unmappedCandidatesLine(summary: Summary): string {
  const count = summary.unmappedCandidates.length;
  return `${count} candidate${count === 1 ? " is" : "s are"} not judged for port readiness: no conformance.json entry maps ${count === 1 ? "it" : "them"} to an upstream rule, so no port gate ran (the conformance gates judge ${count === 1 ? "it" : "them"})`;
}

/**
 * When the plan left tool-packs out, what the counts cover (`the 2 of 9 tool-packs judged`: the planned ones, with a
 * report or missing one) and the line naming the rest; undefined when every gated tool-pack was judged.
 */
export function notJudgedNote(summary: Summary): { scope: string; line: string } | undefined {
  if (summary.notJudged.length === 0) return undefined;
  const judged = summary.toolPacks.length + summary.missingReports.length;
  return {
    scope: `the ${judged} of ${judged + summary.notJudged.length} tool-packs judged`,
    line: `Not judged on this change: ${summary.notJudged.join(", ")} (nothing they read changed against the base, so main's verdict stands).`,
  };
}

/** Where a group goes: in the order the gates run, then anything that is no gate (the tool-pack checks). */
const gateOrder = (title: string): number => {
  const index = GATES.findIndex((gate) => gate.title === title);
  return index === -1 ? GATES.length : index;
};

/** Rows of `rows` grouped by first failing gate, in the order the gates run, each group a table. */
function byGate(rows: BlockingRow[], limit: number): string[] {
  const groups = new Map<string, BlockingRow[]>();
  for (const row of rows) groups.set(row.stoppedAt, [...(groups.get(row.stoppedAt) ?? []), row]);
  return [...groups.entries()].sort(([a], [b]) => gateOrder(a) - gateOrder(b)).flatMap(([gate, members]) => [
    `**${gate}** (${members.length})`,
    "",
    ...table(["Rule", "Check", "Why"], members.slice(0, limit).map((row) => [`\`${row.rule}\``, row.check, trim(row.why)])),
    ...(members.length > limit ? ["", `${members.length - limit} more: see the job summary.`] : []),
    "",
  ]);
}

/** The conformance gates by title, joined for prose: `Compiles and loads on this engine`, `Compact (…)` and `Passes …`. */
const titles = (gates: readonly { title: string }[]): string => gates.map((gate) => `*${gate.title}*`).join(", ");

/** One section's verdict: `✅ every shipped rule passes` or `❌ 2 blocking failures`. */
function verdictText(blocking: number): string {
  return blocking === 0 ? "✅ every shipped rule passes" : `❌ ${blocking} blocking failure${blocking === 1 ? "" : "s"}`;
}

/** The conformance section: every rule of every pack, on the gates every rule must pass. */
function conformanceSection(summary: Summary, rowLimit: number, blockerLimit: number): string[] {
  const section = summary.conformance;
  const blocking = section.blocking.length;
  const notJudged = summary.notJudged.length === 0 ? [] : [`The rules of the tool-packs not judged on this change (${summary.notJudged.join(", ")}) keep main's verdict here too.`, ""];
  const byPack = new Map<string, BlockingRow[]>();
  for (const row of section.candidateRows) byPack.set(row.pack, [...(byPack.get(row.pack) ?? []), row]);
  const notApplicablePacks = [...new Set(section.notApplicable.map((entry) => entry.pack))];
  return [
    `### Conformance: ${verdictText(blocking)}`,
    "",
    `Every rule document of every pack, ported or not: ${titles(CONFORMANCE_GATES)}.`,
    "",
    ...notJudged,
    ...table(
      ["Pack", "Shipped rules passing", "Candidates passing"],
      [...section.packs.map((entry) => [entry.pack, entry.shipped, entry.candidates]), ["**All**", `**${section.shipped.pass}/${section.shipped.rules}**`, `**${section.candidates.pass}/${section.candidates.rules}**`]],
    ),
    "",
    ...(blocking === 0
      ? []
      : [
          `${blocking} blocking failure${blocking === 1 ? "" : "s"}: fix the rule, or move it to \`candidates/\`.`,
          "",
          ...table(["Pack", "Rule", "Stopped at", "Check", "Why"], section.blocking.slice(0, blockerLimit).map((row) => [row.pack, `\`${row.rule}\``, row.stoppedAt, row.check, trim(row.why)])),
          ...(blocking > blockerLimit ? ["", `${blocking - blockerLimit} more: see the job summary.`] : []),
          "",
        ]),
    ...(section.notApplicable.length === 0
      ? []
      : [`*Passes the pack's conformance check* does not apply to ${section.notApplicable.length} rule${section.notApplicable.length === 1 ? "" : "s"} (${notApplicablePacks.join(", ")}): no conformance.json entry maps ${section.notApplicable.length === 1 ? "it" : "them"} to an upstream rule to score against.`, ""]),
    ...[...byPack.entries()].flatMap(([pack, rows]) =>
      details(`${pack}: ${rows.length} candidate${rows.length === 1 ? " fails" : "s fail"} a conformance gate`, rowLimit === 0 ? ["Too many to list here: see the job summary."] : byGate(rows, rowLimit)),
    ),
  ];
}

/** The port readiness section: the ported rules of the gated tool-packs, on the gates that judge a port against upstream. */
function portSection(summary: Summary, rowLimit: number, blockerLimit: number, promoteLimit: number): string[] {
  const blocking = summary.blocking.length;
  const ready = summary.promotable.length;
  const notJudged = notJudgedNote(summary);
  const head = `### Port readiness: ${verdictText(blocking)}${ready === 0 ? "" : ` · 🟢 ${ready} candidate${ready === 1 ? "" : "s"} ready to promote`}${notJudged === undefined ? "" : ` · in ${notJudged.scope}`}`;
  const intro = `The rules a gated tool-pack ports, against the upstream linter's recorded answers: ${titles(PORT_GATES)}, and the tool-pack's checks; *${gateTitle("upstream-status")}* sets \`on\`. A rule that does not load is listed under Conformance only.`;
  if (summary.toolPacks.length === 0 && summary.missingReports.length === 0) {
    return [head, "", intro, "", ...(notJudged === undefined ? ["No gated tool-pack."] : [notJudged.line]), ""];
  }
  const candidatesByPack = new Map<string, (BlockingRow & { toolPack: string })[]>();
  for (const row of summary.candidateRows.filter((item) => item.stoppedAt !== "passes" && item.stoppedAt !== CONFORMANCE_COLUMN)) {
    candidatesByPack.set(row.toolPack, [...(candidatesByPack.get(row.toolPack) ?? []), row]);
  }
  const promoteTable = (rows: Summary["promotable"]) => table(["Tool-pack", "Rule", "Upstream rule", "Starts"], rows.map((entry) => [entry.toolPack, `\`${entry.rule}\``, entry.upstreamRule, entry.startsOn ? "on" : "off"]));
  return [
    head,
    "",
    intro,
    "",
    ...(notJudged === undefined ? [] : [notJudged.line, ""]),
    ...table(
      ["Tool-pack", "Shipped rules passing", "Candidates ready to promote"],
      [
        ...summary.toolPacks.map((entry) => [entry.toolPack, entry.shipped, entry.candidates]),
        ["**All**", `**${summary.shipped.pass}/${summary.shipped.rules}**`, `**${summary.candidates.pass}/${summary.candidates.rules}**`],
      ],
    ),
    "",
    "#### Shipped rules",
    "",
    ...(blocking === 0
      ? [`All ${summary.shipped.rules} shipped ported rules pass.`, ""]
      : [
          `${blocking} blocking failure${blocking === 1 ? "" : "s"}: fix the rule, or move it to \`candidates/\`.`,
          "",
          ...table(["Pack", "Rule", "Stopped at", "Check", "Why"], summary.blocking.slice(0, blockerLimit).map((row) => [row.pack, `\`${row.rule}\``, row.stoppedAt, row.check, trim(row.why)])),
          ...(blocking > blockerLimit ? ["", `${blocking - blockerLimit} more: see the job summary.`] : []),
          "",
        ]),
    ...(summary.engineGaps.length === 0
      ? []
      : details(
          `${summary.engineGaps.length} of them blocked only by engine gaps (the fix is the engine's, NER-1764)`,
          summary.engineGaps.slice(0, blockerLimit).map((row) => `- ${row.pack} \`${row.rule}\`: ${trim(row.detail)}`),
        )),
    "#### Ready to promote",
    "",
    ...(ready === 0
      ? ["No candidate passes every gate yet.", ""]
      : [
          `${ready} candidate${ready === 1 ? " passes" : "s pass"} every gate, conformance and port: the signal to promote ${ready === 1 ? "it" : "them"} into \`rules/\` and \`pack.json\`. Passing the gates is necessary, not sufficient; a reviewer still decides. *Starts* is what upstream-status sets for \`on\`.`,
          "",
          ...promoteTable(summary.promotable.slice(0, promoteLimit)),
          "",
          ...(ready > promoteLimit ? details(`${ready - promoteLimit} more ready to promote`, promoteTable(summary.promotable.slice(promoteLimit))) : []),
        ]),
    "#### Candidates",
    "",
    ...table(
      ["Tool-pack", ...Object.keys(Object.values(summary.candidateFunnel)[0] ?? {})],
      Object.entries(summary.candidateFunnel).map(([toolPack, counts]) => [toolPack, ...Object.values(counts).map(String)]),
    ),
    "",
    `Each candidate is counted once: at the first port gate it fails, then the tool-pack checks, then *${CONFORMANCE_COLUMN}* (listed under Conformance).`,
    "",
    ...(summary.unmappedCandidates.length === 0
      ? []
      : details(unmappedCandidatesLine(summary), summary.unmappedCandidates.slice(0, blockerLimit).map((entry) => `- ${entry.pack}: \`${entry.file}\``)
        .concat(summary.unmappedCandidates.length > blockerLimit ? [`- ${summary.unmappedCandidates.length - blockerLimit} more: see the job summary.`] : []))),
    ...[...candidatesByPack.entries()].flatMap(([toolPack, rows]) =>
      details(`${toolPack}: ${rows.length} candidate${rows.length === 1 ? " stops" : "s stop"} at a port gate`, rowLimit === 0 ? ["Too many to list here: see the job summary."] : byGate(rows, rowLimit)),
    ),
    ...(summary.ungated.notGated.length === 0 && summary.ungated.pending.length === 0
      ? []
      : [
          `No port readiness gates (no upstream adapter, frozen): ${summary.ungated.notGated.map((entry) => `${entry.pack} (${entry.rules})`).join(", ") || "none"}. Pending: ${summary.ungated.pending.map((entry) => entry.toolPack).join(", ") || "none"}. The conformance gates still judge their rules.`,
          "",
        ]),
  ];
}

/**
 * The comment as Markdown: one comment, two sections. Conformance covers every rule of every pack; port readiness
 * only the ported rules of the gated tool-packs. `rowLimit` caps each candidate group's table; the caller lowers it
 * until the comment fits (renderComment does).
 */
function render(summary: Summary, context: CommentContext, rowLimit: number, blockerLimit: number, promoteLimit = 30): string {
  const blocking = summary.blocking.length + summary.conformance.blocking.length;
  const ready = summary.promotable.length;
  const commit = context.commit === undefined || context.commit === "" ? "" : ` at \`${context.commit.slice(0, 10)}\``;
  const run = context.runUrl === undefined || context.runUrl === "" ? "" : ` · [job summary and reports](${context.runUrl})`;
  const lines = [
    COMMENT_MARKER,
    `## Conformance and port readiness: ${verdictText(blocking)}${ready === 0 ? "" : ` · 🟢 ${ready} candidate${ready === 1 ? "" : "s"} ready to promote`}`,
    "",
    `Shipped rules (\`rules/\`) must pass every gate that applies to them; candidates (\`candidates/\`) are only reported${commit}${run}.`,
    "",
    ...conformanceSection(summary, rowLimit, blockerLimit),
    ...portSection(summary, rowLimit, blockerLimit, promoteLimit),
  ];
  return `${lines.join("\n").trimEnd()}\n`;
}

/** The comment, with candidate tables shortened (then dropped) until it fits under COMMENT_LIMIT. */
export function renderComment(summary: Summary, context: CommentContext = {}): string {
  for (const rowLimit of [200, 50, 20, 5, 0]) {
    const text = render(summary, context, rowLimit, 50);
    if (text.length <= COMMENT_LIMIT) return text;
  }
  const text = render(summary, context, 0, 10, 10);
  return text.length <= COMMENT_LIMIT ? text : `${text.slice(0, COMMENT_LIMIT - 200)}\n\n…truncated: see the job summary.\n`;
}
