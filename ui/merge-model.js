/**
 * Merge Assist's page as pure derivations of `GET /api/merge-assist` (`MergeAssistStatus` in
 * `src/engine/merge-assist/model.ts`), without the DOM: the conflicts and their order, what the status strip and the
 * readiness checklist say, the Settings form, the sidebar's count, and which pull request links are safe to open.
 *
 * Conflict titles, summaries, authors, branch names and links come from other people's work. Every string here is
 * plain text for a text node; `safePeerUrl` is the only way a peer's link becomes an `href`.
 */
import { fmt, plural } from "./format.js";

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

/** The sidebar's count: every open conflict, logged ones included, as the Conflicts section counts them; 0 while off. */
export function openConflictCount(status, dismissing = new Set()) {
  if (status === null || status === undefined || !status.enabled) return 0;
  return conflictSections(status, dismissing).open.length;
}

/* ---------- conflicts ---------- */

/** What each level did in the session, as a card says it beside its age. */
export const LEVEL_TEXT = {
  stop: "Asks you before an edit to these files",
  checkpoint: "Your agent shows you a card to decide",
  ambient: "Told your agent and you in the session",
  logged: "Logged, no interruption",
};
/** A level's badge on its card. */
export const LEVEL_BADGE = { stop: "Stop", checkpoint: "Checkpoint", ambient: "Ambient", logged: "Logged" };
/** The open list's order: the loudest first. */
const LEVEL_RANK = { stop: 0, checkpoint: 1, ambient: 2, logged: 3 };

/** Why a queued conflict is quieter than its score calls for: today's checkpoints, or the session's one stop, are used. */
export const QUEUED_TEXT = "Queued: over the interruption budget, so shown as ambient until it frees up";

const newestFirst = field => (a, b) => (Date.parse(b[field]) || 0) - (Date.parse(a[field]) || 0);

/**
 * The Conflicts section's lists from the status and the dismissals in flight (`dismissing`, conflict ids). `open`:
 * loudest first (stop, checkpoint, ambient, logged), each newest first; one being dismissed leaves at once and comes
 * back if the write fails. `dismissed`: as the status orders them.
 */
export function conflictSections(status, dismissing = new Set()) {
  const open = [], dismissed = [];
  for (const conflict of status.conflicts) {
    if (conflict.status === "open") {
      if (!dismissing.has(conflict.id)) open.push(conflict);
    } else {
      dismissed.push(conflict);
    }
  }
  open.sort((a, b) => LEVEL_RANK[a.level] - LEVEL_RANK[b.level] || newestFirst("detectedAt")(a, b));
  return { open, dismissed };
}

/** A side's heading: "PR #398 · Tim", or the label alone when no author is known. */
export const sideHeading = side => (side.author ? `${side.label} · ${side.author}` : side.label);

/** A unified-diff hunk as lines to colour: `add`, `del`, `meta` (headers and `@@`) or `ctx`. */
export function hunkLines(hunk) {
  return hunk.replace(/\n$/u, "").split("\n").map(text => {
    if (text.startsWith("+++") || text.startsWith("---") || text.startsWith("@@")) return { kind: "meta", text };
    if (text.startsWith("+")) return { kind: "add", text };
    if (text.startsWith("-")) return { kind: "del", text };
    return { kind: "ctx", text };
  });
}

/** What Dismiss says once it is done, and when it fails. */
export function dismissToast(conflict) {
  return {
    done: `Marked ${quoted(conflict.title)} as not a conflict. It stays hidden for ${conflict.theirs.label}.`,
    fail: `Couldn’t mark ${quoted(conflict.title)} as not a conflict`,
  };
}

/**
 * The line a card shows when the conflict hook command failed on its last event, or `null`: it ran fine, or none ran.
 * The command never affects delivery, so this is the only place a failure shows.
 */
export function hookFailure(conflict) {
  const hook = conflict.hook;
  if (hook === null || hook === undefined || hook.ok) return null;
  return `Conflict hook failed (${hook.event}): ${hook.error ?? "no reason given"}`;
}

/* ---------- settings ---------- */

export const HOOK_TIMEOUT_DEFAULT = 10;
export const HOOK_TIMEOUT_MAX = 60;
const HOOK_ARGS_MAX = 64;
const HOOK_ARG_MAX = 4_096;

/** The Settings form's fields from the saved settings: the command as JSON (empty for none), the timeout. */
export function settingsDraft(settings) {
  return {
    command: settings.hook === null ? "" : JSON.stringify(settings.hook.command),
    timeout: String(settings.hook?.timeoutSeconds ?? HOOK_TIMEOUT_DEFAULT),
  };
}

const ARGV_PROBLEM = "The hook command must be a JSON array of non-empty strings, like [\"./scripts/on-conflict.sh\", \"--quiet\"].";

