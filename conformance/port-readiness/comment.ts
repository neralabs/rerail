/**
 * The pull request comment CI posts after the gates run, rendered from the merged summary, decisions first: whether
 * every shipped rule passes (and, if not, each blocking failure), the verdict of every rule document the pull request
 * changes, the candidates ready to promote, and then, collapsed, where every candidate stands. Each rule is counted
 * once, at the first step of the funnel it fails (summary.ts STEPS). CI minimizes the earlier gate comments as outdated
 * before posting a new one, so the pull request keeps its history collapsed.
 *
 * A comment is limited to 65,536 characters, and a pack may hold hundreds of candidates, so lists are capped and
 * shortened until the comment fits. The job summary and the uploaded reports always have everything.
 */
import { cell, gateTitle } from "./report.js";
import { bySource, byVerdict, funnel, funnelTable, sourceTable, STEPS, UNMAPPED_SUFFIX, verdictText, type RuleVerdict, type Summary } from "./summary.js";

/** Starts every port readiness gates comment, so CI finds the earlier ones to minimize. */
export const COMMENT_MARKER = "<!-- rerail-port-readiness -->";

/** GitHub refuses a comment over 65,536 characters; stay well under it. */
export const COMMENT_LIMIT = 60_000;

/** At most this many characters of a check's detail in a table cell. */
const WHY_LENGTH = 160;

const trim = (text: string, length = WHY_LENGTH): string => (text.length <= length ? text : `${text.slice(0, length - 1)}…`);
const plural = (count: number, one: string, many = `${one}s`): string => `${count} ${count === 1 ? one : many}`;

function table(header: string[], rows: string[][]): string[] {
  return [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.map((row) => `| ${row.map(cell).join(" | ")} |`)];
}

function details(summary: string, body: string[]): string[] {
  return ["<details>", `<summary>${summary}</summary>`, "", ...body, "", "</details>", ""];
}

/** `rows` capped at `limit`, with a line saying how many more the job summary has. */
function capped<T>(rows: T[], limit: number, render: (rows: T[]) => string[]): string[] {
  return [...render(rows.slice(0, limit)), ...(rows.length > limit ? ["", `${rows.length - limit} more: see the job summary.`] : [])];
}

export interface CommentContext {
  /** The commit the gates judged (the pull request head), shown in the heading. */
  commit?: string;
  /** The workflow run, linked for the full job summary and reports. */
  runUrl?: string;
}

/** The line naming the gated tool-packs the plan left out, whose rules keep main's verdict; undefined when it left none. */
export function notJudgedLine(summary: Summary): string | undefined {
  if (summary.notJudged.length === 0) return undefined;
  return `Not judged on this change: ${summary.notJudged.join(", ")} (nothing they read changed against the base, so main's verdict stands; the counts leave them out).`;
}

/** Which rules `passes-conformance` does not apply to: a pack's with no conformance.json, and documents no entry maps. */
export function notApplicableLine(summary: Summary): string {
  const unmapped = summary.notApplicable.filter((entry) => entry.source.endsWith(UNMAPPED_SUFFIX)).length;
  const packs = new Map<string, number>();
  for (const entry of summary.notApplicable.filter((item) => !item.source.endsWith(UNMAPPED_SUFFIX))) packs.set(entry.source, (packs.get(entry.source) ?? 0) + 1);
  const parts = [
    ...[...packs].map(([pack, count]) => `${pack}'s ${count} (the pack has no conformance.json)`),
    ...(unmapped === 0 ? [] : [`${plural(unmapped, "document")} no conformance.json entry maps`]),
  ];
  return `*${gateTitle("passes-conformance")}* does not apply to ${plural(summary.notApplicable.length, "rule")}, since it has nothing to score them against: ${parts.join(" and ")}.`;
}

/** The gate order in one line: the conformance gates, then the port steps. */
function orderLine(): string {
  const of = (section: string) => STEPS.filter((step) => step.section === section && step.id !== "mapping" && step.id !== "tool-pack").map((step) => step.title).join(" → ");
  return `Each rule is counted once, at the first gate it fails. The conformance gates run on every rule: ${of("conformance")}. The port gates run on a gated tool-pack's rules: ${of("port")}.`;
}

