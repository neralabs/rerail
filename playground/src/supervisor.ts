#!/usr/bin/env node
import { access, copyFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { constants, createReadStream } from "node:fs";
import { delimiter, dirname, join, relative } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import { createServer } from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { Readable } from "node:stream";
import { WebSocket, WebSocketServer } from "ws";
import { watch } from "chokidar";
import {
  childCommandSchema,
  childEventSchema,
  controlSchema,
} from "./protocol.js";
import { playgroundOpenApiDocument } from "./openapi.js";
import { allowedOrigins } from "./origins.js";
import { assetManifestLoader, selectAsset } from "./static-assets.js";
import type {
  PlaygroundChildCommand,
  PlaygroundControl,
  PlaygroundEvent,
  PlaygroundExperiment,
  PlaygroundRuleList,
  PlaygroundRuleSnapshot,
  PlaygroundRunInput,
  PlaygroundTrace,
} from "./protocol.js";
import {
  codeRuleSchema,
  defaultRerailDatabasePath,
  loadBundledRules,
  openScopedRuleCatalog,
  parseConversationRule,
  ruleDocumentDigest,
  startDaemon,
  stopDaemon,
  storedRuleSchema,
} from "../../dist/index.js";
import { createSandbox, managedProcess, spawnProcess, spawnPty, ompExecutable } from "./process.js";
import type { ManagedProcess, PlaygroundProcess, Sandbox } from "./process.js";
import { createWorkerChannel } from "./ipc.js";
import type { WorkerChannel } from "./ipc.js";
import { prepareReplay, REAL_AGENT_LIFETIME_MS } from "./real-agent.js";
import { createExperimentStore, validateScenarioFixtures } from "./experiments.js";
import { generateScenarios } from "./generation.js";
import { executeSandboxCommand } from "./commands.js";
import { daemonJson } from "./readiness.js";

if (process.env.RERAIL_DEV !== "1") throw new Error("Rule playground is available through `just rerail-playground` only.");
// dist/supervisor.js → playground/ → checkout root, which is the Rerail package itself.
const playgroundRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const checkoutRoot = dirname(playgroundRoot);
const harnessPath = join(playgroundRoot, "dist", "harness.js");
const clientAssets = assetManifestLoader(join(playgroundRoot, "dist", "ui"));
const hostHome = process.env.HOME ?? homedir();
const daemonPort = process.env.RERAIL_PLAYGROUND_DAEMON_PORT ?? "4737";
if (!/^\d+$/.test(daemonPort) || Number(daemonPort) < 1024 || Number(daemonPort) > 65535)
  throw new Error("RERAIL_PLAYGROUND_DAEMON_PORT must be 1024–65535");
const daemonOrigin = `http://127.0.0.1:${daemonPort}`;
const store = createExperimentStore(checkoutRoot);
const MAX_MESSAGE_BYTES = 4 * 1_048_576;
const MAX_EVENT_BYTES = 16 * 1_048_576;
const RERUN_INTERVAL_MS = 30_000;
interface Connection {
  messages: number;
  inputBytes: number;
  window: number;
  ruleId: string;
}
type PlaygroundSocket = WebSocket & { data: Connection };
let owner: PlaygroundSocket | undefined;
let experiment: PlaygroundExperiment | undefined;
let rule: PlaygroundRuleSnapshot | undefined;
let cols = 100;
let rows = 30;
let revision = 0;
let active: ManagedProcess | undefined;
let activeSandbox: Sandbox | undefined;
let activeDeadline: NodeJS.Timeout | undefined;
let activeShell: { id: string; abort: AbortController; done: Promise<void> } | undefined;
let generation: AbortController | undefined;
let generationPending = false;
let replayCleanup: Promise<void> = Promise.resolve();
let commandQueue: Promise<void> = Promise.resolve();
let persistence: Promise<void> = Promise.resolve();
let loopEnabled = false;
let lastStartedAt = 0;
let lastRunDigest: string | undefined;
let rerunTimer: NodeJS.Timeout | undefined;
let polling = false;
let closing = false;
let build: ManagedProcess | undefined;
let nativeSandbox: Sandbox | undefined;
let nativeDirty = false;
let sourceRevision = 0;
let builtSourceRevision = 0;

function send(socket: PlaygroundSocket, text: string): void {
  if (socket.readyState !== WebSocket.OPEN) return;
  if (socket.bufferedAmount > MAX_EVENT_BYTES || Buffer.byteLength(text) > MAX_EVENT_BYTES) {
    socket.close(1013, "Experiment exceeds the browser transport limit");
    return;
  }
  socket.send(text);
}
function publish(event: PlaygroundEvent): void {
  if (owner) send(owner, JSON.stringify(event));
}
function status(state: Extract<PlaygroundEvent, { type: "status" }>["state"], message: string): void {
  publish({ type: "status", state, message });
}
function publishExperiment(): void {
  if (experiment && rule) publish({ type: "experiment", experiment, rule });
}
function persist(): Promise<void> {
  if (!experiment) return persistence;
  const snapshot = structuredClone(experiment);
  persistence = persistence.catch(() => undefined).then(() => store.save(snapshot));
  return persistence;
}
/** Message plus every cause, so a wrapped failure is never reported as a bare parse error. */
function describeFailure(error: unknown): string {
  const lines: string[] = [];
  let current: unknown = error;
  while (current !== undefined && lines.length < 8) {
    if (current instanceof Error) lines.push(`${current.name}: ${current.message}`);
    // A thrown object would otherwise reach the client as "[object Object]".
    else if (typeof current === "object" && current !== null) lines.push(JSON.stringify(current).slice(0, 2_048));
    else lines.push(String(current));
    current = current instanceof Error ? current.cause : undefined;
  }
  return lines.join("\n  caused by: ");
}
/** One daemon rule row as the client and worker see it; the daemon schema is the only parser. */
function toSnapshot(value: unknown): PlaygroundRuleSnapshot {
  const stored = storedRuleSchema.parse(value);
  return {
    id: stored.rule.id,
    kind: stored.kind,
    definition: JSON.stringify(stored.rule),
    digest: ruleDocumentDigest(stored),
    updatedAt: String(stored.updatedAt),
    enabled: stored.enabled,
  };
}
async function readRule(id: string): Promise<PlaygroundRuleSnapshot> {
  const body = await daemonJson(
    `${daemonOrigin}/api/rules/${encodeURIComponent(id)}`,
    5_000,
    "Saved rule unavailable; check the development daemon and selected rule",
  );
  return toSnapshot(typeof body === "object" && body !== null && "rule" in body ? body.rule : undefined);
}
/** List rows annotate each stored rule with its pack; the snapshot, and so its digest, is the stored rule alone. */
function storedRuleOfListRow(row: unknown): unknown {
  if (typeof row !== "object" || row === null || !("packId" in row)) return row;
  const { packId: _packId, ...stored } = row;
  return stored;
}
/** The rule picker reads through the supervisor so the browser stays on one origin. */
async function readRules(): Promise<PlaygroundRuleList> {
  const body = await daemonJson(
    `${daemonOrigin}/api/rules`,
    5_000,
    "Saved rules unavailable; check the development daemon",
  );
  const rules = typeof body === "object" && body !== null && "rules" in body ? body.rules : undefined;
  if (!Array.isArray(rules)) throw new Error("Development daemon returned no rule list");
  return { rules: rules.map((row: unknown) => toSnapshot(storedRuleOfListRow(row))) };
}
function stopReplay(): Promise<void> {
  clearTimeout(activeDeadline);
  activeDeadline = undefined;
  const previous = active;
  const sandbox = activeSandbox;
  const command = activeShell;
  command?.abort.abort(new Error("Replay command cancelled."));
  activeShell = undefined;
  active = undefined;
  activeSandbox = undefined;
  if (previous || sandbox)
    replayCleanup = replayCleanup
      .catch(() => undefined)
      .then(async () => {
        try {
          await Promise.all([previous?.stop(), command?.done]);
        } finally {
          await sandbox?.remove();
        }
      });
  return replayCleanup;
}
function cancel(message: string, nextState: "stopped" | "generating" = "stopped"): number {
  const version = ++revision;
  loopEnabled = false;
  clearTimeout(rerunTimer);
  rerunTimer = undefined;
  generation?.abort(new Error(message));
  generation = undefined;
  generationPending = false;
  void stopReplay().catch((error) => status("error", `Cancellation failed: ${String(error)}`));
  void build?.stop().catch((error) => status("error", String(error)));
  status(nextState, message);
  return version;
}
async function restore(id: string, version: number): Promise<void> {
  const [saved, current] = await Promise.all([store.load(id), readRule(id)]);
  if (version !== revision || !owner || closing) return;
  rule = current;
  experiment = saved ?? {
    ruleId: id,
    selection: { ruleId: id, scenarioId: "", mode: "trigger", allowCommands: false },
    scenarios: [],
  };
  publish({ type: "reset" });
  publishExperiment();
  status(
    "stopped",
    "Experiment restored. Run starts evaluation and automatic reruns; restoring never starts paid work.",
  );
}

interface ChildCommandContext {
  version: number;
  child: PlaygroundProcess;
  cwd: string;
  allowCommands: boolean;
}
/** Supervisor side of the worker's shell requests: one bounded command at a time, cancellable by id. */
function handleChildCommand(request: PlaygroundChildCommand, context: ChildCommandContext): void {
  if (request.type === "cancel-command") {
    if (activeShell?.id === request.id) activeShell.abort.abort(new Error("Tool command cancelled."));
    return;
  }
  const reply = (ok: boolean, output: string) => {
    if (context.version === revision && active?.child === context.child && !closing)
      context.child.send({ type: "command-result", id: request.id, ok, output: output.slice(0, 131_072) });
  };
  if (!context.allowCommands || activeShell) {
    reply(false, "Commands are disabled or another command is still running.");
    return;
  }
  const command = { id: request.id, abort: new AbortController(), done: Promise.resolve() };
  activeShell = command;
  command.done = executeSandboxCommand(context.cwd, request.command, command.abort.signal)
    .then(
      (output) => reply(true, output),
      (error) => reply(false, String(error)),
    )
    .finally(() => {
      if (activeShell === command) activeShell = undefined;
    });
}

function runWasCancelled(version: number): boolean {
  return version !== revision || !owner || closing;
}
function isActiveRun(version: number, managed: ManagedProcess): boolean {
  return version === revision && active === managed && !closing;
}
function activateSandboxRule(current: PlaygroundRuleSnapshot, sandbox: Sandbox): void {
  const definition: unknown = JSON.parse(current.definition);
  const document = current.kind === "conversation"
    ? { kind: "conversation" as const, rule: parseConversationRule(definition) }
    : { kind: "file" as const, rule: codeRuleSchema.parse(definition) };
  if (document.rule.id !== current.id || ruleDocumentDigest(document) !== current.digest)
    throw new Error("Selected rule definition does not match its id and digest.");
  const catalog = openScopedRuleCatalog(sandbox.cwd, defaultRerailDatabasePath(sandbox.env));
  try {
    catalog.seed(loadBundledRules());
    catalog.setEnabled({ filter: {} }, false);
    if (catalog.get(document.rule.id)?.kind !== document.kind) catalog.remove([document.rule.id]);
    catalog.upsertDocument(document, { enabled: true, sourceDigest: null });
  } finally { catalog.close(); }
}

async function run(version: number): Promise<void> {
  await stopReplay();
  if (runWasCancelled(version) || !experiment) return;
  const current = await readRule(experiment.ruleId);
  if (runWasCancelled(version)) return;
  rule = current;
  const scenario = experiment.scenarios.find((item) => item.id === experiment!.selection.scenarioId);
  if (!scenario) {
    loopEnabled = false;
    status("error", "Generate or select a scenario before running.");
    publishExperiment();
    return;
  }
  const input: PlaygroundRunInput = structuredClone({ selection: experiment.selection, rule: current, scenario });
  lastRunDigest = current.digest;
  lastStartedAt = Date.now();
  publish({ type: "reset" });
  publishExperiment();
  if (nativeDirty && !(await rebuildNative(version))) return;
  if (runWasCancelled(version)) return;
  const traces: PlaygroundTrace[] = [];
  let completed = false;
  const complete = (outcome: Extract<PlaygroundEvent, { type: "outcome" }>["outcome"]) => {
    if (completed || version !== revision || !experiment) return;
    completed = true;
    experiment = { ...experiment, lastRun: { ...input, outcome, completedAt: Date.now(), trace: [...traces] } };
    publish({ type: "outcome", outcome });
    publishExperiment();
    void persist().catch((error) => status("error", `Could not save result: ${String(error)}`));
  };
  if (scenario.ruleKind !== current.kind) {
    complete({
      observed: "cannot-exercise",
      detail: "Scenario was generated for another rule kind. Generate or revise scenarios explicitly.",
    });
    status("stopped", "Incompatible scenario; automatic reruns remain armed for compatible database changes.");
    return;
  }
  if (sourceRevision !== builtSourceRevision && !(await rebuildSources(version))) return;
  status("starting", "Preparing a fresh isolated OMP worker; only the selected saved rule is activated.");
  const sandbox = await createSandbox();
  const removeSandbox = sandbox.remove;
  let channel: WorkerChannel | undefined;
  sandbox.remove = async () => {
    try { await channel?.close(); }
    finally {
      try { await stopDaemon(sandbox.env.RERAIL_HOME!); }
      finally { await removeSandbox(); }
    }
  };
  try {
    activateSandboxRule(current, sandbox);
    const started = await startDaemon({ home: sandbox.env.RERAIL_HOME!, packageRoot: checkoutRoot }, {
      dev: true, port: 0, cwd: sandbox.cwd,
      env: sandbox.env,
    });
    sandbox.env.RERAIL_PLAYGROUND_DAEMON_URL = started.daemon.state.url;
    const profile = await prepareReplay(sandbox, hostHome, input.selection.mode === "agent");
    if (runWasCancelled(version)) {
      await sandbox.remove();
      return;
    }
    const inputPath = join(sandbox.root, "run-input.json");
    await writeFile(inputPath, JSON.stringify(input), { mode: 0o600, flag: "wx" });
    const ompConfigPath = join(sandbox.root, "omp-isolation.yml");
    await writeFile(
      ompConfigPath,
      [
        "setupVersion: 2",
        "startup:",
        "  quiet: true",
        "  checkUpdate: false",
        "memory:",
        "  backend: \"off\"",
        "compaction:",
        "  enabled: false",
        "retry:",
        "  enabled: false",
        "  modelFallback: false",
        "  usageAwareFallback: false",
        "title:",
        "  refreshOnReplan: false",
        "plan:",
        "  defaultOnStartup: false",
        "prewalk:",
        "  enabled: false",
        "advisor:",
        "  enabled: false",
        "providers:",
        "  openaiWebsockets: \"off\"",
        "codexResets:",
        "  autoRedeem: \"no\"",
        "mcp:",
        "  enableProjectConfig: false",
      ].join("\n"),
      { mode: 0o600, flag: "wx" },
    );
    if (version !== revision || closing) {
      await sandbox.remove();
      return;
    }
    let child: PlaygroundProcess | undefined;
    channel = await createWorkerChannel(join(sandbox.root, "worker.sock"), (message: unknown) => {
      if (version !== revision || closing || !child) return;
      const requested = childCommandSchema.safeParse(message);
      if (requested.success) {
        handleChildCommand(requested.data, { version, child, cwd: sandbox.cwd,
          allowCommands: input.selection.allowCommands });
        return;
      }
      const parsed = childEventSchema.safeParse(message);
      if (!parsed.success) return;
      const event = parsed.data;
      if (event.type === "trace") {
        traces.push(event.entry);
        if (traces.length > 250) traces.shift();
      }
      if (event.type === "outcome") complete(event.outcome);
      else publish(event);
    });
    const managed = await spawnPty(
      [
        "/usr/bin/sandbox-exec",
        "-f",
        profile,
        ompExecutable(),
        "--no-extensions",
        "--extension",
        harnessPath,
        "--config",
        ompConfigPath,
        "--no-tools",
        "--no-lsp",
        "--no-skills",
        "--no-rules",
        "--no-title",
        "--auto-approve",
        "--",
        scenario.prompt,
      ],
      { cwd: sandbox.cwd,
        env: { ...sandbox.env, ...channel.env, RERAIL_PLAYGROUND_INPUT: inputPath },
        cols, rows, send: channel.send,
        data(data) {
          if (version === revision && !closing) publish({ type: "output", data });
        },
      },
    );
    child = managed.child;
    if (runWasCancelled(version)) {
      await managed.stop();
      await sandbox.remove();
      return;
    }
    active = managed;
    activeSandbox = sandbox;
    // Bound every worker, including scripted commands and stalled code-rule evaluation.
    activeDeadline = setTimeout(() => {
      if (!isActiveRun(version, managed)) return;
      complete({ observed: "error", detail: "Worker reached its three-minute lifetime and was cancelled." });
      void stopReplay().catch((error) => status("error", String(error)));
      status("stopped", "Worker lifetime reached; waiting for a rule edit or explicit Run.");
    }, REAL_AGENT_LIFETIME_MS);
    void child.exited
      .then(async (code) => {
        if (!isActiveRun(version, managed)) return;
        if (!completed)
          complete({ observed: "error", detail: `Worker exited (${code}) before reporting an evaluation result.` });
        await stopReplay();
        if (version === revision && !closing)
          status(
            "stopped",
            loopEnabled ? "Worker closed; automatic reruns remain armed for saved rule changes." : "Worker closed.",
          );
      })
      .catch((error) => status("error", String(error)));
  } catch (error) {
    await sandbox.remove();
    complete({ observed: "cannot-exercise", detail: String(error) });
    throw error;
  }
}
function scheduleRerun(): void {
  if (!loopEnabled || !owner || !experiment || rerunTimer || closing) return;
  const delay = Math.max(0, lastStartedAt + RERUN_INTERVAL_MS - Date.now());
  rerunTimer = setTimeout(() => {
    rerunTimer = undefined;
    if (!loopEnabled || !owner || closing) return;
    const version = ++revision;
    commandQueue = commandQueue
      .catch(() => undefined)
      .then(() => run(version))
      .catch((error) => status("error", String(error)));
  }, delay);
}
const poll = setInterval(() => {
  if (polling || !owner || !experiment || closing) return;
  polling = true;
  const id = experiment.ruleId;
  const version = revision;
  void readRule(id)
    .then((current) => {
      if (!experiment || experiment.ruleId !== id || version !== revision || closing) return;
      if (current.digest !== rule?.digest || current.updatedAt !== rule.updatedAt || current.enabled !== rule.enabled) {
        rule = current;
        publishExperiment();
      }
      if (loopEnabled && current.digest !== lastRunDigest) scheduleRerun();
    })
    .catch((error) => {
      if (version === revision) {
        cancel("Saved rule unavailable; automatic reruns stopped.");
        status("error", String(error));
      }
    })
    .finally(() => {
      polling = false;
    });
}, 2_000);

async function handleSelection(
  command: Extract<PlaygroundControl, { type: "select" | "start" }>,
  version: number,
): Promise<void> {
  if (experiment?.ruleId !== command.selection.ruleId) await restore(command.selection.ruleId, version);
  else {
    experiment = { ...experiment, selection: command.selection };
    await persist();
    publishExperiment();
  }
  if (version !== revision || !experiment) return;
  if (command.type === "start") {
    loopEnabled = true;
    await run(version);
  }
}
async function handleGenerate(
  command: Extract<PlaygroundControl, { type: "generate" }>,
  version: number,
): Promise<void> {
  if (experiment?.ruleId !== command.ruleId) await restore(command.ruleId, version);
  if (version !== revision || !experiment) return;
  const current = await readRule(command.ruleId);
  if (version !== revision) return;
  const controller = new AbortController();
  generation = controller;
  status(
    "generating",
    "Generating two rule-specific scenarios using your authenticated account; Stop cancels generation.",
  );
  try {
    const scenarios = await generateScenarios({
      rule: current,
      ...(command.revise && experiment.lastRun ? { previous: experiment.lastRun } : {}),
      checkoutRoot,
      hostHome,
      signal: controller.signal,
    });
    if (version !== revision || controller.signal.aborted) return;
    rule = await readRule(command.ruleId);
    if (version !== revision) return;
    experiment = {
      ...experiment,
      scenarios,
      selection: { ...experiment.selection, scenarioId: scenarios[0]?.id ?? "" },
    };
    await persist();
    publishExperiment();
    status(
      "stopped",
      "Scenarios generated. Inspect them and Run to validate expected versus observed; no evaluation has been claimed.",
    );
  } finally {
    if (generation === controller) generation = undefined;
  }
}
async function handleSaveScenario(command: Extract<PlaygroundControl, { type: "save-scenario" }>): Promise<void> {
  if (!experiment || command.ruleId !== experiment.ruleId) throw new Error("Select the scenario's rule before saving");
  const previous = experiment.scenarios.find((item) => item.id === command.scenario.id);
  if (!previous) throw new Error("Scenario no longer exists");
  if (command.scenario.ruleDigest !== previous.ruleDigest || command.scenario.ruleKind !== previous.ruleKind)
    throw new Error("Scenario provenance is read-only; Generate or Revise explicitly to replace it.");
  validateScenarioFixtures(command.scenario);
  experiment = {
    ...experiment,
    scenarios: experiment.scenarios.map((item) => (item.id === command.scenario.id ? command.scenario : item)),
  };
  await persist();
  publishExperiment();
  status("stopped", "Scenario saved. Run explicitly to evaluate and restart automatic reruns.");
}
async function handle(command: PlaygroundControl, version: number): Promise<void> {
  if (version !== revision || closing || !owner) return;
  if (command.type === "select" || command.type === "start") await handleSelection(command, version);
  else if (command.type === "generate") await handleGenerate(command, version);
  else if (command.type === "save-scenario") await handleSaveScenario(command);
}
function cancellationMessage(type: PlaygroundControl["type"]): string {
  if (type === "stop") return "Stopped. Automatic reruns are off; Run explicitly to resume.";
  if (type === "generate") return "Generating scenarios; previous work cancelled.";
  return "Updating experiment; previous work cancelled.";
}
/** Bounded text frames only, at most 200 per one-second window on the owning socket. */
function parseControl(socket: PlaygroundSocket, message: string | Buffer): PlaygroundControl {
  if (typeof message !== "string" || Buffer.byteLength(message) > MAX_MESSAGE_BYTES)
    throw new Error("Bounded text commands required");
  const now = Date.now();
  if (now - socket.data.window >= 1_000) {
    socket.data.window = now;
    socket.data.messages = 0;
    socket.data.inputBytes = 0;
  }
  if (++socket.data.messages > 200) throw new Error("Command rate exceeded");
  return controlSchema.parse(JSON.parse(message));
}
/** Terminal and script controls reach the live worker directly; experiment commands are not handled here. */
function forwardLiveControl(socket: PlaygroundSocket, command: PlaygroundControl): boolean {
  if (command.type === "resize") {
    cols = command.cols;
    rows = command.rows;
    active?.child.terminal?.resize(cols, rows);
    return true;
  }
  if (command.type === "input") {
    socket.data.inputBytes += Buffer.byteLength(command.data);
    if (socket.data.inputBytes > 65_536) throw new Error("Terminal input rate exceeded");
    active?.child.terminal?.write(command.data);
    return true;
  }
  if (command.type === "pause" || command.type === "resume" || command.type === "step") {
    active?.child.send({ type: command.type });
    return true;
  }
  return false;
}
function queueExperimentCommand(command: PlaygroundControl): void {
  // Reserve generation before queueing it: rapid duplicate clicks must not cancel
  // an in-flight paid request and replace it with another.
  if (command.type === "generate" && generationPending) return;
  const version = cancel(cancellationMessage(command.type), command.type === "generate" ? "generating" : "stopped");
  if (command.type === "generate") generationPending = true;
  if (command.type === "stop") return;
  commandQueue = commandQueue
    .catch(() => undefined)
    .then(() => handle(command, version))
    .catch((error) => {
      if (version === revision) status("error", String(error));
    })
    .finally(() => {
      if (version === revision) generationPending = false;
    });
}
// Filled once the listener has a port: the served client is same-origin with this socket.
let originAllowlist: Record<string, true> = { [daemonOrigin]: true };
function connectionError(request: IncomingMessage): { status: number; message: string } | undefined {
  const url = new URL(request.url ?? "/", daemonOrigin);
  if (url.pathname !== "/__rerail_playground" || request.method !== "GET")
    return { status: 404, message: "Not found" };
  if (originAllowlist[request.headers.origin ?? ""] !== true)
    return { status: 403, message: "Playground origin required" };
  if (closing) return { status: 503, message: "Shutting down" };
  const ruleId = url.searchParams.get("ruleId");
  if (!ruleId || ruleId.length > 128) return { status: 400, message: "Selected rule required" };
  return undefined;
}
function sendJson(response: ServerResponse, status: number, body: unknown): void {
  response.writeHead(status, { "content-type": "application/json", "cache-control": "no-store" });
  response.end(JSON.stringify(body));
}
const server = createServer((request, response) => {
  const url = new URL(request.url ?? "/", daemonOrigin);
  if (request.method === "GET" && url.pathname.startsWith("/__rerail_playground")) {
    if (url.pathname === "/__rerail_playground/health") {
      sendJson(response, 200, { playground: true, pid: process.pid });
      return;
    }
    if (url.pathname === "/__rerail_playground/openapi.json") {
      sendJson(response, 200, playgroundOpenApiDocument);
      return;
    }
    if (url.pathname === "/__rerail_playground/rules") {
      readRules().then(
        (rules) => sendJson(response, 200, rules),
        (error: unknown) => sendJson(response, 502, { error: describeFailure(error) }),
      );
      return;
    }
    const error = connectionError(request) ?? { status: 400, message: "WebSocket upgrade required" };
    response.writeHead(error.status);
    response.end(error.message);
    return;
  }
  // Everything else is the built client. HEAD answers like GET without a body.
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405, { allow: "GET, HEAD" });
    response.end("Method not allowed");
    return;
  }
  clientAssets().then(
    (manifest) => {
      const asset = selectAsset(manifest, url.pathname);
      if (!asset) {
        response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
        response.end("Not found. Run `npm --prefix playground run build` to build the client.");
        return;
      }
      response.writeHead(200, { "content-type": asset.contentType, "cache-control": "no-store" });
      if (request.method === "HEAD") {
        response.end();
        return;
      }
      createReadStream(asset.path).on("error", () => response.destroy()).pipe(response);
    },
    (error: unknown) => {
      response.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
      response.end(describeFailure(error));
    },
  );
});
const sockets = new WebSocketServer({ noServer: true, maxPayload: MAX_MESSAGE_BYTES });
server.on("upgrade", (request, socket, head) => {
  const error = connectionError(request);
  if (error) {
    socket.end(`HTTP/1.1 ${error.status} Rejected\r\nConnection: close\r\n\r\n${error.message}`);
    return;
  }
  sockets.handleUpgrade(request, socket, head, websocket => {
    const client = websocket as PlaygroundSocket;
    client.data = { messages: 0, inputBytes: 0, window: Date.now(),
      ruleId: new URL(request.url!, daemonOrigin).searchParams.get("ruleId")! };
    sockets.emit("connection", client);
  });
});
sockets.on("connection", (websocket) => {
  const socket = websocket as PlaygroundSocket;
  socket.on("error", () => socket.terminate());
  // Newest tab wins. First-wins stranded the user behind a reloaded, frozen, or crashed tab whose
  // socket was still registered, with no way to reclaim the experiment except restarting the
  // supervisor. The evicted tab is closed with 1008 so it reports the takeover and does not retry.
  if (owner) {
    const evicted = owner;
    owner = undefined;
    send(evicted, JSON.stringify({ type: "status", state: "error",
      message: "A newer playground tab took over this experiment; this tab is disconnected." } satisfies PlaygroundEvent));
    evicted.close(1008, "A newer playground tab took over this experiment");
  }
  owner = socket;
  let alive = true;
  socket.on("pong", () => { alive = true; });
  const heartbeat = setInterval(() => {
    if (!alive) { socket.terminate(); return; }
    alive = false;
    socket.ping();
  }, 30_000);
  const version = cancel("Restoring experiment; no automatic execution.");
  commandQueue = commandQueue.catch(() => undefined)
    .then(() => restore(socket.data.ruleId, version))
    .catch(error => status("error", String(error)));
  socket.on("message", (message, binary) => {
    if (socket !== owner || closing) return;
    try {
      if (binary) throw new Error("Bounded text commands required");
      const command = parseControl(socket, message.toString());
      if (!forwardLiveControl(socket, command)) queueExperimentCommand(command);
    } catch (error) {
      send(socket, JSON.stringify({ type: "status", state: "error", message: String(error) } satisfies PlaygroundEvent));
    }
  });
  socket.on("close", () => {
    clearInterval(heartbeat);
    if (socket === owner) {
      owner = undefined;
      cancel("Browser disconnected; experiment stopped and automatic reruns disabled.");
    }
  });
});
const listening = Promise.withResolvers<void>();
server.once("error", listening.reject);
server.listen(0, "127.0.0.1", listening.resolve);
await listening.promise;
const serverAddress = server.address();
if (!serverAddress || typeof serverAddress === "string") throw new Error("Playground HTTP server has no TCP address");
originAllowlist = allowedOrigins(daemonOrigin, serverAddress.port);

