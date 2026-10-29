/**
 * The one sticky pull request comment the conformance job keeps up to date: the shipped failures, the candidates
 * ready to promote and the default-on mismatches, rendered from the summary. CI edits the comment that starts with
 * COMMENT_MARKER, or posts it when there is none.
 *
 * A comment is limited to 65,536 characters, and a pack may hold hundreds of candidates, so the comment leads with
 * counts, keeps every list collapsed, groups candidates by the gate they stop at, and drops detail (longest lists
 * first) until it fits. The job summary and the uploaded summary always have everything.
 */
import type { BlockingRow, Summary } from "./summary.js";

/** A Markdown table cell: pipes escaped, newlines flattened. */
export function cell(text: string): string {
  return text.replaceAll("|", "\\|").replaceAll(/\r?\n/gu, " ");
}

/** Starts the conformance comment, so CI finds it to edit. */
export const COMMENT_MARKER = "<!-- rerail-conformance -->";

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
  /** The commit the check judged (the pull request head), shown in the heading. */
  commit?: string;
  /** The workflow run, linked for the full job summary. */
  runUrl?: string;
}

/** Rows of `rows` grouped by first failing gate, each group a table. */
function byGate(rows: (BlockingRow & { toolPack: string })[], limit: number): string[] {
  const groups = new Map<string, (BlockingRow & { toolPack: string })[]>();
  for (const row of rows) groups.set(row.stoppedAt, [...(groups.get(row.stoppedAt) ?? []), row]);
  return [...groups.entries()].flatMap(([gate, members]) => [
    `**${gate}** (${members.length})`,
    "",
    ...table(["Rule", "Check", "Why"], members.slice(0, limit).map((row) => [`\`${row.rule}\``, row.check, trim(row.why)])),
    ...(members.length > limit ? ["", `${members.length - limit} more: see the job summary.`] : []),
    "",
  ]);
}

/** The comment as Markdown; `rowLimit` caps each candidate group's table, and the caller lowers it until it fits. */
function render(summary: Summary, context: CommentContext, rowLimit: number, blockerLimit: number, listLimit = 30): string {
  const blocking = summary.blocking.length;
  const ready = summary.promotable.length;
  const verdict = `${summary.pass ? "✅ every shipped rule passes" : `❌ ${blocking} blocking failure${blocking === 1 ? "" : "s"}`}${ready === 0 ? "" : ` · 🟢 ${ready} candidate${ready === 1 ? "" : "s"} ready to promote`}`;
  const commit = context.commit === undefined || context.commit === "" ? "" : ` at \`${context.commit.slice(0, 10)}\``;
  const run = context.runUrl === undefined || context.runUrl === "" ? "" : ` · [job summary](${context.runUrl})`;
  const candidatesByPack = new Map<string, (BlockingRow & { toolPack: string })[]>();
  for (const row of summary.candidateRows.filter((item) => item.stoppedAt !== "passes")) {
    candidatesByPack.set(row.toolPack, [...(candidatesByPack.get(row.toolPack) ?? []), row]);
  }
  const promoteRow = (entry: Summary["promotable"][number]) => [entry.toolPack, `\`${entry.rule}\``, entry.upstreamRule, entry.startsOn ? "on" : "off"];
  const onOffRow = (entry: Summary["onOff"][number]) => [entry.pack, `\`${entry.rule}\``, entry.packJson ? "on" : "off", entry.upstreamStatus ? "on" : "off", trim(entry.reason)];
  const lines = [
    COMMENT_MARKER,
    `## Conformance: ${verdict}`,
    "",
    `Shipped rules (\`rules/\`) must pass every check; candidates (\`candidates/\`) are only reported${commit}${run}.`,
    "",
    ...table(
      ["Tool-pack", "Shipped rules passing", "Candidates ready to promote"],
      [
        ...summary.toolPacks.map((entry) => [entry.toolPack, entry.shipped, entry.candidates]),
        ["**All gated**", `**${summary.shipped.pass}/${summary.shipped.rules}**`, `**${summary.candidates.pass}/${summary.candidates.rules}**`],
      ],
    ),
    "",
    "### Shipped rules",
    "",
    ...(blocking === 0
      ? ["Nothing blocks.", ""]
      : [
          `${blocking} blocking failure${blocking === 1 ? "" : "s"}: fix the rule, or move it to \`candidates/\`.`,
          "",
          ...table(["Pack", "Rule", "Stopped at", "Check", "Why"], summary.blocking.slice(0, blockerLimit).map((row) => [row.pack, `\`${row.rule}\``, row.stoppedAt, row.check, trim(row.why)])),
          ...(blocking > blockerLimit ? ["", `${blocking - blockerLimit} more: see the job summary.`] : []),
          "",
        ]),
    "### Ready to promote",
    "",
    ...(ready === 0
      ? ["No candidate passes every check yet.", ""]
      : [
          `${ready} candidate${ready === 1 ? " passes" : "s pass"} every check: the signal to promote ${ready === 1 ? "it" : "them"} into \`rules/\` and \`pack.json\`. Passing is necessary, not sufficient; a reviewer still decides. *Starts* is what upstream-status sets for \`on\`.`,
          "",
          ...table(["Tool-pack", "Rule", "Upstream rule", "Starts"], summary.promotable.slice(0, listLimit).map(promoteRow)),
          "",
          ...(ready > listLimit ? details(`${ready - listLimit} more ready to promote`, table(["Tool-pack", "Rule", "Upstream rule", "Starts"], summary.promotable.slice(listLimit).map(promoteRow))) : []),
        ]),
    "### On or off",
    "",
    ...(summary.onOff.length === 0
      ? ["Every shipped rule's pack.json `on` agrees with upstream-status.", ""]
      : details(
          `${summary.onOff.length} shipped rule(s) whose pack.json \`on\` differs from upstream-status`,
          [...table(["Tool-pack", "Rule", "pack.json", "upstream-status", "Reason"], summary.onOff.slice(0, blockerLimit).map(onOffRow)), ...(summary.onOff.length > blockerLimit ? ["", `${summary.onOff.length - blockerLimit} more: see the job summary.`] : [])],
        )),
    ...(summary.vacuous.length === 0
      ? []
      : [
          "### Passing without evidence",
          "",
          ...details(
            `${summary.vacuous.length} shipped rule check(s) pass with no evidence behind them (reported, not blocking)`,
            [...table(["Tool-pack", "Rule", "Check", "Why"], summary.vacuous.slice(0, blockerLimit).map((entry) => [entry.toolPack, `\`${entry.rule}\``, entry.check, trim(entry.why)])), ...(summary.vacuous.length > blockerLimit ? ["", `${summary.vacuous.length - blockerLimit} more: see the job summary.`] : [])],
          ),
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
      : details(`${summary.unmappedCandidates.length} candidate(s) not judged: no conformance.json entry maps them to an upstream rule`, summary.unmappedCandidates.slice(0, blockerLimit).map((entry) => `- ${entry.pack}: \`${entry.file}\``))),
    ...[...candidatesByPack.entries()].flatMap(([toolPack, rows]) =>
      details(`${toolPack}: ${rows.length} candidate${rows.length === 1 ? " stops" : "s stop"} at a gate`, rowLimit === 0 ? ["Too many to list here: see the job summary."] : byGate(rows, rowLimit)),
    ),
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
