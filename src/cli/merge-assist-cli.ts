/**
 * `rerail merge-assist`: Merge Assist for the checkout at `--repo` (default the current folder), run in this process
 * against the one Rerail database. It runs as the person, so turning Merge Assist on or off, adopting and undoing are
 * allowed here as they are on the dashboard's Merge Assist page. A coding agent with a shell can run it too, as it can
 * any Rerail command: the people-only guarantee covers the MCP and HTTP API. See docs/merge-assist-v1-build.md §5.6 and §7.
 * Internal testing: the whole command.
 */
import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { CANDIDATE_DIRECTORY, candidateFileName } from "../engine/merge-assist/candidate.js";
import { CONFLICT_ACTIONS, type Conflict, type ConflictAction, type Finding, type JudgeRunnerId, type MergeAssistStatus, type OwnDecision } from "../engine/merge-assist/model.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { readRulesDashboard } from "../presentation/session-presentation.js";
import { createMergeAssistService, MergeAssistError, type MergeAssistService } from "../runtime/merge-assist/service.js";

const ID_ACTIONS = ["reject", "adopt", "dismiss", "undo"] as const;
const ACTIONS = ["status", "on", "off", "scan", "conflicts", "resolve", ...ID_ACTIONS] as const;
type MergeAssistAction = (typeof ACTIONS)[number];
type IdAction = (typeof ID_ACTIONS)[number];

function takesId(action: MergeAssistAction): action is IdAction {
  return (ID_ACTIONS as readonly string[]).includes(action);
}
/** The dashboard's Merge Assist page, as its route spells it. */
const DASHBOARD_PAGE = "merge";
const RUNNER_NAMES: Record<JudgeRunnerId, string> = { claude: "Claude Code", codex: "Codex" };

interface Output {
  write(text: string): unknown;
}

/** A service over an open database, closed when the command is done. */
export interface MergeAssistCommandSession {
  readonly service: MergeAssistService;
  close(): void;
}

export interface MergeAssistCommandIo {
  stdout?: Output;
  stderr?: Output;
  /** The service the command runs; by default one over the home's database. */
  openService?: () => MergeAssistCommandSession;
  /** The Merge Assist page of the daemon serving `root`'s checkout, or null when none runs. */
  dashboardUrl?: (root: string) => Promise<string | null>;
}

function openDefaultService(): MergeAssistCommandSession {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  return {
    service: createMergeAssistService({ database, ruleCatalog: loadRuleCatalog() }),
    close: () => database.$client.close(),
  };
}

/** The live dashboard's page for the checkout, never starting a daemon. */
async function defaultDashboardUrl(root: string): Promise<string | null> {
  const dashboard = await readRulesDashboard({ cwd: root });
  if (dashboard.url === null) return null;
  const link = new URL(dashboard.url);
  link.hash = DASHBOARD_PAGE;
  return link.href;
}

/**
 * Text written by other people (decisions, logins, pull request titles, file paths) as one terminal line: control,
 * format and line-separator characters cannot move the cursor, recolour the terminal or forge a line of Rerail's own.
 */
function plain(text: string): string {
  return text.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]+/gu, " ").trim();
}

function quoted(text: string): string {
  return `"${plain(text)}"`;
}

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function timeText(iso: string): string {
  const time = new Date(iso);
  return Number.isNaN(time.getTime()) ? plain(iso) : `${time.toISOString().slice(0, 16).replace("T", " ")} UTC`;
}

/** A heading with its count, then each entry's lines; nothing when there are no entries. */
function section(heading: string, entries: readonly string[][]): string[] {
  if (entries.length === 0) return [];
  return [`  ${heading} (${entries.length}):`, ...entries.flat().map(line => `    ${line}`)];
}

/**
 * Where a finding came from and who is behind it. A pull request names the author GitHub reports; a local branch can
 * only name the author its decision file declares.
 */
function peerText(finding: Finding): string {
  const { peer } = finding;
  const where = peer.kind === "pull-request"
    ? `pull request${peer.number === null ? "" : ` #${peer.number}`}${peer.title === null ? "" : ` ${quoted(peer.title)}`}${peer.author === null ? "" : ` by ${plain(peer.author)}`}`
    : `branch ${plain(peer.ref)}, declared by ${plain(finding.candidate.author)}`;
  return `${where}${peer.trusted ? "" : ", not trusted"}`;
}

function evidenceLines(finding: Finding): string[] {
  const { evidence } = finding;
  return [
    ...evidence.violations.map(violation => `  Your branch breaks it: ${plain(violation.path)}:${violation.line} ${plain(violation.message)}`),
    ...(evidence.conflicts.length === 0 ? [] : [`  Files both branches change: ${evidence.conflicts.map(plain).join(", ")}`]),
  ];
}

