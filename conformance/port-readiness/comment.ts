/**
 * The pull request comment CI posts after the port readiness gates run: the verdicts and blockers for the shipped rules
 * (`rules/`) and for the candidates (`candidates/`), rendered from the merged summary. CI minimizes the earlier gate
 * comments as outdated before posting a new one, so the pull request keeps its history collapsed.
 *
 * A comment is limited to 65,536 characters, and a pack may hold hundreds of candidates, so the comment leads with
 * counts, keeps every list collapsed, groups candidates by the gate they stop at, and drops detail (longest lists
 * first) until it fits. The job summary and the uploaded reports always have everything.
 */
import { RECORD_LABEL, recordingRoutes, runtimesRecordingRoutes } from "./recording/routes.js";
import { cell } from "./report.js";
import type { BlockingRow, RecordingRoute, Summary } from "./summary.js";

/** Starts every port readiness gates comment, so CI finds the earlier ones to minimize. */
export const COMMENT_MARKER = "<!-- rerail-port-readiness -->";

/** GitHub refuses a comment over 65,536 characters; stay well under it. */
export const COMMENT_LIMIT = 60_000;

/** At most this many characters of a check's detail in a table cell. */
const WHY_LENGTH = 160;

const trim = (text: string, length = WHY_LENGTH): string => (text.length <= length ? text : `${text.slice(0, length - 1)}…`);

/** What a why's recording route becomes in a table cell: the recording line above names it whole. */
const SEE_ABOVE = " (how: see above)";

/**
 * A why as a table cell: each recording route in it (`routes`, those of the tool-packs the recording line names) is
 * swapped for SEE_ABOVE, so cutting the cell to WHY_LENGTH never leaves half a route, then it is cut.
 */
function whyCell(why: string, routes: readonly string[]): string {
  return trim(routes.reduce((text, route) => text.replaceAll(`: ${route}`, SEE_ABOVE), why));
}

/** The routes a why can end its sentences with, for each tool-pack the recording line names (whyCell swaps them). */
function lineRoutes(summary: Summary): string[] {
  return summary.toolPacks.filter((entry) => entry.needsRecording !== null).flatMap((entry) => {
    const [pack, tool] = entry.toolPack.split("/") as [string, string];
    return [recordingRoutes(pack, tool), runtimesRecordingRoutes(pack, tool)];
  });
}

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

