/**
 * The machinery shared by the live harness runs (`host-smoke.ts`, `host-lifecycle.ts`, `benchmark/steer-strategy.ts`):
 * one bounded child process, the macOS Seatbelt sandbox and its access-only credential imports, each harness's print-mode
 * command line, and the Rerail ledger and `dist/` checks. Nothing here runs on import.
 */
import { spawn } from "node:child_process";
import { mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { RerailHost } from "../../src/engine/index.js";
import { removeAccessOnlyAuth } from "./host-auth.js";

export type CommandResult = { exitCode: number; stdout: string; stderr: string };

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const DIST = join(ROOT, "dist");
const MAX_OUTPUT = 16_000;
/** Captured output past which a command is killed, unless the caller sets its own `outputLimit`. */
const OUTPUT_LIMIT = 2_000_000;
const SOURCE_OMP_DB = join(process.env.HOME ?? "", ".omp", "agent", "agent.db");

function redactUrlCredentials(token: string): string {
  const scheme = token.indexOf("://");
  const credentialsEnd = token.indexOf("@", scheme + 3);
  const path = token.indexOf("/", scheme + 3);
  return scheme === -1 || credentialsEnd === -1 || (path !== -1 && path < credentialsEnd)
    ? token
    : `${token.slice(0, scheme + 3)}[REDACTED]@${token.slice(credentialsEnd + 1)}`;
}
/** Harness output fit for a report: credentials redacted and the middle cut past 16 000 characters. */
export function bounded(text: string): string {
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

/**
 * Runs one command in its own process group, killed with the group on timeout (exit 124) or once its output passes
 * `outputLimit`. With `jsonEvents`, stdout is read as JSON lines: streaming deltas and thinking are dropped, and
 * `agent_end` keeps only its type.
 */
export async function run(
  command: string,
  args: string[],
  options: { cwd?: string; env?: NodeJS.ProcessEnv; timeout?: number; jsonEvents?: boolean; outputLimit?: number } = {},
): Promise<CommandResult> {
  const { promise, resolve: settle, reject } = Promise.withResolvers<CommandResult>();
  const outputLimit = options.outputLimit ?? OUTPUT_LIMIT;
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
    if (captured > outputLimit) {
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
    if (lineBuffer.length > outputLimit) {
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
/** The first line of `<command> --version`; undefined when the command is missing or fails. */
export async function version(command: string): Promise<string | undefined> {
  try {
    const result = await run(command, ["--version"]);
    return result.exitCode === 0
      ? bounded(`${result.stdout}${result.stderr}`).trim().split("\n")[0] || undefined
      : undefined;
  } catch {
    return undefined;
  }
}
export async function requireBuilt(): Promise<void> {
  await readFile(join(DIST, "hosts", "omp", "default-extension.js"));
  await readFile(join(DIST, "hosts", "codex", "codex-hook.js"));
  await readFile(join(DIST, "hosts", "cursor", "cursor-hook.js"));
}
/** Refuse to prove anything about a `dist/` that is older than `src/`: the run would test stale code. */
export async function ensureFresh(built: string): Promise<void> {
  const output = await stat(built).catch(() => undefined);
  if (output === undefined) throw new Error(`${built} is missing: run \`npm run build:ts\` first`);
  for (const name of await readdir(join(ROOT, "src"), { recursive: true })) {
    const source = await stat(join(ROOT, "src", name));
    if (source.isFile() && source.mtimeMs > output.mtimeMs) throw new Error(`${built} is older than src/${name}: run \`npm run build:ts\` first`);
  }
}

/** Every `.jsonl` file below `directory`; none when it does not exist. */
export async function jsonlFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
  const files = await Promise.all(
    entries.map(async (entry) => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return jsonlFiles(path);
      return entry.name.endsWith(".jsonl") ? [path] : [];
    }),
  );
  return files.flat();
}

/** The Seatbelt profile `host` runs under: writes only inside `sandbox`. */
export async function sandboxProfile(sandbox: string, host: RerailHost): Promise<string> {
  const path = join(sandbox, "rerail.sb");
  await writeFile(
    path,
    [
      "(version 1)",
      "(deny default)",
      "(allow process*)",
      "(allow sysctl-read)",
      "(allow mach-lookup)",
      // Codex 0.158 cannot synchronize its managed preferences without IPC, and then fails every thread start.
      ...(host === "codex" ? ["(allow ipc*)"] : []),
      "(allow network*)",
      "(allow file-read*)",
      `(allow file-write* (subpath ${literal(sandbox)}))`,
      '(allow file-write* (literal "/dev/null"))',
    ].join("\n"),
  );
  return path;
}
export async function sandboxAvailable(): Promise<boolean> {
  try {
    return (await run("sandbox-exec", ["-p", "(version 1) (allow default)", "/usr/bin/true"])).exitCode === 0;
  } catch {
    return false;
  }
}
/** Whether the profile lets a command write outside the sandbox: a write that succeeds outside it is tried inside. */
export async function sandboxLeaks(sandbox: string, profile: string, project: string, env: NodeJS.ProcessEnv): Promise<boolean> {
  const probe = join(dirname(sandbox), `rerail-probe-${crypto.randomUUID()}`);
  await writeFile(probe, "probe");
  await rm(probe);
  const denied = await run("sandbox-exec", ["-f", profile, "/usr/bin/touch", probe], { cwd: project, env });
  if (denied.exitCode === 0) await rm(probe);
  return denied.exitCode === 0;
}
/**
 * The harness environment inside the sandbox: its own HOME, TMPDIR, XDG and npm cache directories (created here),
 * the project's Rerail storage root and Rerail home with autostart off, and only the host variables a provider
 * connection needs.
 */
export async function sandboxEnvironment(sandbox: string, project: string, rerailHome: string): Promise<NodeJS.ProcessEnv> {
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
  return env;
}

function authDestination(kind: "omp" | "pi" | "codex", sandbox: string): string {
  if (kind === "omp") return join(sandbox, "omp", "agent.db");
  if (kind === "pi") return join(sandbox, "pi", "agent", "auth.json");
  return join(sandbox, "codex", "auth.json");
}

/** Copies an access token that outlives `timeoutMs` into the sandbox, from inside the sandbox (see host-auth.ts). */
async function importAccessOnlyAuth(
  kind: "omp" | "pi" | "codex",
  sandbox: string,
  profile: string,
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
): Promise<void> {
  const destination = authDestination(kind, sandbox);
  const source = kind === "codex" ? join(process.env.HOME ?? "", ".codex", "auth.json") : SOURCE_OMP_DB;
  const result = await run(
    "sandbox-exec",
    ["-f", profile, process.execPath, "--import", join(ROOT, "node_modules/tsx/dist/loader.mjs"),
      join(ROOT, "scripts/host/host-auth.ts"), kind, source, destination, String(timeoutMs)],
    { cwd: ROOT, env, timeout: 60_000 },
  );
  if (result.exitCode !== 0)
    throw new Error(
      `FAILED prerequisite: access-only ${kind} credential import failed: ${bounded(result.stderr || result.stdout)}`,
    );
}

/** Deletes every access-only credential copy `configureHostAuth` may have imported into `sandbox`, even a partial one. */
export async function removeHostAuth(sandbox: string): Promise<void> {
  await Promise.all((["omp", "pi", "codex"] as const).map((kind) => removeAccessOnlyAuth(authDestination(kind, sandbox))));
}

/** The harness's print-mode invocation with the prompt as its last argument. */
export function hostCommand(host: RerailHost, project: string, prompt: string): { command: string; args: string[] } {
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
export function hostCli(host: RerailHost): string {
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

/** Points the harness at sandbox-local credentials and configuration; `timeoutMs` is how long an imported token must last. */
export async function configureHostAuth(
  host: RerailHost,
  sandbox: string,
  profile: string,
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
): Promise<void> {
  if (host === "omp") {
    await importAccessOnlyAuth("omp", sandbox, profile, env, timeoutMs);
    env.PI_CODING_AGENT_DIR = join(sandbox, "omp");
  } else if (host === "pi") {
    await importAccessOnlyAuth("pi", sandbox, profile, env, timeoutMs);
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
      await importAccessOnlyAuth("codex", sandbox, profile, env, timeoutMs);
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

/** Resolves a filesystem read to undefined when the path is absent; any other error still throws. */
export async function unlessMissing<T>(read: Promise<T>): Promise<T | undefined> {
  try {
    return await read;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
    throw error;
  }
}

/** Every `delivery` entry of every session ledger (`<storageRoot>/<session>/events.jsonl`), in file order. */
export async function readDeliveries(storageRoot: string): Promise<unknown[]> {
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
