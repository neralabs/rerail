/**
 * `rerail merge-assist`: Merge Assist for the checkout at `--repo` (default the current folder), run in this process
 * against the one Rerail database. It runs as the person, so turning Merge Assist on or off, changing its settings and
 * dismissing a conflict are allowed here as they are on the dashboard's Merge Assist page. A coding agent with a shell
 * can run it too, as it can any Rerail command: the people-only guarantee covers the MCP and HTTP API. See
 * docs/merge-assist-v1-build.md.
 * Internal testing: the whole command. Experimental: refused unless RERAIL_EXPERIMENTAL_MERGE_ASSIST=1.
 */
import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryFolderName, repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { sideName } from "../engine/code-rule/rerail-steer.js";
import { mergeAssistExperimentEnabled } from "../engine/experimental-features.js";
import {
  conflictHookCommandSchema,
  DEFAULT_HOOK_TIMEOUT_SECONDS,
  MAX_HOOK_TIMEOUT_SECONDS,
  type Conflict,
  type GithubState,
  type MergeAssistSettings,
  type MergeAssistStatus,
} from "../engine/merge-assist/model.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { readRulesDashboard } from "../presentation/session-presentation.js";
import { createMergeAssistService, MergeAssistError, type MergeAssistService, type MergeAssistSettingsChange } from "../runtime/merge-assist/service.js";

const ACTIONS = ["status", "on", "off", "scan", "conflicts", "dismiss", "settings"] as const;
type MergeAssistAction = (typeof ACTIONS)[number];
/** The dashboard's Merge Assist page, as its route spells it. */
const DASHBOARD_PAGE = "merge";

interface Output {
  write(text: string): unknown;
}

/** A service over an open database, closed when the command is done. */
export interface MergeAssistCommandSession {
  readonly service: MergeAssistService;
  /** Resolves once the conflict hook's handlers a change started (the person's command) have answered. */
  settled(): Promise<void>;
  close(): void | Promise<void>;
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
  const service = createMergeAssistService({ database });
  // Ctrl-C while the person's hook command runs stops it (its own process group, whose timeout lives in this
  // process) before Rerail exits, so none is left running.
  const interrupt = (): void => {
    void service.stop().finally(() => process.exit(130));
  };
  process.once("SIGINT", interrupt);
  return {
    service,
    settled: () => service.settled(),
    // The conflict hook's handlers (the person's command) record their answers in the database.
    close: async () => {
      await service.settled();
      process.off("SIGINT", interrupt);
      await service.stop();
      database.$client.close();
    },
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
 * Text written by other people (logins, pull request titles, commit subjects, file paths) as one terminal line: control,
 * format and line-separator characters cannot move the cursor, recolour the terminal or forge a line of Rerail's own.
 */
function plain(text: string): string {
  return text.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]+/gu, " ").trim();
}

