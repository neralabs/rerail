import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { accessSync, constants } from "node:fs";
import { chmod, mkdir, mkdtemp, realpath, rm, stat } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, isAbsolute } from "node:path";
import type { Readable } from "node:stream";
import { spawn as spawnTerminal } from "node-pty";
import { setTimeout as sleep } from "node:timers/promises";
import { loadNativeBinding, type NativeBinding } from "../../dist/index.js";

const execute = promisify(execFile);
const ownedSessions = new Set<number>();

function sessionBinding(): NativeBinding {
  const binding = loadNativeBinding();
  if (typeof binding.processSessionId !== "function")
    throw new Error("Playground session cleanup requires the current Rerail native addon. Rebuild native Rerail before starting.");
  return binding;
}

/**
 * node-pty's published prebuilds ship `spawn-helper` without an exec bit (1.1.0 tarball: 0644), so
 * every macOS PTY spawn fails with `posix_spawnp failed`. Restore the owner exec bit once on every
 * regular helper in a directory node-pty may load, without broadening group or other permissions.
 */
let helperReady: Promise<void> | undefined;
function ensureSpawnHelperExecutable(): Promise<void> {
  helperReady ??= (async () => {
    const packageRoot = dirname(createRequire(import.meta.url).resolve("node-pty/package.json"));
    for (const dir of ["build/Release", "build/Debug", `prebuilds/${process.platform}-${process.arch}`]) {
      const helper = join(packageRoot, dir, "spawn-helper");
      const entry = await stat(helper).catch(() => undefined);
      if (entry === undefined || !entry.isFile()) continue;
      if ((entry.mode & 0o100) === 0) await chmod(helper, entry.mode | 0o100);
    }
  })();
  return helperReady;
}
export interface Sandbox {
  root: string;
  cwd: string;
  env: Record<string, string>;
  remove(): Promise<void>;
}

/** An allowlist, not a secret-name blacklist: no inherited provider, proxy, or Orca configuration. */
export async function createSandbox(): Promise<Sandbox> {
  const root = await realpath(await mkdtemp(join(process.platform === "darwin" ? "/tmp" : tmpdir(), "rerail-delivery-playground-")));
  try {
    const cwd = join(root, "workspace");
    const env: Record<string, string> = {
      PATH: process.env.PATH ?? "/usr/bin:/bin:/usr/sbin:/sbin",
      LANG: process.env.LANG ?? "en_US.UTF-8",
      HOME: join(root, "home"), TMPDIR: join(root, "tmp"),
      XDG_CONFIG_HOME: join(root, "config"), XDG_CACHE_HOME: join(root, "cache"),
      XDG_DATA_HOME: join(root, "data"), XDG_STATE_HOME: join(root, "state"),
      PI_CODING_AGENT_DIR: join(root, "agent"), PI_CODING_AGENT_SESSION_DIR: join(root, "sessions"),
      RERAIL_HOME: join(root, "rerail"), RERAIL_STORAGE_ROOT: join(cwd, ".rerail", "sessions"),
      RERAIL_AUTOSTART: "0", TERM: "xterm-256color", COLORTERM: "truecolor",
      npm_config_cache: join(root, "cache", "npm"),
    };
    if (process.env.LC_ALL) env.LC_ALL = process.env.LC_ALL;
    await Promise.all([
      cwd, env.HOME!, env.TMPDIR!, env.XDG_CONFIG_HOME!, env.XDG_CACHE_HOME!,
      env.XDG_DATA_HOME!, env.XDG_STATE_HOME!, env.PI_CODING_AGENT_DIR!,
      env.PI_CODING_AGENT_SESSION_DIR!, env.RERAIL_HOME!, env.RERAIL_STORAGE_ROOT!,
    ].map(path => mkdir(path, { recursive: true })));
    return { root, cwd, env, remove: () => rm(root, { recursive: true, force: true }) };
  } catch (error) {
    await rm(root, { recursive: true, force: true });
    throw error;
  }
}

