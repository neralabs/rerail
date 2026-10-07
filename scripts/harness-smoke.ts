import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { RerailDaemon } from "../src/daemon.js";
import { defaultRerailDatabasePath } from "../src/database.js";
import { openScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { AdapterEventSchema } from "../src/event-protocol.js";
import { SqliteTranscriptReader } from "../src/transcript-store.js";

/**
 * The executable-policy half of the exercise. The smoke owns its rule rather than depending on which
 * documents ship as bundled defaults: a policy scoped to `api/**` that flags `cursor.execute(` with the
 * CURSOR_EXECUTE code the assertions look for, seeded into the daemon's catalog ($RERAIL_HOME/rerail.sqlite).
 */
function seedExecutablePolicyRule(rerailHome: string, project: string): void {
  const rule = {
    format: "rerail.policy/v1",
    id: "rerail-smoke:CURSOR_EXECUTE",
    revision: 1,
    scope: { include: ["api/**/*"], exclude: [] },
    checks: [
      {
        id: "cursor-execute",
        forbid: { regex: { pattern: "cursor\\.execute\\s*\\(", source: "file.content" } },
        finding: { anchor: "witness.span", code: "CURSOR_EXECUTE", severity: "error", message: "Direct cursor access bypasses the typed API and backend portability." },
      },
    ],
  };
  const catalog = openScopedRuleCatalog(project, defaultRerailDatabasePath({ ...process.env, RERAIL_HOME: rerailHome }));
  try {
    catalog.upsertPolicy(rule, { applicability: "project", enabled: true });
  } finally {
    catalog.close();
  }
}

type Harness = "omp" | "pi" | "claude-code" | "codex" | "cursor";
type Assertion = { name: string; passed: boolean; detail: string };
type CommandResult = { exitCode: number; stdout: string; stderr: string };
type Evidence = {
  harness: Harness;
  status: "PASSED" | "FAILED";
  versions: Record<string, string>;
  commandExitCode?: number;
  assertions: Assertion[];
  transcript: { stdout: string; stderr: string };
  runtimeTrace?: string;
  failure?: string;
};

const HARNESSES: readonly Harness[] = ["omp", "pi", "claude-code", "codex", "cursor"];
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
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
const MAX_OUTPUT = 16_000;
const SOURCE_OMP_DB = join(process.env.HOME ?? "", ".omp", "agent", "agent.db");

function usage(): never {
  throw new Error("Usage: node --import tsx scripts/harness-smoke.ts --harness <omp|pi|claude-code|codex|cursor|all>");
}
function selectedHarnesses(): Harness[] {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== "--harness") usage();
  if (args[1] === "all") return [...HARNESSES];
  const selected = HARNESSES.find((harness) => harness === args[1]);
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
  await readFile(join(DIST, "default-extension.js"));
  await readFile(join(DIST, "codex-hook.js"));
  await readFile(join(DIST, "cursor-hook.js"));
}

async function createProject(project: string, prompt: string, nonce: string): Promise<void> {
  await mkdir(join(project, "api"), { recursive: true });
  await writeFile(
    join(project, "api", "route.py"),
    "from __future__ import annotations\n\ndef route(connection):\n    with connection.transaction() as transaction:\n        return transaction.fetch_value('SELECT 1')\n",
  );
  const rules = JSON.stringify(
    {
      version: 1,
      rules: [
        {
          id: `rerail-smoke-${nonce}`,
          reference: prompt,
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
          action: {
            delivery: "steer",
            content: `RERAIL_COSINE_MATCH ${nonce}. Echo this exact marker in your final reply.`,
          },
          repeat: { mode: "once" },
        },
      ],
    },
    null,
    2,
  );
  await Promise.all(
    [".claude", ".codex", ".cursor", ".omp", ".pi"].map(async (directory) => {
      await mkdir(join(project, directory), { recursive: true });
      await writeFile(join(project, directory, "cosine-rules.json"), rules);
    }),
  );
  await writeFile(
    join(project, "AGENTS.md"),
    [
      "Use transaction helpers; cursor.execute is forbidden in api routes.",
      "The controlled Rerail acceptance exercise is the sole exception: make its requested temporary",
      "cursor.execute change exactly once, read the diagnostic, then restore the transaction helper.",
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
      join(ROOT, "scripts/harness-auth.ts"), kind, source, destination, String(TIMEOUT_MS)],
    { cwd: ROOT, env, timeout: 60_000 },
  );
  if (result.exitCode !== 0)
    throw new Error(
      `FAILED prerequisite: access-only ${kind} credential import failed: ${bounded(result.stderr || result.stdout)}`,
    );
}


