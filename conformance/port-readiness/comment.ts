/**
 * The pull request comment and the job summary, rendered from the summary (summary.ts), in two sections of the same
 * shape. The Shipped Rule Summary lists each shipped rule that fails (the blockers), then one collapsed block per pack:
 * its shipped rules by origin, passing and failing. The Candidate Rule Summary lists the candidates ready to promote,
 * then one collapsed block per pack: its candidates by origin (ready, failing conformance, failing port readiness), and
 * the gate they fail at. The job summary is the same page with no list shortened, and each failing candidate's gate and
 * why. CI minimizes the earlier gate comments as outdated before posting a new one, so the pull request keeps its
 * history collapsed.
 *
 * A comment is limited to 65,536 characters, so its lists are capped; the job summary, summary.json and the uploaded
 * reports always have everything.
 */
import { stepLabel, STEPS, type RuleVerdict, type StepId, type Summary } from "./summary.js";

/** Starts every rule gates comment, so CI finds the earlier ones to minimize. */
export const COMMENT_MARKER = "<!-- rerail-port-readiness -->";

/** GitHub refuses a comment over 65,536 characters; stay well under it. */
export const COMMENT_LIMIT = 60_000;

/** GitHub's step-summary limit is 1 MiB. */
export const STEP_SUMMARY_LIMIT = 1024 * 1024;

/** At most this many characters of a check's detail in the comment. */
const WHY_LENGTH = 160;

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
const plural = (count: number, one: string, many = `${one}s`): string => `${count} ${count === 1 ? one : many}`;
/** One line of Markdown: a detail's newlines would end the list item. */
const oneLine = (text: string): string => text.replaceAll(/\s*\n\s*/gu, " ");

function details(summary: string, body: string[], open = false): string[] {
  return [open ? "<details open>" : "<details>", `<summary>${summary}</summary>`, "", ...body, "", "</details>", ""];
}

