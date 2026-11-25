import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { copyFile, cp, lstat, mkdir, mkdtemp, readFile, rm, symlink, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, test } from "vitest";
import { ENTRY_POINTS, LOCKS_DIR, resolveNativeBindingPath } from "../../src/engine/index.js";
import type { DaemonState } from "../../src/runtime/index.js";

/** How each Windows PowerShell start behaves in the uninstall's process (`support/slow-powershell.mjs`). */
type PowerShellStart = { ms: number } | "blocked";

/** What one uninstall beside a busy daemon came to. */
interface Uninstalled {
  code: number;
  stdout: string;
  stderr: string;
  ms: number;
  daemonRuns: boolean;
  versionsGone: boolean;
}

/** Whether a process holds `pid`, by signal 0. */
function runs(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Ends `child` if it still runs, and waits for its exit. */
async function endChild(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const closed = once(child, "close");
  child.kill("SIGKILL");
  await closed;
}

/**
 * A Rerail home as the installer leaves it, in `home`: this checkout's package, its built `dist/` and the engine its
 * tests run with, copied into `versions/`, with `current` linked to it. Returns the package's folder there.
 */
async function installedPackage(home: string): Promise<string> {
  const pkg = join(home, "versions", "0.0.0-test", "node_modules", "@neralabs", "rerail");
  await mkdir(join(pkg, "native"), { recursive: true });
  await copyFile(join(process.cwd(), "package.json"), join(pkg, "package.json"));
  await cp(join(process.cwd(), "dist"), join(pkg, "dist"), { recursive: true });
  await copyFile(resolveNativeBindingPath(), join(pkg, "native", "rerail_engine.node"));
  await symlink(pkg, join(home, "current"), "junction");
  return pkg;
}

/**
 * The daemon.json a daemon of this checkout writes in `home`, from one started there and ended: its fields are the
 * ones this release writes and reads.
 */
async function daemonRecord(home: string, env: NodeJS.ProcessEnv): Promise<DaemonState> {
  const stateFile = join(home, "daemon.json");
  const daemon = spawn(process.execPath, [join(process.cwd(), ENTRY_POINTS.daemon), "--dev", "--port", "0", "--state-file", stateFile], { stdio: ["ignore", "pipe", "ignore"], env });
  try {
    await Promise.race([once(daemon.stdout, "data"), once(daemon, "close").then(([code]) => { throw new Error(`daemon exited with code ${String(code)}`); })]);
    return JSON.parse(await readFile(stateFile, "utf8")) as DaemonState;
  } finally {
    await endChild(daemon);
  }
}

/**
 * A daemon too busy to answer, as one whose threads are all suspended: a process that has loaded the installed
 * package's engine, as a daemon of that package has, and holds the engine pipe the record names and never answers on
 * it. daemon.json names it, stamped after it started, and the record beside the home's lock is gone, so only its start
 * proves it this home's daemon. Windows lets the uninstall delete `versions/` only once it has ended.
 */
async function busyDaemon(home: string, state: DaemonState, pkg: string): Promise<ChildProcess> {
  const engine = join(pkg, "native", "rerail_engine.node");
  const daemon = spawn(process.execPath, ["-e", `require(process.argv[2]); require("node:net").createServer(socket => socket.pause())
    .listen(process.argv[1], () => process.stdout.write("listening\\n"));`, state.kernelSocketPath, engine], { stdio: ["ignore", "pipe", "ignore"] });
  await once(daemon.stdout!, "data");
  await writeFile(join(home, "daemon.json"), JSON.stringify({ ...state, pid: daemon.pid, startedAt: Date.now() }));
  await rm(join(home, LOCKS_DIR, "daemon.json"), { force: true });
  return daemon;
}

/** Runs `rerail install --remove` with `extra`, from the installed package `pkg`, with Windows PowerShell as `powershell` starts. */
async function uninstall(pkg: string, env: NodeJS.ProcessEnv, powershell: PowerShellStart, extra: readonly string[]): Promise<Omit<Uninstalled, "daemonRuns" | "versionsGone">> {
  const preload = pathToFileURL(join(process.cwd(), "integration", "support", "slow-powershell.mjs")).href;
  const slow = powershell === "blocked" ? { RERAIL_TEST_POWERSHELL: "blocked" } : { RERAIL_TEST_POWERSHELL_MS: String(powershell.ms) };
  const started = Date.now();
  return await new Promise(done => {
    execFile(process.execPath, ["--import", preload, join(pkg, ENTRY_POINTS.cli), "install", "--remove", ...extra], { env: { ...env, ...slow }, timeout: 170_000 }, (error, stdout, stderr) => {
      done({ code: error === null ? 0 : Number(error.code ?? 1), stdout, stderr, ms: Date.now() - started });
    });
  });
}

/**
 * `rerail install --remove` with `extra`, from a home installed as the installer leaves it, beside a daemon too busy to
 * answer that only its start proves ({@link busyDaemon}), with each Windows PowerShell start as `powershell`. The
 * command's profile, application data and Rerail home are the case's own.
 */
async function uninstallBesideBusyDaemon(powershell: PowerShellStart, extra: readonly string[] = []): Promise<Uninstalled> {
  const root = await mkdtemp(join(tmpdir(), "rerail-uninstall-stop-"));
  const profile = join(root, "profile");
  const home = join(profile, ".rerail");
  const env: NodeJS.ProcessEnv = {
    ...process.env, RERAIL_HOME: home, HOME: profile, USERPROFILE: profile, RERAIL_AUTOSTART: "0",
    APPDATA: join(profile, "AppData", "Roaming"), LOCALAPPDATA: join(profile, "AppData", "Local"),
  };
  delete env.RERAIL_ENGINE_PATH;
  // The installed package finds its dependencies here, above the Rerail home, as npm's tree beside it gives them.
  const modules = join(root, "node_modules");
  let daemon: ChildProcess | undefined;
  try {
    await symlink(join(process.cwd(), "node_modules"), modules, "junction");
    await mkdir(home, { recursive: true });
    const pkg = await installedPackage(home);
    daemon = await busyDaemon(home, await daemonRecord(home, env), pkg);
    const ran = await uninstall(pkg, env, powershell, extra);
    const result = { ...ran, daemonRuns: runs(daemon.pid!), versionsGone: !existsSync(join(home, "versions")) };
    const row = { powershell, command: ["install", "--remove", ...extra].join(" "), ...result, stdout: ran.stdout.split(/\r?\n/u).filter(line => /daemon|could not|did not answer|versions/iu.test(line)) };
    process.stdout.write(`UNINSTALL-ROW ${JSON.stringify(row)}\n`);
    return result;
  } finally {
    if (daemon !== undefined) await endChild(daemon);
    // The link goes first and alone, so removing the folder can never reach the checkout's own dependencies.
    await unlink(modules).catch(() => undefined);
    if (await lstat(modules).then(() => true, () => false)) throw new Error(`${modules} could not be unlinked, so ${root} is kept`);
    await rm(root, { recursive: true, force: true, maxRetries: 5 });
  }
}

describe.runIf(process.platform === "win32")("rerail install --remove beside a busy daemon", () => {
  // A machine so busy that each Windows PowerShell takes 30 s to start: the stop's reads of the daemon's start do not
  // go through PowerShell, so they answer, the daemon is ended and versions/ goes. The uninstall's other PowerShell, the
  // one that takes Rerail off PATH, still waits out its own 30 s.
  test("stops its daemon and removes versions/ while each Windows PowerShell takes 30 s to start", async () => {
    const result = await uninstallBesideBusyDaemon({ ms: 30_000 });
    expect(result.stdout).toContain("Stopped the Rerail daemon.");
    expect(result.daemonRuns).toBe(false);
    expect(result.versionsGone).toBe(true);
    expect(result.code).toBe(0);
  }, 180_000);

  // A company policy that blocks powershell.exe leaves the stop's reads as they are: they never start PowerShell.
  test("stops its daemon and removes versions/ where a policy blocks powershell.exe", async () => {
    const result = await uninstallBesideBusyDaemon("blocked");
    expect(result.stdout).toContain("Stopped the Rerail daemon.");
    expect(result.daemonRuns).toBe(false);
    expect(result.versionsGone).toBe(true);
    expect(result.code).toBe(0);
  }, 180_000);
});
