/**
 * Merge Assist's page as pure derivations of `GET /api/merge-assist` (`MergeAssistStatus` in
 * `src/engine/merge-assist/model.ts`), without the DOM: the sections and their order, what the status strip and the
 * readiness checklist say, who is behind a finding, the sidebar's count, and which pull request links are safe to open.
 *
 * Decision text, authors, branch names, titles and links come from other people's branches. Every string here is
 * plain text for a text node; `safePeerUrl` is the only way a peer's link becomes an `href`.
 */
import { andList, fmt, plural } from "./format.js";

/** The agent CLIs Merge Assist runs for judgment (`JUDGE_RUNNERS`), by the names people know them by. */
export const RUNNER_NAMES = { claude: "Claude Code", codex: "Codex" };
/** "Claude Code or Codex": the CLIs a person can install to get judgment. */
const SUPPORTED_RUNNERS = andList(Object.values(RUNNER_NAMES)).replace(/ and (?=[^,]*$)/u, " or ");

export const runnerName = id => RUNNER_NAMES[id] ?? id;

/**
 * A peer's pull request link, only when it is an `https:` URL with no credentials or port, on a GitHub host that is
 * this repository's (`github.com`, or a GitHub Enterprise host whose first label is `github`, as `githubRepositoryOf`
 * accepts, and the host in the repository key when the key names one), and, once `gh` named the repository, under
 * its `/<owner>/<name>/pull/`. `null` for anything else: the page then shows the pull request as text.
 */
export function safePeerUrl(url, { repositoryKey = "", repository = null } = {}) {
  if (typeof url !== "string" || !url.startsWith("https://")) return null;
  let parsed;
  try { parsed = new URL(url); } catch { return null; }
  if (parsed.protocol !== "https:" || parsed.username !== "" || parsed.password !== "" || parsed.port !== "") return null;
  const host = parsed.hostname.toLowerCase();
  const labels = host.split(".");
  const enterprise = labels.length >= 2 && labels[0] === "github" && labels.every(label => /^[a-z0-9-]+$/u.test(label));
  if (host !== "github.com" && !enterprise) return null;
  // A key from a GitHub remote names its host; an ssh alias names something else, and then only github.com passes.
  const keyHost = /^remote:([^/]+)\//u.exec(repositoryKey)?.[1] ?? null;
  if (host !== "github.com" && host !== keyHost) return null;
  if (repository !== null && !parsed.pathname.toLowerCase().startsWith(`/${repository.toLowerCase()}/pull/`)) return null;
  return parsed.href;
}

/** What a finding came from: "pull request #12" or "branch jbarnes/session-retry". */
export function peerLabel(peer) {
  return peer.kind === "pull-request" && peer.number !== null ? `pull request #${peer.number}` : `branch ${peer.ref}`;
}

/**
 * Who is behind a finding. A pull request's author is the one GitHub reports (`null` when it named none); a local
 * branch's is the one its decision file declares (`declared`), which nothing checks. A pull request's decision file
 * declares an author too, and that one is never shown: anyone can write any name there.
 */
export function authorOf(finding) {
  return finding.peer.kind === "pull-request"
    ? { name: finding.peer.author, declared: false }
    : { name: finding.candidate.author, declared: true };
}

/** The badge for how a decision is enforced: a code rule Rerail checks, or a note only the person's agent judges. */
export const levelText = level => (level === "code" ? "Checked by Rerail" : "Judged by your agent");
export const levelTitle = level => (level === "code"
  ? "A code rule: Rerail's engine checks the code exactly and steers your agent when it breaks the decision."
  : "A note: Rerail cannot check this exactly, so it is given to your coding agent as guidance.");

/* ---------- sections ---------- */

const pendingFinding = (pending, id) => pending.get(`finding:${id}`);
const pendingCandidate = (pending, id) => pending.get(`candidate:${id}`);
const newestFirst = field => (a, b) => (Date.parse(b[field]) || 0) - (Date.parse(a[field]) || 0);

