import { existsSync } from "node:fs";
import { mkdir, realpath, rename } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { RERAIL_DATABASE_FILE } from "./database.js";
import { lookupDaemon, startDaemon, stopDaemon } from "./daemon-control.js";
import { parseArguments, parsePort } from "./daemon.js";
import {
  HARNESSES,
  registerHarnesses,
  unregisterHarnesses,
  type Harness,
  type RegistrationOptions,
  type RegistrationReport,
} from "./harness-registration.js";
import type { ManagedUpdater } from "./updater.js";

export interface SetupContext {
  /** Rerail's own state directory (`~/.rerail` unless `RERAIL_HOME`); versions, launcher, daemon live here. */
  home: string;
  /** The package this launcher runs from: an npx cache, a global install, or a checkout. */
  packageRoot: string;
  updater: ManagedUpdater;
}

/** Harness config lives under the user's home (`~/.cursor`, `~/.claude`, …), not under Rerail's state directory. */
function harnessHome(): string {
  return process.env.HOME ?? homedir();
}

interface RegistrationArguments {
  harnesses: Harness[] | undefined;
  projectDir: string | undefined;
  dryRun: boolean;
}

/** `--harness a,b` or `--agents a,b`: each name must be a coding agent Rerail supports. */
export function parseHarnessNames(values: readonly string[]): Harness[] {
  return values.map(value => {
    const harness = HARNESSES.find(candidate => candidate === value);
    if (harness === undefined) throw new Error(`Unknown coding agent: ${value} (expected one of ${HARNESSES.join(", ")})`);
    return harness;
  });
}

/** `--harness` may repeat; `parseArguments` keeps the last value, so split a comma list instead. */
function registrationArguments(options: ReadonlyMap<string, string>, flags: ReadonlySet<string>): RegistrationArguments {
  const harnessList = options.get("--harness");
  const projectDir = options.get("--project");
  return {
    harnesses: harnessList === undefined ? undefined : parseHarnessNames(harnessList.split(",").map(name => name.trim()).filter(Boolean)),
    projectDir: projectDir === undefined ? undefined : resolve(projectDir),
    dryRun: flags.has("--dry-run"),
  };
}

function registrationOptions(root: string, home: string, parsed: RegistrationArguments, templateRoot?: string): RegistrationOptions {
  return {
    root,
    ...(templateRoot === undefined ? {} : { templateRoot }),
    home,
    workspaceDir: process.cwd(),
    ...(parsed.harnesses === undefined ? {} : { harnesses: parsed.harnesses }),
    ...(parsed.projectDir === undefined ? {} : { projectDir: parsed.projectDir }),
    dryRun: parsed.dryRun,
  };
}

function printReport(report: RegistrationReport, dryRun: boolean): void {
  for (const change of report.changes) {
    const verb = dryRun && change.action !== "skipped" && change.action !== "unchanged" ? `would ${change.action.replace(/d$/, "")}` : change.action;
    process.stdout.write(`  ${change.harness.padEnd(11)} ${verb.padEnd(12)} ${change.path}${change.detail === "" ? "" : `  (${change.detail})`}\n`);
  }
  for (const note of report.notes) process.stdout.write(`  note: ${note}\n`);
}

/** Registrations point at `<home>/current` once setup has run, so updates never touch harness config again. */
function registrationRoot(context: SetupContext): string {
  const current = context.updater.currentLink;
  return existsSync(join(current, "package.json")) ? current : context.packageRoot;
}

/** Steps both `setup` and `install --root` earn, since both write the launcher and touch harness config. */
function nextSteps(home: string): string[] {
  return [
    `  add ${join(home, "bin")} to PATH to run \`rerail\` directly`,
    "  restart Cursor, Claude Code, Codex, OMP, or Pi so they load the new hooks",
  ];
}

/** True when `<home>/current` already resolves to `root`, so hooks written against it name this checkout. */
async function currentPointsAt(context: SetupContext, root: string): Promise<boolean> {
  const current = await realpath(context.updater.currentLink).catch(() => undefined);
  return current !== undefined && current === (await realpath(root).catch(() => root));
}

