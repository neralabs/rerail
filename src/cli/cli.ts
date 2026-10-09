#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { ADAPTER_CAPABILITIES, type RerailHost } from "../engine/adapter-contract.js";
import { ENTRY_POINTS, PACKAGE_ROOT as packageRoot } from "../engine/package-root.js";
import { claudeWorkspace } from "../hosts/claude/claude-event-adapter.js";
import { readPresentation, readRulesDashboard, readSummaryState } from "../presentation/session-presentation.js";
import { renderRerailSummary } from "../harness/rerail-summary.js";
import {
  assertLocalEngineEntitlement,
  authorizedUpdateChannel,
  runActivateCommand,
  runActivationRequestCommand,
  runProfileCommand,
  signedUpdatePolicy,
  takeOption,
  type SignedUpdatePolicy,
} from "./activation-cli.js";
import { CustomerActivationManager, CustomerProfileStore } from "../activation/customer-profile.js";
import {
  readOnlyStartState, stopRerail, workspaceSetUp, workspaceStartState,
} from "../runtime/repository-start.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { repositoryName } from "../engine/catalog/repository-identity.js";
import { checkSetupAnswer, setupAgents, startRerail } from "./repository-setup.js";
import { isPreSetupHealthCheck, startForPreSetupHealthCheck } from "../runtime/pre-setup-health-check.js";
import { assertHostBindable, parseArguments, parsePort, parsePublicHosts } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import {
  lookupDaemon,
  openInBrowser,
  openDashboard,
  startDaemon,
  stopDaemon,
  DAEMON_LOG_FILE,
  type RunningDaemon,
} from "../runtime/daemon-control.js";
import { deviceSecretStore } from "../activation/device-keystore.js";
import { runRerailMcpBridge } from "../api/mcp-bridge.js";
import { runResetCommand, runRulesCommand, snapshotDatabaseBeforeUpdate } from "./project-cli.js";
import { lookupCheckout, parentCheckouts, recordCheckout } from "../engine/catalog/repository-registry.js";
import { listRepositoryRules, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES, seedRerailDatabase } from "./seed.js";
import { runAuditCommand } from "./audit-cli.js";
import { runRuleDraftCommand } from "./rule-draft.js";
import { JSON_FLAG, machinePlace, runInstallCommand, runSetupCommand, setupAnswerCommand, setupEnvironment, setupIo, startStep, type SetupContext } from "./setup-cli.js";
import { SETUP_FLAGS } from "../harness/setup-screens.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import {
  ManagedUpdater,
  type UpdateResult,
  type UpdateStatus,
} from "../setup/updater.js";

const MANAGED_CHILD_ARGUMENT = "--rerail-managed-child";

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
    "  rerail setup [--answer <answer>] [--repo <dir>] [--agents omp,pi,claude-code,codex,cursor] [--repository] [--port <n>] [--json] [--relay] [--database <file>]",
    "  rerail install [--root <package>] [--agents <list>] [--remove] [--dry-run]",
    "  rerail start [--repo <dir>] [--repository] [--agents <list>] [--answer <answer>] [--json] [--relay] [--database <file>]",
    "  rerail stop [--repo <dir>] [--repository] [--database <file>]",
    "  rerail state [--set-up] [--repo <dir>] [--database <file>]",
    "  rerail status",
    "  rerail daemon start [--dev] [--port <port>] [--host <host>] [--allow-remote] [--public-host <name,...>] [--open]",
    "  rerail daemon stop",
    "  rerail session [<host> [<session-id>]] [--open]",
    "  rerail ui [--print]",
    "  rerail seed",
    "  rerail reset",
    "  rerail rules export [--output <rules.json>]",
    "  rerail rules import <rules.json>",
    "  rerail rules test <draft.json> [--repo <dir>] [--json]",
    "  rerail rules add <draft.json> [--repo <dir>]",
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
    ? `Engine: ok (version ${health.native.kernelVersion})\n`
    : `Engine: unavailable: ${health.native.error}\n`);
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
  if (state === "stopped") process.stdout.write(`Rerail is stopped in ${name}. Start it with rerail start.\n`);
  else if (state === "not-set-up") process.stdout.write(`Rerail is on in ${name}, which is not set up yet. Set it up with rerail start.\n`);
  else process.stdout.write(`Rerail is on in ${name}.\n`);
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

