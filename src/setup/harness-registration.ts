import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { constants, existsSync } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readdir,
  readFile,
  readlink,
  realpath,
  rename,
  rm,
  stat,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, normalize, relative, resolve, sep } from "node:path";
import { promisify } from "node:util";
import {
  isClaudeStatusLine,
  readClaudeStatusRegistration,
  readClaudeStatusSettingsPaths,
  writeClaudeStatusRegistration,
  type ClaudeStatusRegistration,
  type ClaudeStatusLine,
} from "./claude-status-store.js";
import { commandFileText, MANAGED_SKILL_MARKER, type CommandFileContext } from "../harness/rerail-command-files.js";
import { RERAIL_COMMANDS, type RerailCommandName } from "../harness/rerail-commands.js";
import { shellQuote } from "../harness/setup-screens.js";
import { stableJsonText } from "../engine/stable-json.js";
import { ENTRY_POINTS } from "../engine/package-root.js";

export type Harness = "omp" | "pi" | "claude-code" | "codex" | "cursor";

export const HARNESSES: readonly Harness[] = ["omp", "pi", "claude-code", "codex", "cursor"];

export interface RegistrationOptions {
  /** Installed package root containing package.json, dist/ and hooks/. */
  root: string;
  home?: string;
  nodePath?: string;
  harnesses?: readonly Harness[];
  /** Native project settings context; omitted for global-only programmatic registration. */
  workspaceDir?: string;
  dryRun?: boolean;
  /** Test seam for platform-specific command quoting and directory links. */
  platform?: NodeJS.Platform;
}

export interface RegistrationChange {
  harness: Harness;
  path: string;
  action: "created" | "updated" | "unchanged" | "skipped";
  detail: string;
}

export interface RegistrationReport {
  changes: RegistrationChange[];
  notes: string[];
}

type JsonObject = Record<string, unknown>;
type Mode = "register" | "unregister";

interface Context {
  mode: Mode;
  root: string;
  packageRoots: Map<string, Promise<boolean>>;
  home: string;
  nodePath: string;
  dryRun: boolean;
  platform: NodeJS.Platform;
}

interface TextFile {
  exists: boolean;
  text: string;
  mode: number;
}

interface JsonFile {
  exists: boolean;
  value: JsonObject;
  mode: number;
}

/** One harness mutation: what would change, plus the write that performs it (undefined when nothing to do or dry run). */
interface Plan {
  change: RegistrationChange;
  apply?: (() => Promise<void>) | undefined;
}

const CONFIG_DIRS: Record<Harness, string> = {
  omp: join(".omp", "agent"),
  pi: join(".pi", "agent"),
  "claude-code": ".claude",
  codex: ".codex",
  cursor: ".cursor",
};

export const CLAUDE_HOOK_EVENTS: readonly string[] = [
  "SessionStart",
  "SessionEnd",
  "UserPromptSubmit",
  "MessageDisplay",
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "Stop",
  "StopFailure",
];
const CLAUDE_TOOL_EVENTS: readonly string[] = ["PreToolUse", "PostToolUse", "PostToolUseFailure"];

export const CODEX_HOOK_EVENTS: readonly string[] = [
  "SessionStart",
  "UserPromptSubmit",
  "PreToolUse",
  "PostToolUse",
  "Stop",
];
const CODEX_TOOL_EVENTS: readonly string[] = ["PostToolUse"];

/** Mirrors the events wired in hooks/cursor.json. */
export const CURSOR_HOOK_EVENTS: readonly string[] = [
  "sessionStart",
  "sessionEnd",
  "beforeSubmitPrompt",
  "afterAgentResponse",
  "preToolUse",
  "postToolUse",
  "postToolUseFailure",
  "stop",
];



function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMissing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}

/** The path cannot exist: it, or a directory on the way to it, is missing or is not a directory. */
function isAbsent(error: unknown): boolean {
  return isMissing(error) || (error instanceof Error && "code" in error && error.code === "ENOTDIR");
}

function assertInside(path: string, container: string, label: string): void {
  const rel = relative(container, path);
  if (rel === "" || rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel)) {
    throw new Error(`Refusing to write ${path}: outside ${label} ${container}`);
  }
}

function shellArgument(value: string, platform: NodeJS.Platform): string {
  if (platform === "win32") return `"${value.replaceAll("%", "%%")}"`;
  return /^[\w./:@+=-]+$/.test(value) ? value : `"${value.replace(/(["\\$`])/g, "\\$1")}"`;
}

/** `<recorded node> <root>/<executable>`, both quoted for the platform's shell; `executable` is an `ENTRY_POINTS` path. */
function directCommand(context: Context, executable: string): string {
  return `${shellArgument(context.nodePath, context.platform)} ${shellArgument(join(context.root, executable), context.platform)}`;
}

/** The package root of a registered `script` when it is that package's `executable`; undefined for any other script. */
function registeredRoot(script: string, executable: string): string | undefined {
  const tail = `${sep}${normalize(executable)}`;
  return script.endsWith(tail) ? script.slice(0, -tail.length) : undefined;
}

async function isRerailPackageRoot(context: Context, root: string): Promise<boolean> {
  if (root === context.root || root === join(context.home, ".rerail", "current")) return true;
  let verified = context.packageRoots.get(root);
  if (verified === undefined) {
    verified = readFile(join(root, "package.json"), "utf8").then(text => {
      const value: unknown = JSON.parse(text);
      return isObject(value) && value.name === "@neralabs/rerail";
    }).catch(() => false);
    context.packageRoots.set(root, verified);
  }
  return verified;
}

/** False only when the path cannot exist; any other failure to look counts as present. */
async function pathExists(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    return !isAbsent(error);
  }
}

/**
 * Whether registration replaces a generated entry that runs `script` (`<root>/<executable>`, an `ENTRY_POINTS` path): the
 * root is an install of this package, or the install is gone, with neither the script nor a package.json beside it on
 * disk, as after its home was deleted. A gone install's entry can never run again; a root that holds another package's
 * package.json is that package's, whether or not its script is there. A script of any other shape is never replaced.
 */
