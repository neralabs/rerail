#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawn } from "../engine/child-process.js";
import { ADAPTER_CAPABILITIES, type RerailHost } from "../engine/adapter-contract.js";
import { ENTRY_POINTS, PACKAGE_ROOT as packageRoot, RERAIL_PACKAGE_NAME } from "../engine/package-root.js";
import { internalTesting } from "../engine/internal-testing.js";
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
  checkoutRuleCounts, readOnlyStartState, stopRerail, workspaceSetUp, workspaceStartState, type RepositoryStopOptions,
} from "../runtime/repository-start.js";
import type { RepositoryStartState } from "../engine/catalog/repository-model.js";
import { repositoryFolderName, repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { gitExecutable, GitUnavailableError } from "../engine/git.js";
import { checkSetupAnswer, setupAgents, setupPacks, startRerail } from "./repository-setup.js";
import { isPreSetupHealthCheck, skipsSessionsApproval, startForPreSetupHealthCheck } from "../runtime/pre-setup-health-check.js";
import {
  assertHostBindable,
  DAEMON_TRANSPORT_FLAGS,
  DAEMON_TRANSPORT_OPTIONS,
  DEFAULT_DAEMON_HOST,
  parseArguments,
  parseDaemonTransport,
  parsePort,
  parsePublicHosts,
} from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import {
  accessTokenKept,
  dashboardAccessLink,
  dashboardLink,
  lookupDaemon,
  openInBrowser,
  openDashboard,
  startDaemon,
  stopDaemon,
  DAEMON_ACCESS_TOKEN_FILE,
  DAEMON_LOG_FILE,
  type DaemonLookup,
  type RunningDaemon,
} from "../runtime/daemon-control.js";
import { deviceSecretStore } from "../activation/device-keystore.js";
import { runRerailMcpBridge } from "../api/mcp-bridge.js";
import { runResetCommand, runRulesCommand, updateWithSnapshot } from "./project-cli.js";
import { runScrubCommand } from "./scrub-cli.js";
import { lookupCheckout, parentCheckouts, recordCheckout } from "../engine/catalog/repository-registry.js";
import { listRepositoryRules, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { MOCK_CONVERSATION_RULES, MOCK_CODE_RULES, seedRerailDatabase } from "./seed.js";
import { runAuditCommand } from "./audit-cli.js";
import { describeRemoteStatus, runRemoteCommand } from "./remote-cli.js";
import { RemoteOutbox } from "../runtime/remote-outbox.js";
import { runRuleDraftCommand } from "./rule-draft.js";
import { runRuleEvidenceCommand } from "./rule-evidence.js";
import { runRuleApproveCommand } from "./rule-approval-cli.js";
import { runShowMeCommand } from "./show-me.js";
import { runSupportBundleCommand } from "./support-bundle.js";
import { runPacksCommand } from "./packs-cli.js";
import { runFeedbackCommand } from "./feedback-cli.js";
import {
  CARRIED_FLAG_NAMES, carriedFlags, errorStep, JSON_FLAG, machinePlace, runInstallCommand, runSetupCommand, setupAnswerCommand, setupEnvironment, setupIo,
  startStep, type CarriedFlags, type SetupContext,
} from "./setup-cli.js";
import { checkNodeVersion } from "../setup/node-version.js";
import { codingAgentHome, codingAgentStates, registrationRoot } from "../setup/coding-agents.js";
import { missingClaudeHookScripts } from "../setup/registered-hooks.js";
import { HOOK_FAILURE_WINDOW_MS, hookFailureLines, readHookFailures } from "../harness/hook-failures.js";
import { lastObservedEventAt } from "../runtime/session-store.js";
import { modelStatusLine } from "../engine/native/model-status.js";
import { SETUP_FLAGS } from "../harness/setup-screens.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import {
  ManagedUpdater,
  type UpdateResult,
  type UpdateStatus,
} from "../setup/updater.js";
import { packsLeftOutByCore } from "../setup/rule-pack-store.js";

const MANAGED_CHILD_ARGUMENT = "--rerail-managed-child";

interface PackageManifest {
  name: string;
  version: string;
}

function packageManifest(root: string): PackageManifest {
  const value = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as Record<string, unknown>;
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

/**
 * What an update did, and, when it changed the active core, the installed rule packs that version leaves
 * out by their core range: every repository that adds one runs none of its rules until a version that fits is in.
 */
function printUpdateResult(result: UpdateResult, home: string): void {
  process.stdout.write(result.changed
    ? `Activated Rerail ${result.current.version}; new harness sessions will use it.\n`
    : `Rerail ${result.current.version} is current for channel ${result.channel}.\n`);
  if (!result.changed) return;
  const leftOut = packsLeftOutByCore(home, result.current.version, result.previous?.version);
  if (leftOut.length === 0) return;
  process.stdout.write([
    `Rerail ${result.current.version} leaves out ${leftOut.length === 1 ? "this rule pack" : `these ${leftOut.length} rule packs`}, so repositories that add ${leftOut.length === 1 ? "it run none of its" : "them run none of their"} rules:`,
    ...leftOut.map(line => `  ${line}`),
    "Install a version that fits with `rerail packs add <package>`.",
  ].join("\n") + "\n");
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
    "  rerail install [--root <package>] [--agents <list>] [--remove [--delete-data]] [--dry-run]",
    "  rerail install --refresh",
    "  rerail start [--repo <dir>] [--repository] [--agents <list>] [--answer <answer>] [--json] [--relay] [--database <file>]",
    "  rerail stop [<host> [<session-id>]] [--repo <dir>] [--repository] [--database <file>]",
    "  rerail state [--set-up] [--repo <dir>] [--database <file>]",
    "  rerail status",
    "  rerail version",
    "  rerail daemon start [--dev] [--port <port>] [--host <host>] [--allow-remote] [--public-host <name,...>] [--tls-cert <pem> --tls-key <pem>] [--insecure-http] [--workspace-root <dir>] [--open]",
    "  rerail daemon stop",
    "  rerail session [<host> [<session-id>]] [--open]",
    "  rerail manage [--print]",
    "  rerail seed",
    "  rerail reset",
    "  rerail scrub",
    "  rerail packs [list]",
    "  rerail packs add <id[@version] | package[@version] | tarball | folder>...",
    "  rerail packs remove <id | package>...",
    "  rerail packs recommend [--repo <dir>]",
    "  rerail rules export [--output <rules.json>]",
    "  rerail rules import <rules.json> [--approve]",
    "  rerail rules test <draft.json | rule.json> [--repo <dir>] [--json]",
    "  rerail rules add <draft.json> [--repo <dir>]",
    "  rerail rules evidence [--repo <dir>] [--days <n>] [--limit <n>] [--json]",
    "  rerail rules approve [<rule id | pack> …] [--repo <dir>]",
    "  rerail show-me [--cleanup] [--repo <dir>] [--json]",
    "  rerail feedback <rule-id> <reason>",
    "  rerail feedback list [--rule <rule-id>]",
    "  rerail support-bundle [--session <id>] [--repo <dir>] [--output <file>] [--json]",
    "  rerail settings --session <id> [--scope repository|touched-files|ai-output] [--mode log|steer] [--json]",
    "  rerail update",
    "  rerail update check [--channel <tag>]",
    "  rerail update status",
    "  rerail update --channel <tag>",
    "",
    internalTesting(`Not part of the ${packageManifest(packageRoot).version} release:`),
    "  rerail scan --session <id> [--scope repository|git-diff] [--wait] [--json]",
    "  rerail scan [--repo <dir>] [--scope repository|git-diff] [--base <ref>] [--rules-ref <ref>] [--pack <id>]... [--path <p>]... [--paths-from <file>] [--json] [--fail-on-violations] [--no-send]",
    "  rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content]",
    "  rerail remote list|status [--json]",
    "  rerail remote remove <name|url>",
    "  rerail remote push [--json]",
    "  rerail remote exclude|include [--repo <dir>]",
    "  rerail settings --session <id> --steer-strategy message|rewind|subagent   experimental: takes effect with RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1",
    "  rerail [--profile <name>] events",
    "  rerail [--profile <name>] activate --file <profile.jws> --trust-root <roots.json>",
    "  rerail [--profile <name>] activate --code <code> --activation-url <url> --trust-root <roots.json>",
    "  rerail [--profile <name>] activation request --deployment <id> --output <file>",
    "  rerail [--profile <name>] profile [status|select|remove]",
    "  rerail --profile <name> update [check [--channel <tag>]]",
  ].join("\n");
}

function printDaemon(daemon: RunningDaemon): void {
  const { state, health } = daemon;
  process.stdout.write(`Rerail daemon ${state.version} running at ${state.url} (pid ${state.pid}, ${health.ui} UI)\n`);
  process.stdout.write(`Database: ${health.database.path}${health.database.exists ? "" : " (not created yet)"}\n`);
  process.stdout.write(health.native.ok
    ? `Engine: ok (version ${health.native.kernelVersion})\n`
    : `Engine: unavailable: ${health.native.error}\n`);
  process.stdout.write(`${modelStatusLine(health.model)}\n`);
}

async function runDaemonStartCommand(args: readonly string[], home: string): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dev", "--open", "--allow-remote", ...DAEMON_TRANSPORT_FLAGS], options: ["--port", "--host", "--public-host", "--workspace-root", ...DAEMON_TRANSPORT_OPTIONS] },
    argument => `Unknown daemon start option: ${argument}\n\n${usage()}`,
  );
  const port = options.get("--port");
  const host = options.get("--host");
  // The folder the daemon serves and looks for Git from, when the launcher runs elsewhere (autostart runs it in the home).
  const workspaceRoot = options.get("--workspace-root");
  if (host !== undefined) assertHostBindable(host, flags.has("--allow-remote"));
  const publicHosts = parsePublicHosts(options.get("--public-host"));
  const transport = parseDaemonTransport({ flags, options }, host ?? DEFAULT_DAEMON_HOST, publicHosts);
  const result = await startDaemon({ home, packageRoot }, {
    dev: flags.has("--dev"),
    ...(host === undefined ? {} : { host }),
    ...(flags.has("--allow-remote") ? { allowRemote: true } : {}),
    ...(publicHosts.length === 0 ? {} : { publicHosts }),
    ...(port === undefined ? {} : { port: parsePort(port) }),
    ...(workspaceRoot === undefined ? {} : { cwd: resolve(workspaceRoot) }),
    ...transport,
  });
  if (!result.started) process.stdout.write("The Rerail daemon is already running.\n");
  printDaemon(result.daemon);
  process.stdout.write(`Logs: ${join(home, DAEMON_LOG_FILE)}\n`);
  // The capability is shown to the person at a terminal, starting or finding the daemon; `rerail status` never
  // prints it, and neither does a start whose output is piped, as a coding agent's shell would be: the link would
  // land in its transcript and its provider's logs.
  const link = dashboardLink(result.daemon.state);
  if (result.daemon.state.accessToken !== undefined) {
    const lifetime = accessTokenKept(home, result.daemon.state)
      ? `it is kept until you delete ${join(home, DAEMON_ACCESS_TOKEN_FILE)}`
      : "it changes on every start";
    process.stdout.write(process.stdout.isTTY
      ? `Dashboard link (anyone holding it can read and change your rules; ${lifetime}): ${link}\n`
      : "The dashboard needs its access link in a browser; rerail manage opens it, or run rerail daemon start in a terminal to see it.\n");
  }
  if (flags.has("--open")) openInBrowser(link);
  return 0;
}