/** One line counting a set of verdicts: `2 shipped: all pass · 5 candidates: 1 ready to promote, 4 stop at a gate`. */
function countLine(rules: RuleVerdict[]): string {
  const shipped = rules.filter((rule) => rule.location === "shipped");
  const candidates = rules.filter((rule) => rule.location === "candidate");
  const failing = shipped.filter((rule) => rule.stoppedAt !== null).length;
  const ready = candidates.filter((rule) => rule.gated && rule.stoppedAt === null).length;
  const passing = candidates.filter((rule) => !rule.gated && rule.stoppedAt === null).length;
  const stopped = candidates.length - ready - passing;
  return [
    shipped.length === 0 ? "" : `${plural(shipped.length, "shipped rule")}: ${failing === 0 ? "all pass" : `${failing} block`}`,
    candidates.length === 0 ? "" : `${plural(candidates.length, "candidate")}: ${[ready === 0 ? "" : `${ready} ready to promote`, passing === 0 ? "" : `${passing} pass the conformance gates (no port gate runs on them)`, stopped === 0 ? "" : `${stopped} stop at a gate`].filter((part) => part !== "").join(", ")}`,
  ].filter((part) => part !== "").join(" · ");
}

/** A source's candidates that stop at a step, grouped by the step, in the funnel's order. */
function stoppedByStep(rules: RuleVerdict[], limit: number): string[] {
  return STEPS.flatMap((step) => {
    const members = rules.filter((rule) => rule.stoppedAt === step.id);
    if (members.length === 0) return [];
    return [`**${step.title}** (${members.length})`, "", ...capped(members, limit, (rows) => table(["Rule", "Check", "Why"], rows.map((rule) => [`\`${rule.id}\``, rule.check, trim(rule.why)]))), ""];
  });
}

interface Limits {
  blockers: number;
  changed: number;
  promote: number;
  /** Rows per step in each source's candidate list; 0 leaves the lists out. */
  perStep: number;
}

