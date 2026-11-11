#!/usr/bin/env node
// The weekly feedback digest: what came in on this repository's GitHub issues in a week, for the product team. New and
// closed issues, new issues by type and by coding agent, the rule misfires by kind (false positive, missed violation,
// confusing steer), rule and pack, and the open backlog. It reads the issue forms' fields (.github/ISSUE_TEMPLATE/),
// so an issue written without a form counts under "not given". Reporters from outside the team are the ones whose
// author association is not OWNER, MEMBER or COLLABORATOR.
//
// The week ends at 00:00 UTC on --week-ending (or $WEEK_ENDING; default today) and starts seven days before. Without
// --print, it writes the digest as one page of the Notion data source $NOTION_DIGEST_DATA_SOURCE_ID with $NOTION_TOKEN,
// and skips a week that already has a page. In GitHub Actions it also writes the digest to the run summary.
// .github/workflows/feedback-digest.yml runs it every Monday.
//
//   [GITHUB_TOKEN=…] node scripts/feedback/digest.mjs [--week-ending <yyyy-mm-dd>] [--print]
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const DAY = 24 * 60 * 60 * 1000;
const NOTION_VERSION = "2026-03-11";
/** Author associations that mean the reporter is on the team. */
const TEAM = new Set(["OWNER", "MEMBER", "COLLABORATOR"]);
/** An issue's type, from its labels: the first label of the template it came from that it carries. */
const TYPES = [
  ["rule-misfire", "Rule misfire"],
  ["bug", "Bug"],
  ["integration", "Agent, language or linter support"],
  ["enhancement", "Feature request"],
  ["question", "Question"],
  ["trust-request", "Trusted contributor request"],
];
/** The rule misfire form's "What went wrong" answers, by how each starts. */
const KINDS = [
  ["False positive", "False positive"],
  ["False negative", "Missed violation"],
  ["The steer", "Confusing steer"],
];

/** How many rules the open misfires section names, most reported first. */
const OPEN_RULES_SHOWN = 10;
const NOT_GIVEN = "not given";