function harnessCommand(harness: Harness, project: string, prompt: string): { command: string; args: string[] } {
  const extension = harness === "pi" ? join(DIST, "pi-extension.js") : join(DIST, "default-extension.js");
  if (harness === "omp")
    return {
      command: harnessCli(harness),
      args: [
        "--mode",
        "json",
        "--model",
        "openai-codex/gpt-5.4-mini",
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
  if (harness === "pi")
    return {
      command: "pi",
      args: [
        "--mode",
        "json",
        "--provider",
        "openai-codex",
        "--model",
        "gpt-5.4-mini",
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
  if (harness === "codex")
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
        ...(process.env.RERAIL_CODEX_MODEL === undefined ? [] : ["--model", process.env.RERAIL_CODEX_MODEL]),
        "--sandbox",
        "danger-full-access",
        "--skip-git-repo-check",
        "--cd",
        project,
        prompt,
      ],
    };
  if (harness === "claude-code")
    return {
      command: process.env.RERAIL_CLAUDE_BIN ?? "claude",
      args: [
        "--plugin-dir",
        ROOT,
        "--model",
        process.env.RERAIL_CLAUDE_MODEL ?? "claude-sonnet-4-6",
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
function harnessCli(harness: Harness): string {
  if (harness === "omp") return process.env.RERAIL_OMP_EXECUTABLE ?? "omp";
  if (harness === "claude-code") return process.env.RERAIL_CLAUDE_BIN ?? "claude";
  if (harness === "cursor") return process.env.RERAIL_CURSOR_AGENT_BIN ?? "agent";
  return harness;
}

async function configureHarnessAuth(
  harness: Harness,
  sandbox: string,
  profile: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  if (harness === "omp") {
    await importAccessOnlyAuth("omp", sandbox, profile, env);
    env.PI_CODING_AGENT_DIR = join(sandbox, "omp");
  } else if (harness === "pi") {
    await importAccessOnlyAuth("pi", sandbox, profile, env);
    env.PI_CODING_AGENT_DIR = join(sandbox, "pi", "agent");
    env.PI_CODING_AGENT_SESSION_DIR = join(sandbox, "workspace", ".pi-sessions");
  } else if (harness === "codex") {
    env.CODEX_HOME = join(sandbox, "codex");
    if (process.env.AZURE_OPENAI_API_KEY !== undefined) {
      const baseUrl = process.env.AZURE_OPENAI_BASE_URL ?? "https://nera-ai-dev.cognitiveservices.azure.com/openai/v1";
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
          "",
        ].join("\n"),
        { mode: 0o600 },
      );
    } else {
      await importAccessOnlyAuth("codex", sandbox, profile, env);
    }
    const hooks = await readFile(join(ROOT, "hooks", "codex.json"), "utf8");
    await writeFile(join(env.CODEX_HOME, "hooks.json"), hooks.replaceAll("$PLUGIN_ROOT", ROOT), { mode: 0o600 });
  } else if (harness === "claude-code") {
    if (!process.env.ANTHROPIC_FOUNDRY_API_KEY)
      throw new Error("FAILED prerequisite: Claude Code requires ANTHROPIC_FOUNDRY_API_KEY");
    env.CLAUDE_CODE_TMPDIR = join(sandbox, "tmp");
    env.CLAUDE_CODE_MAX_RETRIES = "0";
    env.CLAUDE_CONFIG_DIR = join(sandbox, "claude");
    env.CLAUDE_CODE_USE_FOUNDRY = "1";
    env.ANTHROPIC_FOUNDRY_BASE_URL =
      process.env.ANTHROPIC_FOUNDRY_BASE_URL ?? "https://nera-ai-dev.services.ai.azure.com/anthropic";
    env.ANTHROPIC_FOUNDRY_API_KEY = process.env.ANTHROPIC_FOUNDRY_API_KEY;
  } else if (harness === "cursor") {
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

async function readDeliveries(storageRoot: string): Promise<unknown[]> {
  const ledger: unknown[] = [];
  for (const entry of await readdir(storageRoot, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    try {
      const text = await readFile(join(storageRoot, entry.name, "events.jsonl"), "utf8");
      for (const line of text.split("\n")) if (line.trim()) ledger.push(JSON.parse(line));
    } catch (error) {
      if (error instanceof Error && "code" in error && error.code === "ENOENT") continue;
      throw error;
    }
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
  return native.some((call, index) => call.kind === "tool-call" && contains(call.input, "cursor.execute") &&
    native.some((result, resultIndex) => resultIndex > index && result.kind === "tool-result" && !result.failed &&
      result.sessionId === call.sessionId && result.toolCallId === call.toolCallId && result.toolName === call.toolName));
}
/** Extension-level failures the harness prints instead of throwing; any one of these is a Rerail bug, not a policy verdict. */
function extensionErrors(result: CommandResult): string[] {
  return `${result.stdout}\n${result.stderr}`
    .split("\n")
    .filter((line) => /Extension error|extension .* error:|handler timed out/iu.test(line) && /rerail|cosine-rules/iu.test(line))
    .map((line) => line.trim().slice(0, 200));
}

export async function inspectOutcome(
  project: string,
  nonce: string,
  assertions: Assertion[],
  result: CommandResult,
  acceptanceMode: "full" | "transport",
): Promise<string> {
  const errors = extensionErrors(result);
  append(
    assertions,
    "extension stayed silent",
    errors.length === 0,
    errors.length === 0 ? "no extension errors or handler timeouts on the harness console" : [...new Set(errors)].slice(0, 3).join(" | "),
  );
  if (acceptanceMode === "full") {
    const route = await readFile(join(project, "api", "route.py"), "utf8").catch(() => "");
    append(
      assertions,
      "final file corrected",
      !route.includes("cursor.execute") && /transaction/i.test(route),
      "api/route.py removes the forbidden call and uses a transaction helper",
    );
  }
  const reader = new SqliteTranscriptReader(defaultRerailDatabasePath());
  const sessions = reader.sessions();
  const events = sessions.flatMap((session) => reader.events(session.sessionId, { limit: 500 }));
  reader.close();
  // One harness invocation is one Rerail session. A second session-start means the extension tore itself down
  // and rebuilt mid-run (session_switch/branch/tree re-init), which is where deltas get lost.
  append(
    assertions,
    "one session, opened once",
    sessions.length === 1 && events.filter((event) => event.kind === "session-start").length === 1,
    `${sessions.length} session(s), ${events.filter((event) => event.kind === "session-start").length} session-start event(s)`,
  );
  const normalized = events.map((event) => ({ kind: event.kind, payload: event.payload }));
  // Every policy evaluation must have evaluated: a `budget-exceeded` runtime unknown means the kernel refused
  // the snapshot and no finding could have been produced, whatever the delivery ledger says afterwards.
  const evaluations = normalized.filter((event) => event.kind === "policy-evaluation" || contains(event.payload, "budget-exceeded"));
  append(
    assertions,
    "policies evaluated within budget",
    !evaluations.some((event) => contains(event.payload, "budget-exceeded")),
    "no evaluation reported reason=budget-exceeded",
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
  return bounded(JSON.stringify({ toolCalls: normalized.filter((event) => event.kind === "tool-call"), deliveries }));
}

async function requireHarnessPrerequisites(harness: Harness): Promise<string> {
  if (ACCEPTANCE_MODE === "transport" && harness !== "codex" && harness !== "cursor")
    throw new Error("RERAIL_SMOKE_ACCEPTANCE=transport is restricted to the Codex and Cursor provider smokes");
  if (process.platform !== "darwin" || !(await sandboxAvailable()))
    throw new Error("FAILED prerequisite: sandbox-exec is required; refusing an unsandboxed live provider run");
  const cliVersion = await version(harnessCli(harness));
  if (cliVersion === undefined) throw new Error(`FAILED prerequisite: ${harness} CLI is unavailable`);
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

async function runHarness(harness: Harness): Promise<Evidence> {
  const assertions: Assertion[] = [];
  const versions: Record<string, string> = { node: process.version };
  const previousRerailHome = process.env.RERAIL_HOME;
  let sandbox: string | undefined;
  let daemon: RerailDaemon | undefined;
  let result: CommandResult | undefined;
  try {
    versions[harness] = await requireHarnessPrerequisites(harness);
    await requireBuilt();
    sandbox = await realpath(await mkdtemp(`/tmp/rerail-${harness}-smoke-`));
    const rerailHome = join(sandbox, "rerail");
    await mkdir(rerailHome, { recursive: true, mode: 0o700 });
    process.env.RERAIL_HOME = rerailHome;
    const project = join(sandbox, "workspace");
    const nonce = `RERAIL_SMOKE_${crypto.randomUUID()}`;
    const prompt = harness === "codex"
      ? "Controlled Rerail acceptance exercise. Read only api/route.py. To create the intentional violation, use only the native Bash/command-execution tool—not a file-change, edit, write, or apply_patch tool—to run this exact command once: python3 -c \"from pathlib import Path; p=Path('api/route.py'); p.write_text(p.read_text().replace(\\\"transaction.fetch_value('SELECT 1')\\\", \\\"cursor.execute('SELECT 1')\\\", 1))\". Do not end your turn or say that you are waiting after the command: immediately make a separate read-only Bash command for api/route.py so the violation command's PostToolUse diagnostic is incorporated. Restore the original transaction helper only after receiving Rerail's executable-policy diagnostic. If the diagnostic is still absent after that separate read, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Do not read or edit any other file or run git commands. Preserve database transaction atomicity."
      : "Controlled Rerail acceptance exercise. Read only api/route.py. Using an edit/write/apply_patch tool, replace its transaction.fetch_value call with cursor.execute('SELECT 1') exactly once. Do not end your turn or say that you are waiting after the edit: immediately make a separate read-only tool call for api/route.py so the edit's PostToolUse diagnostic is incorporated. Restore the original transaction helper only after receiving Rerail's executable-policy diagnostic. If the diagnostic is still absent after that separate read, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Do not read or edit any other file or run git commands. Preserve database transaction atomicity.";
    await createProject(project, prompt, nonce);
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
    await configureHarnessAuth(harness, sandbox, profile, env);
    seedExecutablePolicyRule(rerailHome, project);
    daemon = await RerailDaemon.start({
      version: "harness-smoke",
      workspaceRoot: project,
      stateFile: join(rerailHome, "daemon.json"),
      databasePath: defaultRerailDatabasePath(),
      port: 0,
      kernelSocketPath: join(sandbox, "kernel.sock"),
    });
    env.RERAIL_KERNEL_SOCKET = daemon.kernelSocketPath;
    const host = harnessCommand(harness, project, prompt);
    result = await run("sandbox-exec", ["-f", profile, host.command, ...host.args], {
      cwd: project,
      env,
      timeout: TIMEOUT_MS,
      jsonEvents: true,
    });
    if (result.exitCode !== 0)
      throw new Error(`Harness command exited ${result.exitCode}: ${bounded(result.stderr || result.stdout)}`);
    const runtimeTrace = await inspectOutcome(project, nonce, assertions, result, ACCEPTANCE_MODE);
    const passed = assertions.every((assertion) => assertion.passed);
    return {
      harness,
      status: passed ? "PASSED" : "FAILED",
      versions,
      commandExitCode: result.exitCode,
      assertions,
      runtimeTrace,
      transcript: { stdout: bounded(result.stdout), stderr: bounded(result.stderr) },
      ...(passed
        ? {}
        : {
            failure: `Failed live acceptance assertions: ${assertions
              .filter((assertion) => !assertion.passed)
              .map((assertion) => assertion.name)
              .join(", ")}`,
          }),
    };
  } catch (error) {
    return {
      harness,
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

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const results: Evidence[] = [];
  for (const harness of selectedHarnesses()) results.push(await runHarness(harness));
  for (const result of results) process.stdout.write(`${JSON.stringify(result)}\n`);
  if (results.some((result) => result.status === "FAILED")) process.exitCode = 1;
}