function talkEntry(finding: Finding): string[] {
  return [
    finding.id,
    `  Theirs: ${quoted(finding.candidate.decision)} (${peerText(finding)})`,
    ...(finding.evidence.mine === null ? [] : [`  Yours:  ${quoted(finding.evidence.mine.decision)}`]),
    `  ${plain(finding.reason)}`,
    ...evidenceLines(finding),
    ...(finding.message === null ? [] : [`  To send them: ${plain(finding.message)}`]),
  ];
}

function reviewEntry(finding: Finding): string[] {
  return [
    `${finding.id}  ${quoted(finding.candidate.decision)} (${peerText(finding)})`,
    `  ${plain(finding.reason)}`,
    ...evidenceLines(finding),
    `  Adopt: rerail merge-assist adopt ${finding.id}   Dismiss: rerail merge-assist dismiss ${finding.id}`,
  ];
}

function addedEntry(finding: Finding): string[] {
  return [
    `${finding.id}  ${quoted(finding.candidate.decision)} (${peerText(finding)})`,
    `  ${finding.tier === "absorb" ? "Added automatically" : "Adopted"}; ${finding.candidate.level === "code" ? "Rerail checks it exactly" : "a note your agent was given"}. Undo: rerail merge-assist undo ${finding.id}`,
  ];
}

/** Who is on a conflict's side: its label, with its author when known. */
function sideText(side: Conflict["mine"]): string {
  return `${plain(side.label)}${side.author === null ? "" : ` (${plain(side.author)})`}`;
}

/** A side's one sentence and where it is. */
function sideLine(side: Conflict["mine"]): string {
  return `${sideText(side)}: ${quoted(side.summary)}${side.location === null ? "" : ` at ${plain(side.location)}`}`;
}

/** What settled a conflict: the verdict, who, and the decision it wrote. */
function resolutionText(conflict: Conflict): string {
  const resolution = conflict.resolution;
  if (resolution === null) return conflict.status;
  if (resolution.action === "dismiss") return `dismissed by ${plain(resolution.name)}: not a conflict`;
  const verdict = resolution.action === "theirs" ? `took ${plain(conflict.theirs.label)}'s side` : resolution.action === "mine" ? "kept this branch's side" : "wrote a new decision";
  const by = resolution.by === "precedent" ? "by precedent" : `by ${plain(resolution.name)}`;
  return `${verdict}, ${by}${resolution.rule === null ? "" : `; decision ${plain(resolution.rule)}`}`;
}

/** One open conflict: what, both sides, why, and how to settle it. */
function openConflictEntry(conflict: Conflict, hunks: boolean): string[] {
  const hunk = (side: Conflict["mine"]): string[] => (hunks && side.hunk !== null ? side.hunk.split("\n").map(line => `      ${plain(line)}`) : []);
  return [
    `${conflict.id}  ${quoted(conflict.title)}  (${conflict.kind === "overlap" ? "overlapping work" : "decided differently"}${conflict.level === "logged" ? ", logged only" : ""})`,
    `  Yours:  ${sideLine(conflict.mine)}`,
    ...hunk(conflict.mine),
    `  Theirs: ${sideLine(conflict.theirs)}${conflict.theirs.url === null ? "" : ` ${plain(conflict.theirs.url)}`}`,
    ...hunk(conflict.theirs),
    ...conflict.evidence.map(line => `  ${plain(line)}`),
    `  Resolve: rerail merge-assist resolve ${conflict.id} mine|theirs|new|dismiss [--text "..."]`,
  ];
}

function settledConflictEntry(conflict: Conflict): string[] {
  return [`${conflict.id}  ${quoted(conflict.title)}: ${resolutionText(conflict)}`];
}

/** The status's conflicts: open ones in full, then settled ones in a line each. */
function conflictSections(status: MergeAssistStatus, hunks: boolean): string[] {
  const open = status.conflicts.filter(conflict => conflict.status === "open");
  const settled = status.conflicts.filter(conflict => conflict.status !== "open");
  return [...section("Conflicts", open.map(conflict => openConflictEntry(conflict, hunks))), ...section("Settled conflicts", settled.map(settledConflictEntry))];
}