/**
 * `--root <checkout>`: adopt the checkout the way `setup` adopts a published
 * version, so a source install leaves a `rerail` command behind and its hooks
 * name `<home>/current` rather than the checkout, whose path dies with the
 * workspace. Returns the root registrations should be written against.
 */
async function activateExplicitRoot(context: SetupContext, root: string, remove: boolean, dryRun: boolean): Promise<string> {
  const { updater } = context;
  // Removal has to name whichever path the hooks in place were written with.
  if (remove) return (await currentPointsAt(context, root)) ? updater.currentLink : root;
  if (dryRun) {
    process.stdout.write(`Would point ${updater.currentLink} at ${root} and write the launcher ${updater.launcherPath}.\n`);
    return updater.currentLink;
  }
  const result = await updater.activateRoot(root);
  process.stdout.write(`${result.changed ? "Activated" : "Already active:"} ${result.current.version} at ${result.current.packageRoot}\n`);
  process.stdout.write(`Launcher: ${updater.launcherPath}\n`);
  return updater.currentLink;
}

/** The backup folder: `~/.rerail-clean-YYYYMMDD`, or `-YYYYMMDD-HHMMSS` when today's exists. */
function backupRoot(now = new Date()): string {
  const pad = (value: number): string => String(value).padStart(2, "0");
  const day = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  const daily = join(harnessHome(), `.rerail-clean-${day}`);
  return existsSync(daily) ? `${daily}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}` : daily;
}

/** Rules and every repository's history, with their SQLite sidecars. */
const BACKED_UP_DATABASES = ["", "-wal", "-shm"].map(suffix => `${RERAIL_DATABASE_FILE}${suffix}`);

/**
 * After the coding agents are disconnected, nothing reopens the database: stop the daemon, then
 * move rules and history aside so a later install starts clean and can restore them. The files are
 * listed only after the stop: closing the daemon's last connection checkpoints the WAL and deletes
 * the `-wal` and `-shm` sidecars.
 */
async function backUpRulesAndHistory(home: string, dryRun: boolean): Promise<void> {
  const target = join(backupRoot(), "rerail-home");
  const present = () => BACKED_UP_DATABASES.filter(name => existsSync(join(home, name)));
  if (dryRun) {
    const listed = present();
    const moves = listed.length === 0 ? "" : " and move " + listed.join(", ") + " to " + target;
    process.stdout.write(`Would stop the Rerail daemon${moves}.\n`);
    return;
  }
  process.stdout.write(await stopDaemon(home) === "stopped" ? "Stopped the Rerail daemon.\n" : "The Rerail daemon was not running.\n");
  const listed = present();
  if (listed.length === 0) {
    process.stdout.write("No rules or history to back up.\n");
    return;
  }
  await mkdir(target, { recursive: true, mode: 0o700 });
  for (const name of listed) {
    try {
      await rename(join(home, name), join(target, name));
    } catch (error) {
      // A sidecar SQLite removed between the listing and the move holds nothing the main file lacks.
      if (!(error instanceof Error && "code" in error && error.code === "ENOENT" && /-(wal|shm)$/u.test(name))) throw error;
    }
  }
  process.stdout.write(`Backup: ${target} holds your rules and history. Install Rerail again and restore this backup to get them back.\n`);
}

export async function runInstallCommand(args: readonly string[], context: SetupContext): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--remove"], options: ["--root", "--project", "--harness"] },
    argument => `Unknown install option: ${argument}`,
  );
  const parsed = registrationArguments(options, flags);
  const rootArgument = options.get("--root");
  const explicitRoot = rootArgument === undefined ? undefined : resolve(rootArgument);
  const remove = flags.has("--remove");
  const root = explicitRoot === undefined
    ? registrationRoot(context)
    : await activateExplicitRoot(context, explicitRoot, remove, parsed.dryRun);
  // A dry run never creates `<home>/current`, so packaged templates must come from the checkout itself.
  const templateRoot = explicitRoot !== undefined && parsed.dryRun && !remove ? explicitRoot : undefined;
  process.stdout.write(`${remove ? "Removing Rerail from" : "Connecting Rerail to"} coding agents for ${root}${parsed.dryRun ? " (dry run)" : ""}\n`);
  const report = remove
    ? await unregisterHarnesses(registrationOptions(root, harnessHome(), parsed))
    : await registerHarnesses(registrationOptions(root, harnessHome(), parsed, templateRoot));
  printReport(report, parsed.dryRun);
  // Removing every coding agent from the managed install is the uninstall. Removing one coding agent, or a
  // checkout or development bridge registered with `--root`, leaves the shared rules and history in place.
  const uninstall = remove && parsed.harnesses === undefined && (explicitRoot === undefined || root === context.updater.currentLink);
  if (uninstall) await backUpRulesAndHistory(context.home, parsed.dryRun);
  if (explicitRoot !== undefined && !remove) {
    process.stdout.write(["", "Next:", ...nextSteps(context.home), ""].join("\n"));
  }
  return 0;
}