function quoted(text: string): string {
  return `"${plain(text)}"`;
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

/** A side's one sentence and where it is. */
function sideLine(side: Conflict["mine"]): string {
  return `${sideName(side, plain)}: ${quoted(side.summary)}${side.location === null ? "" : ` at ${plain(side.location)}`}`;
}

/** Who dismissed a conflict. */
function dismissalText(conflict: Conflict): string {
  return conflict.dismissal === null ? conflict.status : `dismissed by ${plain(conflict.dismissal.name)}: not a conflict`;
}

/** `count` with its noun: `1 branch`, `3 branches`. */
const plural = (count: number, one: string, many: string): string => `${count} ${count === 1 ? one : many}`;

/** The conflict hook command's failure on the conflict's latest event, when it failed. */
function hookLines(conflict: Conflict): string[] {
  const { hook } = conflict;
  if (hook === null || hook.ok) return [];
  return [`  Hook failed (${hook.event}): ${plain(hook.error ?? "no reason recorded")}`];
}

/** One open conflict: what, whether it is quiet, both sides, why it is a conflict, and how to dismiss it. */
function openConflictEntry(conflict: Conflict, hunks: boolean): string[] {
  const hunk = (side: Conflict["mine"]): string[] => (hunks && side.hunk !== null ? side.hunk.split("\n").map(line => `      ${plain(line)}`) : []);
  return [
    `${conflict.id}  #${plain(conflict.shortId)}  ${quoted(conflict.title)}${conflict.quiet ? "  (quiet)" : ""}`,
    `  Summary: ${plain(conflict.summary)}`,
    `  Yours:  ${sideLine(conflict.mine)}`,
    ...hunk(conflict.mine),
    `  Theirs: ${sideLine(conflict.theirs)}${conflict.theirs.url === null ? "" : ` ${plain(conflict.theirs.url)}`}`,
    ...hunk(conflict.theirs),
    ...conflict.evidence.map(line => `  ${plain(line)}`),
    ...hookLines(conflict),
    `  Not a conflict? rerail merge-assist dismiss ${conflict.id}`,
  ];
}

function dismissedConflictEntry(conflict: Conflict): string[] {
  return [`${conflict.id}  ${quoted(conflict.title)}: ${dismissalText(conflict)}`];
}

/** The status's conflicts: open ones in full, then dismissed ones in a line each. */
function conflictSections(status: MergeAssistStatus, hunks: boolean): string[] {
  const open = status.conflicts.filter(conflict => conflict.status === "open");
  const dismissed = status.conflicts.filter(conflict => conflict.status !== "open");
  return [
    ...section("Conflicts", open.map(conflict => openConflictEntry(conflict, hunks))),
    ...section("Dismissed conflicts", dismissed.map(dismissedConflictEntry)),
  ];
}

/** `rerail merge-assist conflicts`: every conflict, open ones with both sides' hunks. */
export function formatConflicts(status: MergeAssistStatus, name: string, dashboardUrl: string | null): string {
  const open = status.conflicts.filter(conflict => conflict.status === "open").length;
  return [
    `● ${plural(open, "open conflict", "open conflicts")} in ${name}${status.branch === null ? "" : ` (${plain(status.branch)})`}.`,
    ...(status.enabled ? [] : ["  Merge Assist is off here, so Rerail finds no conflicts. Turn it on with rerail merge-assist on."]),
    ...conflictSections(status, true),
    ...(dashboardUrl === null ? [] : [`  Merge Assist page: ${dashboardUrl}`]),
  ].join("\n");
}

/** What `gh`'s readiness means for a scan, but signed in. */
const GITHUB_LINES: Record<Exclude<GithubState, "signed-in">, string> = {
  "signed-out": "gh is not signed in, so open pull requests are not read and recent remote branches stand in. Sign in with gh auth login.",
  missing: "gh is not installed, so open pull requests are not read and recent remote branches stand in.",
  "not-github": "the repository's remote is not on GitHub, so recent remote branches stand in for pull requests.",
  unchecked: "not checked: Rerail runs gh only once Merge Assist is on.",
  unavailable: "gh did not say whether it is signed in (it did not answer in time or failed), so recent remote branches stand in for open pull requests this time.",
};

function githubLine(status: MergeAssistStatus): string {
  const { github } = status.readiness;
  if (github.state !== "signed-in") return `  GitHub: ${GITHUB_LINES[github.state]}`;
  return `  GitHub: gh signed in${github.login === null ? "" : ` as ${plain(github.login)}`}${github.repository === null ? "" : ` (${plain(github.repository)})`}`;
}

function scanLines(status: MergeAssistStatus, scanRequested: boolean): string[] {
  const lines: string[] = [];
  // A scan request that found another process's scan running started none: what follows is the previous scan's.
  if (status.scanning) lines.push(scanRequested ? "  Another scan is running; showing the last results." : "  A scan is running.");
  const scan = status.lastScan;
  if (scan === null) {
    if (!status.scanning) lines.push("  No scan yet. Scan for conflicts with rerail merge-assist scan.");
    return lines;
  }
  lines.push(`  Last scan: ${timeText(scan.at)}, ${plural(scan.peers.branches, "branch", "branches")} and ${plural(scan.peers.pullRequests, "pull request", "pull requests")}`);
  for (const problem of scan.problems) lines.push(`    ${plain(problem)}`);
  return lines;
}

/** What turning Merge Assist on means, before a person does it (docs/merge-assist-v1-build.md). */
const TURNING_ON = [
  "  Turning it on means:",
  "    Rerail compares your work here with open pull requests, which it reads through gh with your sign-in, read-only, and",
  "    with branches checked out in your other worktrees; with remote branches only when it cannot read pull requests.",
  "    It writes no file and never merges, commits or pushes.",
  "    When your work overlaps someone else's, it tells you and your coding agent, unless only lockfiles, changelogs, generated output or docs overlap.",
  "  Turn it on with rerail merge-assist on.",
];

/** The status as text for the repository `name`; `scanRequested` when it answers `rerail merge-assist scan`. */
export function formatMergeAssistStatus(status: MergeAssistStatus, name: string, dashboardUrl: string | null, scanRequested = false): string {
  const branch = status.branch === null ? "detached HEAD" : plain(status.branch);
  const base = status.baseBranch === null ? "" : `, base ${plain(status.baseBranch)}`;
  const page = dashboardUrl === null ? [] : [`  Merge Assist page: ${dashboardUrl}`];
  if (!status.enabled) {
    return [`● Merge Assist is off in ${name} (${branch}${base}).`, githubLine(status), ...TURNING_ON, ...page].join("\n");
  }
  const empty = status.conflicts.length === 0;
  return [
    `● Merge Assist is on in ${name} (${branch}${base}).`,
    githubLine(status),
    ...scanLines(status, scanRequested),
    ...conflictSections(status, false),
    ...(empty ? ["  No conflicts. When your work here overlaps an open pull request or a branch in another worktree, Rerail lists it."] : []),
    ...page,
  ].join("\n");
}

/** One line saying what dismissing a conflict did. */
function dismissLine(id: string, status: MergeAssistStatus): string {
  const conflict = status.conflicts.find(entry => entry.id === id);
  if (conflict === undefined) return `● Dismissed ${id}.`;
  return `● ${quoted(conflict.title)}: ${dismissalText(conflict)}.`;
}

/** What the command prints for `step`: a line for a dismissal, else the conflicts or the status. */
function resultText(step: StatusStep, status: MergeAssistStatus, name: string, dashboardUrl: string | null): string {
  const page = dashboardUrl === null ? "" : `\n  Merge Assist page: ${dashboardUrl}`;
  if (step.action === "dismiss") return `${dismissLine(step.id, status)}${page}`;
  if (step.action === "conflicts") return formatConflicts(status, name, dashboardUrl);
  return formatMergeAssistStatus(status, name, dashboardUrl, step.action === "scan");
}

/**
 * The change `rerail merge-assist settings` asks for, each field undefined where its flags leave the setting as it is:
 * the hook command's argv (null removes the hook) and its timeout.
 */
interface SettingsRequest {
  readonly command: string[] | null | undefined;
  readonly timeoutSeconds: number | undefined;
}

const NO_SETTINGS_CHANGE: SettingsRequest = { command: undefined, timeoutSeconds: undefined };
const SETTINGS_OPTIONS = ["--hook-command", "--hook-timeout"] as const;

type Step =
  | { readonly action: Exclude<MergeAssistAction, "dismiss" | "settings"> }
  | { readonly action: "dismiss"; readonly id: string }
  | { readonly action: "settings"; readonly request: SettingsRequest };
type SettingsStep = Extract<Step, { readonly action: "settings" }>;
type StatusStep = Exclude<Step, SettingsStep>;

interface Invocation {
  readonly step: Step;
  readonly root: string;
  /** The repository's folder name, as the text names it. */
  readonly name: string;
  readonly json: boolean;
}

interface StepArguments {
  readonly step: Step;
  readonly rest: readonly string[];
}

/** `dismiss <conflict-id>`: the conflict, and the arguments after it. */
function dismissStep(rest: readonly string[], usage: () => string): StepArguments {
  const id = rest[0];
  if (id === undefined || id.startsWith("--")) throw new Error(`merge-assist dismiss needs a conflict id\n\n${usage()}`);
  return { step: { action: "dismiss", id }, rest: rest.slice(1) };
}

/** The step the arguments name before their options, and the arguments left. */
function stepOf(action: MergeAssistAction, rest: readonly string[], usage: () => string): StepArguments {
  if (action === "dismiss") return dismissStep(rest, usage);
  if (action === "settings") return { step: { action, request: NO_SETTINGS_CHANGE }, rest };
  return { step: { action }, rest };
}

function parsedJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return undefined;
  }
}