export function ompExecutable(): string {
  const executable = process.env.RERAIL_OMP_EXECUTABLE;
  if (!executable || !isAbsolute(executable))
    throw new Error("RERAIL_OMP_EXECUTABLE must name an absolute standalone OMP executable.");
  // The pinned host lives under the checkout's node_modules/.cache, which any root `npm ci` deletes.
  // Without this check sandbox-exec fails to exec it and every worker reports a bare exit 71.
  try {
    accessSync(executable, constants.X_OK);
  } catch (cause) {
    throw new Error(
      `RERAIL_OMP_EXECUTABLE is no longer executable: ${executable}. `
      + "Reinstalling the checkout's dependencies removes the pinned OMP host; restart just rerail-playground.",
      { cause },
    );
  }
  return executable;
}

/** ps only enumerates processes/groups; the native getsid call supplies the real POSIX SID. */
async function processGroups(): Promise<Array<{ pid: number; group: number }>> {
  const { stdout } = await execute("/bin/ps", ["-axo", "pid=,pgid="], { timeout: 1_000, maxBuffer: 4 * 1_048_576 });
  return stdout.trim().split("\n").map(line => {
    const [pid, group] = line.trim().split(/\s+/);
    return { pid: Number(pid), group: Number(group) };
  }).filter(row => Number.isSafeInteger(row.pid) && row.pid > 0);
}

