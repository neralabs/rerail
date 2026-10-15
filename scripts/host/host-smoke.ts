import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  type RerailHost,
  defaultRerailDatabasePath,
  openRerailDatabase,
  recordCheckout,
  loadBundledCatalog,
  setUpRepository,
  upsertLibraryRule,
  type ConversationRule,
} from "../../src/engine/index.js";
import { RerailDaemon } from "../../src/daemon/index.js";
import { setRepositoryStarted, AdapterEventSchema, SqliteSessionReader } from "../../src/runtime/index.js";

/**
 * The conversation half of the exercise: a steer rule that matches the prompt and asks for the private nonce back.
 */
function smokeConversationRule(prompt: string, nonce: string): ConversationRule {
  return {
    id: `rerail-smoke-${nonce}`,
    mode: "steer",
    reference: prompt,
    match: { operator: "gte", threshold: 0.95 },
    sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
    action: { content: `RERAIL_CONVERSATION_MATCH ${nonce}. Echo this exact marker in your final reply.` },
    repeat: { mode: "once" },
  };
}

/**
 * The smoke's rules, stored in the rule library of the daemon's database ($RERAIL_HOME/rerail.sqlite) and added to the
 * project's repository, which is set up and started as `rerail start` would leave it: only rules a repository adds
 * run. The smoke owns its rules rather than depending on which documents ship as bundled defaults. The
 * code-rule half is a code rule scoped to `api/**` that flags `cursor.execute(` with the CURSOR_EXECUTE code
 * the assertions look for.
 */
function seedSmokeRules(rerailHome: string, project: string, conversationRule: ConversationRule): void {
  const rule = {
    format: "rerail.policy/v1",
    id: "rerail-smoke:CURSOR_EXECUTE",
    revision: 1,
    mode: "steer",
    scope: { include: ["api/**/*"], exclude: [] },
    clauses: [
      {
        id: "cursor-execute",
        forbid: { regex: { pattern: "cursor\\.execute\\s*\\(", source: "file.content" } },
        violation: { anchor: "witness.span", code: "CURSOR_EXECUTE", message: "Direct cursor access bypasses the typed API and backend portability." },
      },
    ],
  };
  const databasePath = defaultRerailDatabasePath({ ...process.env, RERAIL_HOME: rerailHome });
  const database = openRerailDatabase(databasePath);
  try {
    const bundle = loadBundledCatalog();
    upsertLibraryRule(database, bundle, rule, "code");
    upsertLibraryRule(database, bundle, conversationRule, "conversation");
    const { repositoryKey } = recordCheckout(database, project);
    setUpRepository(database, repositoryKey, {
      packs: [],
      rules: [rule.id, conversationRule.id].map(sourceId => ({ sourceKind: "library_rule" as const, sourceId, folder: "" })),
    });
    setRepositoryStarted(database, repositoryKey, true);
  } finally {
    database.$client.close();
  }
}

type Assertion = { name: string; passed: boolean; detail: string };
type CommandResult = { exitCode: number; stdout: string; stderr: string };
/** INCONCLUSIVE: the provider, not Rerail, never got the session to the violation (or stalled); the attempt is retried. */
type Evidence = {
  harness: RerailHost;
  attempt: number;
  status: "PASSED" | "FAILED" | "INCONCLUSIVE";
  versions: Record<string, string>;
  commandExitCode?: number;
  assertions: Assertion[];
  transcript: { stdout: string; stderr: string };
  runtimeTrace?: string;
  failure?: string;
};

const HOSTS: readonly RerailHost[] = ["omp", "pi", "claude-code", "codex", "cursor"];
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DIST = join(ROOT, "dist");
const timeoutSetting = process.env.RERAIL_SMOKE_TIMEOUT_MS ?? "180000";
const TIMEOUT_MS = Number(timeoutSetting);
if (!Number.isSafeInteger(TIMEOUT_MS) || TIMEOUT_MS <= 0) {
  throw new Error(`RERAIL_SMOKE_TIMEOUT_MS must be a positive integer, received ${JSON.stringify(timeoutSetting)}`);
}
const acceptanceSetting = process.env.RERAIL_SMOKE_ACCEPTANCE ?? "full";
if (acceptanceSetting !== "full" && acceptanceSetting !== "transport") {
  throw new Error(`RERAIL_SMOKE_ACCEPTANCE must be "full" or "transport", received ${JSON.stringify(acceptanceSetting)}`);
}
const ACCEPTANCE_MODE: "full" | "transport" = acceptanceSetting;
const attemptsSetting = process.env.RERAIL_SMOKE_ATTEMPTS ?? "1";
const ATTEMPTS = Number(attemptsSetting);
if (!Number.isSafeInteger(ATTEMPTS) || ATTEMPTS <= 0) {
  throw new Error(`RERAIL_SMOKE_ATTEMPTS must be a positive integer, received ${JSON.stringify(attemptsSetting)}`);
}
/** The Codex violation and restore step: a quote-free command, so the model cannot break it by re-quoting. */
const SET_CALL_SCRIPT = join("tools", "set_call.py");
/** How often the smoke reads api/route.py while a harness runs; see watchForViolation. */
const VIOLATION_POLL_MS = 100;
const MAX_OUTPUT = 16_000;
const SOURCE_OMP_DB = join(process.env.HOME ?? "", ".omp", "agent", "agent.db");

