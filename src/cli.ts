#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ADAPTER_CAPABILITIES, type RerailHost } from "./adapter-contract.js";
import { claudeWorkspace } from "./claude-event-adapter.js";
import { readPresentation, readRulesPage } from "./session-presentation.js";
import {
  assertLocalPolicyKernelEntitlement,
  authorizedUpdateChannel,
  runActivateCommand,
  runActivationRequestCommand,
  runProfileCommand,
  signedUpdatePolicy,
  takeOption,
  type SignedUpdatePolicy,
} from "./activation-cli.js";
import { CustomerActivationManager, CustomerProfileStore } from "./customer-profile.js";
import { effectiveStartState, repositoryName, startRerail, stopRerail, workspaceStartState, type RepositorySetupIo } from "./repository-start.js";
import { devWorkspaceEnabled } from "./dev-routing.js";
import { assertHostBindable, parseArguments, parsePort, parsePublicHosts } from "./daemon.js";
import { defaultRerailDatabasePath, RERAIL_DATABASE_FILE } from "./database.js";
import {
  lookupDaemon,
  openInBrowser,
  openPage,
  startDaemon,
  stopDaemon,
  DAEMON_LOG_FILE,
  type RunningDaemon,
} from "./daemon-control.js";
import { deviceSecretStore } from "./device-keystore.js";
import { runRerailMcpBridge } from "./dev-bridge.js";
import { runResetCommand, runRulesCommand, snapshotDatabaseBeforeUpdate } from "./project-cli.js";
import { MOCK_COSINE_RULES, MOCK_POLICY_RULES, seedRerailDatabase } from "./seed.js";
import { runScanCommand } from "./scan-cli.js";
import { openScopedRuleCatalog } from "./scoped-rule-catalog.js";
import { parseHarnessNames, runInstallCommand, runSetupCommand } from "./setup-cli.js";
import { defaultRerailHome } from "./rerail-home.js";
import {
  ManagedUpdater,
  type UpdateResult,
  type UpdateStatus,
} from "./updater.js";

const MANAGED_CHILD_ARGUMENT = "--rerail-managed-child";
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

interface PackageManifest {
  name: string;
  version: string;
}

async function packageManifest(root: string): Promise<PackageManifest> {
  const value = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as Record<string, unknown>;
  if (typeof value.name !== "string" || typeof value.version !== "string") {
    throw new TypeError(`Invalid Rerail package manifest: ${join(root, "package.json")}`);
  }
  return { name: value.name, version: value.version };
}

async function runChild(command: string, args: readonly string[], environment = process.env): Promise<number> {
  const { promise, resolve: resolveExit, reject } = Promise.withResolvers<number>();
  const child = spawn(command, [...args], { env: environment, stdio: "inherit" });
  child.once("error", reject);
  child.once("exit", (code, signal) => {
    if (signal !== null) {
      process.kill(process.pid, signal);
      return;
    }
    resolveExit(code ?? 1);
  });
  return await promise;
}

function printStatus(status: UpdateStatus): void {
  process.stdout.write(`Channel: ${status.channel}\n`);
  process.stdout.write(`Current: ${status.current.version}\n`);
  process.stdout.write(`Location: ${status.current.packageRoot}\n`);
  process.stdout.write(`Previous: ${status.previous?.version ?? "none"}\n`);
  process.stdout.write(`Last check: ${status.lastCheckAt ?? "never"}\n`);
}

function printUpdateResult(result: UpdateResult): void {
  process.stdout.write(result.changed
    ? `Activated Rerail ${result.current.version}; new harness sessions will use it.\n`
    : `Rerail ${result.current.version} is current for channel ${result.channel}.\n`);
}

function selectedChannel(args: readonly string[]): string | undefined {
  const index = args.indexOf("--channel");
  if (index === -1) return undefined;
  const value = args[index + 1];
  if (value === undefined || value.startsWith("--")) throw new Error("--channel requires an npm dist-tag");
  return value;
}