/** Whether a Git Rerail can run is installed, and where it looked when none is. */
function gitLookup(): { found: true } | { found: false; looked: readonly string[] } {
  try {
    gitExecutable();
    return { found: true };
  } catch (error) {
    if (error instanceof GitUnavailableError) return { found: false, looked: error.looked };
    throw error;
  }
}

async function runStatusCommand(home: string): Promise<number> {
  const lookup = await lookupDaemon(home);
  // No managed install, no database and no daemon: Rerail was never set up here, so it is not merely stopped.
  if (lookup.status === "absent" && !existsSync(join(home, "current")) && !existsSync(join(home, RERAIL_DATABASE_FILE))) {
    process.stdout.write("Rerail is not installed on this computer. Install it with npx @neralabs/rerail setup.\n");
    return 3;
  }
  const workspace = process.cwd();
  const state = workspaceStartState(workspace);
  const location = repositoryIdentityLocation(workspace);
  const name = repositoryFolderName(location);
  if (!location.git && state === "stopped") {
    // Without a Git Rerail can run, every folder reads as no repo; say what is actually missing.
    const git = gitLookup();
    process.stdout.write(git.found
      ? `No repo found here (${workspace}). Run rerail status inside your project.\n`
      : `Rerail needs Git to tell whether this is a repo, and found none where it looks: ${git.looked.join(", ")}. Install Git, then run rerail status again.\n`);
  } else if (state === "stopped") process.stdout.write(`Rerail is stopped in ${name}. Start it with rerail start.\n`);
  else if (state === "not-set-up") process.stdout.write(`Rerail is on in ${name}, which is not set up yet. Set it up with rerail start.\n`);
  else process.stdout.write(`Rerail is on in ${name}.\n`);
  const code = printDaemonStatus(lookup);
  for (const line of hookHealthLines(home)) process.stdout.write(`${line}\n`);
  // Internal testing: the remote sink, only when a remote is added.
  try {
    for (const remote of new RemoteOutbox({ home }).status()) process.stdout.write(`${describeRemoteStatus(remote)}\n`);
  } catch (error) {
    process.stdout.write(`Remote sink: status unreadable (${error instanceof Error ? error.message : String(error)})\n`);
  }
  // Last, for each connected coding agent, whether its tools can read Rerail's data on this computer.
  for (const agent of await codingAgentStates(registrationRoot(home, packageRoot), codingAgentHome())) {
    // Each sentence names its coding agent.
    if (agent.connected) process.stdout.write(`${agent.rerailData.why}\n`);
  }
  return code;
}

