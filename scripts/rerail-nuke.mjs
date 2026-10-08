#!/usr/bin/env node
// `just nuke-rerail`: remove a Rerail install (the npx `rerail setup` install and the `just rerail-dev --install-bridge`
// dev bridge) from this machine, moving everything it owned into a dated backup that `--restore` reads back, and stop
// every running `just rerail-dev` container (their volumes stay, so `just rerail-dev` brings them back).
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, renameSync, rmSync, statSync, unlinkSync, utimesSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";

import { BACKUP_PREFIX, BACKUP_RERAIL_HOME, installedStopCommand, loadSqlite, readDaemonState, stopDaemon, rerailHomeFrom, timestamp } from "./rerail-restore.mjs";
import { DEFAULT_REGISTRY, removeManagedNpmrc, validateRegistry } from "./rerail-setup.mjs";

// The retired rerail-disable.md / rerail-enable.md stay listed so older installs are cleaned up too.
export const CLAUDE_COMMANDS = [
  "rerail.md", "rerail-start.md", "rerail-stop.md", "rerail-disable.md", "rerail-enable.md", "rerail-ui.md", "rerail-session.md",
];
export const CURSOR_COMMANDS = ["rerail-start.md", "rerail-stop.md", "rerail-disable.md", "rerail-enable.md", "rerail-ui.md"];
export const CLAUDE_PLUGIN = "rerail@rerail-dev";
export const CLAUDE_MARKETPLACE = "rerail-dev";
const USAGE = "usage: just nuke-rerail [--dry-run] [--yes]";

class NukeError extends Error {}