/** `--hook-command`: a JSON array of non-empty strings, the program and its arguments, never run through a shell. */
function commandOption(value: string | undefined, usage: () => string): string[] | undefined {
  if (value === undefined) return undefined;
  const command = conflictHookCommandSchema.shape.command.safeParse(parsedJson(value));
  if (!command.success) throw new Error(`--hook-command takes a JSON array of non-empty strings, such as '["./notify.sh","--json"]'\n\n${usage()}`);
  return command.data;
}

/** `--hook-timeout`: whole seconds, 1 to {@link MAX_HOOK_TIMEOUT_SECONDS}. */
function timeoutOption(value: string | undefined, usage: () => string): number | undefined {
  if (value === undefined) return undefined;
  const seconds = conflictHookCommandSchema.shape.timeoutSeconds.safeParse(/^\d+$/u.test(value) ? Number(value) : value);
  if (!seconds.success) throw new Error(`--hook-timeout takes whole seconds from 1 to ${MAX_HOOK_TIMEOUT_SECONDS}\n\n${usage()}`);
  return seconds.data;
}

function settingsRequest(flags: ReadonlySet<string>, options: ReadonlyMap<string, string>, usage: () => string): SettingsRequest {
  const command = commandOption(options.get("--hook-command"), usage);
  const timeoutSeconds = timeoutOption(options.get("--hook-timeout"), usage);
  if (!flags.has("--no-hook")) return { command, timeoutSeconds };
  if (command !== undefined || timeoutSeconds !== undefined) throw new Error(`--no-hook removes the hook, so it takes no --hook-command or --hook-timeout\n\n${usage()}`);
  return { command: null, timeoutSeconds };
}