/**
 * What `rerail status` says of the hooks: Claude Code's registration naming Rerail scripts that are gone, where every
 * hook fails before Rerail runs, and the hook events Rerail could not record in the last day, with why.
 */
function hookHealthLines(home: string): string[] {
  const missing = missingClaudeHookScripts(codingAgentHome());
  const registration = missing.length === 0 ? [] : [
    `Claude Code's Rerail hooks run ${missing.join(", ")}, which ${missing.length === 1 ? "is" : "are"} missing, so every hook fails before Rerail runs and nothing is recorded. Run rerail install --refresh.`,
  ];
  const failures = readHookFailures(home, Date.now() - HOOK_FAILURE_WINDOW_MS);
  const recorded = failures.length === 0 ? null : lastObservedEventAt(join(home, RERAIL_DATABASE_FILE));
  return [...registration, ...hookFailureLines(failures, recorded)];
}

/** What `rerail status` says of the daemon, and the exit code that goes with it. */
function printDaemonStatus(lookup: DaemonLookup): number {
  if (lookup.status === "absent") {
    process.stdout.write("The Rerail daemon is not running. Start it with rerail daemon start.\n");
    return 3;
  }
  if (lookup.status === "unresponsive") {
    process.stdout.write(`Rerail daemon pid ${lookup.state.pid} is alive but not answering at ${lookup.state.url}; run rerail daemon stop, then rerail daemon start.\n`);
    return 4;
  }
  if (lookup.status === "refused") {
    process.stdout.write(`Rerail daemon pid ${lookup.state.pid} at ${lookup.state.url} refused the access capability in daemon.json (401); run rerail daemon stop, then rerail daemon start.\n`);
    return 4;
  }
  printDaemon({ state: lookup.state, health: lookup.health });
  return lookup.health.healthy === false ? 5 : 0;
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

/**
 * The rules page of the daemon serving this checkout; `--print` only reports the URL. At a terminal the printed URL
 * carries the daemon's capability, so it opens in any browser of this machine; piped, as a coding agent runs it, the
 * URL is the plain one.
 */
async function runManageCommand(args: readonly string[]): Promise<number> {
  const { flags } = parseArguments(args, { flags: ["--print"], options: [] }, argument => `Unknown manage option: ${argument}\n\n${usage()}`);
  const page = await readRulesDashboard({ cwd: claudeWorkspace(process.cwd()) });
  if (page.url === null) {
    process.stdout.write(`Rerail is not running for this checkout (${page.reason}). Check rerail status, or start the daemon with rerail daemon start --open.\n`);
    return 3;
  }
  process.stdout.write(`${process.stdout.isTTY ? await dashboardAccessLink(page.url, defaultRerailHome()) : page.url}\n`);
  if (!flags.has("--print") && await openDashboard(page.url) === "orca-tab") process.stdout.write("Opened in the Orca window.\n");
  return 0;
}

async function dispatchToManagedVersion(args: readonly string[], updater: ManagedUpdater): Promise<number | undefined> {
  const state = await updater.readState();
  if (resolve(state.current.packageRoot) === packageRoot) return undefined;
  const managedCli = join(state.current.packageRoot, ENTRY_POINTS.cli);
  if (!existsSync(managedCli)) {
    throw new Error(`Active Rerail launcher is missing: ${managedCli}. Run npx ${RERAIL_PACKAGE_NAME} setup to install Rerail again.`);
  }
  return await runChild(process.execPath, [managedCli, MANAGED_CHILD_ARGUMENT, ...args]);
}

/**
 * The persistent event stream for out-of-process hosts. It never checks for or installs updates; `rerail update` is the only way a new version arrives.
 * Internal testing: `rerail events`; the hooks of the released coding agents do not use it.
 */
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
  if (action !== undefined && action !== "--channel") throw new Error(`Unknown update action: ${action}`);
  printUpdateResult(await updateWithSnapshot(updater, join(home, RERAIL_DATABASE_FILE), channel), home);
  // Newer version or not, the version `current` names registers itself again, as its own CLI, with every coding agent
  // already connected (`install --refresh`): its commands, skills and hooks arrive and the ones it retired go. A failure
  // leaves that version active and exits non-zero, naming the command to run once the configuration is fixed. Reading
  // the Claude Code status-line record loads `node:sqlite`, whose experimental warning is not the update's to report.
  return await runChild(process.execPath, [
    "--disable-warning=ExperimentalWarning",
    join(updater.currentLink, ENTRY_POINTS.cli),
    MANAGED_CHILD_ARGUMENT,
    "install",
    "--refresh",
  ]);
}