function usage(): string {
  return [
    "Usage:",
    "  rerail setup [--project <dir>] [--harness omp,pi,claude-code,codex,cursor] [--port <n>] [--skip-start] [--skip-register] [--dry-run]",
    "  rerail install [--root <package>] [--project <dir>] [--harness <list>] [--remove] [--dry-run]",
    "  rerail start [--repo <dir>] [--agents <list>] [--database <file>]",
    "  rerail stop [--repo <dir>] [--database <file>]",
    "  rerail state [--repo <dir>] [--database <file>]",
    "  rerail status",
    "  rerail daemon start [--dev] [--port <port>] [--host <host>] [--allow-remote] [--public-host <name,...>] [--open]",
    "  rerail daemon stop",
    "  rerail session [<host> <session-id>] [--open]",
    "  rerail ui [--print]",
    "  rerail seed",
    "  rerail reset",
    "  rerail rules export [--output <rules.json>]",
    "  rerail rules import <rules.json>",
    "  rerail [--profile <name>] events",
    "  rerail scan --session <id> [--scope repository|git-diff] [--wait] [--json]",
    "  rerail settings --session <id> [--scope repository|touched-files|ai-output] [--mode log|steer] [--json]",
    "  rerail [--profile <name>] activate --file <profile.jws> --trust-root <roots.json>",
    "  rerail [--profile <name>] activate --code <code> --activation-url <url> --trust-root <roots.json>",
    "  rerail [--profile <name>] activation request --deployment <id> --output <file>",
    "  rerail [--profile <name>] profile [status|select|remove]",
    "  rerail [--profile <name>] update",
    "  rerail [--profile <name>] update check [--channel <tag>]",
    "  rerail update status",
    "  rerail update rollback",
    "  rerail update --channel <tag>",
  ].join("\n");
}

function printDaemon(daemon: RunningDaemon): void {
  const { state, health } = daemon;
  process.stdout.write(`Rerail daemon ${state.version} running at ${state.url} (pid ${state.pid}, ${health.ui} UI)\n`);
  process.stdout.write(`Database: ${health.database.path}${health.database.exists ? "" : " (not created yet)"}\n`);
  process.stdout.write(health.native.ok
    ? `Native kernel: ok (version ${health.native.kernelVersion})\n`
    : `Native kernel: unavailable: ${health.native.error}\n`);
}

async function runDaemonStartCommand(args: readonly string[], home: string): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dev", "--open", "--allow-remote"], options: ["--port", "--host", "--public-host"] },
    argument => `Unknown daemon start option: ${argument}\n\n${usage()}`,
  );
  const port = options.get("--port");
  const host = options.get("--host");
  if (host !== undefined) assertHostBindable(host, flags.has("--allow-remote"));
  const publicHosts = parsePublicHosts(options.get("--public-host"));
  const result = await startDaemon({ home, packageRoot }, {
    dev: flags.has("--dev"),
    ...(host === undefined ? {} : { host }),
    ...(flags.has("--allow-remote") ? { allowRemote: true } : {}),
    ...(publicHosts.length === 0 ? {} : { publicHosts }),
    ...(port === undefined ? {} : { port: parsePort(port) }),
  });
  if (!result.started) process.stdout.write("The Rerail daemon is already running.\n");
  printDaemon(result.daemon);
  process.stdout.write(`Logs: ${join(home, DAEMON_LOG_FILE)}\n`);
  if (flags.has("--open")) openInBrowser(result.daemon.state.url);
  return 0;
}

async function runStatusCommand(home: string): Promise<number> {
  const workspace = process.cwd();
  const state = workspaceStartState(workspace);
  const name = repositoryName(workspace);
  if (state === "started") process.stdout.write(`Rerail is on in ${name}.\n`);
  else if (state === "stopped") process.stdout.write(`Rerail is stopped in ${name}. Start it again with rerail start.\n`);
  else if (devWorkspaceEnabled(workspace)) process.stdout.write(`Rerail is on in ${name} through its development workspace.\n`);
  else process.stdout.write(`Rerail has not been started in ${name}. Start it with rerail start.\n`);
  const lookup = await lookupDaemon(home);
  if (lookup.status === "absent") {
    process.stdout.write("The Rerail daemon is not running. Start it with rerail daemon start.\n");
    return 3;
  }
  if (lookup.status === "unresponsive") {
    process.stdout.write(`Rerail daemon pid ${lookup.state.pid} is alive but not answering at ${lookup.state.url}; run rerail daemon stop, then rerail daemon start.\n`);
    return 4;
  }
  printDaemon({ state: lookup.state, health: lookup.health });
  return 0;
}