/** Options that go with another command than the step's. */
function refuseStrayOptions(step: Step, flags: ReadonlySet<string>, options: ReadonlyMap<string, string>, usage: () => string): void {
  const settings = flags.has("--no-hook") || SETTINGS_OPTIONS.some(option => options.has(option));
  if (settings && step.action !== "settings") throw new Error(`--hook-command, --no-hook and --hook-timeout go with merge-assist settings\n\n${usage()}`);
}

/** The step with what its options add: the settings' change. */
function withOptions(step: Step, flags: ReadonlySet<string>, options: ReadonlyMap<string, string>, usage: () => string): Step {
  refuseStrayOptions(step, flags, options, usage);
  if (step.action === "settings") return { action: "settings", request: settingsRequest(flags, options, usage) };
  return step;
}

function invocation(args: readonly string[], usage: () => string): Invocation {
  const first = args[0];
  const named = first !== undefined && !first.startsWith("--");
  const action = named ? ACTIONS.find(known => known === first) : "status";
  if (action === undefined) throw new Error(`Unknown merge-assist command: ${first}\n\n${usage()}`);
  const parsed = stepOf(action, named ? args.slice(1) : args, usage);
  const { flags, options } = parseArguments(parsed.rest, {
    flags: ["--json", "--no-hook"],
    options: ["--repo", ...SETTINGS_OPTIONS],
  }, argument => `Unknown merge-assist option: ${argument}\n\n${usage()}`);
  const step = withOptions(parsed.step, flags, options, usage);
  const requested = resolve(options.get("--repo") ?? process.cwd());
  let physical: string;
  try {
    physical = realpathSync(requested);
  } catch {
    throw new Error(`${requested} does not exist`);
  }
  const location = repositoryIdentityLocation(physical);
  return { step, root: location.workspaceRoot, name: repositoryFolderName(location), json: flags.has("--json") };
}

function changesSettings(request: SettingsRequest): boolean {
  return request.command !== undefined || request.timeoutSeconds !== undefined;
}

/**
 * The hook the request sets: a new command (its timeout {@link DEFAULT_HOOK_TIMEOUT_SECONDS} unless given), none, or
 * the current command with a new timeout, refused when there is no command to time.
 */
async function hookChange(service: MergeAssistService, root: string, request: SettingsRequest): Promise<Pick<MergeAssistSettingsChange, "hook">> {
  const { command, timeoutSeconds } = request;
  if (command === null) return { hook: null };
  if (command !== undefined) return { hook: { command, timeoutSeconds: timeoutSeconds ?? DEFAULT_HOOK_TIMEOUT_SECONDS } };
  if (timeoutSeconds === undefined) return {};
  const current = (await service.settings(root)).hook;
  if (current === null) throw new MergeAssistError("invalid-state", "No conflict hook is set here, so there is no timeout to change. Set one with rerail merge-assist settings --hook-command '<json argv>'.");
  return { hook: { ...current, timeoutSeconds } };
}