/**
 * Rules on in `workspace`'s checkout, and how many of them run in Log waiting for a person's approval, from its
 * repository's configuration, for `/rerail` where the coding agent cannot name its session. A status read: it never
 * writes, and where the configuration cannot be read (busy, or the checkout not recorded) the summary drops the count
 * lines rather than failing.
 */
function repositoryRuleCounts(workspace: string): { on: number; awaitingApproval: number } | null {
  const databasePath = defaultRerailDatabasePath();
  if (!existsSync(databasePath)) return null;
  let database;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    const checkout = lookupCheckout(database, workspace);
    if (checkout === undefined) return null;
    return checkoutRuleCounts(database, checkout);
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
  const counts = presentation === null && repositoryState === "started" ? repositoryRuleCounts(cwd) : null;
  process.stdout.write(`${renderRerailSummary({
    presentation,
    repositoryState,
    ruleCount: counts?.on ?? null,
    awaitingApproval: counts?.awaitingApproval ?? null,
    rulesUrl,
    repositoryName: repositoryName(cwd),
    sigil: ADAPTER_CAPABILITIES[host as RerailHost].commandSigil,
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
  if (command === "manage") return await runManageCommand(args);
  return undefined;
}

/**
 * The coding agent `rerail stop [<host> [<session-id>]]` was typed in, named as `rerail session` names it. The host
 * decides the reply's command sigil; `codex` without an id reads its thread from `CODEX_THREAD_ID`. An id the coding
 * agent left empty names no session, and a bare `stop` (a terminal, or Cursor) names no agent: the stop never depends
 * on knowing either.
 */
function stopSession(positionals: readonly string[]): RepositoryStopOptions["session"] {
  if (positionals.length === 0) return undefined;
  const [host, given] = positionals;
  if (positionals.length > 2 || host === undefined || !Object.hasOwn(ADAPTER_CAPABILITIES, host)) {
    throw new Error(`Usage: rerail stop [<host> [<session-id>]] [options]; rerail stop codex uses CODEX_THREAD_ID\n\n${usage()}`);
  }
  const sessionId = (given ?? (host === "codex" ? process.env.CODEX_THREAD_ID : undefined))?.trim();
  return sessionId ? { host: host as RerailHost, sessionId } : { host: host as RerailHost };
}

/**
 * `rerail start` / `rerail stop` act on the checkout of `--repo` or the current directory and the worktrees inside
 * it, or with `--repository` on every checkout of its repository. Starting a repository that is not set up runs
 * set-up there: its questions are answered with `--answer`, then `rerail setup`. Without a terminal, or with `--relay`,
 * the caller is a coding agent relaying the output, so the text names its slash commands and every answer carries
 * `--relay`. `--json` prints one set-up step. `stop` takes the coding agent's session first, as `rerail session` does,
 * so its reply names the rules that steered that session.
 * `--database` names the database an extension was configured with, as `rerail state` takes it.
 */
async function runRepositoryCommand(command: "start" | "stop", rawArgs: readonly string[], context: SetupContext): Promise<number> {
  const firstOption = rawArgs.findIndex(argument => argument.startsWith("--"));
  const positionals = command === "stop" ? rawArgs.slice(0, firstOption === -1 ? rawArgs.length : firstOption) : [];
  const args = rawArgs.slice(positionals.length);
  const io = setupIo(args);
  try {
    if (command === "stop") return runStopCommand(args, positionals, io);
    if (args.includes(JSON_FLAG)) {
      const step = await startStep(args, context, machinePlace(context));
      io.say(JSON.stringify(step));
      return step.step === "error" ? 1 : 0;
    }
    return await runStartCommand(args, context, io);
  } finally {
    io.close();
  }
}

/** The options that have a value: `startRerail`'s, `stopRerail`'s and `setupAnswerCommand`'s take none as absent, not `undefined`. */
function definedOnly<T extends object>(options: T): { [K in keyof T]?: Exclude<T[K], undefined> } {
  return Object.fromEntries(Object.entries(options).filter(([, value]) => value !== undefined)) as { [K in keyof T]?: Exclude<T[K], undefined> };
}

/** `databasePath` when `--database` names one, as `startRerail` and `stopRerail` take it. */
function databasePathOption(options: ReadonlyMap<string, string>): { databasePath?: string } {
  const database = options.get("--database");
  return database === undefined ? {} : { databasePath: resolve(database) };
}

function runStopCommand(args: readonly string[], positionals: readonly string[], io: ReturnType<typeof setupIo>): number {
  const { flags, options } = parseArguments(args, { flags: ["--repository"], options: ["--repo", "--database"] }, argument => `Unknown stop option: ${argument}\n\n${usage()}`);
  const workspace = resolve(options.get("--repo") ?? process.cwd());
  const session = stopSession(positionals);
  stopRerail(workspace, io, {
    ...databasePathOption(options),
    repository: flags.has("--repository"),
    ...definedOnly({ session }),
  });
  return 0;
}

async function runStartCommand(args: readonly string[], context: SetupContext, io: ReturnType<typeof setupIo>): Promise<number> {
  const { flags, options } = parseArguments(
    args,
    { flags: [SETUP_FLAGS.relay, ...CARRIED_FLAG_NAMES], options: ["--repo", "--agents", SETUP_FLAGS.packs, "--answer", "--database"] },
    argument => `Unknown start option: ${argument}\n\n${usage()}`,
  );
  const workspace = resolve(options.get("--repo") ?? process.cwd());
  // A release from before set-up checks an update with a bare `start`, which set-up would only ask a question of.
  if (isPreSetupHealthCheck(args, io.lane, workspace)) {
    startForPreSetupHealthCheck(workspace, io);
    return 0;
  }
  const carried = carriedFlags(({ flag }) => flags.has(flag));
  const report = await startRerail(workspace, io, startOptions(args, carried, options, context, { lane: io.lane, workspace }));
  // The set-up's own summary ends on the dashboard link; ending its questions early is a failure.
  if (report.setUpRan) return report.cancelled ? 1 : 0;
  const page = await readRulesDashboard({ cwd: workspace });
  if (page.url !== null) process.stdout.write(`  ${page.url}\n`);
  return 0;
}

/** What `rerail start` passes to `startRerail`: the answers given so far, and how the set-up may ask for the rest. */
function startOptions(
  args: readonly string[], carried: CarriedFlags, options: ReadonlyMap<string, string>, context: SetupContext,
  { lane, workspace }: { lane: ReturnType<typeof setupIo>["lane"]; workspace: string },
): Parameters<typeof startRerail>[2] {
  const answer = checkSetupAnswer(options.get("--answer"));
  const { agents, agentsError } = setupAgents(options.get("--agents"), answer);
  const packs = setupPacks(options.get(SETUP_FLAGS.packs), answer);
  const database = databasePathOption(options);
  const setupCommand = setupAnswerCommand(context, {
    ...database, ...carried, ...(lane === "agent" ? { relay: true } : {}), ...definedOnly({ agents }),
  });
  const environment = setupEnvironment(context);
  // An update's health check (`skipsSessionsApproval`) reads no one's sessions, so it is not asked to allow it, and
  // its "I'll pick rule packs myself" starts with none, as `--packs none` does: no pack fits its empty folder, the
  // releases that run it expect Rerail on there, and it fetches nothing from npm.
  const healthCheck = skipsSessionsApproval(args, lane, workspace, () => environment.agentsFound());
  return {
    ...database,
    ...definedOnly({ agents, agentsError, answer, setupCommand }),
    ...(healthCheck ? { packs: [] } : packs),
    repository: carried.repository === true,
    environment: healthCheck ? { ...environment, sessionsApproved: () => true } : environment,
  };
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

/** `rerail version`, or the flags people try first. */
function isVersionRequest(command: string | undefined): boolean {
  return command === "version" || command === "--version" || command === "-v";
}

/** Commands that read shared launcher state or this project's database and must run from whichever launcher the user invoked. */
function isVersionIndependentCommand(args: readonly string[]): boolean {
  const commandArgs = takeOption(args, "--profile").args;
  const command = commandArgs[0];
  // `setup` installs the package it was invoked from, so it must not forward to an older active version.
  // `seed`, `rules`, `show-me`, `reset`, `scrub` and `feedback` write the Rerail database with the invoking package's schema (a checkout build, say).
  // `support-bundle` reports the version it was invoked as, and has to run where the active version cannot.
  if (["setup", "install", "start", "stop", "state", "status", "session", "manage", "seed", "rules", "show-me", "reset", "scrub", "support-bundle", "feedback", "remote"].includes(command ?? "")) return true;
  // A headless `scan` (no `--session`) runs in this process, so it scans with the package invoked.
  if (command === "scan") return !commandArgs.includes("--session");
  if (command === "daemon") return commandArgs[1] === "stop";
  return command === "update" && commandArgs[1] === "status";
}

/**
 * The launcher runs `node` from PATH once the node set-up recorded is gone, and that one can be older than Rerail runs
 * on. Every command but `setup`, which makes the same check in its own output, refuses there with set-up's message,
 * as one set-up step under `--json`; undefined where this Node is new enough.
 */
function refuseUnsupportedNode(args: readonly string[]): number | undefined {
  if (takeOption(args, "--profile").args[0] === "setup") return undefined;
  const json = args.includes(JSON_FLAG);
  const terminal = !args.includes(SETUP_FLAGS.relay) && process.stdin.isTTY && process.stdout.isTTY;
  const node = checkNodeVersion(process.versions.node, json ? "json" : terminal ? "terminal" : "agent");
  if (node.ok) return undefined;
  if (json) process.stdout.write(`${JSON.stringify(errorStep(node.message.replace(/\n$/u, "")))}\n`);
  else process.stderr.write(node.message);
  return 1;
}

async function main(): Promise<number> {
  const manifest = packageManifest(packageRoot);
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
  const tooOld = refuseUnsupportedNode(args);
  if (tooOld !== undefined) return tooOld;
  if (args[0] === "mcp-bridge") {
    if (args.length !== 1) throw new Error("mcp-bridge takes no arguments");
    await runRerailMcpBridge();
    return 0;
  }
  // The version of the package answering: through the launcher, the active one. Nothing else is read for it, and a
  // `--profile` (internal testing) changes nothing about it.
  const versionArgs = takeOption(args, "--profile").args;
  if (isVersionRequest(versionArgs[0])) {
    if (versionArgs.length !== 1) throw new Error(`${versionArgs[0]} takes no arguments`);
    process.stdout.write(`${manifest.name.split("/").pop()} ${manifest.version}\n`);
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

/** Internal testing: activation profiles (`activate`, `activation request`, `profile`, `--profile`). */
async function runAccountCommand(command: string, args: readonly string[], context: CommandContext): Promise<number | undefined> {
  const { activation, profileStore, packageVersion, profile } = context;
  if (command === "activate") return await runActivateCommand(args, activation, packageVersion, profile);
  if (command === "activation") return await runActivationRequestCommand(args, activation, packageVersion, profile);
  if (command === "profile") return await runProfileCommand(args, activation, profileStore, profile);
  return undefined;
}

/** `rerail rules` and its subcommands. */
async function runRulesFamilyCommand(args: readonly string[]): Promise<number> {
  const [subcommand] = args;
  if (subcommand === "test" || subcommand === "add") return await runRuleDraftCommand(subcommand, args.slice(1), usage);
  if (subcommand === "evidence") return await runRuleEvidenceCommand(args.slice(1), usage);
  if (subcommand === "approve") return runRuleApproveCommand(args.slice(1), usage);
  return runRulesCommand(args, usage);
}

/** Every command that runs without the signed update policy; undefined for a word that is none of them. */
async function runProjectCommand(command: string, args: readonly string[], { home, updater, packageVersion }: CommandContext): Promise<number | undefined> {
  switch (command) {
    case "setup": return await runSetupCommand(args, { home, packageRoot, updater });
    case "install": return await runInstallCommand(args, { home, packageRoot, updater });
    case "session": return await runSessionCommand(args);
    case "seed": return runSeedCommand(args);
    case "rules": return await runRulesFamilyCommand(args);
    case "show-me": return runShowMeCommand(args, usage);
    case "support-bundle": return await runSupportBundleCommand(args, usage, { packageVersion });
    case "packs": return await runPacksCommand(args, usage);
    case "reset": return runResetCommand(args, usage);
    case "feedback": return await runFeedbackCommand(args, usage, { packageVersion });
    case "scrub": return await runScrubCommand(args, usage);
    case "start":
    case "stop":
      return await runRepositoryCommand(command, args, { home, packageRoot, updater });
    case "state": return runStateCommand(args);
    // Internal testing: `scan`, explicit full-repository and git-diff audits. Whether `settings` ships is still open.
    case "scan":
    case "settings":
      return await runAuditCommand(command, args, { home, version: packageVersion });
    // Internal testing: the remote sink.
    case "remote": return await runRemoteCommand(args, { home });
    // `daemon`, `status` and `manage`.
    default: return await runDaemonCommand(command, args, home);
  }
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

// `rerail setup … | head -2` closes the pipe while Rerail still has lines to write. The reader chose not to see them:
// no stack trace, and the command still finishes what it started (an install half done would hold its lock).
for (const stream of [process.stdout, process.stderr]) {
  stream.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code !== "EPIPE") throw error;
  });
}

try {
  process.exitCode = await main();
} catch (error) {
  process.stderr.write(`rerail: ${(error as Error).message}\n`);
  process.exitCode = 1;
}