/**
 * The page's sections from the status and the writes still in flight (`pending`: `finding:<id>` → action,
 * `candidate:<id>` → state). A dismissed finding or an undone one leaves its section at once; an adopted one stays
 * in Needs your review, busy, until the daemon confirms it applies. A decision marked as not a rule moves to Not rules
 * at once, and a restored one back. Each item carries `busy` while its write is in flight.
 * - `talk`: open findings that contradict one of this branch's decisions;
 * - `review`: other open findings;
 * - `added`: findings applied here, absorbed or adopted;
 * - `own`: this branch's active decisions; `notRules`: the ones marked as not a rule.
 * Findings are newest first by `updatedAt`, decisions by `createdAt`.
 */
export function sectionsOf(status, pending = new Map()) {
  const talk = [], review = [], added = [];
  for (const finding of [...status.findings].sort(newestFirst("updatedAt"))) {
    const action = pendingFinding(pending, finding.id);
    const item = { ...finding, busy: action !== undefined };
    if (action === "dismiss" || action === "undo") continue;
    if (finding.status === "applied") added.push(item);
    else if (finding.status === "open") (finding.tier === "talk" ? talk : review).push(item);
  }
  const own = [], notRules = [];
  for (const decision of [...status.own].sort(newestFirst("createdAt"))) {
    const state = pendingCandidate(pending, decision.id) ?? decision.state;
    const item = { ...decision, state, busy: pendingCandidate(pending, decision.id) !== undefined };
    (state === "rejected" ? notRules : own).push(item);
  }
  return { talk, review, added, own, notRules };
}

/** The sidebar's count: decided-differently and needs-review findings waiting for the person; 0 while off. */
export function waitingCount(status, pending = new Map()) {
  if (status === null || status === undefined || !status.enabled) return 0;
  const { talk, review } = sectionsOf(status, pending);
  return talk.length + review.length;
}

/* ---------- the status strip ---------- */

/** "tbelanger/export-api" over "main", or what is missing. */
export function branchText(status) {
  return {
    branch: status.branch ?? "Detached HEAD",
    base: status.baseBranch ?? "no base branch found",
  };
}

/** The agent CLI used for judgment, or that none was found. */
export function judgeText(judge) {
  if (judge.available && judge.runner !== null) return { ok: true, text: runnerName(judge.runner) };
  return { ok: false, text: "No agent CLI found", detail: judge.problem };
}

/** `gh`'s state in a few words for the strip. */
export function githubText(github) {
  switch (github.state) {
    case "signed-in": return { ok: true, text: github.login ? `Signed in as ${github.login}` : "Signed in" };
    case "signed-out": return { ok: false, text: "gh signed out" };
    case "missing": return { ok: false, text: "gh not found" };
    default: return { ok: false, text: "Not a GitHub repository" };
  }
}

/** "3 local branches and 2 open pull requests"; "no other branches" when there were none. */
export function scannedWhat(lastScan) {
  const { branches, pullRequests } = lastScan.peers;
  const parts = [];
  if (branches > 0) parts.push(`${fmt(branches)} local ${plural(branches, "branch", "branches")}`);
  if (pullRequests > 0) parts.push(`${fmt(pullRequests)} open ${plural(pullRequests, "pull request")}`);
  return parts.length === 0 ? "no other branches" : parts.join(" and ");
}

/** "12 of 40 this hour", or the window's own minutes when it is not an hour. */
export function budgetText(budget) {
  const window = budget.windowMinutes === 60 ? "this hour" : `in the last ${fmt(budget.windowMinutes)} minutes`;
  return { text: `${fmt(budget.used)} of ${fmt(budget.limit)} ${window}`, spent: budget.used >= budget.limit };
}

const FOUND_SECTIONS = ["talk", "review", "added"];

/**
 * After Scan now, for its toast: what the scan read and what it found that the page did not show before (`before`, the
 * status at the click, or `null`), not the totals. A finding is new when it was not in that section, or its decision
 * changed. "Scanned 3 local branches. New: 1 decided differently, 2 need your review." or "Scanned 3 local branches:
 * nothing new." While another process's scan runs, the request started none, and the status holds the last results.
 */