/** `rerail merge-assist conflicts`: every conflict, open ones with both sides' hunks, and the latest decision log entries. */
export function formatConflicts(status: MergeAssistStatus, dashboardUrl: string | null): string {
  const open = status.conflicts.filter(conflict => conflict.status === "open").length;
  const log = status.decisionLog.entries.slice(0, 5).map(entry => [`${timeText(entry.recordedAt)}  ${quoted(entry.title)}: ${plain(entry.outcome)}, by ${plain(entry.by.name)}`]);
  return [
    `● ${count(open, "open conflict", "open conflicts")} in ${repositoryName(status.checkout)}${status.branch === null ? "" : ` (${plain(status.branch)})`}.`,
    ...(status.enabled ? [] : ["  Merge Assist is off here, so Rerail finds no conflicts. Turn it on with rerail merge-assist on."]),
    ...conflictSections(status, true),
    ...section("Decision log (refs/rerail/decisions)", log),
    ...status.decisionLog.problems.map(problem => `  ${plain(problem)}`),
    ...(dashboardUrl === null ? [] : [`  Merge Assist page: ${dashboardUrl}`]),
  ].join("\n");
}

function ownEntry(decision: OwnDecision): string[] {
  const how = decision.level === "code" ? "Rerail checks it exactly" : "your agent judges it";
  const scope = decision.scope.length === 0 ? "the whole repository" : decision.scope.map(plain).join(", ");
  const shared = decision.committed ? "committed" : "not committed yet";
  const amended = plain(decision.amends?.decision ?? "");
  const note = decision.state === "rejected" ? "  (not a rule)" : decision.amendedBy === null ? "" : `  (amended by ${decision.amendedBy})`;
  return [
    `${decision.id}  ${quoted(decision.decision)}${note}`,
    ...(decision.amends === null ? [] : [`  amends "${amended.length > 60 ? `${amended.slice(0, 59).trimEnd()}…` : amended}"`]),
    `  ${how} · ${scope} · saved ${timeText(decision.createdAt)} · ${shared}`,
    ...(decision.state === "rejected" ? [] : [`  Not a rule: rerail merge-assist reject ${decision.id}`]),
  ];
}

function githubLine(status: MergeAssistStatus): string {
  const { github } = status.readiness;
  if (github.state === "signed-in") {
    return `  GitHub: gh signed in${github.login === null ? "" : ` as ${plain(github.login)}`}${github.repository === null ? "" : ` (${plain(github.repository)})`}`;
  }
  if (github.state === "signed-out") return "  GitHub: gh is not signed in, so open pull requests are not read. Sign in with gh auth login.";
  if (github.state === "missing") return "  GitHub: gh is not installed, so open pull requests are not read.";
  return "  GitHub: the repository's remote is not on GitHub, so only local branches are read.";
}

function readinessLines(status: MergeAssistStatus): string[] {
  const { judge, candidatesIgnored, rulesActive } = status.readiness;
  const { budget } = status;
  return [
    judge.available && judge.runner !== null
      ? `  Judgment: ${RUNNER_NAMES[judge.runner]} · ${budget.used} of ${budget.limit} calls in the last ${budget.windowMinutes} minutes`
      : `  Judgment: unavailable, so nothing is captured or compared. Merge Assist runs Claude Code or Codex.${judge.problem === null ? "" : ` ${plain(judge.problem)}`}`,
    githubLine(status),
    ...(rulesActive ? [] : ["  Rerail isn't set up and started in this checkout, so decisions can't steer yet. Start it with rerail start."]),
    ...(candidatesIgnored ? [`  Git ignores ${CANDIDATE_DIRECTORY}/ here, so decisions stay on this machine.`] : []),
  ];
}

function scanLines(status: MergeAssistStatus, scanRequested: boolean): string[] {
  const lines: string[] = [];
  // A scan request that found another process's scan running started none: what follows is the previous scan's.
  if (status.scanning) lines.push(scanRequested ? "  Another scan is running; showing the last results." : "  A scan is running.");
  const scan = status.lastScan;
  if (scan === null) {
    if (!status.scanning) lines.push("  No scan yet. Scan other branches with rerail merge-assist scan.");
    return lines;
  }
  lines.push(`  Last scan: ${timeText(scan.at)}, ${count(scan.peers.branches, "branch", "branches")} and ${count(scan.peers.pullRequests, "pull request", "pull requests")}`);
  for (const problem of scan.problems) lines.push(`    ${plain(problem)}`);
  return lines;
}