function table(header: string[], rows: string[][]): string[] {
  return [`| ${header.join(" | ")} |`, `| ${header.map(() => "---").join(" | ")} |`, ...rows.map((row) => `| ${row.join(" | ")} |`)];
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

const NOT_DECLARED = "not declared";
const ORIGINAL = "original";

/** Where a rule comes from: its upstream linter (`ruff`), `original` for a rule that is not a port, or `not declared`. */
export function originOf(rule: RuleVerdict): string {
  return rule.upstream?.tool ?? (rule.port ? NOT_DECLARED : ORIGINAL);
}

/** Upstream linters by name, then `original`, then `not declared`. */
function byOrigin(a: string, b: string): number {
  const rank = (origin: string) => (origin === NOT_DECLARED ? 2 : origin === ORIGINAL ? 1 : 0);
  return rank(a) - rank(b) || byText(a, b);
}

/** The rule and where it comes from: `` `ruff-b905` (python, from ruff B905) `` or `` `complexity-x` (complexity, original) ``. */
function ruleName(rule: RuleVerdict): string {
  return `\`${rule.id}\` (${rule.pack}, ${rule.upstream === null ? originOf(rule) : `from ${rule.upstream.tool} ${rule.upstream.rule}`})`;
}

/** The rules of `rules` by pack, packs sorted, and each pack's origins sorted. */
function byPack(rules: RuleVerdict[]): { pack: string; rules: RuleVerdict[]; origins: string[] }[] {
  const packs = [...new Set(rules.map((rule) => rule.pack))].sort(byText);
  return packs.map((pack) => {
    const members = rules.filter((rule) => rule.pack === pack);
    return { pack, rules: members, origins: [...new Set(members.map(originOf))].sort(byOrigin) };
  });
}

/** One row per origin, then a total row when the pack has more than one. */
function originRows(origins: string[], rules: RuleVerdict[], cells: (rules: RuleVerdict[]) => number[]): string[][] {
  const rows = origins.map((origin) => [origin, ...cells(rules.filter((rule) => originOf(rule) === origin)).map(String)]);
  return origins.length < 2 ? rows : [...rows, ["**Total**", ...cells(rules).map((count) => `**${count}**`)]];
}

const stageOf = (id: StepId): string => STEPS.find((step) => step.id === id)!.stage;

interface Limits {
  failing: number | null;
  promote: number | null;
  why: number | null;
  /** List each failing candidate, its gate and why, under its pack (the job summary does; the comment does not). */
  candidateLines: boolean;
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
  const shipped = summary.rules.filter((rule) => rule.location === "shipped");
  const candidates = summary.rules.filter((rule) => rule.location === "candidate");
  const failingIn = (rules: RuleVerdict[]) => rules.filter((rule) => rule.stoppedAt !== null).length;

  const shippedPacks = byPack(shipped).flatMap(({ pack, rules, origins }) => details(
    `${pack}: ${rules.length}`,
    table(["Origin", "✅ Pass", "❌ Fail", "Total"], originRows(origins, rules, (members) => [members.length - failingIn(members), failingIn(members), members.length])),
    failingIn(rules) > 0,
  ));

  const candidatePacks = byPack(candidates).flatMap(({ pack, rules, origins }) => {
    const inStage = (members: RuleVerdict[], stage: string) => members.filter((rule) => rule.stoppedAt !== null && stageOf(rule.stoppedAt) === stage).length;
    const failing = rules.filter((rule) => rule.stoppedAt !== null);
    const gates = STEPS.filter((step) => failing.some((rule) => rule.stoppedAt === step.id));
    return details(`${pack}: ${rules.length}`, [
      ...table(["Origin", "🟢 Ready", "❌ Conformance", "❌ Port readiness", "Total"], originRows(origins, rules, (members) => [
        members.filter((rule) => rule.stoppedAt === null).length, inStage(members, "conformance"), inStage(members, "port"), members.length,
      ])),
      ...(gates.length === 0
        ? []
        : ["", ...table(["Fails at", ...origins], gates.map((step) => [stepLabel(step.id), ...origins.map((origin) => {
            const count = failing.filter((rule) => rule.stoppedAt === step.id && originOf(rule) === origin).length;
            return count === 0 ? "" : String(count);
          })]))]),
      ...(!limits.candidateLines || failing.length === 0
        ? []
        : ["", ...details(`Each failing ${pack} candidate (${failing.length})`, gates.flatMap((step) => [
            `**${stepLabel(step.id)}**`,
            "",
            ...failing.filter((rule) => rule.stoppedAt === step.id).map((rule) => `- ${ruleName(rule)}: ${why(rule.why)}`),
            "",
          ]))]),
    ]);
  });

  const lines = [
    ...(marker ? [COMMENT_MARKER] : []),
    `## Rule gates: ${blocking === 0 ? "✅" : "❌"} ${summary.shipped.pass}/${summary.shipped.rules} shipped rules pass${summary.otherFailures.length === 0 ? "" : ` · ${plural(summary.otherFailures.length, "other blocking failure")}`} · 🟢 ${ready} ready to promote`,
    "",
    ...(commit === "" && run === "" ? [] : [[commit, run].filter((part) => part !== "").join(" · "), ""]),
    "- **Conformance** checks every rule.",
    "- **Port readiness** checks ports, once they pass conformance. A port is a rule that comes from an upstream linter, such as Ruff or ESLint; these checks make sure it is true to the rule it comes from.",
    "",
    ...(summary.notJudged.length === 0 ? [] : [`Not re-run on this change, since nothing their ports read changed: ${summary.notJudged.join(", ")}. Main's results stand for them, and the counts leave them out.`, ""]),
    `### ${blocking === 0 ? "✅" : "❌"} Shipped Rule Summary: ${summary.shipped.pass}/${summary.shipped.rules} pass, ${blocking} blocking`,
    "",
    ...(blocking === 0
      ? []
      : [
          "Fix the rule, or move it to `candidates/`.",
          "",
          ...capped(summary.failing, limits.failing, (rule) => `- ❌ ${ruleName(rule)}: **${stepLabel(rule.stoppedAt!)}**: ${why(rule.why)}`),
          ...capped(summary.otherFailures, limits.failing, (entry) => `- ❌ ${entry.what}: ${entry.check}: ${why(entry.why)}`),
          "",
        ]),
    ...shippedPacks,
    `### Candidate Rule Summary: ${plural(candidates.length, "candidate")}, 🟢 ${ready} ready to promote`,
    "",
    ...(ready === 0
      ? []
      : [
          "Ready to promote: each passes every gate it must. Move it into `rules/` and `pack.json`; a reviewer still decides, and a port's default is what upstream recommends.",
          "",
          ...capped(summary.promotable, limits.promote, (rule) => `- 🟢 ${ruleName(rule)}${rule.onByDefault === undefined ? "" : rule.onByDefault ? ": on by default" : ": off by default"}`),
          "",
        ]),
    ...candidatePacks,
  ];
  return `${lines.join("\n").trimEnd()}\n`;
}

/** The comment, with its lists shortened until it fits under COMMENT_LIMIT. */
export function renderComment(summary: Summary, context: CommentContext = {}): string {
  for (const limit of [50, 10]) {
    const text = render(summary, context, { failing: limit, promote: limit, why: WHY_LENGTH, candidateLines: false }, true);
    if (text.length <= COMMENT_LIMIT) return text;
  }
  const text = render(summary, context, { failing: 5, promote: 5, why: WHY_LENGTH, candidateLines: false }, true);
  return text.length <= COMMENT_LIMIT ? text : `${text.slice(0, COMMENT_LIMIT - 200)}\n\n...truncated: see the job summary.\n`;
}

/** The job summary: the comment's page with every list in full and each failing candidate, under GitHub's step-summary limit. */
export function renderJobSummary(summary: Summary, context: CommentContext = {}): string {
  const text = render(summary, context, { failing: null, promote: null, why: null, candidateLines: true }, false);
  if (Buffer.byteLength(text) <= STEP_SUMMARY_LIMIT - 1024) return text;
  return `${text.slice(0, STEP_SUMMARY_LIMIT / 2)}\n\nTruncated: the full summary is in summary.json and the uploaded report.\n`;
}