export function parseNukeArguments(argv) {
  const options = { dryRun: false, yes: false };
  for (const argument of argv) {
    if (argument === "--dry-run") options.dryRun = true;
    else if (argument === "--yes" || argument === "-y") options.yes = true;
    else throw new NukeError(`${USAGE} (unexpected: ${argument})`);
  }
  return options;
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function pathExists(path) {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

/** `~/.rerail-clean-YYYYMMDD` in local time, or `-YYYYMMDD-HHMMSS` when a backup from today already exists. */
export function backupRootFor(home, now = new Date(), exists = pathExists) {
  const stamp = timestamp(now);
  const daily = join(home, `${BACKUP_PREFIX}${stamp.slice(0, 8)}`);
  return exists(daily) ? join(home, `${BACKUP_PREFIX}${stamp}`) : daily;
}

/** A config's backup name: its path relative to HOME (absolute when outside it) with every separator made `_`. */
export function configBackupName(path, home) {
  const inside = relative(home, path);
  const name = inside !== "" && !inside.startsWith("..") && !isAbsolute(inside) ? inside : path.replace(/^[/\\]+/u, "");
  return name.replaceAll(/[/\\]/gu, "_");
}

/**
 * A predicate for "this string points into the Rerail home": the resolved path, its realpath, and the `~`/`$HOME`
 * spellings when the home sits under HOME. Covers `<home>/current/...`, `<home>/bridges/<hash>/...` and the rest.
 */
export function rerailHomeMatcher(rerailHome, home) {
  const roots = new Set([resolve(rerailHome)]);
  try {
    roots.add(realpathSync(rerailHome));
  } catch {
    // A missing home has no other spelling.
  }
  const needles = [];
  for (const root of roots) {
    needles.push(`${root}${sep}`);
    const inside = relative(home, root);
    if (inside !== "" && !inside.startsWith("..") && !isAbsolute(inside)) {
      for (const prefix of ["~", "$HOME", "${HOME}"]) needles.push(`${prefix}/${inside.split(sep).join("/")}/`);
    }
  }
  return text => typeof text === "string" && ([...roots].includes(text) || needles.some(needle => text.includes(needle)));
}

function referencesDeep(value, matches) {
  if (typeof value === "string") return matches(value);
  if (Array.isArray(value)) return value.some(item => referencesDeep(item, matches));
  if (isObject(value)) return Object.values(value).some(item => referencesDeep(item, matches));
  return false;
}

function isRerailCommand(entry, matches) {
  return isObject(entry) && typeof entry.command === "string" && matches(entry.command);
}

/**
 * Remove every hook command that points into the Rerail home. Grouped entries (`{matcher, hooks: [...]}`, Claude and
 * Codex) lose only those commands and disappear once emptied; flat entries (`{command}`, Cursor) go whole. An event
 * emptied by the removal is dropped; one that was already empty or malformed is left alone.
 */
export function stripHookCommands(hooks, matches) {
  if (!isObject(hooks)) return { hooks, removed: 0 };
  const next = {};
  let removed = 0;
  for (const [event, entries] of Object.entries(hooks)) {
    if (!Array.isArray(entries)) {
      next[event] = entries;
      continue;
    }
    const result = stripHookEvent(entries, matches);
    removed += result.removed;
    if (result.kept.length > 0 || entries.length === 0) next[event] = result.kept;
  }
  return { hooks: next, removed };
}

/** One event's entries without their Rerail commands, and how many commands went. */
function stripHookEvent(entries, matches) {
  const kept = [];
  let removed = 0;
  for (const entry of entries) {
    const result = stripHookEntry(entry, matches);
    removed += result.removed;
    kept.push(...result.kept);
  }
  return { kept, removed };
}

/** A grouped entry loses its Rerail commands (and goes once emptied); a flat Rerail entry goes whole. */
function stripHookEntry(entry, matches) {
  if (isObject(entry) && Array.isArray(entry.hooks)) {
    const commands = entry.hooks.filter(command => !isRerailCommand(command, matches));
    const removed = entry.hooks.length - commands.length;
    if (removed === 0) return { kept: [entry], removed };
    return { kept: commands.length > 0 ? [{ ...entry, hooks: commands }] : [], removed };
  }
  if (isRerailCommand(entry, matches)) return { kept: [], removed: 1 };
  return { kept: [entry], removed: 0 };
}

function withStrippedHooks(value, matches, removed) {
  if (!isObject(value.hooks)) return value;
  const result = stripHookCommands(value.hooks, matches);
  if (result.removed === 0) return value;
  removed.push(`${result.removed} hook command(s)`);
  const next = { ...value };
  if (Object.keys(result.hooks).length > 0) next.hooks = result.hooks;
  else delete next.hooks;
  return next;
}

function withoutKey(value, parent, key, removed, label) {
  if (!isObject(value[parent]) || !(key in value[parent])) return value;
  const rest = { ...value[parent] };
  delete rest[key];
  removed.push(label);
  const next = { ...value };
  if (Object.keys(rest).length > 0) next[parent] = rest;
  else delete next[parent];
  return next;
}

/** `previous_json` holds the pre-Rerail status line, null, or `{previousInherited: true, previous}`. */
export function parseStatusRegistration(previousJson) {
  let parsed;
  try {
    parsed = JSON.parse(previousJson);
  } catch {
    return { previous: null };
  }
  if (isObject(parsed) && parsed.previousInherited === true && "previous" in parsed) return { previous: parsed.previous, inherited: true };
  return { previous: isObject(parsed) && typeof parsed.command === "string" ? parsed : null };
}

/** Status-line registrations Rerail recorded in `<rerail home>/harness.sqlite`, keyed by settings path. */
export async function readStatusRegistrations(rerailHome) {
  const path = join(rerailHome, "harness.sqlite");
  const registrations = new Map();
  if (!existsSync(path)) return registrations;
  const { DatabaseSync } = await loadSqlite();
  const database = new DatabaseSync(path, { readOnly: true });
  try {
    const table = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'harness_status_lines'").get();
    if (table === undefined) return registrations;
    for (const row of database.prepare("SELECT settings_path AS settingsPath, previous_json AS previousJson FROM harness_status_lines").all()) {
      registrations.set(String(row.settingsPath), parseStatusRegistration(String(row.previousJson)));
    }
  } finally {
    database.close();
  }
  return registrations;
}

/**
 * Claude settings: Rerail hooks go, a Rerail status line reverts to the one recorded before registration (or goes when
 * none was recorded or it was inherited from shared settings), and the rerail-dev plugin and marketplace are dropped.
 */
export function stripClaudeSettings(value, { matches, registration }) {
  const removed = [];
  let next = withStrippedHooks(value, matches, removed);
  const status = next.statusLine;
  if (isObject(status) && matches(status.command)) {
    next = { ...next };
    if (registration !== undefined && registration.inherited !== true && registration.previous !== null) {
      next.statusLine = registration.previous;
      removed.push("Rerail status line (restored the previous one)");
    } else {
      delete next.statusLine;
      removed.push("Rerail status line");
    }
  }
  next = withoutKey(next, "enabledPlugins", CLAUDE_PLUGIN, removed, `plugin ${CLAUDE_PLUGIN}`);
  next = withoutKey(next, "extraKnownMarketplaces", CLAUDE_MARKETPLACE, removed, `marketplace ${CLAUDE_MARKETPLACE}`);
  return { value: next, removed };
}

/** Cursor (flat `{command}` entries) and Codex (grouped) hooks.json. */
export function stripHooksFile(value, { matches }) {
  const removed = [];
  return { value: withStrippedHooks(value, matches, removed), removed };
}

export function stripOmpMcp(value, { matches }) {
  if (!isObject(value.mcpServers) || !referencesDeep(value.mcpServers.rerail, matches)) return { value, removed: [] };
  const servers = { ...value.mcpServers };
  delete servers.rerail;
  return { value: { ...value, mcpServers: servers }, removed: ["MCP server rerail"] };
}

export function stripPiSettings(value, { matches, agentDir }) {
  if (!Array.isArray(value.packages)) return { value, removed: [] };
  const kept = value.packages.filter(entry =>
    !(typeof entry === "string" ? matches(resolve(agentDir, entry)) || matches(entry) : referencesDeep(entry, matches)));
  const dropped = value.packages.length - kept.length;
  if (dropped === 0) return { value, removed: [] };
  const next = { ...value };
  if (kept.length > 0) next.packages = kept;
  else delete next.packages;
  return { value: next, removed: [`${dropped} package(s)`] };
}

function readJson(path) {
  try {
    const value = JSON.parse(readFileSync(path, "utf8"));
    return isObject(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

/** Write through a sibling temp file and rename, keeping the file's mode. */
export function writeJsonAtomic(path, value) {
  const mode = statSync(path).mode & 0o777;
  const pending = join(dirname(path), `.${path.split(sep).at(-1)}.nuke-rerail-${process.pid}`);
  writeFileSync(pending, `${JSON.stringify(value, null, 2)}\n`, { mode });
  renameSync(pending, path);
}

function writeTextAtomic(path, text) {
  const mode = statSync(path).mode & 0o777;
  const pending = join(dirname(path), `.${path.split(sep).at(-1)}.nuke-rerail-${process.pid}`);
  writeFileSync(pending, text, { mode });
  renameSync(pending, path);
}

/** `cp -p`: contents, mode, and timestamps. */
function copyPreserving(source, target) {
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const original = statSync(source);
  copyFileSync(source, target);
  chmodSync(target, original.mode & 0o777);
  utimesSync(target, original.atime, original.mtime);
}

/** rename, falling back to copy-then-remove across filesystems. */
function move(source, target) {
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  try {
    renameSync(source, target);
  } catch (error) {
    if (error?.code !== "EXDEV") throw error;
    cpSync(source, target, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
    rmSync(source, { recursive: true, force: true });
  }
}

/**
 * The Docker CLI as `args => {status, stdout}`, never through a shell. `RERAIL_NUKE_DOCKER` names the binary; set to
 * the empty string it disables Docker, so sandboxed runs cannot touch this machine's containers.
 */
export function dockerRunner(env) {
  const binary = env.RERAIL_NUKE_DOCKER ?? "docker";
  if (binary === "") return () => ({ status: 1, stdout: "" });
  return args => {
    const result = spawnSync(binary, args, { encoding: "utf8", env });
    return { status: result.error === undefined ? result.status : null, stdout: result.stdout ?? "" };
  };
}

const CONTAINER_FORMAT = '{{.Names}}\t{{.State}}\t{{.Label "com.docker.compose.project"}}\t{{.Label "com.docker.compose.service"}}';

/** Running `just rerail-dev` containers from every checkout: Compose project `nera_*`, service `rerail` or `rerail-init`. */
export function runningRerailContainers(docker) {
  let result;
  try {
    result = docker(["ps", "--all", "--filter", "label=com.docker.compose.project", "--format", CONTAINER_FORMAT]);
  } catch {
    return [];
  }
  if (result.status !== 0) return [];
  const containers = [];
  for (const line of result.stdout.split("\n")) {
    const [name, state, project, service] = line.trim().split("\t");
    if (!name || state !== "running" || !project?.startsWith("nera_") || (service !== "rerail" && service !== "rerail-init")) continue;
    containers.push({ name, project });
  }
  return containers;
}

function npmrcRegistries(env) {
  const registries = [DEFAULT_REGISTRY];
  if (env.RERAIL_REGISTRY) {
    const checked = validateRegistry(env.RERAIL_REGISTRY);
    if ("registry" in checked && checked.registry !== DEFAULT_REGISTRY) registries.push(checked.registry);
  }
  return registries;
}

/** Every host config Rerail edits, including project Claude settings recorded in harness.sqlite. */
function configTargets(home, registrations) {
  const claude = new Set([join(home, ".claude", "settings.json"), ...registrations.keys()]);
  const agentDir = join(home, ".pi", "agent");
  return [
    ...[...claude].map(path => ({ path, strip: (value, context) => stripClaudeSettings(value, { ...context, registration: registrations.get(path) }) })),
    { path: join(home, ".cursor", "hooks.json"), strip: stripHooksFile },
    { path: join(home, ".codex", "hooks.json"), strip: stripHooksFile },
    { path: join(home, ".omp", "agent", "mcp.json"), strip: stripOmpMcp },
    { path: join(agentDir, "settings.json"), strip: (value, context) => stripPiSettings(value, { ...context, agentDir }) },
  ];
}

/** Stopping the daemon that owns `rerailHome`, when one is installed or recorded. */
function daemonSteps(rerailHome, env) {
  const state = readDaemonState(rerailHome);
  if (installedStopCommand(rerailHome) === undefined && state === undefined) return [];
  const pid = Number.isSafeInteger(state?.pid) ? ` (pid ${state.pid})` : "";
  return [{
    kind: "daemon",
    description: `Stop the Rerail daemon${pid}`,
    run: async log => {
      const how = await stopDaemon(rerailHome, { env, log });
      log(how === "not-running" ? "No daemon was running." : "Daemon stopped.");
    },
  }];
}

function containerSteps(docker) {
  return runningRerailContainers(docker).map(container => ({
    kind: "container",
    description: `Stop Rerail container ${container.name} (project ${container.project}; volumes kept)`,
    run: () => {
      const result = docker(["stop", container.name]);
      if (result.status !== 0) throw new NukeError(`docker stop ${container.name} failed`);
    },
  }));
}

/** Back up every existing host config, and edit those that hold Rerail entries. */
function configSteps({ home, backupRoot, registrations, matches }) {
  const steps = [];
  for (const target of configTargets(home, registrations)) {
    if (!existsSync(target.path)) continue;
    const value = readJson(target.path);
    const backup = join(backupRoot, "configs", configBackupName(target.path, home));
    const result = value === undefined ? { value, removed: [] } : target.strip(value, { matches });
    const edit = result.removed.length > 0;
    steps.push({
      kind: edit ? "config" : "config-backup",
      description: edit
        ? `Edit ${target.path}: remove ${result.removed.join(", ")} (backup: ${backup})`
        : `Back up ${target.path} (no Rerail entries)`,
      run: () => {
        copyPreserving(target.path, backup);
        if (edit) writeJsonAtomic(target.path, result.value);
      },
    });
  }
  return steps;
}

/** Host commands and skill into the backup, the OMP extension symlink away, then the Rerail home itself. */
function fileSteps({ home, rerailHome, backupRoot, matches }) {
  const moves = [
    ...CLAUDE_COMMANDS.map(name => [join(home, ".claude", "commands", name), join(backupRoot, "claude-commands", name)]),
    ...CURSOR_COMMANDS.map(name => [join(home, ".cursor", "commands", name), join(backupRoot, "cursor-commands", name)]),
    [join(home, ".codex", "skills", "rerail"), join(backupRoot, "codex-skill-rerail")],
  ];
  const steps = moves
    .filter(([source]) => pathExists(source))
    .map(([source, target]) => ({ kind: "move", description: `Move ${source} -> ${target}`, run: () => move(source, target) }));
  steps.push(...ompExtensionSteps(home, matches));
  if (pathExists(rerailHome)) {
    const target = join(backupRoot, BACKUP_RERAIL_HOME);
    steps.push({ kind: "move", description: `Move ${rerailHome} -> ${target}`, run: () => move(rerailHome, target) });
  }
  return steps;
}

function ompExtensionSteps(home, matches) {
  const ompExtension = join(home, ".omp", "agent", "extensions", "rerail");
  if (!pathExists(ompExtension) || !lstatSync(ompExtension).isSymbolicLink()) return [];
  const link = readlinkSync(ompExtension);
  if (!matches(resolve(dirname(ompExtension), link))) return [];
  return [{ kind: "remove", description: `Remove symlink ${ompExtension} -> ${link}`, run: () => unlinkSync(ompExtension) }];
}

function npmrcSteps(env, home) {
  const npmrc = env.NPM_CONFIG_USERCONFIG || join(home, ".npmrc");
  if (!existsSync(npmrc)) return [];
  const text = readFileSync(npmrc, "utf8");
  const next = removeManagedNpmrc(text, npmrcRegistries(env));
  if (next === text) return [];
  return [{
    kind: "npmrc",
    description: `Remove the @neralabs scope mapping and managed credential from ${npmrc} (not backed up)`,
    run: () => writeTextAtomic(npmrc, next),
  }];
}

/**
 * Everything `nuke-rerail` would do, computed without changing anything. Each step has a description and a `run`
 * that performs it against `backupRoot`.
 */
export async function planNuke({ env = process.env, now = new Date(), docker = dockerRunner(env) } = {}) {
  const home = env.HOME ?? homedir();
  const rerailHome = rerailHomeFrom(env, home);
  const matches = rerailHomeMatcher(rerailHome, home);
  const backupRoot = backupRootFor(home, now);
  const registrations = await readStatusRegistrations(rerailHome);
  const steps = [
    ...daemonSteps(rerailHome, env),
    ...containerSteps(docker),
    ...configSteps({ home, backupRoot, registrations, matches }),
    ...fileSteps({ home, rerailHome, backupRoot, matches }),
    ...npmrcSteps(env, home),
  ];
  // Untouched configs are backed up only alongside a real removal, so a clean machine stays a no-op.
  const removes = steps.some(step => step.kind !== "config-backup");
  return { home, rerailHome, backupRoot, steps: removes ? steps : [] };
}

export function needsBackup(plan) {
  return plan.steps.some(step => !["daemon", "container", "npmrc", "remove"].includes(step.kind));
}

export async function executeNuke(plan, log = console.log) {
  if (needsBackup(plan)) mkdirSync(plan.backupRoot, { recursive: true, mode: 0o700 });
  for (const step of plan.steps) {
    log(`- ${step.description}`);
    await step.run(log); // NOSONAR: steps run in order; the daemon must stop before its home moves
  }
}

function printPlan(plan, log) {
  log(`Rerail home: ${plan.rerailHome}`);
  if (needsBackup(plan)) log(`Backup: ${plan.backupRoot}`);
  for (const step of plan.steps) log(`  - ${step.description}`);
}

async function confirm(question) {
  const prompt = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return /^y(es)?$/iu.test((await prompt.question(question)).trim());
  } finally {
    prompt.close();
  }
}

export async function main(argv, { env = process.env, log = console.log, interactive = Boolean(process.stdin.isTTY && process.stdout.isTTY), docker = dockerRunner(env) } = {}) {
  const options = parseNukeArguments(argv);
  const plan = await planNuke({ env, docker });
  if (plan.steps.length === 0) {
    log(`Nothing to remove: no Rerail install found for ${plan.home} and no Rerail container running.`);
    return 0;
  }
  log(options.dryRun ? "Dry run; nothing will change. just nuke-rerail would:" : "just nuke-rerail will:");
  printPlan(plan, log);
  if (options.dryRun) return 0;
  if (!options.yes) {
    if (!interactive) throw new NukeError("Refusing to remove Rerail without confirmation: re-run with --yes, or in an interactive terminal.");
    if (!(await confirm("Proceed? [y/N] "))) {
      log("Cancelled; nothing changed.");
      return 1;
    }
  }
  await executeNuke(plan, log);
  log("");
  log("Rerail removed.");
  if (needsBackup(plan)) log(`Backup: ${plan.backupRoot}`);
  log("Restore its databases after reinstalling with: just rerail --restore   (or just rerail-dev --restore)");
  if (plan.steps.some(step => step.kind === "container")) log("Stopped Rerail containers keep their volumes; just rerail-dev in a checkout starts its container again.");
  log("Restart open Claude/Cursor/Codex/OMP sessions; their loaded hooks now point at missing files.");
  return 0;
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof NukeError)) throw error;
    process.stderr.write(`nuke-rerail: ${error.message}\n`);
    process.exitCode = 1;
  }
}