/** What turning Merge Assist on means, before a person does it (docs/merge-assist-v1-build.md §5.1). */
const TURNING_ON = [
  "  Turning it on means:",
  "    Rerail reads the corrections you give your coding agent here and asks your agent's CLI (Claude Code or Codex) to",
  "    state each one as a decision. That sends the correction and the code it corrected to that CLI's provider, under your",
  "    own sign-in, as your session already does.",
  `    Each decision is written to ${CANDIDATE_DIRECTORY}/ in the checkout. Commit it to share it; Rerail never commits.`,
  "    Rerail reads other local branches, and open pull requests through gh with your sign-in, read-only.",
  "    Decisions from teammates with write access may steer your agent; you can undo each one.",
  "  Turn it on with rerail merge-assist on.",
];

/** The status as text; `scanRequested` when it answers `rerail merge-assist scan`. */
export function formatMergeAssistStatus(status: MergeAssistStatus, dashboardUrl: string | null, scanRequested = false): string {
  const name = repositoryName(status.checkout);
  const branch = status.branch === null ? "detached HEAD" : plain(status.branch);
  const base = status.baseBranch === null ? "" : `, base ${plain(status.baseBranch)}`;
  const page = dashboardUrl === null ? [] : [`  Merge Assist page: ${dashboardUrl}`];
  if (!status.enabled) {
    return [`● Merge Assist is off in ${name} (${branch}${base}).`, ...readinessLines(status), ...TURNING_ON, ...page].join("\n");
  }
  const open = (tier: Finding["tier"]): Finding[] => status.findings.filter(finding => finding.tier === tier && finding.status === "open");
  const applied = status.findings.filter(finding => finding.status === "applied");
  const dismissed = status.findings.filter(finding => finding.status === "dismissed").length;
  const empty = status.own.length === 0 && status.findings.length === 0;
  return [
    `● Merge Assist is on in ${name} (${branch}${base}).`,
    ...readinessLines(status),
    ...scanLines(status, scanRequested),
    ...conflictSections(status, false),
    ...section("Decided differently", open("talk").map(talkEntry)),
    ...section("Needs your review", open("review").map(reviewEntry)),
    ...section("Added from other branches", applied.map(addedEntry)),
    ...section("Decisions on this branch", status.own.map(ownEntry)),
    ...(dismissed === 0 ? [] : [`  Dismissed: ${count(dismissed, "finding", "findings")}, hidden until their decisions change.`]),
    ...(empty ? ["  No decisions yet. When you correct your coding agent here, Rerail saves the decision on this branch and lists it."] : []),
    ...page,
  ].join("\n");
}

function decisionOf(status: MergeAssistStatus, findingId: string): string {
  const finding = status.findings.find(entry => entry.id === findingId);
  return finding === undefined ? findingId : quoted(finding.candidate.decision);
}

/** One line saying what resolving a conflict did. */
function resolveLine(id: string, status: MergeAssistStatus): string {
  const conflict = status.conflicts.find(entry => entry.id === id);
  if (conflict === undefined) return `● Resolved ${id}.`;
  const logged = conflict.resolution?.logId === null || conflict.resolution === null ? "" : ` Logged as decision ${conflict.resolution.logId} in refs/rerail/decisions.`;
  return `● ${quoted(conflict.title)}: ${resolutionText(conflict)}.${logged}`;
}

/** What the command prints for `step`: a line for an action, else the conflicts or the status. */
function resultText(step: Step, status: MergeAssistStatus, dashboardUrl: string | null): string {
  const page = dashboardUrl === null ? "" : `\n  Merge Assist page: ${dashboardUrl}`;
  if (step.action === "resolve") return `${resolveLine(step.id, status)}${page}`;
  if ("id" in step) return `${actionLine(step.action, step.id, status)}${page}`;
  if (step.action === "conflicts") return formatConflicts(status, dashboardUrl);
  return formatMergeAssistStatus(status, dashboardUrl, step.action === "scan");
}

/** One line saying what an action on one decision did. */
function actionLine(action: IdAction, id: string, status: MergeAssistStatus): string {
  if (action === "reject") {
    const steers = `● ${id} is not a rule any more and no longer steers here.`;
    return status.readiness.candidatesIgnored
      ? `${steers} Git ignores ${CANDIDATE_DIRECTORY}/ here, so the change stays on this machine.`
      : `${steers} Commit ${CANDIDATE_DIRECTORY}/${candidateFileName(id)} so teammates who added it drop it too.`;
  }
  if (action === "adopt") return `● Adopted ${decisionOf(status, id)}: it steers your coding agent in this checkout. Undo with rerail merge-assist undo ${id}.`;
  if (action === "dismiss") return `● Dismissed ${decisionOf(status, id)}. It stays hidden until the decision changes.`;
  return `● ${decisionOf(status, id)} no longer steers your coding agent in this checkout.`;
}