const nativePaths = [
  "native/src",
  "native/build.rs",
  "native/Cargo.toml",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
];
const watcher = watch(
  [
    ...["src", ...nativePaths].map((path) => join(checkoutRoot, path)),
    join(playgroundRoot, "src"),
  ],
  {
    ignoreInitial: true,
    ignored: (path) => /(?:^|[/\\])(?:node_modules|target|\.rerail|\.git)(?:[/\\]|$)/.test(path),
    awaitWriteFinish: { stabilityThreshold: 150, pollInterval: 50 },
  },
);
watcher.on("all", (_event, path) => {
  const changed = relative(checkoutRoot, path).replaceAll("\\", "/");
  if (nativePaths.some((native) => changed === native || changed.startsWith(`${native}/`))) nativeDirty = true;
  if (changed.startsWith("src/") || changed.startsWith("playground/src/")) sourceRevision++;
  if (loopEnabled) {
    publish({
      type: "trace",
      entry: {
        at: Date.now(),
        kind: "reload",
        text: `${changed} changed; rerunning latest snapshot within the thirty-second rate limit.`,
      },
    });
    scheduleRerun();
  }
});
watcher.on("error", (error) => status("error", `Source watcher failed: ${String(error)}`));
let shutdownPromise: Promise<void> | undefined;
const startupFinished = Promise.withResolvers<void>();
function shutdown(code: number): Promise<void> {
  shutdownPromise ??= (async () => {
    let exitCode = code;
    let firstError: unknown;
    const attempt = async (label: string, work: () => void | Promise<void>): Promise<void> => {
      try {
        await work();
      } catch (error) {
        firstError ??= error;
        exitCode = 1;
        console.error(`${label}:`, error);
      }
    };
    closing = true;
    clearInterval(poll);
    await attempt("Development runtime cancellation failed", () => { cancel("Development runtime shutting down"); });
    await attempt("Development startup shutdown barrier failed", () => startupFinished.promise);
    await attempt("Development owner socket close failed", () => {
      owner?.close(1001, "Development runtime shutting down");
      owner = undefined;
    });
    await attempt("Source watcher close failed", () => watcher.close());
    await attempt("Build process stop failed", async () => { await build?.stop(); });
    await attempt("Queued command cleanup failed", () => commandQueue);
    await attempt("Replay cleanup failed", () => stopReplay());
    await attempt("Persistence drain failed", () => persistence);
    await attempt("Native sandbox cleanup failed", async () => { await nativeSandbox?.remove(); });
    await attempt("WebSocket client cleanup failed", () => {
      for (const socket of sockets.clients) socket.terminate();
    });
    await attempt("WebSocket server close failed", () => { sockets.close(); });
    await attempt("HTTP connection cleanup failed", () => { server.closeAllConnections(); });
    await attempt("HTTP server close failed", async () => {
      const closed = Promise.withResolvers<void>();
      server.close(error => error ? closed.reject(error) : closed.resolve());
      await closed.promise;
    });
    process.exitCode = exitCode;
    if (firstError !== undefined) throw firstError;
  })();
  return shutdownPromise;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"] as const)
  process.on(signal, () => {
    void shutdown(0).catch((error) => {
      console.error(error);
      process.exitCode = 1;
    });
  });
try {
  const daemonHealth = (await daemonJson(`${daemonOrigin}/healthz`, 5_000, "Development daemon unavailable")) as {
    status?: unknown;
  };
  if (daemonHealth.status !== "ok")
    throw new Error(`Development daemon unavailable: ${daemonOrigin}/healthz reported ${JSON.stringify(daemonHealth).slice(0, 200)}`);
  console.log(
    `\nRerail rule playground: http://127.0.0.1:${serverAddress.port}/`
    + `\nProtocol: ws://127.0.0.1:${serverAddress.port}/__rerail_playground`
    + `\nCtrl-C stops owned workers.\n`,
  );
} catch (error) {
  console.error(`Development runtime failed to start: ${describeFailure(error)}`);
  shutdownPromise = shutdown(1);
} finally {
  startupFinished.resolve();
}
if (shutdownPromise) await shutdownPromise;

async function boundedCommandOutput(stream: Readable | null): Promise<string> {
  if (!stream) return "";
  const decoder = new TextDecoder();
  let output = "";
  for await (const chunk of stream) output = (output + decoder.decode(chunk, { stream: true })).slice(-16_384);
  return output;
}

/** Resolve the installed toolchain's bin directory, or undefined when the rebuild was cancelled meanwhile. */
async function locateToolchainBin(sandbox: Sandbox, version: number): Promise<string | undefined> {
  let rustup = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "bin", "rustup");
  for (const directory of (process.env.PATH ?? "").split(delimiter).filter(Boolean)) {
    const candidate = join(directory, "rustup");
    try {
      await access(candidate, constants.X_OK);
      rustup = candidate;
      break;
    } catch (error) {
      if (!["ENOENT", "ENOTDIR", "EACCES"].includes((error as NodeJS.ErrnoException).code ?? "")) throw error;
    }
  }
  const compiler = spawnProcess([rustup, "which", "cargo"], {
    cwd: checkoutRoot,
    env: {
      ...sandbox.env,
      RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(homedir(), ".rustup"),
      RUSTUP_AUTO_INSTALL: "0",
    },
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
  });
  const task = managedProcess(compiler);
  build = task;
  try {
    const [cargo, compilerError, compilerCode] = await Promise.all([
      boundedCommandOutput(compiler.stdout),
      boundedCommandOutput(compiler.stderr),
      compiler.exited,
    ]);
    await task.stop();
    if (version !== revision || closing) return undefined;
    if (compilerCode !== 0 || !cargo.trim())
      throw new Error(`Installed Rust toolchain unavailable: ${compilerError.slice(-16_384)}`);
    return dirname(cargo.trim());
  } finally {
    await task.stop();
    if (build === task) build = undefined;
  }
}
async function publishBuildOutput(stream: Readable | null, version: number): Promise<void> {
  if (!stream) return;
  const decoder = new TextDecoder();
  for await (const chunk of stream) {
    if (version !== revision || closing) continue;
    const text = decoder.decode(chunk, { stream: true });
    for (let offset = 0; offset < text.length; offset += 8_192) {
      publish({
        type: "trace",
        entry: { at: Date.now(), kind: "build", text: text.slice(offset, offset + 8_192) },
      });
    }
  }
}
/** Stage each built addon beside the checkout's native directory; false when cancelled before installation. */
async function installBuiltAddons(outputDirectory: string, version: number): Promise<boolean> {
  const addons = (await readdir(outputDirectory)).filter((file) => file.endsWith(".node"));
  if (addons.length === 0) throw new Error("Native build produced no addon.");
  for (const file of addons) {
    const destination = join(checkoutRoot, "native", file);
    const staged = `${destination}.playground-${process.pid}`;
    try {
      await copyFile(join(outputDirectory, file), staged);
      if (version !== revision || closing) return false;
      // Preserve the inode mapped by existing daemons; only new harnesses load the replacement.
      await rename(staged, destination);
    } finally {
      await rm(staged, { force: true });
    }
  }
  return true;
}
async function rebuildNative(version: number): Promise<boolean> {
  status("rebuilding", "Rust changed: rebuilding native Rerail before replay.");
  const sandbox = (nativeSandbox ??= await createSandbox());
  const outputDirectory = join(sandbox.root, "native");
  let task: ManagedProcess | undefined;
  try {
    if (version !== revision || closing) return false;
    const toolchainBin = await locateToolchainBin(sandbox, version);
    if (toolchainBin === undefined) return false;
    const nativeChild = spawnProcess(["npm", "run", "build:native", "--", "--output-dir", outputDirectory], {
      cwd: checkoutRoot,
      env: {
        ...sandbox.env,
        PATH: `${toolchainBin}:${sandbox.env.PATH}`,
        CARGO_HOME: join(sandbox.root, "cargo"),
        CARGO_TARGET_DIR: join(sandbox.root, "target"),
        RUSTC: join(toolchainBin, "rustc"),
        RUSTDOC: join(toolchainBin, "rustdoc"),
      },
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
    task = managedProcess(nativeChild);
    build = task;
    await Promise.all([
      publishBuildOutput(nativeChild.stdout, version),
      publishBuildOutput(nativeChild.stderr, version),
    ]);
    const exitCode = await task.child.exited;
    if (version !== revision || closing) return false;
    if (exitCode !== 0) {
      status(
        "error",
        `Native build failed (exit ${exitCode}). Fix the source and save again to retry; no stale harness is running.`,
      );
      return false;
    }
    if (!(await installBuiltAddons(outputDirectory, version))) return false;
    nativeDirty = false;
    return true;
  } finally {
    await task?.stop();
    if (build === task) build = undefined;
  }
}

/** Node workers execute built artifacts; source edits must refresh those artifacts before a replay. */
async function rebuildSources(version: number): Promise<boolean> {
  status("rebuilding", "TypeScript changed: rebuilding Rerail and isolated playground workers.");
  const sourceVersion = sourceRevision;
  const sandbox = await createSandbox();
  try {
    const steps: Array<{ cwd: string; script: string }> = [
      { cwd: checkoutRoot, script: "build:ts" },
      { cwd: checkoutRoot, script: "build:omp" },
      // Workers only: the browser client is rebuilt by `npm run build:ui` or served by `dev:ui`.
      { cwd: playgroundRoot, script: "build:workers" },
    ];
    for (const { cwd, script } of steps) {
      if (version !== revision || closing) return false;
      const task = managedProcess(spawnProcess(["npm", "run", script], {
        cwd, env: sandbox.env, stdout: "pipe", stderr: "pipe",
      }));
      build = task;
      let timedOut = false;
      const deadline = setTimeout(() => {
        timedOut = true;
        void task.stop().catch(error => status("error", String(error)));
      }, 180_000);
      try {
        await Promise.all([
          publishBuildOutput(task.child.stdout, version),
          publishBuildOutput(task.child.stderr, version),
        ]);
        const code = await task.child.exited;
        if (version !== revision || closing) return false;
        if (timedOut || code !== 0) {
          const failure = timedOut ? "exceeded its three-minute deadline" : `failed (exit ${code})`;
          throw new Error(`${script} ${failure}; no stale worker was started.`);
        }
      } finally {
        clearTimeout(deadline);
        await task.stop();
        if (build === task) build = undefined;
      }
    }
    builtSourceRevision = sourceVersion;
    return true;
  } finally {
    await sandbox.remove();
  }
}