/** Kill a member only while it still belongs to the captured session. */
function killSessionMember(binding: NativeBinding, session: number, pid: number): void {
  // Re-check after the snapshot: a process may already have exited.
  if (binding.processSessionId(pid) !== session) return;
  try {
    process.kill(pid, "SIGKILL");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
}

/** Gate exec until the spawned leader has its own session, never the supervisor's. */
export async function captureSession(pid: number): Promise<number> {
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error("Owned process PID is invalid.");
  const binding = sessionBinding();
  const supervisorSession = binding.processSessionId(process.pid);
  if (supervisorSession === null || supervisorSession <= 0)
    throw new Error("Could not inspect the playground supervisor's process session.");
  const deadline = Date.now() + 1_000;
  while (true) {
    const session = binding.processSessionId(pid);
    if (session === pid && session !== supervisorSession) {
      ownedSessions.add(session);
      return session;
    }
    if (session === null || Date.now() >= deadline)
      throw new Error("Owned process did not establish an isolated process session.");
    await sleep(10);
  }
}
export async function stopSession(session: number): Promise<void> {
  const binding = sessionBinding();
  if (!Number.isSafeInteger(session) || session <= 0 || !ownedSessions.has(session)
    || session === binding.processSessionId(process.pid))
    throw new Error("Refusing to signal a process session not owned by the playground.");
  const deadline = Date.now() + 3_000;
  while (true) {
    const members = (await processGroups()).filter(row => binding.processSessionId(row.pid) === session);
    if (!members.length) {
      ownedSessions.delete(session);
      return;
    }
    for (const { pid } of members) killSessionMember(binding, session, pid);
    if (Date.now() >= deadline) throw new Error("Process session did not terminate within the cleanup deadline.");
    await sleep(10);
  }
}

export interface PlaygroundProcess {
  pid: number;
  exited: Promise<number>;
  exitCode: number | null;
  stdout: Readable | null;
  stderr: Readable | null;
  session: Promise<number>;
  send(message: unknown): void;
  terminal?: { write(data: string): void; resize(cols: number, rows: number): void };
}
export function spawnProcess(command: string[], options: {
  cwd: string; env: Record<string, string>; stdin?: "ignore"; stdout?: "pipe" | "ignore" | "inherit";
  stderr?: "pipe" | "ignore" | "inherit";
}): PlaygroundProcess {
  const child = spawn("/bin/bash", [
    "--noprofile", "--norc", "-c", 'read -r gate; exec "$@"', "playground", ...command,
  ], {
    cwd: options.cwd, env: options.env, detached: true,
    stdio: ["pipe", options.stdout ?? "pipe", options.stderr ?? "pipe"],
  });
  const completion = Promise.withResolvers<number>();
  child.once("error", completion.reject);
  child.once("exit", (code, signal) => completion.resolve(code ?? (signal ? 128 : 1)));
  const session = captureSession(child.pid!).then(sessionId => {
    child.stdin!.end("start\n");
    return sessionId;
  }, error => {
    child.kill("SIGKILL");
    throw error;
  });
  const exited = Promise.all([completion.promise, session]).then(([code]) => code);
  // Callers may attach after async startup; the original promise still reports errors.
  void exited.catch(() => undefined);
  return {
    get pid() { return child.pid!; }, exited, session, get exitCode() { return child.exitCode; },
    stdout: child.stdout, stderr: child.stderr,
    send() { throw new Error("Process has no control channel"); },
  };
}

/** Gate exec until the real SID is captured, so even a fast job-control child remains reclaimable. */
export async function spawnPty(command: string[], options: {
  cwd: string; env: Record<string, string>; cols: number; rows: number; data(text: string): void;
  send?(message: unknown): void;
}): Promise<ManagedProcess> {
  await ensureSpawnHelperExecutable();
  const terminal = spawnTerminal("/bin/bash", ["--noprofile", "--norc", "-c", 'read -rs gate; exec "$@"', "playground", ...command], {
    cwd: options.cwd, env: options.env, cols: options.cols, rows: options.rows, name: "xterm-256color",
  });
  let exitCode: number | null = null;
  const completion = Promise.withResolvers<number>();
  const exited = completion.promise;
  terminal.onExit(event => { exitCode = event.exitCode; completion.resolve(event.exitCode); });
  terminal.onData(options.data);
  let session: number;
  try { session = await captureSession(terminal.pid); }
  catch (error) { terminal.kill("SIGKILL"); await exited; throw error; }
  terminal.write("\n");
  return managedProcess({ pid: terminal.pid, exited, session: Promise.resolve(session),
    get exitCode() { return exitCode; }, stdout: null, stderr: null,
    terminal, send: options.send ?? (() => { throw new Error("Process has no control channel"); }),
  }, Boolean(options.send));
}

export interface ManagedProcess { child: PlaygroundProcess; stop(): Promise<void> }
/** Idempotent cleanup shared by cancellation, normal exit, and shutdown. */
export function managedProcess(child: PlaygroundProcess, cooperative = false): ManagedProcess {
  let stopping: Promise<void> | undefined;
  const signalGroup = (signal: NodeJS.Signals) => {
    try { process.kill(-child.pid, signal); }
    catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === "ESRCH") return;
      // Darwin sandbox policy may deny process-group signals; the verified SID fallback handles it.
      if (code === "EPERM" && process.platform === "darwin") return;
      throw error;
    }
  };
  const waitForExit = async (milliseconds: number) => {
    let timer: NodeJS.Timeout | undefined;
    const timeout = Promise.withResolvers<false>();
    timer = setTimeout(() => timeout.resolve(false), milliseconds);
    try { return await Promise.race([child.exited.then(() => true), timeout.promise]); }
    finally { clearTimeout(timer); }
  };
  return { child, stop() {
    stopping ??= (async () => {
      const session = await child.session;
      if (cooperative && child.exitCode === null) {
        try { child.send({ type: "shutdown" }); } catch { /* The channel may already be closed. */ }
        await waitForExit(1_200);
      }
      signalGroup("SIGTERM");
      if (!(await waitForExit(500))) signalGroup("SIGKILL");
      // Reclaim the verified SID before waiting without a deadline: group signals may be denied.
      await stopSession(session);
      await child.exited;
    })();
    return stopping;
  } };
}