type Step =
  | { readonly action: Exclude<MergeAssistAction, IdAction | "resolve"> }
  | { readonly action: IdAction; readonly id: string }
  | { readonly action: "resolve"; readonly id: string; readonly verdict: ConflictAction; readonly text: string | undefined };

interface Invocation {
  readonly step: Step;
  readonly root: string;
  readonly json: boolean;
}

/** `resolve <conflict-id> mine|theirs|new|dismiss`: the conflict and the verdict, and the arguments after them. */
function resolveStep(rest: readonly string[], usage: () => string): { id: string; verdict: ConflictAction; rest: readonly string[] } {
  const [id, verdict] = rest;
  if (id === undefined || id.startsWith("--")) throw new Error(`merge-assist resolve needs a conflict id\n\n${usage()}`);
  const known = CONFLICT_ACTIONS.find(action => action === verdict);
  if (known === undefined) throw new Error(`merge-assist resolve needs mine, theirs, new or dismiss after the conflict id\n\n${usage()}`);
  return { id, verdict: known, rest: rest.slice(2) };
}

/** The step the arguments name before their options, and the arguments left. */
function stepOf(action: MergeAssistAction, rest: readonly string[], usage: () => string): { step: Step; rest: readonly string[] } {
  if (action === "resolve") {
    const resolved = resolveStep(rest, usage);
    return { step: { action, id: resolved.id, verdict: resolved.verdict, text: undefined }, rest: resolved.rest };
  }
  if (!takesId(action)) return { step: { action }, rest };
  const id = rest[0];
  if (id === undefined || id.startsWith("--")) throw new Error(`merge-assist ${action} needs ${action === "reject" ? "a candidate id" : "a finding id"}\n\n${usage()}`);
  return { step: { action, id }, rest: rest.slice(1) };
}

function invocation(args: readonly string[], usage: () => string): Invocation {
  const first = args[0];
  const named = first !== undefined && !first.startsWith("--");
  const action = named ? ACTIONS.find(known => known === first) : "status";
  if (action === undefined) throw new Error(`Unknown merge-assist command: ${first}\n\n${usage()}`);
  const parsed = stepOf(action, named ? args.slice(1) : args, usage);
  const { flags, options } = parseArguments(parsed.rest, { flags: ["--json"], options: ["--repo", "--text"] }, argument => `Unknown merge-assist option: ${argument}\n\n${usage()}`);
  const text = options.get("--text");
  if (text !== undefined && parsed.step.action !== "resolve") throw new Error(`--text goes with merge-assist resolve\n\n${usage()}`);
  const step = parsed.step.action === "resolve" ? { ...parsed.step, text } : parsed.step;
  const requested = resolve(options.get("--repo") ?? process.cwd());
  let physical: string;
  try {
    physical = realpathSync(requested);
  } catch {
    throw new Error(`${requested} does not exist`);
  }
  return { step, root: repositoryIdentityLocation(physical).workspaceRoot, json: flags.has("--json") };
}

async function run(service: MergeAssistService, step: Step, root: string): Promise<MergeAssistStatus> {
  if (step.action === "resolve") return await service.resolveConflict(root, step.id, step.verdict, step.text, "person");
  if ("id" in step) {
    if (step.action === "reject") return await service.setCandidateState(root, step.id, "rejected");
    return await service.resolveFinding(root, step.id, step.action, "person");
  }
  if (step.action === "status" || step.action === "conflicts") return await service.status(root);
  if (step.action === "scan") return await service.scan(root);
  return await service.setEnabled(root, step.action === "on");
}

/**
 * `rerail merge-assist [status|on|off|scan|conflicts|reject <id>|adopt <id>|dismiss <id>|undo <id>|resolve <id> <verdict>
 * [--text "..."]] [--repo <dir>] [--json]`.
 */
export async function runMergeAssistCommand(args: readonly string[], usage: () => string, io: MergeAssistCommandIo = {}): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const { step, root, json } = invocation(args, usage);
  const session = (io.openService ?? openDefaultService)();
  let status: MergeAssistStatus;
  try {
    status = await run(session.service, step, root);
  } catch (error) {
    if (!(error instanceof MergeAssistError)) throw error;
    if (json) stdout.write(`${JSON.stringify({ error: error.code, message: error.message })}\n`);
    else (io.stderr ?? process.stderr).write(`● ${error.message}\n`);
    return 1;
  } finally {
    session.close();
  }
  if (json) {
    stdout.write(`${JSON.stringify(status)}\n`);
    return 0;
  }
  const dashboardUrl = await (io.dashboardUrl ?? defaultDashboardUrl)(root);
  stdout.write(`${resultText(step, status, dashboardUrl)}\n`);
  return 0;
}
