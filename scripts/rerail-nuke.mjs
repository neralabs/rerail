#!/usr/bin/env node
// `just nuke-rerail`: remove Rerail (the npm `rerail setup` install and a checkout's `just rerail-dev --install-bridge`
// dev bridge) from this machine, moving everything it owned into a dated backup that `--restore` reads back, and stop
// every registered checkout's dev daemon (their state stays, so `just rerail-dev` brings them back).
import { chmodSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, renameSync, rmSync, statSync, unlinkSync, utimesSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";

import { BACKUP_PREFIX, BACKUP_RERAIL_HOME, launcherPath, loadSqlite, processAlive, readDaemonState, rerailHomeFrom, runLauncher, stopDaemon, timestamp } from "./rerail-restore.mjs";

/** The coding agents `rerail install` registers with, and the config directory whose presence selects each. */
export const HARNESS_CONFIG_DIRS = { omp: join(".omp", "agent"), pi: join(".pi", "agent"), "claude-code": ".claude", codex: ".codex", cursor: ".cursor" };
/** The Rerail command set (`src/rerail-commands.ts`): command files for Claude Code and Cursor, skills for Codex. */
export const RERAIL_COMMAND_NAMES = ["rerail", "rerail-ui", "rerail-create", "rerail-start", "rerail-stop", "rerail-update", "rerail-rage-quit"];
/** Plus `/rerail-session`, which an earlier version wrote. */
export const CLAUDE_COMMANDS = [...RERAIL_COMMAND_NAMES.map(name => `${name}.md`), "rerail-session.md"];
export const CURSOR_COMMANDS = RERAIL_COMMAND_NAMES.map(name => `${name}.md`);
export const CLAUDE_PLUGIN = "rerail@rerail-dev";
export const CLAUDE_MARKETPLACE = "rerail-dev";
export const MCP_SERVER = "rerail";
const USAGE = "usage: just nuke-rerail [--dry-run] [--yes]";

export class NukeError extends Error {}

export function parseNukeArguments(argv) {
  const options = { dryRun: false, yes: false, help: false };
  for (const argument of argv) {
    if (argument === "--dry-run") options.dryRun = true;
    else if (argument === "--yes" || argument === "-y") options.yes = true;
    else if (argument === "--help" || argument === "-h") options.help = true;
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
 * spellings when the home sits under HOME. Covers `<home>/current/...` (the npm install and the dev bridge alike).
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
  return isObject(entry) && (
    (typeof entry.command === "string" && matches(entry.command)) || (Array.isArray(entry.args) && entry.args.some(arg => matches(arg))));
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
    const kept = [];
    for (const entry of entries) {
      const result = stripHookEntry(entry, matches);
      removed += result.removed;
      kept.push(...result.kept);
    }
    if (kept.length > 0 || entries.length === 0) next[event] = kept;
  }
  return { hooks: next, removed };
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

/** Where `rerail install` records Claude status-line registrations: always `~/.rerail/harness.sqlite`. */
export function harnessDatabasePath(home) {
  return join(home, ".rerail", "harness.sqlite");
}

/** Status-line registrations Rerail recorded in harness.sqlite, keyed by settings path. */
export async function readStatusRegistrations(path) {
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
  if (!isObject(value.mcpServers) || !referencesDeep(value.mcpServers[MCP_SERVER], matches)) return { value, removed: [] };
  const servers = { ...value.mcpServers };
  delete servers[MCP_SERVER];
  return { value: { ...value, mcpServers: servers }, removed: [`MCP server ${MCP_SERVER}`] };
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
  const pending = join(dirname(path), `.${basename(path)}.nuke-rerail-${process.pid}`);
  writeFileSync(pending, `${JSON.stringify(value, null, 2)}\n`, { mode });
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

function stripTarget(target, matches) {
  const value = readJson(target.path);
  return value === undefined ? { value, removed: [] } : target.strip(value, { matches });
}

/** Stopping the daemon that owns the Rerail home, when one is installed or recorded. */
function daemonSteps(rerailHome, env) {
  const state = readDaemonState(rerailHome);
  if (!existsSync(launcherPath(rerailHome)) && state === undefined) return [];
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

/** A checkout's dev instance; one whose instance.json no longer parses has no daemon this script can stop. */
function readInstance(dev, checkout) {
  try {
    return dev.readDevInstance(checkout);
  } catch {
    return undefined;
  }
}

/** The registered checkouts whose dev daemon is running on its own dev home; a bridged one on the Rerail home is the daemon step. */
async function devCheckoutSteps(rerailHome, dev, env) {
  const steps = [];
  for (const checkout of await dev.listDevCheckouts(env)) {
    const instance = readInstance(dev, checkout);
    if (instance === undefined || resolve(instance.home) === resolve(rerailHome)) continue;
    const pid = readDaemonState(instance.home)?.pid;
    if (!processAlive(pid)) continue;
    steps.push({
      kind: "dev-daemon",
      description: `Stop the dev daemon of ${checkout} (pid ${pid}; its state is kept)`,
      run: async log => {
        const result = await dev.stopDevCheckout(checkout);
        log(result?.stopped === false ? "It was no longer running." : "Dev daemon stopped.");
      },
    });
  }
  return steps;
}

/** Back up every existing host config before anything edits it. */
function configBackupSteps({ home, backupRoot, targets, matches }) {
  return targets.filter(target => existsSync(target.path)).map(target => {
    const backup = join(backupRoot, "configs", configBackupName(target.path, home));
    const owned = stripTarget(target, matches).removed.length > 0;
    return {
      kind: owned ? "config-backup-owned" : "config-backup",
      description: `Back up ${target.path} to ${backup}${owned ? "" : " (no Rerail entries)"}`,
      run: () => copyPreserving(target.path, backup),
    };
  });
}

/** Rerail's prompt commands and Codex skills into the backup, only when they run the Rerail home's CLI. */
function commandSteps({ home, backupRoot, matches }) {
  const moves = [
    ...CLAUDE_COMMANDS.map(name => [join(home, ".claude", "commands", name), join(backupRoot, "claude-commands", name)]),
    ...CURSOR_COMMANDS.map(name => [join(home, ".cursor", "commands", name), join(backupRoot, "cursor-commands", name)]),
    ...RERAIL_COMMAND_NAMES.map(name => [join(home, ".codex", "skills", name), join(backupRoot, `codex-skill-${name}`)]),
  ];
  return moves
    .filter(([source]) => referencesRerailHome(source, matches))
    .map(([source, target]) => ({ kind: "move", description: `Move ${source} -> ${target}`, run: () => move(source, target) }));
}

/** A command file, or a skill directory's SKILL.md, that names a path inside the Rerail home. */
function referencesRerailHome(path, matches) {
  try {
    const stat = statSync(path);
    if (stat.isFile()) return matches(readFileSync(path, "utf8"));
    return stat.isDirectory() && referencesRerailHome(join(path, "SKILL.md"), matches);
  } catch {
    return false;
  }
}

/** The harnesses `rerail install --remove` should visit: those whose config directory exists. */
export function detectedHarnesses(home) {
  return Object.entries(HARNESS_CONFIG_DIRS).filter(([, dir]) => existsSync(join(home, dir))).map(([harness]) => harness);
}

/**
 * `rerail install --remove --harness <detected>` through the installed launcher: Rerail's own unregistration (hooks,
 * status line, MCP bridge, OMP extension, Pi package, commands, skill). Naming the harnesses keeps the launcher from
 * also moving the database; this script moves the whole home. A launcher that fails leaves the sweep that follows.
 */
function unregisterSteps(rerailHome, home, env) {
  const harnesses = detectedHarnesses(home);
  if (!existsSync(launcherPath(rerailHome)) || harnesses.length === 0) return [];
  const args = ["install", "--remove", "--harness", harnesses.join(",")];
  return [{
    kind: "unregister",
    description: `Run ${launcherPath(rerailHome)} ${args.join(" ")}`,
    run: log => {
      const result = runLauncher(rerailHome, args, { env: { ...env, HOME: home } });
      const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
      if (output !== "") for (const line of output.split("\n")) log(`    ${line}`);
      if (result.status !== 0) log(`rerail install --remove exited ${String(result.status)}; removing the remaining entries directly.`);
    },
  }];
}

/**
 * Rerail entries left in each config, recomputed when the step runs: after `rerail install --remove`, whatever it did
 * not cover; without a working launcher, everything.
 */
function configSweepSteps(targets, matches, afterUnregister) {
  const steps = [];
  for (const target of targets) {
    if (!existsSync(target.path)) continue;
    const found = stripTarget(target, matches).removed;
    if (found.length === 0) continue;
    steps.push({
      kind: "config",
      description: afterUnregister
        ? `Remove whatever rerail install --remove leaves in ${target.path} (now: ${found.join(", ")})`
        : `Edit ${target.path}: remove ${found.join(", ")}`,
      run: log => {
        const result = stripTarget(target, matches);
        if (result.removed.length === 0) return;
        writeJsonAtomic(target.path, result.value);
        log(`    removed ${result.removed.join(", ")}`);
      },
    });
  }
  return steps;
}

function ompExtensionSteps(home, matches) {
  const ompExtension = join(home, ".omp", "agent", "extensions", "rerail");
  if (!pathExists(ompExtension) || !lstatSync(ompExtension).isSymbolicLink()) return [];
  const link = readlinkSync(ompExtension);
  if (!matches(resolve(dirname(ompExtension), link))) return [];
  return [{
    kind: "remove",
    description: `Remove symlink ${ompExtension} -> ${link}`,
    run: () => {
      if (pathExists(ompExtension)) unlinkSync(ompExtension);
    },
  }];
}

/** The Rerail home itself, and harness.sqlite when RERAIL_HOME moved the home away from `~/.rerail`. */
function homeSteps({ home, rerailHome, backupRoot }) {
  const steps = [];
  const harnessDatabase = harnessDatabasePath(home);
  if (dirname(harnessDatabase) !== resolve(rerailHome) && pathExists(harnessDatabase)) {
    const target = join(backupRoot, "harness.sqlite");
    steps.push({ kind: "move", description: `Move ${harnessDatabase} -> ${target}`, run: () => move(harnessDatabase, target) });
  }
  if (pathExists(rerailHome)) {
    const target = join(backupRoot, BACKUP_RERAIL_HOME);
    steps.push({ kind: "move", description: `Move ${rerailHome} -> ${target}`, run: () => move(rerailHome, target) });
  }
  return steps;
}

/**
 * Everything `nuke-rerail` would do, computed without changing anything. Each step has a description and a `run`
 * that performs it against `backupRoot`. `dev` supplies the checkout dev-daemon registry (scripts/rerail-dev.mjs).
 */
export async function planNuke({ env = process.env, now = new Date(), dev } = {}) {
  const home = env.HOME ?? homedir();
  const rerailHome = rerailHomeFrom(env, home);
  const matches = rerailHomeMatcher(rerailHome, home);
  const backupRoot = backupRootFor(home, now);
  const targets = configTargets(home, await readStatusRegistrations(harnessDatabasePath(home)));
  const devCheckouts = dev ?? await import("./rerail-dev.mjs");
  const unregister = unregisterSteps(rerailHome, home, env);
  const steps = [
    ...daemonSteps(rerailHome, env),
    ...await devCheckoutSteps(rerailHome, devCheckouts, env),
    ...configBackupSteps({ home, backupRoot, targets, matches }),
    ...commandSteps({ home, backupRoot, matches }),
    ...unregister,
    ...configSweepSteps(targets, matches, unregister.length > 0),
    ...ompExtensionSteps(home, matches),
    ...homeSteps({ home, rerailHome, backupRoot }),
  ];
  // Untouched configs are backed up only alongside a real removal, so a clean machine stays a no-op.
  const removes = steps.some(step => step.kind !== "config-backup");
  return { home, rerailHome, backupRoot, steps: removes ? steps : [] };
}

export function needsBackup(plan) {
  return plan.steps.some(step => !["daemon", "dev-daemon", "unregister", "config", "remove"].includes(step.kind));
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

export async function main(argv, { env = process.env, log = console.log, interactive = Boolean(process.stdin.isTTY && process.stdout.isTTY), dev } = {}) {
  const options = parseNukeArguments(argv);
  if (options.help) {
    log(`${USAGE}\nRemoves Rerail from this machine into ~/.rerail-clean-YYYYMMDD and stops every checkout's dev daemon.`);
    return 0;
  }
  const plan = await planNuke({ env, dev });
  if (plan.steps.length === 0) {
    log(`Nothing to remove: no Rerail install found for ${plan.home} and no checkout dev daemon running.`);
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
  if (plan.steps.some(step => step.kind === "dev-daemon")) log("Stopped dev daemons keep their state; just rerail-dev in a checkout starts its daemon again.");
  log("Restart open Claude/Cursor/Codex/OMP/Pi sessions; their loaded hooks now point at missing files.");
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