async function isReplaceableScript(context: Context, script: string, executable: string): Promise<boolean> {
  const root = registeredRoot(script, executable);
  if (root === undefined) return false;
  if (await isRerailPackageRoot(context, root)) return true;
  return !(await pathExists(script)) && !(await pathExists(join(root, "package.json")));
}

/** Whether `path` resolves to something on disk, following symlinks: a dangling link does not. */
async function resolves(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch (error) {
    return !isAbsent(error);
  }
}

/**
 * Whether an extension link or Pi package names an install of this package that is gone. Nothing is left on disk to
 * identify it, so only the path does: a root of the shape setup registers (a Rerail home's `current`, under `~/.rerail`
 * or any `$RERAIL_HOME`, or an npm install at `node_modules/@neralabs/rerail`) that no longer resolves, as after its
 * home was deleted or its `current` link outlived its versions. A root that still resolves stays whatever it holds,
 * since Pi also loads a folder of only extensions/ or a single file; so does a gone root of any other shape, such as a
 * tool on an unmounted volume or a gone checkout.
 */
async function isGoneInstallRoot(root: string): Promise<boolean> {
  const segments = root.split(/[\\/]/u);
  if (segments.at(-1) !== "current" && segments.slice(-3).join("/") !== "node_modules/@neralabs/rerail") return false;
  return !(await resolves(root));
}