/** The settings after the request's change; the current ones when it changes nothing. */
async function settingsAfter(service: MergeAssistService, root: string, request: SettingsRequest): Promise<MergeAssistSettings> {
  if (!changesSettings(request)) return await service.settings(root);
  return (await service.updateSettings(root, await hookChange(service, root, request))).settings;
}

/**
 * The status a step leaves. A scan or a dismissal hands its conflict events to the person's hook command: its status is
 * read again once the command answered, so it shows the hook's outcome.
 */
async function run(session: MergeAssistCommandSession, step: StatusStep, root: string): Promise<MergeAssistStatus> {
  const { service } = session;
  if (step.action === "status" || step.action === "conflicts") return await service.status(root);
  if (step.action === "on" || step.action === "off") return await service.setEnabled(root, step.action === "on");
  if (step.action === "dismiss") await service.dismissConflict(root, step.id);
  else await service.scan(root);
  await session.settled();
  return await service.status(root);
}

/** What a step answered: the checkout's status, or for `settings` the settings. */
type Outcome =
  | { readonly step: StatusStep; readonly status: MergeAssistStatus }
  | { readonly step: SettingsStep; readonly settings: MergeAssistSettings };

async function outcomeOf(session: MergeAssistCommandSession, step: Step, root: string): Promise<Outcome> {
  if (step.action === "settings") return { step, settings: await settingsAfter(session.service, root, step.request) };
  return { step, status: await run(session, step, root) };
}

/** The settings of the repository `name` as text: the conflict hook command. */
function formatSettings(name: string, settings: MergeAssistSettings, changed: boolean, dashboardUrl: string | null): string {
  const { hook } = settings;
  return [
    `● ${changed ? "Saved Merge Assist settings" : "Merge Assist settings"} for ${name}:`,
    `  Hook: ${hook === null ? "none" : `${plain(JSON.stringify(hook.command))} (timeout ${hook.timeoutSeconds} s)`}`,
    ...(dashboardUrl === null ? [] : [`  Merge Assist page: ${dashboardUrl}`]),
  ].join("\n");
}

function outcomeText(outcome: Outcome, name: string, dashboardUrl: string | null): string {
  if ("settings" in outcome) return formatSettings(name, outcome.settings, changesSettings(outcome.step.request), dashboardUrl);
  return resultText(outcome.step, outcome.status, name, dashboardUrl);
}

/** What the command answers while the experiment is off, under `experimental-off`. */
const EXPERIMENTAL_OFF = "Merge Assist is experimental and off: set RERAIL_EXPERIMENTAL_MERGE_ASSIST=1 (or RERAIL_EXPERIMENTAL=1 for every experimental feature) to use it.";

/** A refusal: `{ error, message }` on stdout under `--json`, else `● <message>` on stderr; the command exits 1. */
function refuse(io: MergeAssistCommandIo, json: boolean, code: string, message: string): number {
  if (json) (io.stdout ?? process.stdout).write(`${JSON.stringify({ error: code, message })}\n`);
  else (io.stderr ?? process.stderr).write(`● ${message}\n`);
  return 1;
}

/**
 * `rerail merge-assist [status|on|off|scan|conflicts|dismiss <conflict-id>|settings
 * [--hook-command '<json argv>'|--no-hook] [--hook-timeout <s>]] [--repo <dir>] [--json]`.
 * Refused before anything is read or opened while the experiment is off ({@link mergeAssistExperimentEnabled}).
 */
export async function runMergeAssistCommand(args: readonly string[], usage: () => string, io: MergeAssistCommandIo = {}): Promise<number> {
  if (!mergeAssistExperimentEnabled()) return refuse(io, args.includes("--json"), "experimental-off", EXPERIMENTAL_OFF);
  const stdout = io.stdout ?? process.stdout;
  const { step, root, name, json } = invocation(args, usage);
  const session = (io.openService ?? openDefaultService)();
  let outcome: Outcome;
  try {
    outcome = await outcomeOf(session, step, root);
  } catch (error) {
    if (!(error instanceof MergeAssistError)) throw error;
    return refuse(io, json, error.code, error.message);
  } finally {
    await session.close();
  }
  if (json) {
    stdout.write(`${JSON.stringify("settings" in outcome ? outcome.settings : outcome.status)}\n`);
    return 0;
  }
  const dashboardUrl = await (io.dashboardUrl ?? defaultDashboardUrl)(root);
  stdout.write(`${outcomeText(outcome, name, dashboardUrl)}\n`);
  return 0;
}
