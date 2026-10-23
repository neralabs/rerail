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
import { failureLabel, failurePoint } from "./failure-points.js";
import { STEPS, type RuleVerdict, type StepId, type Summary } from "./summary.js";

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
  /** The per-rule summaries of the tool-packs this run's legs recorded (readRecordingSummaries), sorted by tool-pack. */
  recordings?: RecordingSummary[];
}

/** One gate leg's recording summary: what `port-readiness:record --changes` wrote as recording-<pack>--<tool>.md. */
export interface RecordingSummary {
  toolPack: string;
  markdown: string;
}

/** At most this many characters of one tool-pack's recording summary, and of all of them together. */
const RECORDING_LENGTH = 8_000;
const RECORDINGS_LENGTH = 24_000;

/**
 * The recording summaries as collapsed sections, one per tool-pack, under a heading that says this run's recorder made
 * them: a leg records when its pull request changes the tool-pack's recording.lock (or nothing held the recordings), so
 * these are computed by CI from the pinned upstream, not taken from the pull request. Each is cut at RECORDING_LENGTH,
 * and the tool-packs past RECORDINGS_LENGTH are only named.
 */
function recordingSections(recordings: RecordingSummary[]): string[] {
  if (recordings.length === 0) return [];
  const lines = [
    "### Recorded by this run",
    "",
    "This run recorded these tool-packs from their pinned upstream with the pull request's recorder, and each recording had to match the committed `recording.lock`. What upstream answers differently from the recording the base's lock names:",
    "",
  ];
  let used = 0;
  const omitted: string[] = [];
  for (const { toolPack, markdown } of recordings) {
    const [first = "", ...rest] = markdown.trimEnd().split("\n");
    const headline = first.replace(/^## Port readiness recordings: /u, "");
    const body = rest.join("\n").trim();
    const shown = body.length <= RECORDING_LENGTH ? body : `${body.slice(0, RECORDING_LENGTH - 1)}…\n\nCut short: the leg's job summary has all of it.`;
    if (used + shown.length > RECORDINGS_LENGTH) {
      omitted.push(toolPack);
      continue;
    }
    used += shown.length;
    lines.push(...details(`${toolPack}: ${headline}`, [shown]));
  }
  if (omitted.length > 0) lines.push(`Also recorded, too long to show here (see each leg's job summary): ${omitted.join(", ")}.`, "");
  return lines;
}

const NOT_DECLARED = "not declared";
const ORIGINAL = "original";

/**
 * Where a rule comes from, as its pack declares it (conformance/origins.ts): its upstream linter (`ruff`) for a port,
 * `original` for Rerail's own rule, or `not declared` for a document that declares neither.
 */
export function originOf(rule: RuleVerdict): string {
  return rule.upstream?.tool ?? (rule.origin === "original" ? ORIGINAL : NOT_DECLARED);
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
const stepIndex = (rule: RuleVerdict): number => STEPS.findIndex((step) => step.id === rule.stoppedAt);

/** A failing rule's failure, as the tables group it: its stage and what is wrong. */
const failureOf = (rule: RuleVerdict): string => failureLabel(rule.stoppedAt!, rule.check);

/** The distinct failures of failing `rules`, in the order a rule meets them. */
function failuresOf(rules: RuleVerdict[]): string[] {
  return [...new Set([...rules].sort((a, b) => stepIndex(a) - stepIndex(b)).map(failureOf))];
}

/**
 * The steps that mean no port readiness gate could judge a port: no conformance.json entry maps the port origins.json
 * declares (its reason is that entry's), or its linter is not set up. A document that declares no origin fails
 * conformance instead, and is counted with the other failures.
 */
const UNJUDGED: readonly StepId[] = ["mapping", "set-up"];

interface Limits {
  failing: number | null;
  promote: number | null;
  why: number | null;
  /** List each failing candidate, its failure and why, under its pack (the job summary does; the comment does not). */
  candidateLines: boolean;
}

interface Render {
  limits: Limits;
  why: (text: string) => string;
}

function shippedSection(summary: Summary, { limits, why }: Render): string[] {
  const blocking = summary.failing.length + summary.otherFailures.length;
  const shipped = summary.rules.filter((rule) => rule.location === "shipped");
  const failingIn = (rules: RuleVerdict[]) => rules.filter((rule) => rule.stoppedAt !== null).length;
  const sentence = (text: string) => why(text).replace(/\.$/u, "");
  return [
    `### ${blocking === 0 ? "✅" : "❌"} Shipped Rule Summary: ${summary.shipped.pass}/${summary.shipped.rules} pass, ${blocking} blocking`,
    "",
    ...(blocking === 0
      ? []
      : [
          "A shipped rule that fails blocks the merge: fix it, or move it to `candidates/`.",
          "",
          ...capped(summary.failing, limits.failing, (rule) => `- ❌ ${ruleName(rule)}: **${failureOf(rule)}.** ${sentence(rule.why)}. **Fix:** ${failurePoint(rule.stoppedAt!, rule.check).fix}.`),
          ...capped(summary.otherFailures, limits.failing, (entry) => `- ❌ ${entry.what}: ${entry.check}: ${why(entry.why)}`),
          "",
        ]),
    ...byPack(shipped).flatMap(({ pack, rules, origins }) => details(
      `${pack}: ${rules.length}`,
      table(["Origin", "✅ Pass", "❌ Fail", "Total"], originRows(origins, rules, (members) => [members.length - failingIn(members), failingIn(members), members.length])),
      failingIn(rules) > 0,
    )),
  ];
}

/** The candidates no port readiness gate could judge, and why: each one nothing maps, and each linter not set up. */
function unjudgedLines(failing: RuleVerdict[], why: (text: string) => string): string[] {
  const unmapped = failing.filter((rule) => rule.stoppedAt === "mapping");
  const byTool = new Map<string, RuleVerdict[]>();
  for (const rule of failing.filter((entry) => entry.stoppedAt === "set-up")) byTool.set(rule.upstream!.tool, [...(byTool.get(rule.upstream!.tool) ?? []), rule]);
  if (unmapped.length + byTool.size === 0) return [];
  return [
    "",
    "Not judged against upstream:",
    "",
    ...unmapped.map((rule) => `- \`${rule.id}\`: ${why(rule.why)}`),
    ...[...byTool].map(([tool, rules]) => `- ${plural(rules.length, `${tool} candidate`)}: ${why(rules[0]!.why)}`),
  ];
}

function candidatePack(pack: string, rules: RuleVerdict[], origins: string[], { limits, why }: Render): string[] {
  const inStage = (members: RuleVerdict[], stage: string) => members.filter((rule) => rule.stoppedAt !== null && stageOf(rule.stoppedAt) === stage).length;
  const failing = rules.filter((rule) => rule.stoppedAt !== null);
  const failures = failuresOf(failing);
  return details(`${pack}: ${rules.length}`, [
    ...table(["Origin", "🟢 Ready", "❌ Conformance", "❌ Port readiness", "Total"], originRows(origins, rules, (members) => [
      members.filter((rule) => rule.stoppedAt === null).length, inStage(members, "conformance"), inStage(members, "port"), members.length,
    ])),
    ...(failures.length === 0
      ? []
      : ["", ...table(["Fails because", ...origins], failures.map((label) => [label, ...origins.map((origin) => {
          const count = failing.filter((rule) => failureOf(rule) === label && originOf(rule) === origin).length;
          return count === 0 ? "" : String(count);
        })]))]),
    ...unjudgedLines(failing.filter((rule) => UNJUDGED.includes(rule.stoppedAt!)), why),
    ...(!limits.candidateLines || failing.length === 0
      ? []
      : ["", ...details(`Each failing ${pack} candidate (${failing.length})`, failures.flatMap((label) => [
          `**${label}**`,
          "",
          ...failing.filter((rule) => failureOf(rule) === label).map((rule) => `- ${ruleName(rule)}: ${why(rule.why)}`),
          "",
        ]))]),
  ]);
}

function candidateSection(summary: Summary, render: Render): string[] {
  const candidates = summary.rules.filter((rule) => rule.location === "candidate");
  const ready = summary.promotable.length;
  return [
    `### Candidate Rule Summary: ${plural(candidates.length, "candidate")}, 🟢 ${ready} ready to promote`,
    "",
    ...(ready === 0
      ? []
      : [
          "Ready to promote: each passes every gate it must. Move it into `rules/` and `pack.json`; a reviewer still decides, and a port's default is what upstream recommends.",
          "",
          ...capped(summary.promotable, render.limits.promote, (rule) => `- 🟢 ${ruleName(rule)}${rule.onByDefault === undefined ? "" : rule.onByDefault ? ": on by default" : ": off by default"}`),
          "",
        ]),
    ...byPack(candidates).flatMap(({ pack, rules, origins }) => candidatePack(pack, rules, origins, render)),
  ];
}

/** `text` on one line, shortened to `limits.why` characters when the limit is set. */
function shortener(limits: Limits): (text: string) => string {
  return (text) => {
    const line = oneLine(text);
    return limits.why === null || line.length <= limits.why ? line : `${line.slice(0, limits.why - 3)}...`;
  };
}

/** The commit the gates judged and the run, on one line; empty when CI set neither. */
function whereLines(context: CommentContext): string[] {
  const commit = context.commit ?? "";
  const runUrl = context.runUrl ?? "";
  const where = [commit === "" ? "" : `At \`${commit.slice(0, 10)}\``, runUrl === "" ? "" : `[job summary and reports](${runUrl})`].filter((part) => part !== "");
  return where.length === 0 ? [] : [where.join(" · "), ""];
}

/** The heading, the commit and run, one line on each stage, and the upstream linters the plan did not re-run. */
function header(summary: Summary, context: CommentContext): string[] {
  const blocking = summary.failing.length + summary.otherFailures.length;
  const others = summary.otherFailures.length === 0 ? "" : ` · ${plural(summary.otherFailures.length, "other blocking failure")}`;
  const notRerun = `Not re-run on this change, since nothing their ports read changed: ${summary.notJudged.join(", ")}. Main's results stand for them, and the counts leave them out.`;
  return [
    `## Rule gates: ${blocking === 0 ? "✅" : "❌"} ${summary.shipped.pass}/${summary.shipped.rules} shipped rules pass${others} · 🟢 ${summary.promotable.length} ready to promote`,
    "",
    ...whereLines(context),
    "- **Conformance** checks every rule.",
    "- **Port readiness** checks ports, once they pass conformance. A port is a rule that comes from an upstream linter, such as Ruff or ESLint; these checks make sure it is true to the rule it comes from.",
    "",
    ...(summary.notJudged.length === 0 ? [] : [notRerun, ""]),
  ];
}

function render(summary: Summary, context: CommentContext, limits: Limits, marker: boolean): string {
  const options = { limits, why: shortener(limits) };
  const lines = [
    ...(marker ? [COMMENT_MARKER] : []),
    ...header(summary, context),
    ...shippedSection(summary, options),
    ...recordingSections(context.recordings ?? []),
    ...candidateSection(summary, options),
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