/** Why `argv` is not a hook command (a JSON array of 1 to 64 non-empty strings), or `null`. */
function argvProblem(argv) {
  if (!Array.isArray(argv) || argv.length === 0 || argv.some(arg => typeof arg !== "string" || arg === "")) return ARGV_PROBLEM;
  if (argv.length > HOOK_ARGS_MAX) return `The hook command takes at most ${fmt(HOOK_ARGS_MAX)} arguments.`;
  if (argv.some(arg => arg.length > HOOK_ARG_MAX)) return `Each hook argument takes at most ${fmt(HOOK_ARG_MAX)} characters.`;
  return null;
}

/** The command field as JSON, or `null` when it is not JSON (`argvProblem` says why). */
function parseArgv(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** The timeout field in whole seconds (empty: the default), or `null` when it is not 1 to 60. */
function parseTimeout(text) {
  const seconds = text.trim() === "" ? HOOK_TIMEOUT_DEFAULT : Number(text);
  return Number.isInteger(seconds) && seconds >= 1 && seconds <= HOOK_TIMEOUT_MAX ? seconds : null;
}

/**
 * The body of `PUT /api/merge-assist/settings` (without the scope) from the form's fields, `{ body }`, or why it can't
 * be saved, `{ problem, field }` (`command` or `timeout`). An empty command removes the hook, whatever the timeout says.
 */
export function settingsChange(draft) {
  const text = draft.command.trim();
  if (text === "") return { body: { hook: null } };
  const argv = parseArgv(text);
  const problem = argvProblem(argv);
  if (problem !== null) return { problem, field: "command" };
  const timeoutSeconds = parseTimeout(draft.timeout);
  if (timeoutSeconds === null) return { problem: `The timeout must be a whole number of seconds from 1 to ${fmt(HOOK_TIMEOUT_MAX)}.`, field: "timeout" };
  return { body: { hook: { command: argv, timeoutSeconds } } };
}

/** The Settings fold's summary: whether a conflict hook command is set, and which. */
export function settingsSummary(settings) {
  return settings.hook === null ? "Settings: no conflict hook" : `Settings: conflict hook ${settings.hook.command[0]}`;
}

/* ---------- the status strip ---------- */

/** "tbelanger/export-api" over "main", or what is missing. */
export function branchText(status) {
  return {
    branch: status.branch ?? "Detached HEAD",
    base: status.baseBranch ?? "no base branch found",
  };
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

/**
 * After Scan now, for its toast: what the scan read and how many open conflicts it found that the page did not show
 * before (`before`, the status at the click, or `null`), not the totals. "Scanned 3 local branches. New: 2 conflicts."
 * or "Scanned 3 local branches: nothing new." While another process's scan runs, the request started none, and the
 * status holds the last results.
 */
export function scanSummary(before, after) {
  if (after.scanning) return "Another scan is running; showing the last results.";
  if (after.lastScan === null) return "Scan finished.";
  const shown = new Set(before === null ? [] : conflictSections(before).open.map(c => c.id));
  const fresh = conflictSections(after).open.filter(c => !shown.has(c.id)).length;
  const problems = after.lastScan.problems.length;
  const tail = problems === 0 ? "" : ` ${fmt(problems)} ${plural(problems, "thing")} could not be read; see Last scan.`;
  const what = scannedWhat(after.lastScan);
  return `${fresh === 0 ? `Scanned ${what}: nothing new.` : `Scanned ${what}. New: ${fmt(fresh)} ${plural(fresh, "conflict")}.`}${tail}`;
}

/* ---------- the readiness checklist ---------- */

/**
 * What Rerail found before it is turned on, one line each: `ok`, and `text` as segments (a string, or `{ code }`
 * for a command or path). The page renders each segment as a text node.
 */
export function readinessChecks({ github }) {
  switch (github.state) {
    case "signed-in":
      return [{ key: "github", ok: true, text: [{ code: "gh" }, ` is signed in${github.login ? ` as ${github.login}` : ""}. Open pull requests are read with your sign-in.`] }];
    case "signed-out":
      return [{ key: "github", ok: false, text: [{ code: "gh" }, " is not signed in, so only local branches are read. Run ", { code: "gh auth login" }, " to read open pull requests."] }];
    case "missing":
      return [{ key: "github", ok: false, text: [{ code: "gh" }, " is not installed, so only local branches are read."] }];
    default:
      return [{ key: "github", ok: false, text: ["This repository is not on GitHub, so only local branches are read."] }];
  }
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

/** A conflict's title quoted in a toast: “…”, cut at 80 characters. */
export function quoted(text) {
  const plain = String(text).replace(/\s+/gu, " ").trim();
  return `“${plain.length > 80 ? `${plain.slice(0, 79).trimEnd()}…` : plain}”`;
}