function usage(): never {
  throw new Error("Usage: node --import tsx scripts/host/host-smoke.ts --host <omp|pi|claude-code|codex|cursor|all>");
}
function selectedHosts(): RerailHost[] {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== "--host") usage();
  if (args[1] === "all") return [...HOSTS];
  const selected = HOSTS.find((host) => host === args[1]);
  return selected === undefined ? usage() : [selected];
}
function redactUrlCredentials(token: string): string {
  const scheme = token.indexOf("://");
  const credentialsEnd = token.indexOf("@", scheme + 3);
  const path = token.indexOf("/", scheme + 3);
  return scheme === -1 || credentialsEnd === -1 || (path !== -1 && path < credentialsEnd)
    ? token
    : `${token.slice(0, scheme + 3)}[REDACTED]@${token.slice(credentialsEnd + 1)}`;
}
function bounded(text: string): string {
  const redacted = text
    .replace(/(Authorization:\s*Bearer\s+|(?:api[_-]?key|token|password|secret)\s*[=:]\s*)\S+/gi, "$1[REDACTED]")
    .replace(/\b(sk-[A-Za-z0-9_-]{12,}|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)\b/g, "[REDACTED]")
    .replace(/\S+/g, redactUrlCredentials);
  return redacted.length <= MAX_OUTPUT
    ? redacted
    : `${redacted.slice(0, MAX_OUTPUT / 2)}\n…[truncated]\n${redacted.slice(-MAX_OUTPUT / 2)}`;
}
function literal(value: string): string {
  const escaped = value.replace(/["\\]/g, String.raw`\$&`);
  return `"${escaped}"`;
}
function append(assertions: Assertion[], name: string, passed: boolean, detail: string): void {
  assertions.push({ name, passed, detail });
}

async function run(
  command: string,
  args: string[],
  options: { cwd?: string; env?: NodeJS.ProcessEnv; timeout?: number; jsonEvents?: boolean } = {},
): Promise<CommandResult> {
  const { promise, resolve: settle, reject } = Promise.withResolvers<CommandResult>();
  const child = spawn(command, args, {
    cwd: options.cwd,
    env: options.env,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  let captured = 0;
  let timedOut = false;
  let overflow = false;
  const kill = (signal: NodeJS.Signals): void => {
    if (child.pid !== undefined) {
      try {
        process.kill(-child.pid, signal);
      } catch {
        child.kill(signal);
      }
    }
  };
  const timeout = setTimeout(() => {
    timedOut = true;
    kill("SIGTERM");
  }, options.timeout ?? 30_000);
  const force = setTimeout(() => kill("SIGKILL"), (options.timeout ?? 30_000) + 5_000);
  const capture = (chunk: string, isError: boolean): void => {
    captured += chunk.length;
    if (captured > 2_000_000) {
      overflow = true;
      kill("SIGKILL");
      return;
    }
    if (isError) stderr += chunk;
    else stdout += chunk;
  };
  let lineBuffer = "";
  const captureEvent = (line: string): void => {
    if (line.trim() === "") return;
    let event: unknown;
    try {
      event = JSON.parse(line);
    } catch {
      capture(`${line}\n`, false);
      return;
    }
    if (event !== null && typeof event === "object" && "type" in event) {
      if (event.type === "message_update" || event.type === "message_start" || event.type === "thinking") return;
      if (event.type === "agent_end") event = { type: event.type };
    }
    capture(
      `${JSON.stringify(event, (key, value: unknown) => (["thinkingSignature", "encrypted_content", "thinking"].includes(key) ? undefined : value))}\n`,
      false,
    );
  };
  const captureStdout = (chunk: string): void => {
    if (options.jsonEvents !== true) {
      capture(chunk, false);
      return;
    }
    lineBuffer += chunk;
    if (lineBuffer.length > 2_000_000) {
      overflow = true;
      kill("SIGKILL");
      return;
    }
    let newline: number;
    while ((newline = lineBuffer.indexOf("\n")) >= 0) {
      captureEvent(lineBuffer.slice(0, newline));
      lineBuffer = lineBuffer.slice(newline + 1);
    }
  };
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", captureStdout);
  child.stderr.on("data", (chunk: string) => capture(chunk, true));
  child.once("exit", () => kill("SIGKILL"));
  child.once("error", (error) => {
    clearTimeout(timeout);
    clearTimeout(force);
    reject(error);
  });
  child.once("close", (code) => {
    clearTimeout(timeout);
    clearTimeout(force);
    if (lineBuffer !== "") captureEvent(lineBuffer);
    if (timedOut) stderr += "\nSmoke command timed out";
    if (overflow) stderr += "\nSmoke command exceeded its output limit";
    let exitCode = code ?? 1;
    if (overflow) exitCode = 1;
    if (timedOut) exitCode = 124;
    settle({ exitCode, stdout, stderr });
  });
  return promise;
}
async function version(command: string): Promise<string | undefined> {
  try {
    const result = await run(command, ["--version"]);
    return result.exitCode === 0
      ? bounded(`${result.stdout}${result.stderr}`).trim().split("\n")[0] || undefined
      : undefined;
  } catch {
    return undefined;
  }
}
async function requireBuilt(): Promise<void> {
  await readFile(join(DIST, "hosts", "omp", "default-extension.js"));
  await readFile(join(DIST, "hosts", "codex", "codex-hook.js"));
  await readFile(join(DIST, "hosts", "cursor", "cursor-hook.js"));
}

async function createProject(project: string, host: RerailHost): Promise<void> {
  await mkdir(join(project, "api"), { recursive: true });
  await writeFile(
    join(project, "api", "route.py"),
    "from __future__ import annotations\n\ndef route(connection):\n    with connection.transaction() as transaction:\n        return transaction.fetch_value('SELECT 1')\n",
  );
  await writeFile(
    join(project, "AGENTS.md"),
    [
      "Use transaction helpers; cursor.execute is forbidden in api routes.",
      "The controlled Rerail acceptance exercise is the sole exception: make its requested one-off",
      "cursor.execute change exactly once, read the diagnostic, then restore the transaction helper.",
      "",
    ].join("\n"),
  );
  if (host !== "codex") return;
  await mkdir(join(project, "tools"), { recursive: true });
  await writeFile(
    join(project, SET_CALL_SCRIPT),
    [
      "import pathlib",
      "import re",
      "import sys",
      "",
      'CALLS = ("cursor.execute", "transaction.fetch_value")',
      "if len(sys.argv) != 2 or sys.argv[1] not in CALLS:",
      '    sys.exit("usage: python3 tools/set_call.py " + "|".join(CALLS))',
      "call = sys.argv[1]",
      "tools = pathlib.Path(__file__).resolve().parent",
      'route = tools.parent / "api" / "route.py"',
      "text = route.read_text()",
      String.raw`updated = re.sub(r"return [\w.]+\('SELECT 1'\)", "return " + call + "('SELECT 1')", text, count=1)`,
      "if updated == text:",
      '    sys.exit("api/route.py already returns " + call)',
      "route.write_text(updated)",
      "",
    ].join("\n"),
  );
}

async function sandboxProfile(sandbox: string): Promise<string> {
  const path = join(sandbox, "rerail.sb");
  await writeFile(
    path,
    [
      "(version 1)",
      "(deny default)",
      "(allow process*)",
      "(allow sysctl-read)",
      "(allow mach-lookup)",
      "(allow network*)",
      "(allow file-read*)",
      `(allow file-write* (subpath ${literal(sandbox)}))`,
      '(allow file-write* (literal "/dev/null"))',
    ].join("\n"),
  );
  return path;
}
async function sandboxAvailable(): Promise<boolean> {
  try {
    return (await run("sandbox-exec", ["-p", "(version 1) (allow default)", "/usr/bin/true"])).exitCode === 0;
  } catch {
    return false;
  }
}

function authDestination(kind: "omp" | "pi" | "codex", sandbox: string): string {
  if (kind === "omp") return join(sandbox, "omp", "agent.db");
  if (kind === "pi") return join(sandbox, "pi", "agent", "auth.json");
  return join(sandbox, "codex", "auth.json");
}

async function importAccessOnlyAuth(
  kind: "omp" | "pi" | "codex",
  sandbox: string,
  profile: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  const destination = authDestination(kind, sandbox);
  const source = kind === "codex" ? join(process.env.HOME ?? "", ".codex", "auth.json") : SOURCE_OMP_DB;
  const result = await run(
    "sandbox-exec",
    ["-f", profile, process.execPath, "--import", join(ROOT, "node_modules/tsx/dist/loader.mjs"),
      join(ROOT, "scripts/host/host-auth.ts"), kind, source, destination, String(TIMEOUT_MS)],
    { cwd: ROOT, env, timeout: 60_000 },
  );
  if (result.exitCode !== 0)
    throw new Error(
      `FAILED prerequisite: access-only ${kind} credential import failed: ${bounded(result.stderr || result.stdout)}`,
    );
}


function hostCommand(host: RerailHost, project: string, prompt: string): { command: string; args: string[] } {
  const extension = host === "pi" ? join(DIST, "hosts", "pi", "pi-extension.js") : join(DIST, "hosts", "omp", "default-extension.js");
  if (host === "omp")
    return {
      command: hostCli(host),
      args: [
        "--mode",
        "json",
        "--model",
        process.env.RERAIL_OMP_MODEL ?? "openai-codex/gpt-5.5",
        "--thinking",
        "low",
        "--cwd",
        project,
        "--session-dir",
        join(project, ".omp-sessions"),
        "--no-extensions",
        "--extension",
        extension,
        "--auto-approve",
        "--print",
        prompt,
      ],
    };
  if (host === "pi")
    return {
      command: "pi",
      args: [
        "--mode",
        "json",
        "--provider",
        "openai-codex",
        "--model",
        process.env.RERAIL_PI_MODEL ?? "gpt-5.5",
        "--thinking",
        "low",
        "--no-context-files",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--session-dir",
        join(project, ".pi-sessions"),
        "--no-extensions",
        "--extension",
        extension,
        "--print",
        prompt,
      ],
    };
  // The enclosing, probed Seatbelt profile provides isolation. macOS rejects nested sandbox_apply.
  if (host === "codex")
    return {
      command: "codex",
      args: [
        "--ask-for-approval",
        "never",
        "--dangerously-bypass-hook-trust",
        "-c",
        'model_reasoning_effort="low"',
        "exec",
        "--json",
        ...(process.env.RERAIL_CODEX_MODEL ? ["--model", process.env.RERAIL_CODEX_MODEL] : []),
        "--sandbox",
        "danger-full-access",
        "--skip-git-repo-check",
        "--cd",
        project,
        prompt,
      ],
    };
  if (host === "claude-code")
    return {
      command: process.env.RERAIL_CLAUDE_BIN ?? "claude",
      args: [
        "--plugin-dir",
        ROOT,
        "--model",
        process.env.RERAIL_CLAUDE_MODEL || "claude-sonnet-4-6",
        "--permission-mode",
        "bypassPermissions",
        "--output-format",
        "stream-json",
        "--verbose",
        "-p",
        prompt,
      ],
    };
  return {
    command: process.env.RERAIL_CURSOR_AGENT_BIN ?? "agent",
    args: [
      "--print",
      "--output-format",
      "stream-json",
      "--force",
      "--trust",
      "--workspace",
      project,
      "--plugin-dir",
      ROOT,
      prompt,
    ],
  };
}
function hostCli(host: RerailHost): string {
  if (host === "omp") return process.env.RERAIL_OMP_EXECUTABLE ?? "omp";
  if (host === "claude-code") return process.env.RERAIL_CLAUDE_BIN ?? "claude";
  if (host === "cursor") return process.env.RERAIL_CURSOR_AGENT_BIN ?? "agent";
  return host;
}

/** A provider setting the run cannot guess: endpoints and deployments belong to whoever runs the smoke. */
function requiredEnvironment(name: string, purpose: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`FAILED prerequisite: set ${name} for ${purpose}`);
  return value;
}

async function configureHostAuth(
  host: RerailHost,
  sandbox: string,
  profile: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  if (host === "omp") {
    await importAccessOnlyAuth("omp", sandbox, profile, env);
    env.PI_CODING_AGENT_DIR = join(sandbox, "omp");
  } else if (host === "pi") {
    await importAccessOnlyAuth("pi", sandbox, profile, env);
    env.PI_CODING_AGENT_DIR = join(sandbox, "pi", "agent");
    env.PI_CODING_AGENT_SESSION_DIR = join(sandbox, "workspace", ".pi-sessions");
  } else if (host === "codex") {
    env.CODEX_HOME = join(sandbox, "codex");
    if (process.env.AZURE_OPENAI_API_KEY) {
      const baseUrl = requiredEnvironment("AZURE_OPENAI_BASE_URL", "the Codex Azure OpenAI provider (AZURE_OPENAI_API_KEY is set)");
      requiredEnvironment("RERAIL_CODEX_MODEL", "the Azure OpenAI deployment Codex runs against");
      env.AZURE_OPENAI_API_KEY = process.env.AZURE_OPENAI_API_KEY;
      await mkdir(env.CODEX_HOME, { recursive: true });
      await writeFile(
        join(env.CODEX_HOME, "config.toml"),
        [
          'model_provider = "azure"',
          "",
          "[model_providers.azure]",
          'name = "Azure OpenAI"',
          `base_url = ${JSON.stringify(baseUrl)}`,
          'env_key = "AZURE_OPENAI_API_KEY"',
          'wire_api = "responses"',
          // Codex's 300 s default outlasts a whole attempt, so a stalled response stream would never be retried.
          "stream_idle_timeout_ms = 60000",
          "",
        ].join("\n"),
        { mode: 0o600 },
      );
    } else {
      await importAccessOnlyAuth("codex", sandbox, profile, env);
    }
    const hooks = await readFile(join(ROOT, "hooks", "codex.json"), "utf8");
    await writeFile(join(env.CODEX_HOME, "hooks.json"), hooks.replaceAll("$PLUGIN_ROOT", ROOT), { mode: 0o600 });
  } else if (host === "claude-code") {
    if (!process.env.ANTHROPIC_FOUNDRY_API_KEY)
      throw new Error("FAILED prerequisite: Claude Code requires ANTHROPIC_FOUNDRY_API_KEY");
    const foundryBaseUrl = requiredEnvironment("ANTHROPIC_FOUNDRY_BASE_URL", "Claude Code's Microsoft Foundry provider");
    env.CLAUDE_CODE_TMPDIR = join(sandbox, "tmp");
    env.CLAUDE_CODE_MAX_RETRIES = "0";
    env.CLAUDE_CONFIG_DIR = join(sandbox, "claude");
    env.CLAUDE_CODE_USE_FOUNDRY = "1";
    env.ANTHROPIC_FOUNDRY_BASE_URL = foundryBaseUrl;
    env.ANTHROPIC_FOUNDRY_API_KEY = process.env.ANTHROPIC_FOUNDRY_API_KEY;
  } else if (host === "cursor") {
    if (!process.env.CURSOR_API_KEY) throw new Error("FAILED prerequisite: Cursor Agent requires CURSOR_API_KEY");
    env.CURSOR_CONFIG_DIR = join(sandbox, "cursor");
    env.CURSOR_API_KEY = process.env.CURSOR_API_KEY;
  }
}

function finalAssistantText(events: { kind: string; payload: unknown }[]): string {
  return events
    .filter((event) => event.kind === "turn-end" || event.kind === "assistant-message")
    .map((event) =>
      event.payload !== null &&
      typeof event.payload === "object" &&
      "text" in event.payload &&
      typeof event.payload.text === "string"
        ? event.payload.text
        : "",
    )
    .join("\n");
}

/** Resolves a filesystem read to undefined when the path is absent; any other error still throws. */
async function unlessMissing<T>(read: Promise<T>): Promise<T | undefined> {
  try {
    return await read;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}

async function readDeliveries(storageRoot: string): Promise<unknown[]> {
  const ledger: unknown[] = [];
  // A session that died before its first hook (a harness that never started) leaves no storage root.
  for (const entry of (await unlessMissing(readdir(storageRoot, { withFileTypes: true }))) ?? []) {
    if (!entry.isDirectory()) continue;
    const text = await unlessMissing(readFile(join(storageRoot, entry.name, "events.jsonl"), "utf8"));
    for (const line of text?.split("\n") ?? []) if (line.trim()) ledger.push(JSON.parse(line));
  }
  return ledger.filter(
    (entry) => entry !== null && typeof entry === "object" && "type" in entry && entry.type === "delivery",
  );
}


function contains(value: unknown, needle: string): boolean {
  return JSON.stringify(value)?.includes(needle) ?? false;
}

export function hasCompletedViolation(events: readonly { kind: string; payload: unknown }[]): boolean {
  const native = events.flatMap(({ kind, payload }) => {
    const event = AdapterEventSchema.safeParse(payload).data;
    return event?.kind === kind ? [event] : [];
  });
  return native.some((call, index) => call.kind === "call" && contains(call.input, "cursor.execute") &&
    native.some((result, resultIndex) => resultIndex > index && result.kind === "result" && !result.failed &&
      result.sessionId === call.sessionId && result.toolCallId === call.toolCallId && result.toolName === call.toolName));
}

/**
 * Polls a file while a harness runs; the returned stop reports whether it ever held the forbidden call. This is
 * whether the violation reached disk, judged without Rerail's records, for every host: Rerail records no exit
 * status for a Codex Bash result, and an edit-tool host's model restores the file before the run ends. The
 * violation stays on disk for at least one model round trip, since restoring it takes another tool call.
 */
export function watchForViolation(path: string): () => Promise<boolean> {
  let seen = false;
  let failure: unknown;
  const read = async (): Promise<void> => {
    try {
      if ((await unlessMissing(readFile(path, "utf8")))?.includes("cursor.execute")) seen = true;
    } catch (error) {
      failure ??= error;
    }
  };
  // One read at a time, so stop can wait for the read in flight and lose neither its sighting nor its error.
  let polling: Promise<void> | undefined;
  const timer = setInterval(() => {
    polling ??= read().finally(() => (polling = undefined));
  }, VIOLATION_POLL_MS);
  return async () => {
    clearInterval(timer);
    await polling;
    await read();
    if (failure !== undefined) throw failure;
    return seen;
  };
}

/** Failures that follow from the violation never reaching the file: no code-rule verdict was possible. */
const AFTER_VIOLATION: Record<string, true> = {
  "violation reached the file": true,
  "real violation evidence": true,
  "executable policy delivered": true,
  "final file corrected": true,
  "real final assistant output": true,
};

/**
 * An attempt is inconclusive, and worth another session, only when everything Rerail could be held to passed: the
 * failures are a harness timeout (a stalled provider) or follow from the violation never reaching the file. A
 * violation that reached the file without Rerail's record or diagnostic, a crash, an extension error or a lost
 * delivery stays FAILED.
 */
export function isInconclusive(assertions: readonly Assertion[], exitCode: number): boolean {
  const failed = assertions.filter((assertion) => !assertion.passed);
  const violationMissing = failed.some((assertion) => assertion.name === "violation reached the file");
  return failed.length > 0 && failed.every((assertion) =>
    assertion.name === "harness exited cleanly" ? exitCode === 124 : violationMissing && AFTER_VIOLATION[assertion.name] === true);
}

/** A host line about Rerail: it names the extension file the host loaded (`Extension error (<path>): ...`), Rerail or its engine. */
const RERAIL_LINE = /rerail|(?:default|pi)-extension\.js/iu;

/** Extension-level failures the host prints instead of throwing; any one of these is a Rerail bug, not a code-rule verdict. */
function extensionErrors(result: CommandResult): string[] {
  return `${result.stdout}\n${result.stderr}`
    .split("\n")
    .filter((line) => /Extension error|extension .* error:|handler timed out/iu.test(line) && RERAIL_LINE.test(line))
    .map((line) => line.trim().slice(0, 200));
}

export async function inspectOutcome(
  project: string,
  nonce: string,
  assertions: Assertion[],
  result: CommandResult,
  violationSeen: boolean,
  acceptanceMode: "full" | "transport",
): Promise<string> {
  append(
    assertions,
    "harness exited cleanly",
    result.exitCode === 0,
    result.exitCode === 124 ? "the harness command timed out" : `the harness command exited ${result.exitCode}`,
  );
  const errors = extensionErrors(result);
  append(
    assertions,
    "extension stayed silent",
    errors.length === 0,
    errors.length === 0 ? "no extension errors or handler timeouts on the harness console" : [...new Set(errors)].slice(0, 3).join(" | "),
  );
  const route = await unlessMissing(readFile(join(project, "api", "route.py"), "utf8"));
  if (acceptanceMode === "full") {
    append(
      assertions,
      "final file corrected",
      route !== undefined && !route.includes("cursor.execute") && /transaction/i.test(route),
      "api/route.py removes the forbidden call and uses a transaction helper",
    );
  }
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  const sessions = reader.sessions();
  const events = sessions.flatMap((session) => reader.events(session.sessionId, { limit: 500 }));
  reader.close();
  // One host invocation is one Rerail session. A second session-start means the extension tore itself down
  // and rebuilt mid-run (session_switch/branch/tree re-init), which is where deltas get lost.
  append(
    assertions,
    "one session, opened once",
    sessions.length === 1 && events.filter((event) => event.kind === "session-start").length === 1,
    `${sessions.length} session(s), ${events.filter((event) => event.kind === "session-start").length} session-start event(s)`,
  );
  const normalized = events.map((event) => ({ kind: event.kind, payload: event.payload }));
  // Every code-rule evaluation must have evaluated: a `budget-exceeded` runtime unknown means the engine refused
  // the snapshot and no violation could have been produced, whatever the delivery ledger says afterwards.
  const audits = normalized.filter((event) => contains(event.payload, "budget-exceeded"));
  append(
    assertions,
    "policies evaluated within budget",
    !audits.some((event) => contains(event.payload, "budget-exceeded")),
    "no evaluation reported reason=budget-exceeded",
  );
  append(
    assertions,
    "violation reached the file",
    violationSeen,
    "api/route.py held the forbidden call while the harness ran",
  );
  append(
    assertions,
    "real violation evidence",
    hasCompletedViolation(normalized),
    "the forbidden invocation has a matching completed native tool-result",
  );
  if (acceptanceMode === "full") {
    const assistantText = finalAssistantText(normalized);
    append(
      assertions,
      "real final assistant output",
      assistantText.includes(nonce) && assistantText.includes("CURSOR_EXECUTE"),
      "actual assistant text echoes the private delivery nonce and policy diagnostic",
    );
  }
  const deliveries = await readDeliveries(join(project, ".rerail/sessions"));
  append(
    assertions,
    "single authoritative delivery",
    deliveries.filter((entry) => contains(entry, nonce)).length === 1,
    "exactly one persisted delivery contains the private cosine nonce",
  );
  append(
    assertions,
    "executable policy delivered",
    deliveries.some((entry) => contains(entry, "CURSOR_EXECUTE")),
    "a separate persisted executable-policy delivery contains CURSOR_EXECUTE",
  );
  return bounded(JSON.stringify({ toolCalls: normalized.filter((event) => event.kind === "call"), deliveries }));
}

async function requireHostPrerequisites(host: RerailHost): Promise<string> {
  if (ACCEPTANCE_MODE === "transport" && host !== "codex" && host !== "cursor")
    throw new Error("RERAIL_SMOKE_ACCEPTANCE=transport is restricted to the Codex and Cursor provider smokes");
  if (process.platform !== "darwin" || !(await sandboxAvailable()))
    throw new Error("FAILED prerequisite: sandbox-exec is required; refusing an unsandboxed live provider run");
  const cliVersion = await version(hostCli(host));
  if (cliVersion === undefined) throw new Error(`FAILED prerequisite: ${host} CLI is unavailable`);
  return cliVersion;
}

async function enforceSandboxIsolation(
  sandbox: string,
  profile: string,
  project: string,
  env: NodeJS.ProcessEnv,
  assertions: Assertion[],
): Promise<void> {
  const probe = join(dirname(sandbox), `rerail-probe-${crypto.randomUUID()}`);
  await writeFile(probe, "probe");
  await rm(probe);
  const denied = await run("sandbox-exec", ["-f", profile, "/usr/bin/touch", probe], { cwd: project, env });
  append(
    assertions,
    "sandbox enforced",
    denied.exitCode !== 0,
    "sandbox rejects a write that succeeds outside the sandbox",
  );
  if (denied.exitCode === 0) {
    await rm(probe);
    throw new Error("Sandbox did not enforce write isolation");
  }
}

async function runHost(host: RerailHost, attempt: number): Promise<Evidence> {
  const assertions: Assertion[] = [];
  const versions: Record<string, string> = { node: process.version };
  const previousRerailHome = process.env.RERAIL_HOME;
  let sandbox: string | undefined;
  let daemon: RerailDaemon | undefined;
  let result: CommandResult | undefined;
  try {
    versions[host] = await requireHostPrerequisites(host);
    await requireBuilt();
    sandbox = await realpath(await mkdtemp(`/tmp/rerail-${host}-smoke-`));
    const rerailHome = join(sandbox, "rerail");
    await mkdir(rerailHome, { recursive: true, mode: 0o700 });
    process.env.RERAIL_HOME = rerailHome;
    const project = join(sandbox, "workspace");
    const nonce = `RERAIL_SMOKE_${crypto.randomUUID()}`;
    const prompt = host === "codex"
      ? "Controlled Rerail acceptance exercise. Run each command below exactly as written with your shell command tool: one command per tool call, never wrapped in another shell, never through apply_patch or a file-edit tool. First create the intentional violation: `python3 tools/set_call.py cursor.execute`. Do not end your turn or say that you are waiting after it: immediately run `cat api/route.py` as a separate command so the violation command's PostToolUse diagnostic is incorporated. Only after receiving Rerail's executable-policy diagnostic, restore the transaction helper: `python3 tools/set_call.py transaction.fetch_value`. If the diagnostic is still absent after the cat, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Run no other commands, read or edit no other files, and run no git commands. Preserve database transaction atomicity."
      : "Controlled Rerail acceptance exercise. Read only api/route.py. Using an edit/write/apply_patch tool, replace its transaction.fetch_value call with cursor.execute('SELECT 1') exactly once. Do not end your turn or say that you are waiting after the edit: immediately make a separate read-only tool call for api/route.py so the edit's PostToolUse diagnostic is incorporated. Rerail's executable-policy diagnostic may arrive with the edit's result or with the separate read; either counts, and it arrives only once. As soon as you have it, restore the original transaction helper. Only if you have received no such diagnostic by the end of that separate read, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Do not read or edit any other file or run git commands. Preserve database transaction atomicity.";
    await createProject(project, host);
    await mkdir(join(sandbox, "home"), { recursive: true });
    await mkdir(join(sandbox, "tmp"), { recursive: true });
    await mkdir(join(sandbox, "cache"), { recursive: true });
    const env: NodeJS.ProcessEnv = {
      HOME: join(sandbox, "home"),
      TMPDIR: join(sandbox, "tmp"),
      XDG_CONFIG_HOME: join(sandbox, "config"),
      XDG_CACHE_HOME: join(sandbox, "cache"),
      npm_config_cache: join(sandbox, "cache", "npm"),
      RERAIL_STORAGE_ROOT: join(project, ".rerail", "sessions"),
      RERAIL_HOME: rerailHome,
      RERAIL_AUTOSTART: "0",
    };
    for (const key of [
      "PATH",
      "LANG",
      "LC_ALL",
      "TERM",
      "HTTP_PROXY",
      "HTTPS_PROXY",
      "ALL_PROXY",
      "NO_PROXY",
      "SSL_CERT_FILE",
      "NODE_EXTRA_CA_CERTS",
    ])
      if (process.env[key] !== undefined) env[key] = process.env[key];
    const profile = await sandboxProfile(sandbox);
    await enforceSandboxIsolation(sandbox, profile, project, env, assertions);
    await configureHostAuth(host, sandbox, profile, env);
    seedSmokeRules(rerailHome, project, smokeConversationRule(prompt, nonce));
    daemon = await RerailDaemon.start({
      version: "host-smoke",
      workspaceRoot: project,
      stateFile: join(rerailHome, "daemon.json"),
      databasePath: defaultRerailDatabasePath(),
      port: 0,
      kernelSocketPath: join(sandbox, "engine.sock"),
    });
    env.RERAIL_ENGINE_SOCKET = daemon.kernelSocketPath;
    const invocation = hostCommand(host, project, prompt);
    const stopWatching = watchForViolation(join(project, "api", "route.py"));
    try {
      result = await run("sandbox-exec", ["-f", profile, invocation.command, ...invocation.args], {
        cwd: project,
        env,
        timeout: TIMEOUT_MS,
        jsonEvents: true,
      });
    } catch (error) {
      // The harness failure is the one to report; the watcher only needs stopping.
      await stopWatching().catch(() => undefined);
      throw error;
    }
    const violationSeen = await stopWatching();
    const runtimeTrace = await inspectOutcome(project, nonce, assertions, result, violationSeen, ACCEPTANCE_MODE);
    const passed = assertions.every((assertion) => assertion.passed);
    const failed = assertions.filter((assertion) => !assertion.passed).map((assertion) => assertion.name).join(", ");
    let status: Evidence["status"] = "PASSED";
    if (!passed) status = isInconclusive(assertions, result.exitCode) ? "INCONCLUSIVE" : "FAILED";
    return {
      harness: host,
      attempt,
      status,
      versions,
      commandExitCode: result.exitCode,
      assertions,
      runtimeTrace,
      transcript: { stdout: bounded(result.stdout), stderr: bounded(result.stderr) },
      ...(passed ? {} : { failure: `Failed live acceptance assertions: ${failed}` }),
    };
  } catch (error) {
    return {
      harness: host,
      attempt,
      status: "FAILED",
      versions,
      ...(result === undefined ? {} : { commandExitCode: result.exitCode }),
      assertions,
      transcript: { stdout: bounded(result?.stdout ?? ""), stderr: bounded(result?.stderr ?? "") },
      failure: bounded(error instanceof Error ? error.message : String(error)),
    };
  } finally {
    try {
      await daemon?.close();
    } finally {
      if (previousRerailHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = previousRerailHome;
      if (sandbox !== undefined) await rm(sandbox, { recursive: true, force: true });
    }
  }
}

/** Inconclusive attempts get a fresh session, up to RERAIL_SMOKE_ATTEMPTS; every attempt is reported. */
async function runHostAttempts(host: RerailHost): Promise<Evidence[]> {
  const evidence: Evidence[] = [];
  for (let attempt = 1; ; attempt += 1) {
    const result = await runHost(host, attempt);
    if (result.status !== "INCONCLUSIVE") return [...evidence, result];
    if (attempt === ATTEMPTS)
      return [...evidence, { ...result, status: "FAILED", failure: `${result.failure ?? ""}; inconclusive on all ${ATTEMPTS} attempt(s)` }];
    evidence.push(result);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const results: Evidence[] = [];
  for (const host of selectedHosts()) results.push(...(await runHostAttempts(host)));
  for (const result of results) process.stdout.write(`${JSON.stringify(result)}\n`);
  if (results.some((result) => result.status === "FAILED")) process.exitCode = 1;
}
