/**
 * Seeds a scenario into a folder and runs the dashboard's daemon over it, from this checkout's sources (`--dev` serves
 * `ui/` as committed), isolated from the machine: its own Rerail home, `HOME`, content store and a free port. Used by
 * the Playwright fixtures (`e2e/dashboard-test.ts`) and the design-review launcher (`scripts/dev/ui-scenario.ts`); it imports
 * nothing from `src/`, which runs in the child processes.
 */
import { type ChildProcess, spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import { SEED_NOW, type ScenarioManifest, type ScenarioName, type ScenarioRepository } from "./scenario-manifest.js";

const SUPPORT = dirname(fileURLToPath(import.meta.url));
export const REPOSITORY_ROOT = resolve(SUPPORT, "..", "..");
const REGISTER_TSX = pathToFileURL(join(SUPPORT, "register-tsx.mjs")).href;
const DAEMON_ENTRY = join(REPOSITORY_ROOT, "src", "cli", "daemon-entry.ts");
const DAEMON_START_TIMEOUT_MS = 90_000;
const DAEMON_STOP_TIMEOUT_MS = 10_000;
const POLL_MS = 100;

/**
 * The environment of every scenario process: the machine's `RERAIL_*` settings dropped, the scenario's home and `HOME`
 * in their place, no autostart, no keychain, no shell profile edits, and UTC so "today" is the seed day's.
 */
export function scenarioEnvironment(root: string): NodeJS.ProcessEnv {
  const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("RERAIL_")));
  return {
    ...inherited,
    RERAIL_HOME: join(root, "home"),
    HOME: join(root, "user"),
    RERAIL_AUTOSTART: "0",
    RERAIL_DEVICE_SECRET_STORE: "file",
    RERAIL_MODIFY_PATH: "0",
    RERAIL_CONTENT_STORE_PATH: join(root, "content.sqlite"),
    GIT_CONFIG_NOSYSTEM: "1",
    TZ: "UTC",
  };
}

/** Exit code and collected stderr of a finished child. */
interface ChildOutcome {
  readonly code: number | null;
  readonly signal: NodeJS.Signals | null;
  readonly stderr: string;
}

function finished(child: ChildProcess): Promise<ChildOutcome> {
  let stderr = "";
  child.stderr?.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
  const { promise, resolve: settle, reject } = Promise.withResolvers<ChildOutcome>();
  child.once("error", reject);
  child.once("exit", (code, signal) => settle({ code, signal, stderr }));
  return promise;
}

/** Seeds `scenario` into `folder` (created) as of `now` in a child process and returns its manifest. */
export async function seedScenarioFolder(scenario: ScenarioName, folder: string, now: number = SEED_NOW): Promise<ScenarioManifest> {
  mkdirSync(folder, { recursive: true });
  const root = realpathSync(folder);
  const args = ["--import", REGISTER_TSX, join(SUPPORT, "seed-scenario.ts"), scenario, root, String(now)];
  const child = spawn(process.execPath, args, {
    cwd: REPOSITORY_ROOT,
    env: scenarioEnvironment(root),
    stdio: ["ignore", "ignore", "pipe"],
  });
  const { code, signal, stderr } = await finished(child);
  if (code !== 0) throw new Error(`seeding scenario ${scenario} failed (${signal ?? `exit ${String(code)}`}):\n${stderr}`);
  return JSON.parse(readFileSync(join(root, "scenario.json"), "utf8")) as ScenarioManifest;
}

/**
 * The dashboard address scoped to `repository`, as `rulesDashboardUrl` (src/api/session-contracts.ts) builds it:
 * `?repository=<key>&checkout=<path>`, then `#<hash>` when given.
 */
export function scenarioDashboardUrl(origin: string, repository: ScenarioRepository, hash?: string): string {
  const url = new URL(origin);
  url.search = "";
  url.searchParams.set("repository", repository.repositoryKey);
  url.searchParams.set("checkout", repository.checkoutPath);
  url.hash = hash ?? "";
  return url.href;
}

export interface RunningScenario {
  readonly manifest: ScenarioManifest;
  /** `http://127.0.0.1:<port>`. */
  readonly origin: string;
  /** Where the daemon's stdout and stderr go (`<root>/daemon.log`) unless it inherits the terminal. */
  readonly logPath: string;
  stop(): Promise<void>;
}

export interface DaemonStartOptions {
  /** `0` (default) takes a free port. */
  readonly port?: number;
  /** Write the daemon's output to the terminal instead of `<root>/daemon.log`. */
  readonly inheritOutput?: boolean;
}

/** The daemon's origin once it wrote its state file and answers `/healthz`; throws when it exits or times out first. */
async function awaitDaemon(child: ChildProcess, stateFile: string, logPath: string): Promise<string> {
  const deadline = Date.now() + DAEMON_START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (child.exitCode !== null || child.signalCode !== null) {
      const log = existsSync(logPath) ? readFileSync(logPath, "utf8") : "(output went to the terminal)";
      throw new Error(`the scenario daemon stopped before answering (${child.signalCode ?? `exit ${String(child.exitCode)}`}):\n${log}`);
    }
    if (existsSync(stateFile)) {
      const { url } = JSON.parse(readFileSync(stateFile, "utf8")) as { url: string };
      const origin = new URL(url).origin;
      const health = await fetch(`${origin}/healthz`).catch(() => undefined);
      if (health?.ok === true) return origin;
    }
    await sleep(POLL_MS);
  }
  child.kill("SIGKILL");
  throw new Error(`the scenario daemon did not answer /healthz within ${DAEMON_START_TIMEOUT_MS} ms; see ${logPath}`);
}

/** Stops `child` with SIGTERM (the daemon's clean shutdown), then SIGKILL if it does not exit in time. */
async function terminate(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const { promise: exit, resolve: exited } = Promise.withResolvers<void>();
  child.once("exit", () => exited());
  child.kill("SIGTERM");
  const timer = setTimeout(() => child.kill("SIGKILL"), DAEMON_STOP_TIMEOUT_MS);
  await exit;
  clearTimeout(timer);
}

/** Starts `rerail daemon --dev` in the scenario's primary checkout and waits until it answers. */
export async function startScenarioDaemon(manifest: ScenarioManifest, options: DaemonStartOptions = {}): Promise<RunningScenario> {
  const logPath = join(manifest.root, "daemon.log");
  const stateFile = join(manifest.rerailHome, "daemon.json");
  const args = ["--import", REGISTER_TSX, DAEMON_ENTRY, "--dev", "--port", String(options.port ?? 0), "--state-file", stateFile];
  const child = spawn(process.execPath, args, {
    cwd: manifest.primary.checkoutPath,
    env: scenarioEnvironment(manifest.root),
    stdio: options.inheritOutput === true ? ["ignore", "inherit", "inherit"] : ["ignore", "pipe", "pipe"],
  });
  if (options.inheritOutput !== true) {
    const log = createWriteStream(logPath);
    child.stdout?.pipe(log);
    child.stderr?.pipe(log);
  }
  try {
    const origin = await awaitDaemon(child, stateFile, logPath);
    return { manifest, origin, logPath, stop: () => terminate(child) };
  } catch (error) {
    await terminate(child);
    throw error;
  }
}