function render(summary: Summary, context: CommentContext, limits: Limits): string {
  const blocking = summary.blocking.length;
  const ready = summary.promotable.length;
  const commit = context.commit === undefined || context.commit === "" ? "" : `At \`${context.commit.slice(0, 10)}\``;
  const run = context.runUrl === undefined || context.runUrl === "" ? "" : `[job summary and reports](${context.runUrl})`;
  const counts = funnel(summary);
  const changed = summary.changed;
  const candidates = summary.rules.filter((rule) => rule.location === "candidate");
  const sources = bySource(summary).map((row) => row.source);
  const notJudged = notJudgedLine(summary);
  const lines = [
    COMMENT_MARKER,
    `## Rule gates: ${blocking === 0 ? `✅ every shipped rule passes (${summary.shipped.rules})` : `❌ ${plural(blocking, "blocking failure")}`}${ready === 0 ? "" : ` · 🟢 ${ready} ready to promote`}`,
    "",
    ...(commit === "" && run === "" ? [] : [[commit, run].filter((part) => part !== "").join(" · "), ""]),
    orderLine(),
    "",
    ...(notJudged === undefined ? [] : [notJudged, ""]),
    ...(blocking === 0
      ? []
      : [
          "### Blocking failures",
          "",
          "Fix the rule, or move it to `candidates/`.",
          "",
          ...capped(summary.blocking, limits.blockers, (rows) => table(["Rule", "Rules from", "Stopped at", "Check", "Why"], rows.map((row) => [row.rule === "-" ? "-" : `\`${row.rule}\``, row.source, row.stoppedAt, row.check, trim(row.why)]))),
          "",
          ...(summary.engineGaps.length === 0
            ? []
            : details(
                `${summary.engineGaps.length} of them blocked only by engine gaps (the fix is the engine's, NER-1764)`,
                summary.engineGaps.slice(0, limits.blockers).map((row) => `- ${row.source} \`${row.rule}\`: ${trim(row.detail)}`),
              )),
        ]),
    ...(changed === null
      ? []
      : "error" in changed
        ? ["### Rules this pull request changes", "", `Could not diff against the base: ${changed.error}. The job summary lists every rule's verdict.`, ""]
        : changed.verdicts.length + changed.notJudged.length + changed.removed.length === 0
          ? ["### Rules this pull request changes", "", "None: it changes no rule document.", ""]
          : [
              `### Rules this pull request changes (${changed.verdicts.length + changed.notJudged.length})`,
              "",
              ...(changed.verdicts.length === 0 ? [] : [countLine(changed.verdicts), ""]),
              ...capped([...changed.verdicts].sort(byVerdict), limits.changed, (rows) => table(["Rule", "Rules from", "Verdict"], rows.map((rule) => [`\`${rule.id}\``, `${rule.source} · ${rule.location}`, trim(verdictText(rule), WHY_LENGTH + 60)]))),
              "",
              ...(changed.notJudged.length === 0 ? [] : [`Not judged (their tool-pack has no report): ${changed.notJudged.slice(0, 10).map((path) => `\`${path}\``).join(", ")}${changed.notJudged.length > 10 ? `, and ${changed.notJudged.length - 10} more` : ""}.`, ""]),
              ...(changed.removed.length === 0 ? [] : [`${plural(changed.removed.length, "rule document")} removed or moved away.`, ""]),
            ]),
    ...(ready === 0
      ? []
      : [
          `### Ready to promote (${ready})`,
          "",
          `${ready === 1 ? "This candidate passes" : "These candidates pass"} every gate, conformance and port: the signal to promote ${ready === 1 ? "it" : "them"} into \`rules/\` and \`pack.json\`. Passing the gates is necessary, not sufficient; a reviewer still decides. *Starts* is what upstream-status sets for \`on\`.`,
          "",
          ...capped(summary.promotable, limits.promote, (rows) => table(["Rule", "Tool-pack", "Upstream rule", "Starts"], rows.map((entry) => [`\`${entry.rule}\``, entry.toolPack, entry.upstreamRule, entry.startsOn ? "on" : "off"]))),
          "",
        ]),
    ...details(`Where all ${counts.gatedTotal + counts.restTotal} candidates stand: ${counts.gatedTotal} of gated tool-packs, ${counts.restTotal} that no gated tool-pack judges`, [
      ...funnelTable(summary),
      "",
      "*Not judged by a gated tool-pack*: complexity, whose rules port no upstream; the ports of tools with no `gates.yaml` yet; and documents no `conformance.json` entry maps. Only the conformance gates run on them, so none is ready to promote.",
      "",
      ...sourceTable(summary, true),
      "",
      ...sourceTable(summary, false),
      "",
      ...(summary.notApplicable.length === 0 ? [] : [notApplicableLine(summary), ""]),
      ...(limits.perStep === 0
        ? ["Each candidate's verdict: see the job summary."]
        : sources.flatMap((source) => {
            const stopped = candidates.filter((rule) => rule.source === source && rule.stoppedAt !== null);
            return stopped.length === 0 ? [] : details(`${source}: ${plural(stopped.length, "candidate")} stop${stopped.length === 1 ? "s" : ""} at ${stopped.length === 1 ? "a gate" : "gates"}`, stoppedByStep(stopped, limits.perStep));
          })),
    ]),
    ...(summary.ungated.notGated.length === 0 ? [] : [`Frozen in \`ungated-rules.json\`, since no port gate can judge them: ${summary.ungated.notGated.map((entry) => `${entry.pack} (${plural(entry.rules, "shipped rule")})`).join(", ")}.`, ""]),
    ...(summary.ungated.pending.length === 0 ? [] : [`Pending, validated in nera-rule-validation until recorded: ${summary.ungated.pending.map((entry) => entry.toolPack).join(", ")}.`, ""]),
  ];
  return `${lines.join("\n").trimEnd()}\n`;
}

/** The comment, with its lists shortened (the candidate lists dropped last) until it fits under COMMENT_LIMIT. */
export function renderComment(summary: Summary, context: CommentContext = {}): string {
  for (const perStep of [50, 20, 5, 0]) {
    const text = render(summary, context, { blockers: 50, changed: 100, promote: 30, perStep });
    if (text.length <= COMMENT_LIMIT) return text;
  }
  const text = render(summary, context, { blockers: 10, changed: 20, promote: 10, perStep: 0 });
  return text.length <= COMMENT_LIMIT ? text : `${text.slice(0, COMMENT_LIMIT - 200)}\n\n…truncated: see the job summary.\n`;
}

