/**
 * The pull request comment and the job summary, rendered from the summary (summary.ts). Both say whether every shipped
 * rule passes the gates it must, list each shipped rule that does not with the gate and why, list the candidates ready
 * to promote, and, collapsed, the shipped rules per pack and each candidate that fails a gate, grouped by gate. The job
 * summary is the same page with no list shortened. CI minimizes the earlier gate comments as outdated before posting a
 * new one, so the pull request keeps its history collapsed.
 *
 * A comment is limited to 65,536 characters and a pack may hold hundreds of candidates, so the comment's lists are
 * capped and shortened until it fits; the job summary, summary.json and the uploaded reports always have everything.
 */
import { stepLabel, STEPS, type RuleVerdict, type Summary } from "./summary.js";

/** Starts every rule gates comment, so CI finds the earlier ones to minimize. */
export const COMMENT_MARKER = "<!-- rerail-port-readiness -->";

/** GitHub refuses a comment over 65,536 characters; stay well under it. */
export const COMMENT_LIMIT = 60_000;

/** GitHub's step-summary limit is 1 MiB. */
export const STEP_SUMMARY_LIMIT = 1024 * 1024;

/** At most this many characters of a check's detail in the comment. */
const WHY_LENGTH = 160;

const plural = (count: number, one: string, many = `${one}s`): string => `${count} ${count === 1 ? one : many}`;
/** One line of Markdown: a detail's newlines would end the list item. */
const oneLine = (text: string): string => text.replaceAll(/\s*\n\s*/gu, " ");

function details(summary: string, body: string[]): string[] {
  return ["<details>", `<summary>${summary}</summary>`, "", ...body, "", "</details>", ""];
}

/** `rows` capped at `limit` (none when null), with a line saying how many more the job summary has. */
function capped<T>(rows: T[], limit: number | null, render: (row: T) => string): string[] {
  const shown = limit === null ? rows : rows.slice(0, limit);
  return [...shown.map(render), ...(shown.length < rows.length ? [`- ${rows.length - shown.length} more: see the job summary.`] : [])];
}

export interface CommentContext {
  /** The commit the gates judged (the pull request head), shown under the heading. */
  commit?: string;
  /** The workflow run, linked for the full job summary and reports. */
  runUrl?: string;
}

/**
 * The rule and where it comes from: `` `ruff-b905` (python, port of ruff B905) ``, `` `complexity-x` (complexity,
 * original) ``, or `` `x` (python) `` for a document that declares no origin.
 */
function ruleName(rule: RuleVerdict): string {
  const origin = rule.upstream !== null ? `, port of ${rule.upstream.tool} ${rule.upstream.rule}` : rule.origin === "original" ? ", original" : "";
  return `\`${rule.id}\` (${rule.pack}${origin})`;
}

interface Limits {
  failing: number | null;
  promote: number | null;
  /** Rules listed per gate under the failing candidates; 0 lists only how many stop at each gate. */
  perGate: number | null;
  why: number | null;
}