/**
 * Store throwaway mock rules in the library, add them to this checkout's repository (only the first conversation
 * rule on) and set it up; then a session and work links for it.
 */
function runSeedCommand(args: readonly string[]): number {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown seed option: ${argument}\n\n${usage()}`);
  const databasePath = defaultRerailDatabasePath();
  const database = openRerailDatabase(databasePath);
  let result;
  try {
    const { repositoryKey } = recordCheckout(database, process.cwd());
    result = seedRerailDatabase(databasePath, { repositoryId: repositoryKey });
    setUpRepository(database, repositoryKey, {
      packs: [],
      rules: [...MOCK_CODE_RULES, ...MOCK_CONVERSATION_RULES].map(rule => ({ sourceKind: "library_rule" as const, sourceId: rule.id, folder: "" })),
      settings: MOCK_CONVERSATION_RULES.slice(1).map(rule => ({
        owner: { kind: "repository" as const, repositoryKey }, folder: "", targetKind: "rule" as const, targetId: rule.id, on: false, mode: null,
      })),
    });
  } finally {
    database.$client.close();
  }
  process.stdout.write(
    `Seeded ${databasePath}: ${result.codeRules} code rules, `
    + `${result.conversationRules} conversation rules added to this repository, ${result.sessions} session, ${result.events} events, `
    + `${result.workLinks} work links.\n`,
  );
  return 0;
}

/** The rules page of the daemon serving this checkout; `--print` only reports the URL. */
async function runUiCommand(args: readonly string[]): Promise<number> {
  const { flags } = parseArguments(args, { flags: ["--print"], options: [] }, argument => `Unknown ui option: ${argument}\n\n${usage()}`);
  const page = await readRulesDashboard({ cwd: claudeWorkspace(process.cwd()) });
  if (page.url === null) {
    process.stdout.write(`Rerail is not running for this checkout (${page.reason}). Check rerail status, or start the daemon with rerail daemon start --open.\n`);
    return 3;
  }
  process.stdout.write(`${page.url}\n`);
  if (!flags.has("--print") && await openDashboard(page.url) === "orca-tab") process.stdout.write("Opened in the Orca window.\n");
  return 0;
}

async function dispatchToManagedVersion(args: readonly string[], updater: ManagedUpdater): Promise<number | undefined> {
  const state = await updater.readState();
  if (resolve(state.current.packageRoot) === packageRoot) return undefined;
  const managedCli = join(state.current.packageRoot, ENTRY_POINTS.cli);
  if (!existsSync(managedCli)) {
    throw new Error(`Active Rerail launcher is missing: ${managedCli}. Run rerail update rollback.`);
  }
  return await runChild(process.execPath, [managedCli, MANAGED_CHILD_ARGUMENT, ...args]);
}

/** The persistent event stream for out-of-process hosts. It never checks for or installs updates; `rerail update` is the only way a new version arrives. */
async function runEvents(policy?: SignedUpdatePolicy): Promise<number> {
  assertLocalEngineEntitlement(policy);
  return await runChild(process.execPath, [join(packageRoot, ENTRY_POINTS.eventStream)]);
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

/**
 * Rules on in `workspace`'s checkout, from its repository's configuration, for `/rerail` where the coding agent
 * cannot name its session. A status read: it never writes, and where the configuration cannot be read (busy, or
 * the checkout not recorded) the summary drops the count line rather than failing.
 */
function enabledRuleCount(workspace: string): number | null {
  const databasePath = defaultRerailDatabasePath();
  if (!existsSync(databasePath)) return null;
  let database;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    const checkout = lookupCheckout(database, workspace);
    if (checkout === undefined) return null;
    const parentCheckoutPaths = parentCheckouts(database, checkout).map(row => row.checkoutPath);
    return listRepositoryRules(database, loadBundledCatalog(), checkout.repositoryKey, { checkoutPath: checkout.checkoutPath, parentCheckoutPaths })
      .rules.filter(rule => rule.on).length;
  } catch (error) {
    process.stderr.write(`rerail: could not count this repo's rules: ${error instanceof Error ? error.message : String(error)}\n`);
    return null;
  } finally {
    database?.$client.close();
  }
}