export function scanSummary(before, after) {
  if (after.scanning) return "Another scan is running; showing the last results.";
  if (after.lastScan === null) return "Scan finished.";
  const keyOf = (section, finding) => `${section}\u0000${finding.id}\u0000${JSON.stringify(finding.candidate)}`;
  const shown = new Set();
  if (before !== null) {
    const sections = sectionsOf(before);
    for (const section of FOUND_SECTIONS) for (const finding of sections[section]) shown.add(keyOf(section, finding));
  }
  const sections = sectionsOf(after);
  const fresh = Object.fromEntries(FOUND_SECTIONS.map(section =>
    [section, sections[section].filter(finding => !shown.has(keyOf(section, finding))).length]));
  const found = [];
  if (fresh.talk > 0) found.push(`${fmt(fresh.talk)} decided differently`);
  if (fresh.review > 0) found.push(`${fmt(fresh.review)} ${plural(fresh.review, "needs", "need")} your review`);
  if (fresh.added > 0) found.push(`${fmt(fresh.added)} added from other branches`);
  const problems = after.lastScan.problems.length;
  const tail = problems === 0 ? "" : ` ${fmt(problems)} ${plural(problems, "thing")} could not be read; see Last scan.`;
  const what = scannedWhat(after.lastScan);
  return `${found.length === 0 ? `Scanned ${what}: nothing new.` : `Scanned ${what}. New: ${found.join(", ")}.`}${tail}`;
}

/* ---------- the readiness checklist ---------- */

/**
 * What Rerail found before it is turned on, one line each: `ok`, and `text` as segments (a string, or `{ code }`
 * for a command or path). The page renders each segment as a text node.
 */
export function readinessChecks(readiness) {
  const { judge, github, candidatesIgnored, rulesActive } = readiness;
  const checks = [];
  checks.push(judge.available && judge.runner !== null
    ? { key: "judge", ok: true, text: [`Your agent CLI: ${runnerName(judge.runner)}. Rerail runs it to summarise corrections and compare decisions.`] }
    : { key: "judge", ok: false, text: [`No agent CLI found. Capturing corrections and comparing decisions need ${SUPPORTED_RUNNERS} on your PATH. Without one, scans still apply exact checks.`] });
  switch (github.state) {
    case "signed-in":
      checks.push({ key: "github", ok: true, text: [{ code: "gh" }, ` is signed in${github.login ? ` as ${github.login}` : ""}. Open pull requests are read with your sign-in.`] });
      break;
    case "signed-out":
      checks.push({ key: "github", ok: false, text: [{ code: "gh" }, " is not signed in, so only local branches are read. Run ", { code: "gh auth login" }, " to read open pull requests."] });
      break;
    case "missing":
      checks.push({ key: "github", ok: false, text: [{ code: "gh" }, " is not installed, so only local branches are read."] });
      break;
    default:
      checks.push({ key: "github", ok: false, text: ["This repository is not on GitHub, so only local branches are read."] });
  }
  checks.push(candidatesIgnored
    ? { key: "ignored", ok: false, text: ["Git ignores ", { code: ".rerail/candidates/" }, " here, so decisions stay on this machine and teammates don't see them."] }
    : { key: "ignored", ok: true, text: [{ code: ".rerail/candidates/" }, " is not ignored, so committed decisions travel with your branch."] });
  checks.push(rulesActive
    ? { key: "rules", ok: true, text: ["Rerail is set up and started in this checkout, so decisions can steer."] }
    : { key: "rules", ok: false, text: ["Rerail isn't set up and started in this checkout, so decisions can't steer yet."], overview: true });
  return checks;
}

/* ---------- write errors ---------- */

/**
 * A failed write's reason for a person: the API's own `message` when it answered one (`MergeAssistError`), else the
 * error as `send` reported it.
 */
export function writeError(error) {
  const text = String(error?.message ?? error ?? "");
  const at = text.indexOf("{");
  if (/ answered \d{3}: /u.test(text) && at >= 0) {
    try {
      const body = JSON.parse(text.slice(at));
      if (typeof body?.message === "string" && body.message.trim() !== "") return body.message;
    } catch { /* a truncated or non-JSON body: the status line says enough */ }
  }
  return text;
}

/** A decision quoted in a toast: “…”, cut at 80 characters. */
export function quoted(text) {
  const plain = String(text).replace(/\s+/gu, " ").trim();
  return `“${plain.length > 80 ? `${plain.slice(0, 79).trimEnd()}…` : plain}”`;
}