/**
 * `npx @neralabs/rerail setup`: make this version the managed install under
 * `<home>/versions`, point harnesses at `<home>/current`, and bring the daemon
 * up. Idempotent, so it doubles as "upgrade and re-register".
 */
export async function runSetupCommand(args: readonly string[], context: SetupContext): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--skip-start", "--skip-register"], options: ["--project", "--harness", "--port"] },
    argument => `Unknown setup option: ${argument}`,
  );
  const parsed = registrationArguments(options, flags);
  const portOption = options.get("--port");
  const port = portOption === undefined ? undefined : parsePort(portOption);
  const { updater, home } = context;

  if (parsed.dryRun) {
    process.stdout.write(`Would install Rerail ${updater.packageVersionLabel} into ${updater.versionsRoot} and point ${updater.currentLink} at it.\n`);
  } else {
    process.stdout.write(`Installing Rerail ${updater.packageVersionLabel} into ${updater.versionsRoot}...\n`);
    const result = await updater.install();
    process.stdout.write(`${result.changed ? "Activated" : "Already active:"} ${result.current.version} at ${result.current.packageRoot}\n`);
    process.stdout.write(`Launcher: ${updater.launcherPath}\n`);
  }

  if (flags.has("--skip-register")) {
    process.stdout.write("Skipping harness registration (--skip-register).\n");
  } else {
    process.stdout.write(`${parsed.dryRun ? "Would register" : "Registering"} harnesses against ${updater.currentLink}:\n`);
    const report = await registerHarnesses(registrationOptions(updater.currentLink, harnessHome(), parsed));
    printReport(report, parsed.dryRun);
  }

  if (flags.has("--skip-start")) {
    process.stdout.write("Not starting the daemon (--skip-start).\n");
  } else if (parsed.dryRun) {
    process.stdout.write("Would start the daemon.\n");
  } else {
    // A daemon from before this setup keeps serving the old version's UI and kernel; replace it.
    const lookup = await lookupDaemon(home);
    const activeRoot = await realpath(updater.currentLink);
    const runningRoot = lookup.status === "running"
      ? await realpath(lookup.state.packageRoot).catch(() => lookup.state.packageRoot)
      : undefined;
    const stale = lookup.status === "unresponsive" || (lookup.status === "running" && runningRoot !== activeRoot);
    if (stale) {
      process.stdout.write(`Stopping daemon pid ${lookup.state.pid} (${lookup.status === "running" ? `version ${lookup.state.version}` : "not answering"})...\n`);
      await stopDaemon(home);
    }
    // The version directory, not the current link: the daemon and its SQLite binding belong to this install.
    const started = await startDaemon(
      { home, packageRoot: activeRoot },
      {
        dev: false,
        // The daemon's page lists the repository it starts in: the project, when one was named.
        ...(parsed.projectDir === undefined ? {} : { cwd: parsed.projectDir }),
        ...(port === undefined ? {} : { port }),
      },
    );
    process.stdout.write(`Daemon ${started.started ? "started" : "already running"} at ${started.daemon.state.url} (version ${started.daemon.state.version})\n`);
  }

  process.stdout.write([
    "",
    "Next:",
    ...nextSteps(home),
    "  drop rules into <project>/.<harness>/conversation-rules.json (setup seeded empty ones when --project was given)",
    "",
  ].join("\n"));
  return 0;
}