/** An issue form's answers, by field label: the `### <label>` sections GitHub writes, without `_No response_` ones. */
export function parseForm(body) {
  const fields = new Map();
  const sections = (body ?? "").replace(/\r\n/g, "\n").split(/^### (.+)$/m);
  for (let index = 1; index < sections.length; index += 2) {
    const value = sections[index + 1].trim();
    if (value !== "" && value !== "_No response_") fields.set(sections[index].trim(), value);
  }
  return fields;
}

/** A rule misfire's kind from the form's "What went wrong" answer, or "not given". */
export function misfireKind(answer) {
  return KINDS.find(([start]) => answer?.startsWith(start))?.[1] ?? NOT_GIVEN;
}

/** An issue's label names. */
const labelNames = (issue) => new Set(issue.labels.map((label) => (typeof label === "string" ? label : label.name)));

/** An issue's type from its labels, or "Other". */
export function issueType(issue) {
  const labels = labelNames(issue);
  return TYPES.find(([label]) => labels.has(label))?.[1] ?? "Other";
}

/** A pack answer (`javascript@0.1.4`, `own rule`) without its version. */
const packName = (answer) => (answer === undefined ? NOT_GIVEN : answer.replace(/@[^@\s]*$/, "").trim() || NOT_GIVEN);

/** Adds one to `key` of the count map `counts`. */
const count = (counts, key) => counts.set(key, (counts.get(key) ?? 0) + 1);

/** A count map's entries, largest first, then by name. */
const ranked = (counts) => [...counts].sort(([a, x], [b, y]) => y - x || a.localeCompare(b));

/**
 * A misfire's rule id: the form's, or else the title's, when the title starts with one and a colon the way the team
 * writes them (`eslint:prefer-const: …`, `n-hashbang: …`): one word with no spaces, up to the last colon before a space.
 */
export function ruleId(form, title) {
  return form.get("Rule id") ?? /^([^\s:]+(?::[^\s:]+)*):\s/.exec(title ?? "")?.[1] ?? NOT_GIVEN;
}

/** A rule misfire as the digest groups it. */
function misfire(issue) {
  const form = parseForm(issue.body);
  return { issue, kind: misfireKind(form.get("What went wrong")), rule: ruleId(form, issue.title), pack: packName(form.get("Pack and version")) };
}

/**
 * The digest of `issues` (GitHub REST issue objects, pull requests included and ignored) for the week [start, end):
 * the issues opened and closed in it, and the backlog open now.
 */
export function summarize(issues, { start, end }) {
  const inWeek = (time) => time !== null && time !== undefined && Date.parse(time) >= start.getTime() && Date.parse(time) < end.getTime();
  const all = issues.filter((issue) => issue.pull_request === undefined);
  const opened = all.filter((issue) => inWeek(issue.created_at)).sort((a, b) => a.number - b.number);
  const open = all.filter((issue) => issue.state === "open");

  const types = new Map();
  const agents = new Map();
  for (const issue of opened) {
    count(types, issueType(issue));
    const agent = parseForm(issue.body).get("Coding agent");
    if (agent !== undefined) count(agents, agent);
  }

  const misfires = opened.filter((issue) => issueType(issue) === "Rule misfire").map(misfire);
  const kinds = new Map();
  const packs = new Map();
  const rules = new Map();
  for (const entry of misfires) {
    count(kinds, entry.kind);
    count(packs, entry.pack);
    const key = `${entry.rule}\u0000${entry.pack}`;
    const rule = rules.get(key) ?? { rule: entry.rule, pack: entry.pack, kinds: new Map(), issues: [] };
    count(rule.kinds, entry.kind);
    rule.issues.push(entry.issue);
    rules.set(key, rule);
  }

  const openMisfires = open.filter((issue) => issueType(issue) === "Rule misfire").map(misfire);
  const openKinds = new Map();
  const openPacks = new Map();
  const openRules = new Map();
  for (const entry of openMisfires) {
    count(openKinds, entry.kind);
    count(openPacks, entry.pack);
    count(openRules, entry.rule);
  }

  return {
    start,
    end,
    opened,
    outside: opened.filter((issue) => !TEAM.has(issue.author_association)),
    closed: all.filter((issue) => inWeek(issue.closed_at)),
    open: open.length,
    needsTriage: open.filter((issue) => labelNames(issue).has("needs-triage")).length,
    types,
    agents,
    misfires: { total: misfires.length, kinds, packs, rules: [...rules.values()].sort((a, b) => b.issues.length - a.issues.length || a.rule.localeCompare(b.rule)) },
    openMisfires: { total: openMisfires.length, kinds: openKinds, packs: openPacks, rules: openRules },
  };
}

const day = (date) => date.toISOString().slice(0, 10);
const link = (issue) => `[#${issue.number}](${issue.html_url})`;
const list = (counts) => ranked(counts).map(([name, n]) => `${name} ${n}`).join(", ");
const kindsOf = (counts) => KINDS.map(([, kind]) => kind).concat(NOT_GIVEN).filter((kind) => counts.has(kind)).map((kind) => `${kind} ${counts.get(kind)}`).join(", ");

/** The digest as Markdown, for the Notion page and the run summary. */
export function render(digest) {
  const lines = [];
  const last = new Date(digest.end.getTime() - DAY);
  lines.push(`GitHub issues opened or closed from ${day(digest.start)} to ${day(last)} (UTC), and the backlog open when this ran.`, "");

  lines.push("## Summary", "");
  lines.push(`- New issues: ${digest.opened.length} (${digest.outside.length} from outside the team)`);
  lines.push(`- Closed: ${digest.closed.length}`);
  lines.push(`- Open: ${digest.open} (${digest.needsTriage} need triage)`);
  lines.push(`- New rule misfires: ${digest.misfires.total}${digest.misfires.total > 0 ? ` (${kindsOf(digest.misfires.kinds)})` : ""}`, "");

  lines.push("## New rule misfires", "");
  if (digest.misfires.total === 0) lines.push("None this week.", "");
  else {
    lines.push(`By pack: ${list(digest.misfires.packs)}`, "");
    for (const rule of digest.misfires.rules) {
      lines.push(`- \`${rule.rule}\` (${rule.pack}): ${kindsOf(rule.kinds)}. ${rule.issues.map(link).join(", ")}`);
    }
    lines.push("");
  }

  lines.push("## New issues", "");
  if (digest.opened.length === 0) lines.push("None this week.", "");
  else {
    lines.push(`By type: ${list(digest.types)}`, "");
    if (digest.agents.size > 0) lines.push(`By coding agent (bug reports): ${list(digest.agents)}`, "");
    for (const issue of digest.opened) {
      const from = TEAM.has(issue.author_association) ? "team" : "outside";
      lines.push(`- ${link(issue)} ${issue.title} (${issueType(issue)}, ${from})`);
    }
    lines.push("");
  }

  lines.push("## Open rule misfires", "");
  if (digest.openMisfires.total === 0) lines.push("None open.");
  else {
    lines.push(`- ${digest.openMisfires.total} open: ${kindsOf(digest.openMisfires.kinds)}`);
    lines.push(`- By pack: ${list(digest.openMisfires.packs)}`);
    lines.push(`- Rules with the most: ${ranked(digest.openMisfires.rules).slice(0, OPEN_RULES_SHOWN).map(([rule, n]) => `\`${rule}\` ${n}`).join(", ")}`);
  }
  return `${lines.join("\n")}\n`;
}

/** Every issue of `repository` the REST API lists for `query`, following its pages. */
async function listIssues(repository, query, token) {
  const issues = [];
  let url = `https://api.github.com/repos/${repository}/issues?${new URLSearchParams({ per_page: "100", ...query })}`;
  while (url !== undefined) {
    const response = await fetch(url, { headers: { accept: "application/vnd.github+json", "x-github-api-version": "2022-11-28", ...(token ? { authorization: `Bearer ${token}` } : {}) } });
    if (!response.ok) throw new Error(`GitHub answered ${response.status} for ${url}: ${await response.text()}`);
    issues.push(...(await response.json()));
    url = /<([^>]+)>;\s*rel="next"/.exec(response.headers.get("link") ?? "")?.[1];
  }
  return issues;
}

/** Calls the Notion API. */
async function notion(path, body, token) {
  const response = await fetch(`https://api.notion.com/v1/${path}`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "notion-version": NOTION_VERSION, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const answer = await response.json();
  if (!response.ok) throw new Error(`Notion answered ${response.status} for ${path}: ${answer.message ?? JSON.stringify(answer)}`);
  return answer;
}

/** Writes `digest` as a page of the data source, unless the week already has one; the page's URL, or undefined. */
async function publish(digest, markdown, { dataSource, token, runUrl }) {
  const title = `Week of ${day(digest.start)}`;
  const existing = await notion(`data_sources/${dataSource}/query`, { filter: { property: "Week", title: { equals: title } }, page_size: 1 }, token);
  if (existing.results.length > 0) {
    console.log(`${title} already has a page: ${existing.results[0].url}`);
    return undefined;
  }
  const number = (value) => ({ number: value });
  const page = await notion(
    "pages",
    {
      parent: { type: "data_source_id", data_source_id: dataSource },
      properties: {
        Week: { title: [{ text: { content: title } }] },
        "Week start": { date: { start: day(digest.start) } },
        "New issues": number(digest.opened.length),
        "Outside reports": number(digest.outside.length),
        "Rule misfires": number(digest.misfires.total),
        "False positives": number(digest.misfires.kinds.get("False positive") ?? 0),
        "Missed violations": number(digest.misfires.kinds.get("Missed violation") ?? 0),
        "Confusing steers": number(digest.misfires.kinds.get("Confusing steer") ?? 0),
        Closed: number(digest.closed.length),
        Open: number(digest.open),
        "Needs triage": number(digest.needsTriage),
        ...(runUrl === undefined ? {} : { Run: { url: runUrl } }),
      },
      markdown,
    },
    token,
  );
  return page.url;
}

/** The week's end: 00:00 UTC of `date` (yyyy-mm-dd), or of today. */
export function weekEnding(date) {
  if (date === undefined || date === "") {
    const now = new Date();
    return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || Number.isNaN(Date.parse(`${date}T00:00:00Z`))) throw new Error(`--week-ending takes a date as yyyy-mm-dd, not ${date}`);
  return new Date(`${date}T00:00:00Z`);
}

async function main(argv) {
  const at = argv.indexOf("--week-ending");
  const end = weekEnding(at === -1 ? process.env.WEEK_ENDING : argv[at + 1]);
  const start = new Date(end.getTime() - 7 * DAY);
  const print = argv.includes("--print");
  const { NOTION_TOKEN: token, NOTION_DIGEST_DATA_SOURCE_ID: dataSource } = process.env;
  if (!print && (!token || !dataSource)) throw new Error("writing to Notion needs NOTION_TOKEN and NOTION_DIGEST_DATA_SOURCE_ID; --print prints the digest instead");

  const repository = process.env.GITHUB_REPOSITORY ?? "neralabs/rerail";
  const github = process.env.GITHUB_TOKEN;
  // Updated since the week began covers every issue opened or closed in it; the open ones give the backlog.
  const byNumber = new Map();
  for (const issue of [...(await listIssues(repository, { state: "all", since: start.toISOString() }, github)), ...(await listIssues(repository, { state: "open" }, github))]) {
    byNumber.set(issue.number, issue);
  }
  const digest = summarize([...byNumber.values()], { start, end });
  const markdown = render(digest);

  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `# Feedback digest: week of ${day(start)}\n\n${markdown}`);
  if (print) {
    process.stdout.write(markdown);
    return;
  }
  const runUrl = process.env.GITHUB_RUN_ID ? `${process.env.GITHUB_SERVER_URL}/${repository}/actions/runs/${process.env.GITHUB_RUN_ID}` : undefined;
  const url = await publish(digest, markdown, { dataSource, token, runUrl });
  if (url !== undefined) console.log(`Wrote the week of ${day(start)}: ${url}`);
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(`feedback digest: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}