function unquoteShellWord(word: string, windows = false): string {
  const value = word.replace(/^"|"$/gu, "");
  return windows ? value.replaceAll("%%", "%") : value.replace(/\\(["\\$`])/gu, "$1");
}

/** The words of `text` as `shellArgument` writes them, still quoted: double-quoted or bare, and always quoted on Windows. */
function shellWords(text: string, windows: boolean): string[] {
  return text.match(windows ? /"[^"]+"/gu : /"(?:\\["\\$`]|[^"\\$`])*"|[\w./:@+=-]+/gu) ?? [];
}

interface HookTarget {
  nodePath: string;
  script: string;
}

/** Node and hook script of a generated shell-form entry: direct, the POSIX `sh` fallback, or the Windows `cmd` fallback. */
function shellHookTarget(context: Context, command: string, executable: string): HookTarget | undefined {
  const fallbackPrefix = `sh -c 'if [ -x "$0" ]; then exec "$0" "$1"; fi; exec node "$1"' `;
  const windowsFallback = /^cmd \/d \/c "if exist ("[^"]+") \(\1 ("[^"]+")\) else \(node \2\)"$/u.exec(command);
  let direct = windowsFallback === null ? command : `${windowsFallback[1]} ${windowsFallback[2]}`;
  if (command.startsWith(fallbackPrefix)) direct = command.slice(fallbackPrefix.length);
  const windows = context.platform === "win32";
  const words = shellWords(direct, windows);
  if (words.length !== 2 || words.join(" ") !== direct) return undefined;
  const target = { nodePath: unquoteShellWord(words[0]!, windows), script: unquoteShellWord(words[1]!, windows) };
  const root = registeredRoot(target.script, executable);
  if (root === undefined) return undefined;
  const recorded = { ...context, root, nodePath: target.nodePath };
  return hookCommand(recorded, executable) === command || directCommand(recorded, executable) === command ? target : undefined;
}

/** A node binary as setup records it: `node` or `node.exe`, bare or at any path. */
function isNodeExecutable(path: string): boolean {
  return /^node(?:\.exe)?$/iu.test(path.split(/[\\/]/u).at(-1)!);
}

/** Node and hook script of a generated exec-form entry: `node <root>/<hook launcher> <recorded node> <root>/<hook>`. */
function execHookTarget(command: string, args: unknown, executable: string): HookTarget | undefined {
  if (command !== "node" || !Array.isArray(args) || args.length !== 3 || !args.every(arg => typeof arg === "string")) return undefined;
  const [launcher, nodePath, script] = args as [string, string, string];
  const root = registeredRoot(script, executable);
  return root !== undefined && launcher === join(root, ENTRY_POINTS.hookLauncher) ? { nodePath, script } : undefined;
}

/**
 * Recognize only generated entries — the exec-form hook launcher or a direct or fallback shell launcher — never
 * arbitrary compound shell commands.
 */
async function isRerailHookEntry(context: Context, entry: JsonObject, executable: string): Promise<boolean> {
  const { command, args } = entry;
  if (typeof command !== "string") return false;
  const target = args === undefined ? shellHookTarget(context, command, executable) : execHookTarget(command, args, executable);
  if (target === undefined) return false;
  const { nodePath, script } = target;
  const root = registeredRoot(script, executable);
  if (!isNodeExecutable(nodePath) || !isAbsolute(script) || root === undefined) return false;
  return context.mode === "unregister" ? root === context.root : isReplaceableScript(context, script, executable);
}

/**
 * Runs the node recorded at registration, falling back to node from PATH once that binary is gone —
 * an nvm upgrade removes it and would otherwise break every hook silently. Recorded first (the order
 * the launcher shim uses) keeps the version deterministic when a project `.nvmrc` puts an older node
 * on PATH. The POSIX form execs only inside the test, so a non-zero hook exit never re-runs the hook
 * through the fallback; `exec` also leaves stdio alone, so hook input still arrives on stdin.
 */
function hookCommand(context: Context, executable: string): string {
  const node = shellArgument(context.nodePath, context.platform);
  const hook = shellArgument(join(context.root, executable), context.platform);
  if (context.platform === "win32") return `cmd /d /c "if exist ${node} (${node} ${hook}) else (node ${hook})"`;
  return `sh -c 'if [ -x "$0" ]; then exec "$0" "$1"; fi; exec node "$1"' ${node} ${hook}`;
}

/**
 * The same recorded-first, PATH-fallback order without a shell, for hosts that spawn exec-form hooks directly:
 * `node` from PATH starts the hook launcher, which hands the hook to the recorded node while that binary exists.
 */
function hookExecEntry(context: Context, executable: string): JsonObject {
  return {
    type: "command",
    command: "node",
    args: [join(context.root, ENTRY_POINTS.hookLauncher), context.nodePath, join(context.root, executable)],
  };
}


type PromptCommandHarness = "claude-code" | "cursor";

/**
 * Command files and Codex skills earlier versions installed and this one no longer does; removed when they are ours.
 * The earlier `$rerail` skill sits where the `$rerail` command skill goes, so registering replaces it. `rerail-ui` and
 * `rerail-rage-quit` were renamed `rerail-manage` and `rerail-uninstall`.
 */
const RENAMED_COMMANDS: readonly string[] = ["rerail-ui", "rerail-rage-quit"];
const RETIRED_PROMPT_COMMANDS: readonly string[] = ["rerail-session", "rerail-disable", "rerail-enable", ...RENAMED_COMMANDS];
const RETIRED_CODEX_SKILLS: readonly string[] = RENAMED_COMMANDS;

function commandContext(context: Context): CommandFileContext {
  return { cli: directCommand(context, ENTRY_POINTS.cli), packageRoot: context.root, setupLauncher: setupLauncher(context) };
}

/** The launcher set-up's answer commands start with (`Updater.launcherPath`), quoted as set-up prints it. */
function setupLauncher(context: Context): string {
  const home = process.env.RERAIL_HOME === undefined ? join(context.home, ".rerail") : resolve(process.env.RERAIL_HOME);
  return shellQuote(join(home, "bin", context.platform === "win32" ? "rerail.cmd" : "rerail"), context.platform);
}

function describe(context: Context, detail: string): string {
  return context.dryRun ? `${detail} (dry run)` : detail;
}

async function readTextFile(path: string): Promise<TextFile> {
  try {
    return { exists: true, text: await readFile(path, "utf8"), mode: (await stat(path)).mode & 0o777 };
  } catch (error) {
    if (isMissing(error)) return { exists: false, text: "", mode: 0o644 };
    throw error;
  }
}

async function readJsonFile(path: string): Promise<JsonFile> {
  const { exists, text, mode } = await readTextFile(path);
  if (!exists || text.trim() === "") return { exists, value: {}, mode };
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`Cannot parse JSON in ${path}: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (!isObject(parsed)) throw new Error(`Expected a JSON object at the top level of ${path}`);
  return { exists: true, value: parsed, mode };
}

/**
 * The file a write to `path` must land in: the end of any symlink chain, so a user-managed link
 * (dotfiles) survives and its target receives the content. A dangling chain resolves to the file
 * the last link names, which the write then creates.
 */
async function resolveWriteTarget(path: string): Promise<string> {
  try {
    return await realpath(path);
  } catch (error) {
    if (!isMissing(error)) throw error;
  }
  let target = path;
  for (let hops = 0; hops < 40; hops += 1) {
    let link: string;
    try {
      if (!(await lstat(target)).isSymbolicLink()) return target;
      link = await readlink(target);
    } catch (error) {
      if (isMissing(error)) return target;
      throw error;
    }
    target = resolve(dirname(target), link);
  }
  throw new Error(`Too many levels of symbolic links resolving ${path}`);
}

/**
 * Replaces the file behind `path` via temp file + rename beside it, first backing it up when it exists and `backup`:
 * a file Rerail generated whole (a command file or skill) is not backed up, since registration writes it again.
 */
async function writeTextFile(path: string, text: string, existing: { exists: boolean; mode: number }, backup = true): Promise<void> {
  const target = await resolveWriteTarget(path);
  const directory = dirname(target);
  await mkdir(directory, { recursive: true });
  if (existing.exists && backup) {
    const backupPath = `${target}.${new Date().toISOString().replace(/[:.]/g, "-")}.${randomUUID()}.bak`;
    await copyFile(target, backupPath, constants.COPYFILE_EXCL);
    await chmod(backupPath, 0o600);
  }
  const temp = join(directory, `.${randomUUID()}.tmp`);
  await writeFile(temp, text, { mode: existing.mode });
  await chmod(temp, existing.mode);
  if (process.platform === "win32" && existing.exists) await rm(target, { force: true });
  await rename(temp, target);
}

function planMutation(
  context: Context, harness: Harness, path: string, action: RegistrationChange["action"], detail: string, apply: () => Promise<void>,
): Plan {
  return {
    change: { harness, path, action, detail: describe(context, detail) },
    apply: context.dryRun ? undefined : apply,
  };
}

async function planJsonFile(
  harness: Harness,
  path: string,
  context: Context,
  detail: string,
  transform: (value: JsonObject) => JsonObject | Promise<JsonObject>,
): Promise<Plan> {
  assertInside(path, context.home, "home");
  const existing = await readJsonFile(path);
  const next = await transform(structuredClone(existing.value));
  if (stableJsonText(existing.value) === stableJsonText(next)) {
    const reason = context.mode === "register" ? "already in place" : "nothing to remove";
    return { change: { harness, path, action: "unchanged", detail: `${detail}: ${reason}` } };
  }
  return planMutation(context, harness, path, existing.exists ? "updated" : "created", detail,
    () => writeTextFile(path, `${JSON.stringify(next, null, 2)}\n`, existing));
}

function hooksObject(value: JsonObject, path: string): JsonObject {
  if (value.hooks === undefined) return {};
  if (!isObject(value.hooks)) throw new Error(`Cannot register rerail hooks: "hooks" in ${path} is not an object`);
  return value.hooks;
}

async function stripHookCommands(entries: unknown[], context: Context, executable: string): Promise<unknown[]> {
  const kept: unknown[] = [];
  for (const entry of entries) {
    if (!(isObject(entry) && await isRerailHookEntry(context, entry, executable))) kept.push(entry);
  }
  return kept;
}

async function stripHookGroups(groups: unknown[], context: Context, executable: string): Promise<unknown[]> {
  const kept: unknown[] = [];
  for (const group of groups) {
    if (!isObject(group) || !Array.isArray(group.hooks)) {
      kept.push(group);
      continue;
    }
    const commands = await stripHookCommands(group.hooks, context, executable);
    if (commands.length === group.hooks.length) kept.push(group);
    else if (commands.length > 0) kept.push({ ...group, hooks: commands });
  }
  return kept;
}

/** Preserve malformed and originally empty events; drop only events emptied by removing Rerail. */
async function stripHookEvents(hooks: JsonObject, context: Context, executable: string, grouped = true): Promise<JsonObject> {
  const next: JsonObject = {};
  for (const [event, entries] of Object.entries(hooks)) {
    if (!Array.isArray(entries)) {
      next[event] = entries;
      continue;
    }
    const kept = await (grouped ? stripHookGroups(entries, context, executable) : stripHookCommands(entries, context, executable));
    if (kept.length > 0 || kept.length === entries.length) next[event] = kept;
  }
  return next;
}

function addHookGroups(
  hooks: JsonObject,
  events: readonly string[],
  toolEvents: readonly string[],
  entry: JsonObject,
  path: string,
): JsonObject {
  for (const event of events) {
    const groups = hooks[event] ?? [];
    if (!Array.isArray(groups))
      throw new Error(`Cannot register rerail hooks: hooks.${event} in ${path} is not an array`);
    hooks[event] = [...groups, toolEvents.includes(event) ? { matcher: "", hooks: [entry] } : { hooks: [entry] }];
  }
  return hooks;
}

function withHooks(value: JsonObject, hooks: JsonObject): JsonObject {
  const next = { ...value };
  const keepEmpty = isObject(value.hooks) && Object.keys(value.hooks).length === 0;
  if (Object.keys(hooks).length > 0 || keepEmpty) next.hooks = hooks;
  else delete next.hooks;
  return next;
}

async function updateGroupedHooks(
  value: JsonObject, path: string, context: Context, events: readonly string[], toolEvents: readonly string[], executable: string,
  entry: JsonObject,
): Promise<JsonObject> {
  const hooks = await stripHookEvents(hooksObject(value, path), context, executable);
  if (context.mode === "register") addHookGroups(hooks, events, toolEvents, entry, path);
  return withHooks(value, hooks);
}

function planGroupedHooks(
  harness: Harness,
  path: string,
  context: Context,
  events: readonly string[],
  toolEvents: readonly string[],
  executable: string,
  label: string,
): Promise<Plan> {
  const detail =
    context.mode === "register"
      ? `register rerail hooks for ${events.length} ${label} events`
      : `remove rerail hooks from ${label} settings`;
  const entry = { type: "command", command: hookCommand(context, executable) };
  return planJsonFile(harness, path, context, detail, value => updateGroupedHooks(value, path, context, events, toolEvents, executable, entry));
}

function composedClaudeStatus(previous: ClaudeStatusLine | null | undefined, command: string): ClaudeStatusLine {
  return { ...previous, type: "command", command, refreshInterval: previous?.refreshInterval ?? 5 };
}

/**
 * `replaceable`: the predecessor is a status line of this package, or of an install that is gone, whose composition
 * this home has no record of.
 * It keeps its display settings but is not recorded, so removal never brings back a status line of another install.
 */
function registerClaudeStatus(
  context: Context, path: string, next: JsonObject, statusLine: ClaudeStatusLine | undefined,
  inherited: ClaudeStatusLine | undefined, registration: ClaudeStatusRegistration | undefined, replaceable: boolean,
): ClaudeStatusRegistration {
  const command = directCommand(context, ENTRY_POINTS.claudeStatus)
    + (path === join(context.home, ".claude", "settings.json") ? "" : ` ${shellArgument(path, context.platform)}`);
  const ownsStatus = registration !== undefined && statusLine?.command === registration.installedCommand;
  const previous = statusLine ?? inherited;
  const wrapper = previous !== undefined
    && (previous.command === command || claudeStatusScript(previous.command, context.platform) !== undefined);
  if (!ownsStatus && !replaceable && wrapper) {
    throw new Error(`Cannot preserve the existing rerail status line in ${path}: registration ownership is missing`);
  }
  const replaced = !ownsStatus && replaceable;
  const previousInherited = ownsStatus ? registration.previousInherited : !replaced && statusLine === undefined && inherited !== undefined;
  next.statusLine = composedClaudeStatus(previous, command);
  const unownedPrevious = replaced ? null : previous ?? null;
  return {
    settingsPath: path, installedRoot: context.root, installedCommand: command,
    previous: ownsStatus ? registration.previous : unownedPrevious,
    ...(previousInherited ? { previousInherited: true } : {}),
  };
}

function restoreClaudeStatus(next: JsonObject, statusLine: ClaudeStatusLine | undefined, registration: ClaudeStatusRegistration): void {
  if (statusLine?.command !== registration.installedCommand) return;
  const installedStatus = composedClaudeStatus(registration.previous, registration.installedCommand);
  if (registration.previous === null || (registration.previousInherited && stableJsonText(statusLine) === stableJsonText(installedStatus))) {
    delete next.statusLine;
    return;
  }
  const restored: ClaudeStatusLine = { ...statusLine, type: registration.previous.type, command: registration.previous.command };
  if (statusLine.refreshInterval === installedStatus.refreshInterval) {
    if (registration.previous.refreshInterval === undefined) delete restored.refreshInterval;
    else restored.refreshInterval = registration.previous.refreshInterval;
  }
  next.statusLine = restored;
}

/** Global hooks and statusLine share one preflight and atomic settings replacement. */
async function planClaudeSettings(
  context: Context,
  path = join(context.home, ".claude", "settings.json"),
  inherited?: ClaudeStatusLine,
): Promise<Plan> {
  const global = path === join(context.home, ".claude", "settings.json");
  const existing = await readJsonFile(path);
  const registration = readClaudeStatusRegistration(context.home, path);
  const statusLine = existing.value.statusLine;
  if (statusLine !== undefined && !isClaudeStatusLine(statusLine)) {
    throw new Error(`Cannot register rerail status line: "statusLine" in ${path} is not a command specification`);
  }
  if (context.mode === "register" && registration !== undefined) {
    context.packageRoots.set(registration.installedRoot, Promise.resolve(true));
  }
  // Claude Code runs shell-form hooks through Git Bash on Windows, which rewrites `cmd /d /c` into drive paths;
  // cmd then reads the hook's JSON from stdin as commands. Exec form (`args`) spawns node with no shell.
  const hookEntry = context.platform === "win32"
    ? hookExecEntry(context, ENTRY_POINTS.claudeHook)
    : { type: "command", command: hookCommand(context, ENTRY_POINTS.claudeHook) };
  const next = global
    ? await updateGroupedHooks(existing.value, path, context, CLAUDE_HOOK_EVENTS, CLAUDE_TOOL_EVENTS, ENTRY_POINTS.claudeHook, hookEntry)
    : { ...existing.value };
  let nextRegistration: ClaudeStatusRegistration | undefined = registration;
  if (context.mode === "register") {
    const replaceable = await isPackageClaudeStatus(context, statusLine ?? inherited);
    nextRegistration = registerClaudeStatus(context, path, next, statusLine, inherited, registration, replaceable);
  } else if (registration?.installedRoot === context.root) {
    restoreClaudeStatus(next, statusLine, registration);
    nextRegistration = undefined;
  }
  const settingsChanged = stableJsonText(existing.value) !== stableJsonText(next);
  const ownershipChanged = stableJsonText(registration ?? {}) !== stableJsonText(nextRegistration ?? {});
  let detail = context.mode === "register" ? "compose the project Claude Code status line" : "restore the project Claude Code status line";
  if (global) detail = context.mode === "register"
    ? `register rerail hooks for ${CLAUDE_HOOK_EVENTS.length} Claude Code events and compose the status line`
    : "remove rerail hooks and restore the Claude Code status line";
  if (!settingsChanged && !ownershipChanged) {
    return { change: { harness: "claude-code", path, action: "unchanged", detail: `${detail}: already in place` } };
  }
  return planMutation(context, "claude-code", path, existing.exists ? "updated" : "created", detail, async () => {
    const writeSettings = async (): Promise<void> => {
      if (settingsChanged) await writeTextFile(path, `${JSON.stringify(next, null, 2)}\n`, existing);
    };
    if (ownershipChanged) await writeClaudeStatusRegistration(context.home, nextRegistration, writeSettings, path);
    else await writeSettings();
  });
}

/** The script of the generated executable shape, recognized even when its old package/ownership is gone. */
function claudeStatusScript(command: string, platform: NodeJS.Platform): string | undefined {
  const words = command.match(/"(?:\\.|[^"\\])*"|[\w./:@+=-]+/gu);
  if (words === null || (words.length !== 2 && words.length !== 3) || words.join(" ") !== command) return undefined;
  const windows = platform === "win32";
  const script = unquoteShellWord(words[1]!, windows);
  return isNodeExecutable(unquoteShellWord(words[0]!, windows))
    && script.split(/[\\/]/u).at(-1) === basename(ENTRY_POINTS.claudeStatus) ? script : undefined;
}

/** A status line generated by any install of this package, or by one that is gone, identified as hooks are (`isReplaceableScript`). */
async function isPackageClaudeStatus(context: Context, statusLine: ClaudeStatusLine | undefined): Promise<boolean> {
  const script = statusLine === undefined ? undefined : claudeStatusScript(statusLine.command, context.platform);
  return script !== undefined && isAbsolute(script) && await isReplaceableScript(context, script, ENTRY_POINTS.claudeStatus);
}

const execGit = promisify(execFile);

/** Claude uses the safe main-checkout root for local settings, but shared settings stay at cwd. */
async function claudeLocalDirectory(workspaceDir: string, context: Context): Promise<string> {
  if (context.platform === "win32" || process.getuid === undefined) return workspaceDir;
  let root: string;
  try {
    // rev-parse rejects non-repositories/bare repositories; Git also enforces safe.directory.
    await execGit("git", ["rev-parse", "--show-toplevel"], { cwd: workspaceDir });
    const { stdout } = await execGit("git", ["worktree", "list", "--porcelain", "-z"], { cwd: workspaceDir });
    const first = stdout.split("\0", 1)[0];
    if (first === undefined || !first.startsWith("worktree ")) return workspaceDir;
    root = await realpath(first.slice("worktree ".length));
  } catch {
    return workspaceDir;
  }
  if (root === await realpath(context.home).catch(() => context.home)) return workspaceDir;
  const uid = process.getuid();
  for (const path of [root, join(root, ".git"), join(root, ".claude")]) {
    try {
      if ((await lstat(path)).uid !== uid || (await stat(path)).uid !== uid) return workspaceDir;
    } catch (error) {
      if (!isMissing(error)) throw error;
    }
  }
  return root;
}

async function planClaudeProjectSettings(workspace: string, context: Context): Promise<Plan[]> {
  const workspaceDir = await realpath(resolve(workspace));
  const localDir = await claudeLocalDirectory(workspaceDir, context);
  const localPaths = [...new Set([
    join(localDir, ".claude", "settings.local.json"),
    join(workspaceDir, ".claude", "settings.local.json"),
  ])];
  if (context.mode === "unregister") {
    const plans: Plan[] = [];
    for (const path of localPaths) {
      if (readClaudeStatusRegistration(context.home, path) !== undefined) plans.push(await planClaudeSettings(context, path));
    }
    return plans;
  }
  // User-local predecessors win; generated inherited wrappers resolve cwd inheritance on every invocation.
  for (const path of localPaths) {
    const existing = await readJsonFile(path);
    if (existing.value.statusLine !== undefined) return [await planClaudeSettings(context, path)];
  }
  const sharedPath = join(workspaceDir, ".claude", "settings.json");
  const shared = await readJsonFile(sharedPath);
  if (shared.value.statusLine === undefined || workspaceDir === await realpath(context.home).catch(() => context.home)) return [];
  if (!isClaudeStatusLine(shared.value.statusLine)) {
    throw new Error(`Cannot register rerail status line: "statusLine" in ${sharedPath} is not a command specification`);
  }
  return [await planClaudeSettings(context, localPaths[0]!, shared.value.statusLine)];
}

function planCursor(context: Context): Promise<Plan> {
  const path = join(context.home, ".cursor", "hooks.json");
  const command = hookCommand(context, ENTRY_POINTS.cursorHook);
  const detail =
    context.mode === "register"
      ? `register rerail hooks for ${CURSOR_HOOK_EVENTS.length} Cursor events`
      : "remove rerail hooks from Cursor hooks.json";
  return planJsonFile("cursor", path, context, detail, async (value) => {
    const hooks = await stripHookEvents(hooksObject(value, path), context, ENTRY_POINTS.cursorHook, false);
    // Removal changes only what it removes: a file without Rerail's hooks stays as it is, or absent.
    if (context.mode === "unregister") return withHooks(value, hooks);
    for (const event of CURSOR_HOOK_EVENTS) {
      const entries = hooks[event] ?? [];
      if (!Array.isArray(entries))
        throw new Error(`Cannot register rerail hooks: hooks.${event} in ${path} is not an array`);
      hooks[event] = [...entries, { command }];
    }
    return { ...value, version: typeof value.version === "number" ? value.version : 1, hooks };
  });
}

/**
 * Whether a Pi package entry is this install or, on registration, another install of this package or one that is gone
 * (`isGoneInstallRoot`). Only an absolute path, the shape setup writes, can be gone; an `npm:` or `git:` source never is.
 */
async function isPackagePiEntry(context: Context, agentDir: string, entry: string): Promise<boolean> {
  const root = resolve(agentDir, entry);
  if (context.mode === "unregister") return root === context.root;
  return await isRerailPackageRoot(context, root) || (isAbsolute(entry) && await isGoneInstallRoot(root));
}

function planPi(context: Context): Promise<Plan> {
  const agentDir = join(context.home, ".pi", "agent");
  const path = join(agentDir, "settings.json");
  const detail =
    context.mode === "register" ? `register ${context.root} in Pi packages` : "remove rerail from Pi packages";
  return planJsonFile("pi", path, context, detail, async (value) => {
    const packages = value.packages ?? [];
    if (!Array.isArray(packages))
      throw new Error(`Cannot register rerail package: "packages" in ${path} is not an array`);
    const kept: unknown[] = [];
    for (const entry of packages as unknown[]) {
      if (typeof entry === "string" && await isPackagePiEntry(context, agentDir, entry)) continue;
      kept.push(entry);
    }
    if (context.mode === "register") kept.push(context.root);
    const next = { ...value };
    if (kept.length > 0 || (Array.isArray(value.packages) && value.packages.length === 0)) next.packages = kept;
    else delete next.packages;
    return next;
  });
}

const OMP_EXTENSION_NAME = "rerail";

/**
 * Registration replaces another install of this package (identified as hooks are, by its package.json) under any
 * extension name, so one OMP session never loads two copies, and a dangling link to an install that is gone
 * (`isGoneInstallRoot`).
 */
async function planOmpPackageDuplicates(context: Context): Promise<Plan[]> {
  if (context.mode !== "register") return [];
  const directory = join(context.home, ".omp", "agent", "extensions");
  const names = await readdir(directory).catch((error: unknown) => {
    if (isMissing(error)) return [];
    throw error;
  });
  const plans: Plan[] = [];
  for (const name of names) {
    if (name === OMP_EXTENSION_NAME) continue;
    const path = join(directory, name);
    if (!(await lstat(path)).isSymbolicLink()) continue;
    let target: string;
    let gone = false;
    try {
      target = await realpath(path);
    } catch (error) {
      if (!isAbsent(error)) continue;
      target = resolve(directory, await readlink(path));
      gone = true;
    }
    if (!(await (gone ? isGoneInstallRoot(target) : isRerailPackageRoot(context, target)))) continue;
    const detail = gone ? `remove symlink to an install that is gone at ${target}` : `remove symlink to another install at ${target}`;
    plans.push(planMutation(context, "omp", path, "updated", detail, () => unlink(path)));
  }
  return plans;
}

async function planOmp(context: Context): Promise<Plan> {
  const path = join(context.home, ".omp", "agent", "extensions", OMP_EXTENSION_NAME);
  assertInside(path, context.home, "home");
  let kind: "missing" | "symlink" | "other";
  try {
    kind = (await lstat(path)).isSymbolicLink() ? "symlink" : "other";
  } catch (error) {
    if (!isMissing(error)) throw error;
    kind = "missing";
  }
  const canonicalRoot = await realpath(context.root).catch(() => context.root);
  let target: string | undefined;
  if (kind === "symlink") {
    try {
      target = await realpath(path);
    } catch {
      target = resolve(dirname(path), await readlink(path));
    }
  }
  const unchanged = (detail: string): Plan => ({ change: { harness: "omp", path, action: "unchanged", detail } });

  if (context.mode === "register") {
    if (kind === "other") throw new Error(`Refusing to replace ${path}: it exists and is not a symlink`);
    if (target === canonicalRoot) return unchanged(`symlink to ${context.root}: already in place`);
    const detail =
      kind === "symlink" ? `repoint symlink from ${target} to ${context.root}` : `symlink to ${context.root}`;
    return planMutation(context, "omp", path, kind === "symlink" ? "updated" : "created", detail, async () => {
      await mkdir(dirname(path), { recursive: true });
      if (kind === "symlink") await unlink(path);
      await symlink(context.root, path, context.platform === "win32" ? "junction" : undefined);
    });
  }

  if (kind === "missing") return unchanged("no symlink: nothing to remove");
  if (kind === "other") return unchanged("not a symlink: left in place");
  if (target !== canonicalRoot) return unchanged(`symlink points at ${target}: left in place`);
  return planMutation(context, "omp", path, "updated", `remove symlink to ${context.root}`, () => unlink(path));
}

/** The immutable stdio bridge selects the checkout-local MCP endpoint from its launch cwd. */
export const RERAIL_MCP_SERVER_NAME = "rerail";

/**
 * A bridge entry another install of this package wrote, or one that is gone (`isReplaceableScript`), under any server
 * name. Only the shape setup writes counts, `<node> <root>/dist/cli/cli.js mcp-bridge` (`ENTRY_POINTS.cli`), so a gone
 * entry of another tool (an `npx` command, or a script outside `dist/cli/`) stays.
 */
async function isPackageMcpBridge(context: Context, server: unknown): Promise<boolean> {
  if (!isObject(server) || typeof server.command !== "string" || !Array.isArray(server.args) || server.args.length !== 2) return false;
  const [script, command] = server.args as unknown[];
  if (command !== "mcp-bridge" || typeof script !== "string" || !isAbsolute(script) || !isNodeExecutable(server.command)) return false;
  return await isReplaceableScript(context, script, ENTRY_POINTS.cli);
}

function planOmpMcp(context: Context): Promise<Plan> {
  const path = join(context.home, ".omp", "agent", "mcp.json");
  const detail =
    context.mode === "register"
      ? `register the ${RERAIL_MCP_SERVER_NAME} MCP bridge`
      : `remove the ${RERAIL_MCP_SERVER_NAME} MCP bridge`;
  return planJsonFile("omp", path, context, detail, async (value) => {
    const servers = value.mcpServers === undefined ? {} : value.mcpServers;
    if (!isObject(servers))
      throw new Error(`Cannot register rerail MCP server: "mcpServers" in ${path} is not an object`);
    const { [RERAIL_MCP_SERVER_NAME]: _previous, ...others } = servers;
    const kept: JsonObject = {};
    for (const [name, server] of Object.entries(others)) {
      if (context.mode === "register" && await isPackageMcpBridge(context, server)) continue;
      kept[name] = server;
    }
    const next: JsonObject =
      context.mode === "register"
        ? {
          ...kept,
          [RERAIL_MCP_SERVER_NAME]: {
            command: context.nodePath,
            args: [join(context.root, ENTRY_POINTS.cli), "mcp-bridge"],
          },
        }
        : kept;
    if (Object.keys(next).length === 0 && !isObject(value.mcpServers)) return value;
    return { ...value, mcpServers: next };
  });
}

function codexSkillPath(context: Context, name: string): string {
  const path = join(context.home, ".codex", "skills", name, "SKILL.md");
  assertInside(path, context.home, "home");
  return path;
}

/** Removes a Codex skill only when Rerail wrote it (`runsPackageCli`); a skill someone else owns stays. */
async function planCodexSkillRemoval(context: Context, name: string, path: string, existing: TextFile): Promise<Plan> {
  if (!existing.text.includes(MANAGED_SKILL_MARKER) || !(await runsPackageCli(context, existing.text))) {
    return { change: { harness: "codex", path, action: "unchanged", detail: `no $${name} skill from this Rerail installation to remove` } };
  }
  return planMutation(context, "codex", path, "updated", `remove the $${name} skill`, () => unlink(path));
}

async function planCodexSkill(context: Context, name: RerailCommandName): Promise<Plan> {
  const path = codexSkillPath(context, name);
  const existing = await readTextFile(path);
  if (existing.exists && !existing.text.includes(MANAGED_SKILL_MARKER)) {
    return { change: { harness: "codex", path, action: "unchanged", detail: `user-owned $${name} skill: left in place` } };
  }
  if (context.mode === "unregister") return planCodexSkillRemoval(context, name, path, existing);
  const text = commandFileText("codex", name, commandContext(context));
  if (existing.text === text) return { change: { harness: "codex", path, action: "unchanged", detail: `$${name} skill: already in place` } };
  // Only a skill carrying Rerail's marker gets here: Rerail generated it, so it is not backed up.
  return planMutation(context, "codex", path, existing.exists ? "updated" : "created", `register the $${name} skill`,
    () => writeTextFile(path, text, existing, false));
}

function promptCommandPath(context: Context, harness: PromptCommandHarness, name: string): string {
  const path = join(context.home, CONFIG_DIRS[harness], "commands", `${name}.md`);
  assertInside(path, context.home, "home");
  return path;
}

/**
 * Whether a command file or skill runs this installation's CLI or, on registration, that of another install of this
 * package or of one that is gone, recognized as hooks are (`isReplaceableScript`): an earlier version wrote its files
 * under its own install root. A generated file names exactly one `<root>/dist/cli/cli.js` (`ENTRY_POINTS.cli`).
 */
async function runsPackageCli(context: Context, text: string): Promise<boolean> {
  if (text.includes(join(context.root, ENTRY_POINTS.cli))) return true;
  if (context.mode === "unregister") return false;
  const windows = context.platform === "win32";
  const scripts = new Set(shellWords(text, windows).map(word => unquoteShellWord(word, windows))
    .filter(word => isAbsolute(word) && registeredRoot(word, ENTRY_POINTS.cli) !== undefined));
  const [script] = scripts;
  return scripts.size === 1 && await isReplaceableScript(context, script!, ENTRY_POINTS.cli);
}

/** Removes a prompt command only when it runs this package's CLI (`runsPackageCli`); anyone else's file is left in place. */
async function planPromptCommandRemoval(
  context: Context, harness: PromptCommandHarness, name: string, path: string, existing: TextFile,
): Promise<Plan> {
  if (!existing.exists) return { change: { harness, path, action: "unchanged", detail: `no /${name} command: nothing to remove` } };
  if (!(await runsPackageCli(context, existing.text)))
    return { change: { harness, path, action: "unchanged", detail: `/${name} command is not Rerail's: left in place` } };
  return planMutation(context, harness, path, "updated", `remove the /${name} command`, () => unlink(path));
}

/** Prompt commands share root ownership checks, atomic installation, and conservative removal. */
async function planPromptCommand(context: Context, harness: PromptCommandHarness, name: RerailCommandName): Promise<Plan> {
  const path = promptCommandPath(context, harness, name);
  const existing = await readTextFile(path);
  if (context.mode === "unregister") return planPromptCommandRemoval(context, harness, name, path, existing);
  const text = commandFileText(harness, name, commandContext(context));
  if (existing.exists && existing.text === text) return { change: { harness, path, action: "unchanged", detail: `/${name} command: already in place` } };
  // A command file of this package (`runsPackageCli`) is generated whole; anyone else's is backed up before it goes.
  const backup = existing.exists && !(await runsPackageCli(context, existing.text));
  return planMutation(context, harness, path, existing.exists ? "updated" : "created", `register the /${name} command`,
    () => writeTextFile(path, text, existing, backup));
}

/** The Rerail commands of one coding agent, plus removal of the ones earlier versions installed. */
async function planAgentCommands(context: Context, harness: PromptCommandHarness | "codex"): Promise<Plan[]> {
  const plans: Plan[] = [];
  for (const { name } of RERAIL_COMMANDS) {
    plans.push(harness === "codex" ? await planCodexSkill(context, name) : await planPromptCommand(context, harness, name));
  }
  // A retired file of this installation goes in either mode, and on registration one of another install of this package
  // or of one that is gone; absent or foreign files produce no plan at all.
  for (const name of harness === "codex" ? RETIRED_CODEX_SKILLS : RETIRED_PROMPT_COMMANDS) {
    const path = harness === "codex" ? codexSkillPath(context, name) : promptCommandPath(context, harness, name);
    const existing = await readTextFile(path);
    if (!existing.exists) continue;
    const plan = harness === "codex"
      ? await planCodexSkillRemoval(context, name, path, existing)
      : await planPromptCommandRemoval(context, harness, name, path, existing);
    if (plan.change.action !== "unchanged") plans.push(plan);
  }
  return plans;
}

function planHarness(harness: Harness, context: Context): Promise<Plan> {
  switch (harness) {
    case "omp":
      return planOmp(context);
    case "pi":
      return planPi(context);
    case "claude-code":
      return planClaudeSettings(context);
    case "codex":
      return planGroupedHooks(
        harness,
        join(context.home, ".codex", "hooks.json"),
        context,
        CODEX_HOOK_EVENTS,
        CODEX_TOOL_EVENTS,
        ENTRY_POINTS.codexHook,
        "Codex",
      );
    case "cursor":
      return planCursor(context);
  }
}

/** Harnesses whose user config directory exists under `home`. */
export function detectHarnesses(home: string = homedir()): Harness[] {
  const base = resolve(home);
  return HARNESSES.filter((harness) => existsSync(join(base, CONFIG_DIRS[harness])));
}

async function run(options: RegistrationOptions, mode: Mode): Promise<RegistrationReport> {
  const home = resolve(options.home ?? homedir());
  const root = resolve(options.root);
  const context: Context = {
    mode,
    root,
    packageRoots: new Map(),
    home,
    nodePath: options.nodePath ?? process.execPath,
    dryRun: options.dryRun === true,
    platform: options.platform ?? process.platform,
  };
  const selected = new Set(options.harnesses ?? detectHarnesses(home));
  const plans: Plan[] = [];

  // Plan every harness before touching anything so a malformed config aborts with nothing written.
  for (const harness of HARNESSES) {
    if (!selected.has(harness)) {
      plans.push({
        change: {
          harness,
          path: join(home, CONFIG_DIRS[harness]),
          action: "skipped",
          detail: options.harnesses === undefined ? "config directory not found" : "not selected",
        },
      });
      continue;
    }
    plans.push(await planHarness(harness, context));
    if (harness === "omp") plans.push(await planOmpMcp(context), ...await planOmpPackageDuplicates(context));
    if (harness === "claude-code" && options.workspaceDir !== undefined) {
      plans.push(...await planClaudeProjectSettings(options.workspaceDir, context));
    }
    if (harness === "claude-code") {
      for (const path of readClaudeStatusSettingsPaths(home)) {
        if (plans.some(plan => plan.change.path === path)) continue;
        const registration = readClaudeStatusRegistration(home, path);
        const statusLine = (await readJsonFile(path)).value.statusLine;
        if (mode === "unregister" || (isClaudeStatusLine(statusLine) && statusLine.command === registration?.installedCommand)) {
          plans.push(await planClaudeSettings(context, path));
        }
      }
    }
    if (harness === "codex" || harness === "claude-code" || harness === "cursor") plans.push(...await planAgentCommands(context, harness));
  }
  for (const plan of plans) await plan.apply?.();
  const changes = plans.map((plan) => plan.change);

  const notes: string[] = [];
  if (changes.some((change) => change.action === "created" || change.action === "updated")) {
    notes.push("Harness sessions pick up registration changes when they next start.");
  }
  // Codex asks to trust a hook again only when its command changes.
  const codexHooks = join(home, ".codex", "hooks.json");
  if (mode === "register" && changes.some(change => change.path === codexHooks && (change.action === "created" || change.action === "updated"))) {
    notes.push("Codex only runs trusted hooks: open Codex and run /hooks to trust the rerail hook.");
  }
  return { changes, notes };
}

export function registerHarnesses(options: RegistrationOptions): Promise<RegistrationReport> {
  return run(options, "register");
}

export function unregisterHarnesses(options: RegistrationOptions): Promise<RegistrationReport> {
  return run(options, "unregister");
}