/** Populate the Rerail database with throwaway mock rules, and a session and work links for this repository. */
function runSeedCommand(args: readonly string[]): number {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown seed option: ${argument}\n\n${usage()}`);
  const database = defaultRerailDatabasePath();
  const catalog = openScopedRuleCatalog(process.cwd(), database);
  let result;
  try {
    for (const rule of MOCK_POLICY_RULES) catalog.upsertPolicy(rule);
    for (const [index, rule] of MOCK_COSINE_RULES.entries()) catalog.upsertCosine(rule, { enabled: index === 0 });
    result = seedRerailDatabase(database, { repositoryId: catalog.context().repositoryId });
  } finally {
    catalog.close();
  }
  process.stdout.write(
    `Seeded ${database}: ${result.policyRules} policy rules, `
    + `${result.cosineRules} cosine rules, ${result.sessions} session, ${result.events} events, `
    + `${result.workLinks} work links.\n`,
  );
  return 0;
}

/** The rules page of the daemon serving this checkout; `--print` only reports the URL. */
async function runUiCommand(args: readonly string[]): Promise<number> {
  const { flags } = parseArguments(args, { flags: ["--print"], options: [] }, argument => `Unknown ui option: ${argument}\n\n${usage()}`);
  const page = await readRulesPage({ cwd: claudeWorkspace(process.cwd()) });
  if (page.url === null) {
    process.stdout.write(`Rerail is not running for this checkout (${page.reason}). Check rerail status, or start the daemon with rerail daemon start --open.\n`);
    return 3;
  }
  process.stdout.write(`${page.url}\n`);
  if (!flags.has("--print") && await openPage(page.url) === "orca-tab") process.stdout.write("Opened in the Orca window.\n");
  return 0;
}

async function dispatchToManagedVersion(args: readonly string[], updater: ManagedUpdater): Promise<number | undefined> {
  const state = await updater.readState();
  if (resolve(state.current.packageRoot) === packageRoot) return undefined;
  const managedCli = join(state.current.packageRoot, "dist", "cli.js");
  if (!existsSync(managedCli)) {
    throw new Error(`Active Rerail launcher is missing: ${managedCli}. Run rerail update rollback.`);
  }
  return await runChild(process.execPath, [managedCli, MANAGED_CHILD_ARGUMENT, ...args]);
}

/** The persistent event stream for out-of-process harnesses. It never checks for or installs updates; `rerail update` is the only way a new version arrives. */
async function runEvents(policy?: SignedUpdatePolicy): Promise<number> {
  assertLocalPolicyKernelEntitlement(policy);
  return await runChild(process.execPath, [join(packageRoot, "dist", "event-stream.js")]);
}

async function runUpdateCommand(
  args: readonly string[],
  updater: ManagedUpdater,
  home: string,
  policy?: SignedUpdatePolicy,
): Promise<number> {
  const action = args[0];
  if (action === "status") {
    printStatus(await updater.status());
    return 0;
  }
  const requestedChannel = selectedChannel(args);
  const channel = authorizedUpdateChannel(policy, requestedChannel);
  if (action === "check") {
    const result = await updater.check(channel);
    printStatus(result);
    process.stdout.write(`Available: ${result.availableVersion}\n`);
    process.stdout.write(`Update available: ${result.updateAvailable ? "yes" : "no"}\n`);
    return 0;
  }
  if (action === "rollback") {
    printUpdateResult(await updater.rollback());
    return 0;
  }
  if (action !== undefined && action !== "--channel") throw new Error(`Unknown update action: ${action}`);
  snapshotDatabaseBeforeUpdate(join(home, RERAIL_DATABASE_FILE));
  printUpdateResult(await updater.update(channel));
  return 0;
}

/** Read the caller's exact session; Codex supplies its thread identity to shell tools. `--open` also opens the session page. */
async function runSessionCommand(rawArgs: readonly string[]): Promise<number> {
  const open = rawArgs.includes("--open");
  const args = open ? rawArgs.filter(argument => argument !== "--open") : rawArgs;
  const host = args.length === 0 ? "codex" : args[0];
  const sessionId = args.length === 0 ? process.env.CODEX_THREAD_ID : args[1];
  if ((args.length !== 0 && args.length !== 2) || host === undefined
      || !Object.hasOwn(ADAPTER_CAPABILITIES, host) || !sessionId?.trim()) {
    throw new Error("Usage: rerail session [<host> <session-id>] [--open]; inside Codex, rerail session uses CODEX_THREAD_ID");
  }
  const presentation = await readPresentation({
    host: host as RerailHost, sessionId, cwd: claudeWorkspace(process.cwd()), onDemand: true,
  });
  const link = presentation.url === null ? "" : `\n${presentation.url}`;
  process.stdout.write(`${presentation.text}${link}\n`);
  if (!open) return 0;
  if (presentation.url === null) return 3;
  if (await openPage(presentation.url) === "orca-tab") process.stdout.write("Opened in the Orca window.\n");
  return 0;
}

/** The daemon commands need only the launcher home; they never load an activation policy. */
async function runDaemonCommand(command: string, args: readonly string[], home: string): Promise<number | undefined> {
  if (command === "daemon") {
    const [verb, ...rest] = args;
    if (verb === "start") return await runDaemonStartCommand(rest, home);
    if (verb === "stop" && rest.length === 0) {
      process.stdout.write(await stopDaemon(home) === "stopped" ? "Rerail daemon stopped.\n" : "The Rerail daemon is not running.\n");
      return 0;
    }
    throw new Error(`Usage: rerail daemon start [options] | rerail daemon stop\n\n${usage()}`);
  }
  if (command === "status") return await runStatusCommand(home);
  if (command === "ui") return await runUiCommand(args);
  return undefined;
}

/**
 * `rerail start` / `rerail stop` act on the checkout of `--repo` or the current directory. Without a
 * terminal the caller is a coding agent relaying the output, so the text names its slash commands.
 * `--database` names the database an extension was configured with, as `rerail state` takes it.
 */
async function runRepositoryCommand(command: "start" | "stop", args: readonly string[]): Promise<number> {
  const { options } = parseArguments(
    args,
    { flags: [], options: command === "start" ? ["--repo", "--agents", "--database"] : ["--repo", "--database"] },
    argument => `Unknown ${command} option: ${argument}\n\n${usage()}`,
  );
  const workspace = resolve(options.get("--repo") ?? process.cwd());
  const database = options.get("--database");
  const databasePath = database === undefined ? undefined : resolve(database);
  const io: RepositorySetupIo = {
    lane: process.stdout.isTTY ? "terminal" : "agent",
    say: text => { process.stdout.write(`${text}\n`); },
  };
  if (command === "stop") {
    stopRerail(workspace, io, databasePath);
    return 0;
  }
  const agents = options.get("--agents");
  await startRerail(workspace, io, {
    ...(databasePath === undefined ? {} : { databasePath }),
    ...(agents === undefined ? {} : { agents: parseHarnessNames(agents.split(",").map(name => name.trim()).filter(Boolean)) }),
  });
  const page = await readRulesPage({ cwd: workspace });
  if (page.url !== null) process.stdout.write(`  ${page.url}\n`);
  return 0;
}

/**
 * `rerail state`: started, stopped or never for the repository, for callers that cannot open SQLite (OMP runs on
 * Bun). `--database` names the database file when a caller configured one other than the home's default.
 */
function runStateCommand(args: readonly string[]): number {
  const { options } = parseArguments(args, { flags: [], options: ["--repo", "--database"] }, argument => `Unknown state option: ${argument}\n\n${usage()}`);
  const databasePath = options.get("--database");
  const workspace = resolve(options.get("--repo") ?? process.cwd());
  process.stdout.write(`${effectiveStartState(workspace, databasePath === undefined ? undefined : resolve(databasePath))}\n`);
  return 0;
}

/** Commands that read shared launcher state or this project's database and must run from whichever launcher the user invoked. */
function isVersionIndependentCommand(args: readonly string[]): boolean {
  const commandArgs = takeOption(args, "--profile").args;
  const command = commandArgs[0];
  // `setup` installs the package it was invoked from, so it must not forward to an older active version.
  // `seed`, `rules`, and `reset` write the Rerail database with the invoking package's schema (a checkout under `just rerail-dev`).
  if (["setup", "install", "start", "stop", "state", "status", "session", "ui", "seed", "rules", "reset"].includes(command ?? "")) return true;
  if (command === "daemon") return commandArgs[1] === "stop";
  return command === "update" && (commandArgs[1] === "status" || commandArgs[1] === "rollback");
}

async function main(): Promise<number> {
  const manifest = await packageManifest(packageRoot);
  const home = defaultRerailHome();
  const updater = new ManagedUpdater({
    home,
    packageRoot,
    packageVersion: manifest.version,
  });
  const profileStore = new CustomerProfileStore(home);
  const activation = new CustomerActivationManager(profileStore, deviceSecretStore());
  const rawArgs = process.argv.slice(2);
  const managedChild = rawArgs[0] === MANAGED_CHILD_ARGUMENT;
  const args = managedChild ? rawArgs.slice(1) : rawArgs;
  if (args[0] === "mcp-bridge") {
    if (args.length !== 1) throw new Error("mcp-bridge takes no arguments");
    await runRerailMcpBridge();
    return 0;
  }

  if (!managedChild && !isVersionIndependentCommand(args)) {
    const exit = await dispatchToManagedVersion(args, updater);
    if (exit !== undefined) return exit;
  }

  const profileOption = takeOption(args, "--profile");
  const commandArgs = profileOption.args;
  const command = commandArgs[0];
  if (command === undefined) throw new Error(`Missing command\n\n${usage()}`);
  const context: CommandContext = {
    home,
    updater,
    activation,
    profileStore,
    packageVersion: manifest.version,
    profile: profileOption.value,
  };
  const commandArguments = commandArgs.slice(1);
  const exit = (await runAccountCommand(command, commandArguments, context))
    ?? (await runProjectCommand(command, commandArguments, context));
  if (exit !== undefined) return exit;
  return await runReleaseCommand(command, commandArguments, context);
}

interface CommandContext {
  home: string;
  updater: ManagedUpdater;
  activation: CustomerActivationManager;
  profileStore: CustomerProfileStore;
  packageVersion: string;
  profile: ReturnType<typeof takeOption>["value"];
}

async function runAccountCommand(command: string, args: readonly string[], context: CommandContext): Promise<number | undefined> {
  const { activation, profileStore, packageVersion, profile } = context;
  if (command === "activate") return await runActivateCommand(args, activation, packageVersion, profile);
  if (command === "activation") return await runActivationRequestCommand(args, activation, packageVersion, profile);
  if (command === "profile") return await runProfileCommand(args, activation, profileStore, profile);
  return undefined;
}

async function runProjectCommand(command: string, args: readonly string[], context: CommandContext): Promise<number | undefined> {
  const { home, updater } = context;
  if (command === "setup") return await runSetupCommand(args, { home, packageRoot, updater });
  if (command === "install") return await runInstallCommand(args, { home, packageRoot, updater });
  if (command === "session") return await runSessionCommand(args);
  if (command === "seed") return runSeedCommand(args);
  if (command === "rules") return runRulesCommand(args, usage);
  if (command === "reset") return runResetCommand(args, usage);
  const daemonExit = await runDaemonCommand(command, args, home);
  if (daemonExit !== undefined) return daemonExit;
  if (command === "start" || command === "stop") return await runRepositoryCommand(command, args);
  if (command === "state") return runStateCommand(args);
  if (command === "scan" || command === "settings") return await runScanCommand(command, args, { home });
  return undefined;
}

/** Commands gated by the signed update policy, plus help; anything else is unknown. */
async function runReleaseCommand(command: string, args: readonly string[], context: CommandContext): Promise<number> {
  const policy = await signedUpdatePolicy(context.activation, context.profileStore, context.profile);
  if (command === "events") return await runEvents(policy);
  if (command === "update") return await runUpdateCommand(args, context.updater, context.home, policy);
  if (command === "help" || command === "--help" || command === "-h") {
    process.stdout.write(`${usage()}\n`);
    return 0;
  }
  throw new Error(`Unknown command: ${command}\n\n${usage()}`);
}

try {
  process.exitCode = await main();
} catch (error) {
  process.stderr.write(`rerail: ${(error as Error).message}\n`);
  process.exitCode = 1;
}