function render(summary: Summary, context: CommentContext, limits: Limits, marker: boolean): string {
  const why = (text: string): string => {
    const line = oneLine(text);
    return limits.why === null || line.length <= limits.why ? line : `${line.slice(0, limits.why - 3)}...`;
  };
  const blocking = summary.failing.length + summary.otherFailures.length;
  const ready = summary.promotable.length;
  const commit = context.commit === undefined || context.commit === "" ? "" : `At \`${context.commit.slice(0, 10)}\``;
  const run = context.runUrl === undefined || context.runUrl === "" ? "" : `[job summary and reports](${context.runUrl})`;
  const candidates = summary.rules.filter((rule) => rule.location === "candidate" && rule.stoppedAt !== null);
  const packs = [...new Set(summary.rules.filter((rule) => rule.location === "shipped").map((rule) => rule.pack))].sort();
  const lines = [
    ...(marker ? [COMMENT_MARKER] : []),
    `## Rule gates: ${blocking === 0 ? "[PASS]" : "[FAIL]"} ${summary.shipped.pass}/${summary.shipped.rules} shipped rules pass their required gates${summary.otherFailures.length === 0 ? "" : `; ${plural(summary.otherFailures.length, "other blocking failure")}`}${ready === 0 ? "" : `; ${plural(ready, "candidate")} ready to promote`}`,
    "",
    ...(commit === "" && run === "" ? [] : [[commit, run].filter((part) => part !== "").join(" - "), ""]),
    "- **Conformance** checks every rule.",
    "- **Port readiness** checks ports, once they pass conformance. A port is a rule that comes from an upstream linter, such as Ruff or ESLint; these checks make sure it is true to the rule it comes from.",
    "",
    ...(summary.notJudged.length === 0 ? [] : [`Not re-run on this change, since nothing their ports read changed: ${summary.notJudged.join(", ")}. Main's results stand for them, and the counts leave them out.`, ""]),
    ...(summary.failing.length === 0
      ? []
      : [
          `### Shipped rules that fail (${summary.failing.length})`,
          "",
          "Fix the rule, or move it to `candidates/`.",
          "",
          ...capped(summary.failing, limits.failing, (rule) => `- ${ruleName(rule)}: **${stepLabel(rule.stoppedAt!)}**: ${why(rule.why)}`),
          "",
        ]),
    ...(summary.otherFailures.length === 0
      ? []
      : [`### Other blocking failures (${summary.otherFailures.length})`, "", ...capped(summary.otherFailures, limits.failing, (entry) => `- ${entry.what}: ${entry.check}: ${why(entry.why)}`), ""]),
    `### Ready to promote (${ready})`,
    "",
    ...(ready === 0
      ? ["No candidate is ready to promote yet."]
      : [
          "These candidates pass every gate they must: move them into `rules/` and `pack.json`. A reviewer still decides; a port's default is what upstream recommends.",
          "",
          ...capped(summary.promotable, limits.promote, (rule) => `- ${ruleName(rule)}${rule.onByDefault === undefined ? "" : rule.onByDefault ? ": on by default" : ": off by default"}`),
        ]),
    "",
    ...details(`Shipped rules per pack (${summary.shipped.rules})`, [
      "| Rule pack | Shipped rules passing |",
      "| --- | --- |",
      ...packs.map((pack) => {
        const shipped = summary.rules.filter((rule) => rule.pack === pack && rule.location === "shipped");
        return `| ${pack} | ${shipped.filter((rule) => rule.stoppedAt === null).length}/${shipped.length} |`;
      }),
    ]),
    ...(candidates.length === 0
      ? []
      : details(`Candidates that fail a gate (${candidates.length})`, STEPS.flatMap((step) => {
          const members = candidates.filter((rule) => rule.stoppedAt === step.id);
          if (members.length === 0) return [];
          const label = `${stepLabel(step.id)}: ${members.length}`;
          return limits.perGate === 0 ? [`- ${label}`] : details(label, capped(members, limits.perGate, (rule) => `- ${ruleName(rule)}: ${why(rule.why)}`));
        }))),
  ];
  return `${lines.join("\n").trimEnd()}\n`;
}

/** The comment, with its lists shortened (the failing candidates first) until it fits under COMMENT_LIMIT. */
export function renderComment(summary: Summary, context: CommentContext = {}): string {
  for (const perGate of [50, 20, 5, 0]) {
    const text = render(summary, context, { failing: 50, promote: 30, perGate, why: WHY_LENGTH }, true);
    if (text.length <= COMMENT_LIMIT) return text;
  }
  const text = render(summary, context, { failing: 10, promote: 10, perGate: 0, why: WHY_LENGTH }, true);
  return text.length <= COMMENT_LIMIT ? text : `${text.slice(0, COMMENT_LIMIT - 200)}\n\n...truncated: see the job summary.\n`;
}

/** The job summary: the comment's page with every list in full, under GitHub's step-summary limit. */
export function renderJobSummary(summary: Summary, context: CommentContext = {}): string {
  const text = render(summary, context, { failing: null, promote: null, perGate: null, why: null }, false);
  if (Buffer.byteLength(text) <= STEP_SUMMARY_LIMIT - 1024) return text;
  return `${text.slice(0, STEP_SUMMARY_LIMIT / 2)}\n\nTruncated: the full summary is in summary.json and the uploaded report.\n`;
}