/** The line over the candidates no gate judged because conformance.json has no entry for them. */
export function unmappedCandidatesLine(summary: Summary): string {
  const count = summary.unmappedCandidates.length;
  return `${count} candidate${count === 1 ? " is" : "s are"} not judged: no conformance.json entry maps ${count === 1 ? "it" : "them"} to an upstream rule, so no gate ran`;
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

/**
 * The line naming the tool-packs whose report says they must be recorded again (Summary.toolPacks' needsRecording),
 * what a maintainer does for each route, and where a record run's result shows; undefined when none does. It is in the
 * job summary too, where a push to main and a fork's pull request read it. A tool-pack whose runtimes changed is sent to
 * main alone: the record run takes the runtimes from there, so the label would record main's runtimes again. The line
 * promises no comment, since a record run that fails before it publishes (refused, the head moved, a leg failed)
 * posts none.
 */
export function recordingNeededLine(summary: Summary): string | undefined {
  const named = (route: RecordingRoute) => summary.toolPacks.filter((entry) => entry.needsRecording === route).map((entry) => `\`${entry.toolPack}\``);
  const listed = (names: string[]) => `${names.length === 1 ? names[0]! : `${names.slice(0, -1).join(", ")} and ${names.at(-1)!}`} ${names.length === 1 ? "needs" : "need"}`;
  const inPullRequest = named("pull-request");
  const mainFirst = named("main-first");
  const sentences = [
    ...(inPullRequest.length === 0 ? [] : [`${listed(inPullRequest)} recording again: in a pull request, a maintainer adds the \`${RECORD_LABEL}\` label`
      + " (or runs `port-readiness-record.yml` by hand with the pull request's number and head sha); on `main`, a maintainer runs `port-readiness-record.yml` from `main` with `pr` and `sha` empty."]),
    ...(mainFirst.length === 0 ? [] : [`${listed(mainFirst)} recording on \`main\`: the record run takes the recorder's runtimes from \`main\`, so a change to them lands on \`main\` first,`
      + " and a maintainer then runs `port-readiness-record.yml` from `main` with `pr` and `sha` empty."]),
  ];
  if (sentences.length === 0) return undefined;
  return [...sentences, "A record run that works comments on the pull request with what it recorded (one for `main` writes that, and a link to open a pull request, to its job summary); one that fails says why in its log."].join(" ");
}

/** Rows of `rows` grouped by first failing gate, in the order the gates run, each group a table (`routes`: whyCell's). */
function byGate(rows: (BlockingRow & { toolPack: string })[], limit: number, routes: readonly string[]): string[] {
  const groups = new Map<string, (BlockingRow & { toolPack: string })[]>();
  for (const row of rows) groups.set(row.stoppedAt, [...(groups.get(row.stoppedAt) ?? []), row]);
  return [...groups.entries()].flatMap(([gate, members]) => [
    `**${gate}** (${members.length})`,
    "",
    ...table(["Rule", "Check", "Why"], members.slice(0, limit).map((row) => [`\`${row.rule}\``, row.check, whyCell(row.why, routes)])),
    ...(members.length > limit ? ["", `${members.length - limit} more: see the job summary.`] : []),
    "",
  ]);
}

/**
 * The comment as Markdown. `rowLimit` caps each candidate group's table; the caller lowers it until the comment fits
 * (renderComment does).
 */
function render(summary: Summary, context: CommentContext, rowLimit: number, blockerLimit: number, promoteLimit = 30): string {
  const blocking = summary.blocking.length;
  const ready = summary.promotable.length;
  const notJudged = notJudgedNote(summary);
  const recording = recordingNeededLine(summary);
  const routes = lineRoutes(summary);
  const verdict = `${summary.pass ? "✅ every shipped rule passes" : `❌ ${blocking} blocking failure${blocking === 1 ? "" : "s"}`}${ready === 0 ? "" : ` · 🟢 ${ready} candidate${ready === 1 ? "" : "s"} ready to promote`}${notJudged === undefined ? "" : ` · in ${notJudged.scope}`}`;
  const commit = context.commit === undefined || context.commit === "" ? "" : ` at \`${context.commit.slice(0, 10)}\``;
  const run = context.runUrl === undefined || context.runUrl === "" ? "" : ` · [job summary and reports](${context.runUrl})`;
  const candidatesByPack = new Map<string, (BlockingRow & { toolPack: string })[]>();
  for (const row of summary.candidateRows.filter((item) => item.stoppedAt !== "passes")) {
    candidatesByPack.set(row.toolPack, [...(candidatesByPack.get(row.toolPack) ?? []), row]);
  }
  const lines = [
    COMMENT_MARKER,
    `## Port readiness: ${verdict}`,
    "",
    `Shipped rules (\`rules/\`) must pass every gate; candidates (\`candidates/\`) are only reported${commit}${run}.`,
    "",
    ...(notJudged === undefined ? [] : [notJudged.line, ""]),
    ...(recording === undefined ? [] : [recording, ""]),
    ...table(
      ["Tool-pack", "Shipped rules passing", "Candidates ready to promote"],
      [
        ...summary.toolPacks.map((entry) => [entry.toolPack, entry.shipped, entry.candidates]),
        ["**All**", `**${summary.shipped.pass}/${summary.shipped.rules}**`, `**${summary.candidates.pass}/${summary.candidates.rules}**`],
      ],
    ),
    "",
    "### Shipped rules",
    "",
    ...(blocking === 0
      ? [`All ${summary.shipped.rules} shipped rules pass.`, ""]
      : [
          `${blocking} blocking failure${blocking === 1 ? "" : "s"}: fix the rule, or move it to \`candidates/\`.`,
          "",
          ...table(["Pack", "Rule", "Stopped at", "Check", "Why"], summary.blocking.slice(0, blockerLimit).map((row) => [row.pack, `\`${row.rule}\``, row.stoppedAt, row.check, whyCell(row.why, routes)])),
          ...(blocking > blockerLimit ? ["", `${blocking - blockerLimit} more: see the job summary.`] : []),
          "",
        ]),
    ...(summary.engineGaps.length === 0
      ? []
      : details(
          `${summary.engineGaps.length} of them blocked only by engine gaps (the fix is the engine's, NER-1764)`,
          summary.engineGaps.slice(0, blockerLimit).map((row) => `- ${row.pack} \`${row.rule}\`: ${trim(row.detail)}`),
        )),
    "### Ready to promote",
    "",
    ...(ready === 0
      ? ["No candidate passes every gate yet.", ""]
      : [
          `${ready} candidate${ready === 1 ? " passes" : "s pass"} every gate: the signal to promote ${ready === 1 ? "it" : "them"} into \`rules/\` and \`pack.json\`. Passing the gates is necessary, not sufficient; a reviewer still decides. *Starts* is what upstream-status sets for \`on\`.`,
          "",
          ...table(["Tool-pack", "Rule", "Upstream rule", "Starts"], summary.promotable.slice(0, promoteLimit).map((entry) => [entry.toolPack, `\`${entry.rule}\``, entry.upstreamRule, entry.startsOn ? "on" : "off"])),
          "",
          ...(ready > promoteLimit
            ? details(`${ready - promoteLimit} more ready to promote`, table(["Tool-pack", "Rule", "Upstream rule", "Starts"], summary.promotable.slice(promoteLimit).map((entry) => [entry.toolPack, `\`${entry.rule}\``, entry.upstreamRule, entry.startsOn ? "on" : "off"])))
            : []),
        ]),
    "### Candidates",
    "",
    ...table(
      ["Tool-pack", ...Object.keys(Object.values(summary.candidateFunnel)[0] ?? {})],
      Object.entries(summary.candidateFunnel).map(([toolPack, counts]) => [toolPack, ...Object.values(counts).map(String)]),
    ),
    "",
    "Each gate column counts the candidates that stop there first.",
    "",
    ...(summary.unmappedCandidates.length === 0
      ? []
      : details(unmappedCandidatesLine(summary), summary.unmappedCandidates.slice(0, blockerLimit).map((entry) => `- ${entry.pack}: \`${entry.file}\``)
        .concat(summary.unmappedCandidates.length > blockerLimit ? [`- ${summary.unmappedCandidates.length - blockerLimit} more: see the job summary.`] : []))),
    ...[...candidatesByPack.entries()].flatMap(([toolPack, rows]) =>
      details(`${toolPack}: ${rows.length} candidate${rows.length === 1 ? " stops" : "s stop"} at a gate`, rowLimit === 0 ? ["Too many to list here: see the job summary."] : byGate(rows, rowLimit, routes)),
    ),
    ...(summary.ungated.notGated.length === 0 && summary.ungated.pending.length === 0
      ? []
      : [
          `Not gated (no upstream adapter, frozen): ${summary.ungated.notGated.map((entry) => `${entry.pack} (${entry.rules})`).join(", ") || "none"}. Pending: ${summary.ungated.pending.map((entry) => entry.toolPack).join(", ") || "none"}.`,
          "",
        ]),
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