/**
 * `/rerail`: what Rerail is doing for the caller's exact session, then every command. Codex supplies its
 * thread identity to shell tools; Cursor names only itself, so it gets the repository's state instead.
 * `--open` also opens the session page (or the dashboard, without a session).
 */
async function runSessionCommand(rawArgs: readonly string[]): Promise<number> {
  const open = rawArgs.includes("--open");
  const args = open ? rawArgs.filter(argument => argument !== "--open") : rawArgs;
  const host = args.length === 0 ? "codex" : args[0];
  const sessionId = args.length === 0 ? process.env.CODEX_THREAD_ID : args[1];
  if (args.length > 2 || host === undefined || !Object.hasOwn(ADAPTER_CAPABILITIES, host)
      || (args.length !== 1 && !sessionId?.trim())) {
    throw new Error("Usage: rerail session [<host> [<session-id>]] [--open]; inside Codex, rerail session uses CODEX_THREAD_ID");
  }
  const cwd = claudeWorkspace(process.cwd());
  // Nothing autostarts where Rerail is off, so with no daemon answering the checkout's own state decides,
  // read without ever taking the write lock.
  const startState = (): RepositoryStartState | undefined => readOnlyStartState(cwd);
  const presentation = sessionId === undefined
    ? null
    : await readPresentation({ host: host as RerailHost, sessionId, cwd, onDemand: true, startState });
  const repositoryState = presentation === null ? await readSummaryState(cwd, host as RerailHost, startState) : presentation.repositoryState;
  const rulesUrl = (await readRulesDashboard({ cwd })).url;
  process.stdout.write(`${renderRerailSummary({
    presentation,
    repositoryState,
    ruleCount: presentation === null && repositoryState === "started" ? enabledRuleCount(cwd) : null,
    rulesUrl,
    repositoryName: repositoryName(cwd),
    sigil: host === "codex" ? "$" : "/",
  })}\n`);
  if (!open) return 0;
  const page = presentation?.url ?? rulesUrl;
  if (page === null) return 3;
  if (await openDashboard(page) === "orca-tab") process.stdout.write("Opened in the Orca window.\n");
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
 * `rerail start` / `rerail stop` act on the checkout of `--repo` or the current directory and the worktrees inside
 * it, or with `--repository` on every checkout of its repository. Starting a repository that is not set up runs
 * set-up there: its questions are answered with `--answer`, then `rerail setup`. Without a terminal, or with `--relay`,
 * the caller is a coding agent relaying the output, so the text names its slash commands and every answer carries
 * `--relay`. `--json` prints one set-up step.
 * `--database` names the database an extension was configured with, as `rerail state` takes it.
 */
async function runRepositoryCommand(command: "start" | "stop", args: readonly string[], context: SetupContext): Promise<number> {
  const io = setupIo(args);
  try {
    if (command === "start" && args.includes(JSON_FLAG)) {
      const step = await startStep(args, context, machinePlace(context));
      io.say(JSON.stringify(step));
      return step.step === "error" ? 1 : 0;
    }
    const { flags, options } = parseArguments(
      args,
      {
        flags: command === "start" ? ["--repository", SETUP_FLAGS.relay] : ["--repository"],
        options: command === "start" ? ["--repo", "--agents", "--answer", "--database"] : ["--repo", "--database"],
      },
      argument => `Unknown ${command} option: ${argument}\n\n${usage()}`,
    );
    const workspace = resolve(options.get("--repo") ?? process.cwd());
    const database = options.get("--database");
    const databasePath = database === undefined ? undefined : resolve(database);
    const repository = flags.has("--repository");
    if (command === "stop") {
      stopRerail(workspace, io, { ...(databasePath === undefined ? {} : { databasePath }), repository });
      return 0;
    }
    // A release from before set-up checks an update with a bare `start`, which set-up would only ask a question of.
    if (isPreSetupHealthCheck(args, io.lane, workspace)) {
      startForPreSetupHealthCheck(workspace, io);
      return 0;
    }
    const answer = checkSetupAnswer(options.get("--answer"));
    const { agents, agentsError } = setupAgents(options.get("--agents"), answer);
    const setupCommand = setupAnswerCommand(context, {
      ...(databasePath === undefined ? {} : { databasePath }),
      repository,
      ...(io.lane === "agent" ? { relay: true } : {}),
      ...(agents === undefined ? {} : { agents }),
    });
    const report = await startRerail(workspace, io, {
      ...(databasePath === undefined ? {} : { databasePath }),
      ...(agents === undefined ? {} : { agents }),
      ...(agentsError === undefined ? {} : { agentsError }),
      ...(answer === undefined ? {} : { answer }),
      ...(setupCommand === undefined ? {} : { setupCommand }),
      repository,
      environment: setupEnvironment(context),
    });
    // The set-up's own summary ends on the dashboard link; ending its questions early is a failure.
    if (report.setUpRan) return report.cancelled ? 1 : 0;
    const page = await readRulesDashboard({ cwd: workspace });
    if (page.url !== null) process.stdout.write(`  ${page.url}\n`);
    return 0;
  } finally {
    io.close();
  }
}

/**
 * `rerail state`: started, stopped or not-set-up for the checkout, for callers that cannot open SQLite (OMP runs on
 * Bun). `--database` names the database file when a caller configured one other than the home's default.
 */
/** `rerail state`: this checkout's start state; with `--set-up`, whether its repository is set up (`set-up` or `not-set-up`). */
function runStateCommand(args: readonly string[]): number {
  const { flags, options } = parseArguments(args, { flags: ["--set-up"], options: ["--repo", "--database"] }, argument => `Unknown state option: ${argument}\n\n${usage()}`);
  const databasePath = options.get("--database");
  const database = databasePath === undefined ? undefined : resolve(databasePath);
  const workspace = resolve(options.get("--repo") ?? process.cwd());
  const answer = flags.has("--set-up")
    ? (workspaceSetUp(workspace, database) ? "set-up" : "not-set-up")
    : workspaceStartState(workspace, database);
  process.stdout.write(`${answer}\n`);
  return 0;
}

/** Commands that read shared launcher state or this project's database and must run from whichever launcher the user invoked. */
function isVersionIndependentCommand(args: readonly string[]): boolean {
  const commandArgs = takeOption(args, "--profile").args;
  const command = commandArgs[0];
  // `setup` installs the package it was invoked from, so it must not forward to an older active version.
  // `seed`, `rules`, and `reset` write the Rerail database with the invoking package's schema (a checkout build, say).
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
  if (command === "rules" && (args[0] === "test" || args[0] === "add")) return await runRuleDraftCommand(args[0], args.slice(1), usage);
  if (command === "rules") return runRulesCommand(args, usage);
  if (command === "reset") return runResetCommand(args, usage);
  const daemonExit = await runDaemonCommand(command, args, home);
  if (daemonExit !== undefined) return daemonExit;
  if (command === "start" || command === "stop") return await runRepositoryCommand(command, args, { home, packageRoot, updater });
  if (command === "state") return runStateCommand(args);
  if (command === "scan" || command === "settings") return await runAuditCommand(command, args, { home });
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
