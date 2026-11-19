import * as childProcess from "node:child_process";
import { spawn, type ChildProcess } from "node:child_process";
import { EventEmitter, once } from "node:events";
import { connect, createServer, type AddressInfo, type Socket } from "node:net";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, stat, utimes, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defaultEngineSocketPath, RerailDaemon } from "../../src/daemon/index.js";
import {
  type DaemonHealth,
  type DaemonMachine,
  type DaemonState,
  type FoundDaemon,
  type NativeLaunch,
  type StartedChild,
  type StopDaemonResult,
  AUTOSTART_CLAIM_FILE,
  AutostartOffError,
  autostartDaemon,
  autostartOff,
  browserOpener,
  DAEMON_ACCESS_HEADER,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  daemonEngineEndpoint,
  daemonMayHold,
  daemonNotRunningMessage,
  dashboardAccessLink,
  daemonUsesPackageRoot,
  findRunningDaemon,
  isUsableEngineSocketPath,
  keepAutostartClaim,
  launchDetached,
  lookupDaemon,
  openDashboard,
  openInBrowser,
  processAlive,
  readDaemonState,
  recordedDaemonAlive,
  relaunchDaemon,
  runningDaemonEngineEndpoint,
  startDaemon,
  startDaemonForPackage,
  stillRunningMessage,
  STOP_ANSWERS,
  stopDaemon,
  stopFoundDaemon,
  withStartLock,
} from "../../src/runtime/daemon-control.js";
import { LOCKS_DIR, processStartedAt } from "../../src/engine/index.js";
import { enginePipeName, fallbackPipe, isEnginePipe, pipeNameOf } from "../../src/runtime/engine-pipe.js";
import { engineCapabilityProof } from "../../src/runtime/engine-proof.js";
import { DEAD_PID, olderDaemon, until } from "../support/processes/daemon-home.js";
import { type ProbeLaunch, type StartedProbe, runConsoleProbe } from "../support/platform/console-probe.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { otherUserPid } from "../support/other-user-pid.js";
import { removeFolder, runningIn } from "../support/platform/remove-folder.js";

// Spies that call through, so a test can stand a program in for Windows PowerShell.
vi.mock("node:child_process", { spy: true });

const originalRerailHome = process.env.RERAIL_HOME;

/** The home lock's holder record naming `pid`, as a daemon that took the lock of `home` writes it. */
async function holdHome(home: string, pid: number): Promise<void> {
  await mkdir(join(home, LOCKS_DIR), { recursive: true });
  await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid }));
}

function stateFor(overrides: Partial<DaemonState>): DaemonState {
  return {
    pid: DEAD_PID,
    host: "127.0.0.1",
    port: 1,
    url: "http://127.0.0.1:1/",
    version: "0.0.0-test",
    databasePath: "/nowhere/rerail.sqlite",
    kernelSocketPath: "/nowhere/engine.sock",
    ui: "dev",
    startedAt: 0,
    packageRoot: "/nowhere/rerail",
    ...overrides,
  };
}

describe("engine service IPC endpoints", () => {
  it("uses a valid per-database Windows named pipe", () => {
    const first = defaultEngineSocketPath(String.raw`C:\work\one\rerail.sqlite`, "win32");
    const second = defaultEngineSocketPath(String.raw`C:\work\two\rerail.sqlite`, "win32");
    expect(first).toMatch(/^\\\\\.\\pipe\\rerail-[a-f0-9]{24}$/u);
    expect(second).not.toBe(first);
    expect(isUsableEngineSocketPath(first, "win32")).toBe(true);
    expect(isUsableEngineSocketPath(String.raw`C:\temp\engine.sock`, "win32")).toBe(false);
  });

  it("knows the pipe the daemon names, and the fallback it moves to, for a Rerail engine's, and nothing else", () => {
    const pipe = defaultEngineSocketPath(String.raw`C:\Users\me\.rerail\rerail.sqlite`, "win32");
    const fallback = fallbackPipe(pipe);
    expect(isEnginePipe(pipeNameOf(pipe))).toBe(true);
    expect(isEnginePipe(pipeNameOf(fallback))).toBe(true);
    expect(isEnginePipe(pipeNameOf(fallback).toUpperCase())).toBe(true);
    expect(isUsableEngineSocketPath(fallback, "win32")).toBe(true);
    expect(fallbackPipe(pipe)).not.toBe(fallback);
    expect(isEnginePipe("rerail-squatted")).toBe(false);
    expect(isEnginePipe(`${pipeNameOf(pipe)}x`)).toBe(false);
  });
});

describe("Rerail daemon control", () => {
  let home: string;
  /**
   * This machine's processes, read by the macOS and Linux rules: the states these tests record name a Unix socket,
   * which a Windows lookup refuses, and no engine pipe. The Windows rules have their own tests below.
   */
  const posix = fakeMachine("linux", { kill: (pid, signal) => process.kill(pid, signal) });

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-home-"));
    process.env.RERAIL_HOME = join(home, "catalog-home");
  });

  afterEach(async () => {
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await removeFolder(home);
  });

  it("treats a missing or malformed state file as no daemon", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    expect(await readDaemonState(stateFile)).toBeUndefined();
    await writeFile(stateFile, "{not json");
    expect(await readDaemonState(stateFile)).toBeUndefined();
    await writeFile(stateFile, JSON.stringify({ pid: "1" }));
    expect(await readDaemonState(stateFile)).toBeUndefined();
    expect(await findRunningDaemon(home)).toBeUndefined();
  });

  it("removes a stale state file whose process is gone", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    await writeFile(stateFile, JSON.stringify(stateFor({})));
    expect(processAlive(DEAD_PID)).toBe(false);
    expect(await findRunningDaemon(home, posix)).toBeUndefined();
    expect(existsSync(stateFile)).toBe(false);
    expect(await stopDaemon(home, posix)).toBe("not-running");
  });

  const foreignPid = otherUserPid();
  it.runIf(foreignPid !== undefined)("treats a state file whose pid another OS user runs as stale, and never sends it the capability", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: foreignPid!, accessToken: "secret-capability" })));
    const probe = vi.fn(async () => Response.json({ status: "ok", pid: foreignPid }));
    vi.stubGlobal("fetch", probe);
    try {
      expect(processAlive(foreignPid!)).toBe(true);
      expect(await lookupDaemon(home)).toEqual({ status: "absent" });
      expect(probe).not.toHaveBeenCalled();
      expect(existsSync(stateFile)).toBe(false);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("ignores a state file whose listener belongs to a different process", async () => {
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(home, "rerail.sqlite"),
      workspaceRoot: home,
      port: 0,
      nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
    });
    try {
      // The recorded pid is alive (this test process) but the listener reports process.pid as well,
      // so use the parent pid: alive, yet not the pid the daemon answers with.
      const stateFile = join(home, DAEMON_STATE_FILE);
      await writeFile(stateFile, JSON.stringify(stateFor({ pid: process.ppid, url: daemon.url, port: daemon.port, accessToken: daemon.state().accessToken! })));
      expect(await findRunningDaemon(home, posix)).toBeUndefined();
      expect(existsSync(stateFile)).toBe(false);
    } finally {
      await daemon.close();
    }
  });

  it("keeps the state file when the home's daemon stops answering, and refuses to start over it", async () => {
    // A closed port with a live pid that holds the home: this process is alive, nothing listens on the recorded URL.
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: process.pid, url: "http://127.0.0.1:1/" });
    await writeFile(stateFile, JSON.stringify(state));
    await holdHome(home, process.pid);
    const holding = { ...posix, homeDaemonLockHeld: () => true };
    expect(await lookupDaemon(home, holding)).toEqual({ status: "unresponsive", state });
    expect(existsSync(stateFile)).toBe(true);
    expect(await findRunningDaemon(home, holding)).toBeUndefined();
    expect(existsSync(stateFile)).toBe(true);
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }, holding))
      .rejects.toThrow(/alive but not answering/);
    expect(await withStartLock(home, async () => "the start lock is free")).toBe("the start lock is free");
  });

  it("stops an unresponsive daemon that holds the home and clears its state file", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    let daemonPid: number | undefined;
    try {
      const launcher = spawn(
        process.execPath,
        ["-e", `
          const { spawn } = require("node:child_process");
          const daemon = spawn(
            process.execPath,
            ["-e", "require('node:net').createServer().listen(0)"],
            { detached: true, stdio: "ignore", windowsHide: true },
          );
          daemon.unref();
          process.stdout.write(String(daemon.pid));
        `],
        { stdio: ["ignore", "pipe", "ignore"], windowsHide: true },
      );
      if (launcher.stdout === null) throw new Error("test launcher did not start");
      const [pidOutput] = await once(launcher.stdout, "data") as [Buffer];
      await once(launcher, "close");
      daemonPid = Number(pidOutput.toString());
      if (!Number.isInteger(daemonPid)) throw new Error("test daemon did not receive a pid");
      await writeFile(stateFile, JSON.stringify(stateFor({ pid: daemonPid, url: "http://127.0.0.1:1/" })));
      await holdHome(home, daemonPid);
      const pid = daemonPid;

      expect(await stopDaemon(home, { ...posix, homeDaemonLockHeld: () => processAlive(pid) })).toBe("stopped");
      expect(processAlive(daemonPid)).toBe(false);
      expect(existsSync(stateFile)).toBe(false);
    } finally {
      if (daemonPid !== undefined && processAlive(daemonPid)) process.kill(daemonPid, "SIGKILL");
    }
  }, 15_000);

  it.runIf(process.platform === "linux")("counts a daemon nothing reaped as stopped, so stop returns in a container without init", async () => {
    // `sh` backgrounds a child, then becomes `sleep`, which never reaps it: the zombie a daemon becomes when PID 1 is
    // `sleep infinity` rather than an init. The child outlives the `exec`, so `sh` itself can never reap it first.
    const parent = spawn("sh", ["-c", "sleep 0.2 & echo $!; exec sleep 30"], { stdio: ["ignore", "pipe", "ignore"] });
    try {
      if (parent.stdout === null) throw new Error("test parent did not start");
      const [pidOutput] = await once(parent.stdout, "data") as [Buffer];
      const zombie = Number(pidOutput.toString());
      // The child exits on the OS's schedule and signals nothing this process can await, so its state is polled.
      const deadline = Date.now() + 5_000;
      while (processAlive(zombie) && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 20));
      expect(processAlive(zombie)).toBe(false);
      expect(() => process.kill(zombie, 0)).not.toThrow();

      await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ pid: zombie, url: "http://127.0.0.1:1/" })));
      expect(await stopDaemon(home)).toBe("not-running");
      expect(existsSync(join(home, DAEMON_STATE_FILE))).toBe(false);
    } finally {
      parent.kill("SIGKILL");
    }
  });

  it("finds a live daemon through its state file", async () => {
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(home, "rerail.sqlite"),
      workspaceRoot: home,
      port: 0,
      stateFile: join(home, DAEMON_STATE_FILE),
      nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
    });
    try {
      const running = await findRunningDaemon(home);
      expect(running?.state).toEqual(daemon.state());
      expect(running?.health).toMatchObject({ status: "ok", pid: process.pid, native: { ok: true } });
    } finally {
      await daemon.close();
    }
    expect(await findRunningDaemon(home)).toBeUndefined();
  });

  it("writes its record back when it is removed, names another process or holds what this daemon did not write, and removes it for good when it closes", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const daemon = await RerailDaemon.start({
      version: "0.0.0-test",
      databasePath: join(home, "rerail.sqlite"),
      workspaceRoot: home,
      port: 0,
      stateFile,
      recordCheck: { everyMs: 20, home, holdsHome: () => true },
      nativeHealth: () => ({ ok: true, kernelVersion: "1" }),
    });
    const written = readFileSync(stateFile, "utf8");
    const restored = (): boolean => {
      try {
        return readFileSync(stateFile, "utf8") === written;
      } catch {
        return false;
      }
    };
    try {
      await rm(stateFile);
      await until(restored);
      expect(await readDaemonState(stateFile)).toEqual(daemon.state());
      await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID })));
      await until(restored);
      // This daemon's pid with a capability it does not accept: every hook and start reading it would be refused (401).
      await writeFile(stateFile, JSON.stringify({ ...daemon.state(), accessToken: "not-this-daemons" }));
      await until(restored);
    } finally {
      await daemon.close();
    }
    expect(existsSync(stateFile)).toBe(false);
    await delay(100);
    expect(existsSync(stateFile)).toBe(false);
  }, 20_000);

  // Asking the home's lock loads the engine from the running version's folder, and Windows keeps a loaded DLL from being
  // deleted: the uninstall stops the daemon, then removes `versions/`.
  it("asks the home's lock only of a recorded holder that runs, so a stop where no daemon runs loads no engine", async () => {
    const asked: string[] = [];
    const machine = (platform: NodeJS.Platform, pipes: readonly string[] = []): DaemonMachine => fakeMachine(platform, {
      kill: (pid, signal) => process.kill(pid, signal),
      pipes: () => pipes,
      homeDaemonLockHeld: asking => {
        asked.push(asking);
        return false;
      },
    });
    // What a home keeps once a daemon of this build has run there: the lock file, and no holder once it stopped.
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(join(home, LOCKS_DIR, "daemon.lock"), "");
    expect(await stopDaemon(home, machine("win32"))).toBe("not-running");
    // A holder killed outright left its record, naming a pid that is gone.
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: DEAD_PID }));
    expect(await stopDaemon(home, machine("win32"))).toBe("not-running");
    // A record Windows left at a logoff, a restart or a stop's kill, naming a pid it has given to another process since:
    // no engine pipe of this home is there, and another home's daemon holds only its own.
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: process.pid }));
    expect(await stopDaemon(home, machine("win32"))).toBe("not-running");
    const otherHomes = pipeNameOf(enginePipeName(join(home, "other", "rerail.sqlite")));
    expect(await stopDaemon(home, machine("win32", [otherHomes]))).toBe("not-running");
    expect(asked).toEqual([]);
    // With this home's engine pipe there, or the fallback its daemon moves to, the recorded pid may be the holder or a
    // process that took the pid since: the lock tells. On macOS and Linux a library loads and is deleted as any file.
    const pipe = enginePipeName(join(home, "rerail.sqlite"));
    expect(await stopDaemon(home, machine("win32", [pipeNameOf(pipe).toUpperCase()]))).toBe("not-running");
    expect(await stopDaemon(home, machine("win32", [pipeNameOf(fallbackPipe(pipe))]))).toBe("not-running");
    expect(await stopDaemon(home, machine("linux"))).toBe("not-running");
    expect(asked).toEqual([home, home, home]);
  });

  // The lock cannot be asked where the engine does not load or the lock file does not open (`posix` asks none). The
  // daemon a start launches then takes the lock itself, or exits naming the daemon that holds it.
  it("takes a home lock it cannot ask as free: a stop answers at once, and a start goes on to launch", async () => {
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(join(home, LOCKS_DIR, "daemon.lock"), "");
    // The record a holder killed outright left, naming a pid another process that runs has taken since.
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: process.pid }));
    const asked = Date.now();
    expect(await stopDaemon(home, posix)).toBe("not-running");
    const launching = new Error("the start went on to launch a daemon");
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    await expect(startDaemon({ home, packageRoot, ensureSqlite: async () => { throw launching; } }, { dev: true, port: 0 }, posix))
      .rejects.toBe(launching);
    expect(Date.now() - asked).toBeLessThan(5_000);
  }, 40_000);

  it("repairs the SQLite binding before spawning, and surfaces an unrepairable binding instead of a daemon crash", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const repaired: string[] = [];
    const running = await startDaemon(
      { home, packageRoot, ensureSqlite: async root => { repaired.push(root); return { prefix: root, rebuilt: false }; } },
      { dev: true, port: 0 },
    );
    try {
      expect(repaired).toEqual([packageRoot]);
      expect(running.started).toBe(true);
      // The daemon it launched holds none of the start's handles, the lock among them.
      expect(await withStartLock(home, async () => "the start lock is free")).toBe("the start lock is free");
    } finally {
      await stopDaemon(home);
    }

    const failure = new Error("better-sqlite3 in /nowhere cannot load under Node 25.2.1 (ABI 141) and npm rebuild did not fix it.");
    await expect(startDaemon(
      { home, packageRoot, ensureSqlite: async () => { throw failure; } },
      { dev: true, port: 0 },
    )).rejects.toBe(failure);
    expect(existsSync(join(home, DAEMON_STATE_FILE))).toBe(false);
    expect(await withStartLock(home, async () => "the start lock is free")).toBe("the start lock is free");
  });

  it("ends the autostart claim when its start ends, whether it fails or leaves a daemon answering", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const control = { home, packageRoot, ensureSqlite: async (root: string) => ({ prefix: root, rebuilt: false }) };
    const claim = join(home, AUTOSTART_CLAIM_FILE);
    // The claim names its launcher, here this process, which runs the start.
    await writeFile(claim, String(process.pid));
    const failure = new Error("better-sqlite3 cannot load");
    await expect(startDaemon({ ...control, ensureSqlite: async () => { throw failure; } }, { dev: true, port: 0 })).rejects.toBe(failure);
    // The launch is over and no daemon came up: the next process to find it down launches again at once.
    expect(existsSync(claim)).toBe(false);
    try {
      await writeFile(claim, String(process.pid));
      expect((await startDaemon(control, { dev: true, port: 0 })).started).toBe(true);
      expect(existsSync(claim)).toBe(false);
      await writeFile(claim, String(process.pid));
      // A launch that finds the daemon running ends its claim too.
      expect((await startDaemon(control, { dev: true, port: 0 })).started).toBe(false);
      expect(existsSync(claim)).toBe(false);
    } finally {
      await stopDaemon(home);
    }
  }, 30_000);

  it("leaves the autostart claim of another launcher to it, whether this start got the start lock or not", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const failure = new Error("better-sqlite3 cannot load");
    const failing = { home, packageRoot, ensureSqlite: async () => { throw failure; } };
    const claim = join(home, AUTOSTART_CLAIM_FILE);
    // The launcher an autostart started, which runs while its start does: here a process that runs, this one's parent.
    const launcher = String(process.ppid);
    await writeFile(claim, launcher);
    // A start that waits out the start lock the launcher holds for its whole start, and gives up.
    await withStartLock(home, async () => {
      await expect(startDaemon(failing, { dev: true, port: 0 })).rejects.toThrow(/Another Rerail start/u);
    });
    expect(await readFile(claim, "utf8")).toBe(launcher);
    // A `rerail daemon start` someone runs, which gets the start lock before the launcher does.
    await expect(startDaemon(failing, { dev: true, port: 0 })).rejects.toBe(failure);
    expect(await readFile(claim, "utf8")).toBe(launcher);
  }, 40_000);

  it("shows only this start's output when the daemon exits before it is ready, not earlier runs' log", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    await writeFile(join(home, DAEMON_LOG_FILE), "rerail daemon 0.0.0-earlier listening at http://127.0.0.1:1/ (pid 1)\n");
    const blocker = createServer();
    blocker.listen(0, "127.0.0.1");
    await once(blocker, "listening");
    try {
      const { port } = blocker.address() as AddressInfo;
      const failure = await startDaemon(
        { home, packageRoot, ensureSqlite: async root => ({ prefix: root, rebuilt: false }) },
        { dev: true, port },
      ).then(() => undefined, (error: unknown) => String(error));
      expect(failure).toMatch(/EADDRINUSE/u);
      expect(failure).not.toContain("0.0.0-earlier");
    } finally {
      blocker.close();
    }
  });

  it("stops a daemon that has a hook's request still unanswered, without the kill that follows its wait", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const { state } = (await startDaemon(
      { home, packageRoot, ensureSqlite: async root => ({ prefix: root, rebuilt: false }) },
      { dev: true, port: 0 },
    )).daemon;
    // A hook's event the daemon has not answered yet: its request is open on the engine socket.
    const hook = connect(state.kernelSocketPath);
    hook.on("error", () => undefined);
    const kill = vi.spyOn(process, "kill");
    let passed = false;
    try {
      await once(hook, "connect");
      hook.write(`POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n${DAEMON_ACCESS_HEADER}: ${state.accessToken ?? ""}\r\n\r\n{`);
      expect(await stopDaemon(home)).toBe("stopped");
      // The daemon ended itself; before, it waited on the hook and stop killed it once its 10 s were up.
      expect(kill.mock.calls.filter(([, signal]) => signal === "SIGKILL")).toEqual([]);
      expect(processAlive(state.pid)).toBe(false);
      passed = true;
    } finally {
      kill.mockRestore();
      hook.destroy();
      if (processAlive(state.pid)) process.kill(state.pid, "SIGKILL");
      // The stop answers once the daemon's main thread has exited, while Node counts the daemon as running until its
      // last thread has: the clean-up removes the home only once nothing this test started runs there. The wait runs
      // here and not in onTestFinished, which Vitest runs after the afterEach that removes the home.
      const gone = vi.waitFor(() => expect(runningIn(home)).toEqual([]), { timeout: 5_000, interval: 10 });
      // A wait that fails after a body that failed would replace the body's error, which says what went wrong; the
      // home's removal still fails then, naming what runs there.
      await (passed ? gone : gone.catch(() => undefined));
    }
  }, 30_000);

  it("replaces a healthy daemon from the previous managed package before using the active package", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    // The daemon of an earlier build keeps the record written over its own here; the daemon this build starts would
    // write its own back within a second, as it does for any record it did not write.
    const previous = await olderDaemon(home, { ...process.env, RERAIL_HOME: home });
    try {
      const stateFile = join(home, DAEMON_STATE_FILE);
      await writeFile(stateFile, JSON.stringify({ ...await readDaemonState(stateFile), packageRoot: join(home, "versions", "0.0.0-old") }));

      const replacement = await startDaemonForPackage(
        { home, packageRoot },
        { dev: true, port: 0 },
      );

      expect(replacement.started).toBe(true);
      expect(replacement.daemon.state.pid).not.toBe(previous.pid);
      expect(await daemonUsesPackageRoot(replacement.daemon.state, packageRoot)).toBe(true);
    } finally {
      await stopDaemon(home);
      if (previous.exitCode === null && previous.signalCode === null) previous.kill("SIGKILL");
    }
  }, 30_000);

  it("names RERAIL_AUTOSTART as the person set it when the daemon it finds runs another package", async () => {
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const first = await startDaemon({ home, packageRoot }, { dev: true, port: 0 });
    const autostart = process.env.RERAIL_AUTOSTART;
    try {
      await writeFile(
        join(home, DAEMON_STATE_FILE),
        JSON.stringify({ ...first.daemon.state, packageRoot: join(home, "versions", "0.0.0-old") }),
      );
      process.env.RERAIL_HOME = home;
      process.env.RERAIL_AUTOSTART = "false";

      await expect(daemonEngineEndpoint(true)).rejects.toThrow("RERAIL_AUTOSTART=false requires a daemon from the active Rerail package");
    } finally {
      if (autostart === undefined) delete process.env.RERAIL_AUTOSTART;
      else process.env.RERAIL_AUTOSTART = autostart;
      await stopDaemon(home);
    }
  }, 15_000);
});

describe("openDashboard", () => {
  const url = "http://127.0.0.1:4737/#rules";
  // Orca runs on macOS and Linux, and never on Windows: these tests open pages as macOS does, on every OS.
  const desktopOpener = browserOpener(url, "darwin").command;
  let outsideOrca: NodeJS.ProcessEnv;
  let underOrca: NodeJS.ProcessEnv;
  /** An installed Orca CLI, preceded by an absent candidate that must be skipped. */
  let orca: string;
  let candidates: string[];
  let scratch: string;

  beforeEach(async () => {
    scratch = await mkdtemp(join(tmpdir(), "rerail-orca-"));
    orca = join(scratch, "orca");
    await writeFile(orca, "");
    candidates = [join(scratch, "missing", "orca"), orca];
    outsideOrca = { RERAIL_HOME: join(scratch, "home") };
    underOrca = { ...outsideOrca, ORCA_TAB_ID: "tab-1" };
  });

  /**
   * A macOS machine with `env` that records every spawn in `commands`; `orca` children settle per `settle`, desktop
   * openers are inert.
   */
  function opening(env: NodeJS.ProcessEnv, settle: (child: EventEmitter) => void, opened = url): { commands: string[]; machine: DaemonMachine } {
    const commands: string[] = [];
    const start = (command: string, args: readonly string[]): StartedChild => {
      commands.push(command);
      const child = Object.assign(new EventEmitter(), { unref() {} });
      if (command === orca) {
        expect(args).toEqual(["tab", "create", "--url", opened]);
        process.nextTick(() => settle(child));
      }
      return child;
    };
    return { commands, machine: fakeMachine("darwin", { env, spawn: start, spawnShown: start }) };
  }

  afterEach(async () => {
    await removeFolder(scratch);
  });

  it("opens the Windows browser through System32's rundll32 from the home folder, never a rundll32 the repository holds", () => {
    const opener = browserOpener(url, "win32", { SystemRoot: String.raw`D:\Windows` });
    expect(opener).toEqual({ command: String.raw`D:\Windows\System32\rundll32.exe`, args: ["url.dll,FileProtocolHandler", url], cwd: homedir() });
    expect(browserOpener(url, "darwin")).toEqual({ command: "open", args: [url] });
    expect(browserOpener(url, "linux")).toEqual({ command: "xdg-open", args: [url] });
  });

  it("opens no browser and no Orca tab under BROWSER=none, and says so, so the caller shows the link", async () => {
    for (const env of [outsideOrca, underOrca]) {
      const { commands, machine } = opening({ ...env, BROWSER: "none" }, () => { throw new Error("orca must not be spawned"); });
      expect(await openDashboard(url, machine, candidates)).toBe("not-opened");
      expect(commands).toEqual([]);
    }
    // `rerail daemon start --open` hands its link to the browser opener alone; the fake machine refuses every spawn.
    expect(openInBrowser(url, fakeMachine("win32", { env: { BROWSER: "none" } }))).toBe(false);
  });

  it("uses the desktop browser outside Orca", async () => {
    const { commands, machine } = opening(outsideOrca, () => { throw new Error("orca must not be spawned"); });
    expect(await openDashboard(url, machine, candidates)).toBe("system-browser");
    expect(commands).toEqual([desktopOpener]);
  });

  it("opens a tab in the Orca window through its installed absolute path", async () => {
    const { commands, machine } = opening(underOrca, child => child.emit("exit", 0, null));
    expect(await openDashboard(url, machine, candidates)).toBe("orca-tab");
    expect(commands).toEqual([orca]);
  });

  it("uses the desktop browser without consulting PATH when no Orca CLI is installed", async () => {
    const { commands, machine } = opening(underOrca, () => { throw new Error("orca must not be spawned"); });
    expect(await openDashboard(url, machine, [join(scratch, "missing", "orca")])).toBe("system-browser");
    expect(commands).toEqual([desktopOpener]);
  });

  it("falls back to the desktop browser when orca cannot be spawned", async () => {
    const { commands, machine } = opening(underOrca, child => child.emit("error", Object.assign(new Error("spawn orca EACCES"), { code: "EACCES" })));
    expect(await openDashboard(url, machine, candidates)).toBe("system-browser");
    expect(commands).toEqual([orca, desktopOpener]);
  });

  it("falls back to the desktop browser when orca fails", async () => {
    const { commands, machine } = opening(underOrca, child => child.emit("exit", 1, null));
    expect(await openDashboard(url, machine, candidates)).toBe("system-browser");
    expect(commands).toEqual([orca, desktopOpener]);
  });

  it("hands the home daemon's access capability to the browser, only for that daemon's own pages", async () => {
    const home = outsideOrca.RERAIL_HOME!;
    await mkdir(home, { recursive: true });
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4737/", accessToken: "cap" })));
    const { commands, machine } = opening(underOrca, child => child.emit("exit", 0, null), "http://127.0.0.1:4737/?rerail-access=cap#rules");
    expect(await openDashboard(url, machine, candidates)).toBe("orca-tab");
    expect(commands).toEqual([orca]);
    // Another origin is not this daemon, and a loopback-only daemon has nothing to hand over.
    expect(await dashboardAccessLink("http://127.0.0.1:1/#rules", home, "darwin")).toBe("http://127.0.0.1:1/#rules");
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4737/" })));
    expect(await dashboardAccessLink(url, home, "darwin")).toBe(url);
  });
});

describe("stopping the daemon", () => {
  it("names a check that runs where the person is when the daemon outlives the kill", () => {
    expect(stillRunningMessage(123, "win32")).toContain(`tasklist /FI "PID eq 123"`);
    expect(stillRunningMessage(123, "win32")).not.toContain("ps -p");
    expect(stillRunningMessage(123, "linux")).toContain("ps -p 123");
  });

  it("counts a daemon that ends just as its signal is sent as stopped, at the SIGTERM or at the SIGKILL", async () => {
    // A daemon's own shutdown waits out its sessions' requests, about as long as a stop waits before its SIGKILL; it can
    // end between the stop's last look and the signal, which kill then answers with ESRCH. The stop failed with it.
    const home = await mkdtemp(join(tmpdir(), "rerail-stop-race-"));
    try {
      for (const endsAt of ["SIGTERM", "SIGKILL"]) {
        const stateFile = join(home, DAEMON_STATE_FILE);
        const state = stateFor({});
        await writeFile(stateFile, JSON.stringify(state));
        const signals: Array<string | number | undefined> = [];
        let running = true;
        const machine = fakeMachine("linux", {
          // The daemon ends as `endsAt` is sent, before kill reaches it; until then it outlives each signal.
          kill: (_pid, signal) => {
            signals.push(signal);
            if (signal === endsAt) running = false;
            if (!running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
            return true;
          },
          // It started no later than its record says, which proves it the home's daemon, as a stop asks before a signal.
          processStartedAt: () => state.startedAt,
        });
        await stopFoundDaemon(home, { status: "unresponsive", state }, machine);
        expect(signals.filter(signal => signal !== 0)).toEqual(endsAt === "SIGTERM" ? ["SIGTERM"] : ["SIGTERM", "SIGKILL"]);
        expect(existsSync(stateFile)).toBe(false);
      }
    } finally {
      await removeFolder(home);
    }
  }, 20_000);
});

describe("RERAIL_AUTOSTART", () => {
  // One reading for every caller: a hook's engine endpoint, the session-start autostart, set-up and `rerail status`.
  it.each(["0", "false"])("reads %s as off, so a hook's engine endpoint starts no daemon and asks for a running one", async setting => {
    expect(autostartOff({ RERAIL_AUTOSTART: setting })).toBe(true);
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-off-"));
    try {
      // No state file, so no daemon runs; the fake machine's spawns fail the test, so a start would show.
      const machine = fakeMachine("linux", { env: { RERAIL_HOME: home, RERAIL_AUTOSTART: setting } });
      await expect(daemonEngineEndpoint(true, undefined, machine)).rejects.toBeInstanceOf(AutostartOffError);
      // It names the setting as the person spelled it, wherever the error shows.
      await expect(daemonEngineEndpoint(true, undefined, machine)).rejects.toThrow(`RERAIL_AUTOSTART=${setting} requires a running daemon`);
    } finally {
      await removeFolder(home);
    }
  });

  it("reads anything else as on", () => {
    for (const env of [{}, { RERAIL_AUTOSTART: "1" }, { RERAIL_AUTOSTART: "" }]) expect(autostartOff(env)).toBe(false);
  });
});

describe("what rerail status says of a daemon that is not running", () => {
  it("says RERAIL_AUTOSTART=0 keeps hooks from starting it, since hooks log nothing about a daemon the person turned off", () => {
    expect(daemonNotRunningMessage({})).toBe("The Rerail daemon is not running. Start it with rerail daemon start.");
    expect(daemonNotRunningMessage({ RERAIL_AUTOSTART: "0" })).toBe(
      "The Rerail daemon is not running, and RERAIL_AUTOSTART=0 keeps hooks from starting it, so nothing is recorded until it runs. Start it with rerail daemon start.",
    );
    expect(daemonNotRunningMessage({ RERAIL_AUTOSTART: "false" })).toContain("and RERAIL_AUTOSTART=false keeps hooks from starting it");
  });
});

describe("a state file Windows left behind", () => {
  const PIPE = enginePipeName(String.raw`C:\Users\me\.rerail\rerail.sqlite`);
  /** The engine pipe as `\\.\pipe\` lists it. */
  const PIPE_NAME = pipeNameOf(PIPE);
  let home: string;
  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-stale-"));
  });
  afterEach(async () => {
    vi.unstubAllGlobals();
    await removeFolder(home);
  });

  /**
   * A Windows machine where every pid, even one no process can hold, is a live process of this user, with `pipes`, where
   * every engine refuses the capability proof, and whose main-instance records forgotten go to `forgotten`.
   */
  function windows(pipes: readonly string[], signals: Array<string | number | undefined> = [], forgotten: number[] = []): DaemonMachine {
    return fakeMachine("win32", {
      kill: (_pid, signal) => {
        signals.push(signal);
        return true;
      },
      pipes: () => pipes,
      forgetMainInstanceHolder: pid => {
        forgotten.push(pid);
      },
    });
  }

  const denied = (): boolean => {
    throw Object.assign(new Error("kill EPERM"), { code: "EPERM" });
  };

  /** An engine that proves it holds the capability it is asked about. */
  const proving: Pick<DaemonMachine, "capabilityProof"> = { capabilityProof: async () => "proved" };

  it("has one rule for whether the daemon a state file records may be alive, which every reader of it asks", async () => {
    const state = stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability" });
    expect(processAlive(4, denied)).toBe(true);
    expect(await recordedDaemonAlive(state, { ...windows([PIPE_NAME]), ...proving })).toBe(true);
    // A live pid without the daemon's pipe is one Windows gave to another process once the daemon was gone.
    expect(await recordedDaemonAlive(state, { ...windows(["other"]), ...proving })).toBe(false);
    // A live pid of this user and the pipe's name are not enough: Windows reuses pids, and anyone can create the name.
    expect(await recordedDaemonAlive(state, windows([PIPE_NAME]))).toBe(false);
    // EPERM is another user's process, or this user's elevated daemon, which alone holds the capability.
    expect(await recordedDaemonAlive(state, { ...windows([PIPE_NAME]), kill: denied })).toBe(false);
    expect(await recordedDaemonAlive(state, { ...windows([PIPE_NAME]), kill: denied, ...proving })).toBe(true);
    // An engine silent to the proof has proved nothing, so it is sent nothing.
    expect(await recordedDaemonAlive(state, { ...windows([PIPE_NAME]), capabilityProof: async () => "silent" })).toBe(false);
    // Elsewhere the pid decides, and an EPERM is another user's process.
    expect(await recordedDaemonAlive(state, { ...windows([]), platform: "linux" })).toBe(true);
    expect(await recordedDaemonAlive(state, { ...windows([]), platform: "linux", kill: denied, ...proving })).toBe(false);
  });

  it("keeps the state of a daemon this user started elevated, found by its engine's proof, and drops one nothing proves", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability" });
    const probe = vi.fn(async () => Response.json({ status: "ok", pid: DEAD_PID }));
    vi.stubGlobal("fetch", probe);
    const asked: Array<[string, string]> = [];
    // An elevated process answers a medium-integrity `kill(pid, 0)` with EPERM, as another user's does.
    const elevated: DaemonMachine = {
      ...windows([PIPE_NAME]),
      kill: denied,
      capabilityProof: async (socketPath, accessToken) => {
        asked.push([socketPath, accessToken]);
        return "proved";
      },
    };
    await writeFile(stateFile, JSON.stringify(state));
    expect(await lookupDaemon(home, elevated)).toMatchObject({ status: "running", state: { pid: DEAD_PID } });
    expect(asked).toEqual([[PIPE, "capability"]]);
    expect(existsSync(stateFile)).toBe(true);

    // Another user's process on a reused pid holds no such engine: the state is stale, and its URL is never probed.
    probe.mockClear();
    expect(await lookupDaemon(home, { ...elevated, capabilityProof: async () => "refused" })).toEqual({ status: "absent" });
    expect(probe).not.toHaveBeenCalled();
    expect(existsSync(stateFile)).toBe(false);

    // On macOS and Linux an EPERM is another user's process, and nothing is asked.
    await writeFile(stateFile, JSON.stringify(state));
    asked.length = 0;
    expect(await lookupDaemon(home, { ...elevated, platform: "linux" })).toEqual({ status: "absent" });
    expect(asked).toEqual([]);
  });

  it("sends the capability to a daemon this user may signal only once its engine proves it holds it", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability" });
    const sent: Array<string | null> = [];
    vi.stubGlobal("fetch", vi.fn(async (_url: URL, init: RequestInit) => {
      sent.push(new Headers(init.headers).get("x-rerail-access"));
      return Response.json({ status: "ok", pid: DEAD_PID });
    }));
    // A pid Windows gave to another of this user's processes, and the pipe's name made by someone else: no proof.
    await writeFile(stateFile, JSON.stringify(state));
    expect(await lookupDaemon(home, windows([PIPE_NAME]))).toEqual({ status: "absent" });
    expect(sent).toEqual([]);
    expect(existsSync(stateFile)).toBe(false);

    await writeFile(stateFile, JSON.stringify(state));
    expect(await lookupDaemon(home, { ...windows([PIPE_NAME]), ...proving })).toMatchObject({ status: "running", state: { pid: DEAD_PID } });
    expect(sent).toEqual(["capability"]);
  });

  it("waits for a daemon whose engine is silent to the proof, as for a busy one, and sends it nothing", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability", nativeToken: "native" });
    const requests: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (url: URL) => {
      requests.push(String(url));
      return Response.json({ status: "ok", pid: DEAD_PID });
    }));
    const waits: number[] = [];
    const signals: Array<string | number | undefined> = [];
    let running = true;
    await holdHome(home, DEAD_PID);
    // Elevated or not, a daemon whose one event loop is in an audit answers no proof within the wait.
    for (const kill of [denied, () => true]) {
      await writeFile(stateFile, JSON.stringify(state));
      waits.length = 0;
      const busy: DaemonMachine = {
        ...windows([PIPE_NAME]),
        env: { RERAIL_HOME: home },
        homeDaemonLockHeld: () => true,
        kill,
        capabilityProof: async (_socketPath, _accessToken, timeoutMs) => {
          waits.push(timeoutMs);
          return "silent";
        },
      };
      expect(await lookupDaemon(home, busy)).toEqual({ status: "unresponsive", state });
      expect(existsSync(stateFile)).toBe(true);
      // A hook asks once more, giving the proof the busy wait.
      expect(await runningDaemonEngineEndpoint(undefined, busy)).toEqual({ status: "unresponsive", state });
      expect(waits).toEqual([2_000, 2_000, 10_000]);
      expect(existsSync(stateFile)).toBe(true);
    }
    expect(requests).toEqual([]);

    // Stopping it sends no shutdown request with the capability: the process is ended instead.
    const silent: DaemonMachine = {
      ...windows([PIPE_NAME]),
      homeDaemonLockHeld: () => running,
      kill: (_pid, signal) => {
        signals.push(signal);
        if (signal === "SIGTERM") running = false;
        if (signal === 0 && !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
      capabilityProof: async () => "silent",
    };
    expect(await stopDaemon(home, silent)).toBe("stopped");
    expect(requests).toEqual([]);
    expect(signals.filter(signal => signal !== 0)).toEqual(["SIGTERM"]);
    expect(existsSync(stateFile)).toBe(false);
  });

  it("takes a live pid without the daemon's engine pipe for a reused one: it removes the state and kills nothing", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "secret" })));
    const probe = vi.fn(async () => Response.json({ status: "ok", pid: DEAD_PID }));
    vi.stubGlobal("fetch", probe);
    const signals: Array<string | number | undefined> = [];
    expect(await stopDaemon(home, windows([], signals))).toBe("not-running");
    expect(signals.filter(signal => signal !== 0)).toEqual([]);
    expect(probe).not.toHaveBeenCalled();
    expect(existsSync(stateFile)).toBe(false);

    // A daemon that holds the home, whose pipe is there but which does not answer, is still the daemon, and start
    // refuses to go over it.
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new Error("connect ECONNREFUSED 127.0.0.1:1");
    }));
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    await holdHome(home, DEAD_PID);
    expect(await lookupDaemon(home, { ...windows([PIPE_NAME]), homeDaemonLockHeld: () => true })).toMatchObject({ status: "unresponsive" });
  });

  it("finds the engine pipe whatever its case, and leaves a daemon whose pipes cannot be listed to the health probe", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new Error("connect ECONNREFUSED 127.0.0.1:1");
    }));
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    await holdHome(home, DEAD_PID);
    const holding = { homeDaemonLockHeld: (): boolean => true };
    expect(await lookupDaemon(home, { ...windows(["other", PIPE_NAME.toUpperCase()]), ...holding })).toMatchObject({ status: "unresponsive" });
    const unlisted = {
      ...windows([]),
      ...holding,
      pipes: (): readonly string[] => {
        throw Object.assign(new Error("EACCES"), { code: "EACCES" });
      },
    };
    expect(await lookupDaemon(home, unlisted)).toMatchObject({ status: "unresponsive" });
    expect(existsSync(stateFile)).toBe(true);
  });

  it("asks a Windows daemon to stop over its API before any kill, since no signal reaches its shutdown there", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability", nativeToken: "native" })));
    let running = true;
    const requests: Array<{ url: string; method: string; headers: Headers }> = [];
    vi.stubGlobal("fetch", vi.fn(async (url: URL, init: RequestInit) => {
      requests.push({ url: String(url), method: init.method ?? "GET", headers: new Headers(init.headers) });
      if (String(url).endsWith("/native/shutdown")) {
        running = false;
        return Response.json({ stopping: true }, { status: 202 });
      }
      return Response.json({ status: "ok", pid: DEAD_PID });
    }));
    const signals: Array<string | number | undefined> = [];
    const forgotten: number[] = [];
    const machine = {
      ...windows([PIPE_NAME], [], forgotten),
      ...proving,
      kill: (_pid: number, signal?: string | number) => {
        signals.push(signal);
        if (signal === 0 && !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
    };
    expect(await stopDaemon(home, machine)).toBe("stopped");
    const shutdown = requests.find(request => request.url.endsWith("/native/shutdown"));
    expect(shutdown?.method).toBe("POST");
    expect(shutdown?.headers.get("authorization")).toBe("Bearer native");
    expect(shutdown?.headers.get("x-rerail-access")).toBe("capability");
    // process.kill with a signal is TerminateProcess on Windows: the daemon stopped by itself, so none was sent.
    expect(signals.filter(signal => signal !== 0)).toEqual([]);
    // A daemon Windows ends leaves its main-instance record naming a pid Windows soon reuses; the stop removes it.
    expect(forgotten).toEqual([DEAD_PID]);
    expect(existsSync(stateFile)).toBe(false);
  });

  /**
   * Stops the daemon daemon.json names (DEAD_PID), which Windows ends with no exit handler, while the home lock's
   * record names `holder`; whether that record is still there after the stop.
   */
  async function holderRecordAfterStop(holder: number): Promise<boolean> {
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    const record = join(home, LOCKS_DIR, "daemon.json");
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(record, JSON.stringify({ pid: holder }));
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({ status: "ok", pid: DEAD_PID })));
    let running = true;
    const machine = {
      ...windows([PIPE_NAME]),
      // No native token, so no shutdown request: SIGTERM is TerminateProcess on Windows.
      kill: (_pid: number, signal?: string | number) => {
        if (signal === "SIGTERM") running = false;
        if (signal === 0 && !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
    };
    expect(await stopDaemon(home, machine)).toBe("stopped");
    return existsSync(record);
  }

  it("forgets the home lock's record of the daemon a stop ended, which Windows ends with no exit handler", async () => {
    // Left there, it names a pid Windows soon reuses, and a later stop would ask the lock, loading the engine.
    expect(await holderRecordAfterStop(DEAD_PID)).toBe(false);
    // A record naming another pid is another daemon's, written once it took the lock: it holds the home still.
    expect(await holderRecordAfterStop(DEAD_PID - 1)).toBe(true);
  });

  it("stops the daemon a lookup found without looking it up, or probing it, again", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability", nativeToken: "native" })));
    let running = true;
    const probes: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (url: URL) => {
      if (String(url).endsWith("/native/shutdown")) {
        running = false;
        return Response.json({ stopping: true }, { status: 202 });
      }
      probes.push(String(url));
      return Response.json({ status: "ok", pid: DEAD_PID });
    }));
    const machine = {
      ...windows([PIPE_NAME]),
      ...proving,
      kill: (_pid: number, signal?: string | number) => {
        if (signal === 0 && !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
    };
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the recorded daemon was not found");
    await stopFoundDaemon(home, found, machine);
    expect(probes).toHaveLength(1);
    expect(existsSync(stateFile)).toBe(false);
  });

  it("counts a recorded cache holder only while a Rerail engine pipe exists on Windows", () => {
    expect(daemonMayHold(DEAD_PID, windows([]))).toBe(false);
    expect(daemonMayHold(DEAD_PID, windows(["other-pipe"]))).toBe(false);
    expect(daemonMayHold(DEAD_PID, windows([PIPE_NAME]))).toBe(true);
    expect(daemonMayHold(DEAD_PID, { ...windows([]), platform: "linux" })).toBe(true);
  });
});

describe("a record whose live pid is not the home's daemon", () => {
  const PIPE = enginePipeName(String.raw`C:\Users\me\.rerail\rerail.sqlite`);
  const LIVE_PID = 4242;
  /** Another daemon of the home, which runs. */
  const OTHER_PID = LIVE_PID + 1;
  let home: string;
  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-not-daemon-"));
    // Nothing answers on the recorded URL: a daemon killed without its cleanup took its listener with it.
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new Error("connect ECONNREFUSED 127.0.0.1:1");
    }));
  });
  afterEach(async () => {
    vi.unstubAllGlobals();
    await removeFolder(home);
  });

  /**
   * A machine on `platform` where `LIVE_PID` runs until a SIGTERM or SIGKILL reaches it and `OTHER_PID` runs, the home's
   * lock is held while `lockHeld` says so, `LIVE_PID` started at `started()` (undefined: it cannot be read), every signal
   * goes to `signals`, and on Windows the engine pipe is there and silent to the proof.
   */
  function machineWith(
    platform: NodeJS.Platform,
    signals: Array<string | number | undefined>,
    lockHeld: () => boolean | undefined,
    started: () => number | undefined = () => undefined,
  ): DaemonMachine {
    let running = true;
    return fakeMachine(platform, {
      env: { RERAIL_HOME: home },
      kill: (pid, signal) => {
        if (pid === OTHER_PID && signal === 0) return true;
        if (pid !== LIVE_PID || !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        if (signal !== 0) signals.push(signal);
        if (signal === "SIGTERM" || signal === "SIGKILL") running = false;
        return true;
      },
      pipes: () => [pipeNameOf(PIPE)],
      capabilityProof: async () => "silent",
      homeDaemonLockHeld: () => lockHeld(),
      processStartedAt: pid => (pid === LIVE_PID ? started() : undefined),
    });
  }

  const STARTED_AT = 1_700_000_000_000;
  const state = (platform: NodeJS.Platform): DaemonState => stateFor({
    pid: LIVE_PID,
    kernelSocketPath: platform === "win32" ? PIPE : "/nowhere/engine.sock",
    accessToken: "capability",
    startedAt: STARTED_AT,
  });
  /** A process that took the dead daemon's pid an hour after the daemon stamped its record. */
  const anHourLater = (): number => STARTED_AT + 3_600_000;

  it.each(["linux", "win32"] as const)("ends nothing and removes the record when the pid it names started after the record (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    // The daemon was killed: both its records still name its pid, which a process started since has now, and its lock
    // is free.
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    expect(await lookupDaemon(home, machineWith(platform, signals, () => false, anHourLater))).toEqual({ status: "absent", notDaemon: LIVE_PID });
    expect(existsSync(stateFile)).toBe(false);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    expect(await stopDaemon(home, machineWith(platform, signals, () => false, anHourLater))).toBe("not-daemon");
    expect(signals).toEqual([]);
    expect(existsSync(stateFile)).toBe(false);
  });

  // The home's daemon holds its lock and is busy, beside a holder record an earlier holder left that it could neither
  // replace nor remove, whose pid another live process has now: here the daemon of a second home.
  it.each(["linux", "win32"] as const)("keeps a busy daemon that started before its record beside a holder record naming another live process, and its stop ends only it (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    const sent: Array<[number, string | number | undefined]> = [];
    await holdHome(home, OTHER_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    // Each read of a start time goes to `bounds` with the time it may take.
    const bounds: number[] = [];
    const machine = (): DaemonMachine => {
      const base = machineWith(platform, signals, () => true, () => STARTED_AT - 500);
      return { ...base, kill: (pid, signal) => {
        if (signal !== 0) sent.push([pid, signal]);
        return base.kill(pid, signal);
      }, processStartedAt: (pid, timeoutMs) => {
        bounds.push(timeoutMs);
        return base.processStartedAt(pid, timeoutMs);
      } };
    };
    expect(await lookupDaemon(home, machine())).toEqual({ status: "unresponsive", state: state(platform) });
    expect(existsSync(stateFile)).toBe(true);
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }, machine())).rejects.toThrow(/alive but not answering/u);
    bounds.length = 0;
    expect(await stopDaemon(home, machine())).toBe("stopped");
    expect(sent).toEqual([[LIVE_PID, "SIGTERM"]]);
    expect(existsSync(stateFile)).toBe(false);
    // The stop's lookup and its check just before the SIGTERM each read within the stop's own 10 s wait, where a status
    // lookup reads within its 2 s.
    expect(bounds).toHaveLength(2);
    for (const bound of bounds) {
      expect(bound).toBeGreaterThan(2_000);
      expect(bound).toBeLessThanOrEqual(10_000);
    }
    bounds.length = 0;
    await writeFile(stateFile, JSON.stringify(state(platform)));
    await lookupDaemon(home, machine());
    expect(bounds).toHaveLength(1);
    expect(bounds[0]).toBeGreaterThan(0);
    expect(bounds[0]).toBeLessThanOrEqual(2_000);
  });

  /**
   * A machine as {@link machineWith}'s where `OTHER_PID`, which the holder record names, started at `otherStarted`; each
   * read of its start goes to `bounds` with the time it may take.
   */
  const withHolderStarted = (machine: DaemonMachine, otherStarted: number, bounds: number[] = []): DaemonMachine => ({
    ...machine,
    processStartedAt: (pid, timeoutMs) => {
      if (pid !== OTHER_PID) return machine.processStartedAt(pid, timeoutMs);
      bounds.push(timeoutMs);
      return otherStarted;
    },
  });
  /** Whether every bound in `bounds` leaves the read some time and at most a lookup's 2 s, and there is one. */
  const withinALookup = (bounds: readonly number[]): boolean => bounds.length > 0 && bounds.every(bound => bound > 0 && bound <= 2_000);
  const holderRecordWritten = (): number => statSync(join(home, LOCKS_DIR, "daemon.json")).mtimeMs;

  // The pid's own start cannot be read, so the lock decides alone.
  it.each(["linux", "win32"] as const)("removes the record when another live process wrote the home's holder record and the lock is held, and keeps it when that process started after the record (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, OTHER_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const bounds: number[] = [];
    const machine = (otherStarted: number): DaemonMachine => withHolderStarted(machineWith(platform, signals, () => true), otherStarted, bounds);
    expect(await lookupDaemon(home, machine(holderRecordWritten() - 1_000))).toEqual({ status: "absent", notDaemon: LIVE_PID });
    expect(existsSync(stateFile)).toBe(false);
    expect(withinALookup(bounds)).toBe(true);
    // The holder record's pid belongs to a process started after the record: nothing shows that process holds the lock.
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const later = machine(holderRecordWritten() + 3_600_000);
    expect(await lookupDaemon(home, later)).toEqual({ status: "unresponsive", state: state(platform) });
    await expect(stopDaemon(home, later)).rejects.toThrow(new RegExp(`could not prove that pid ${LIVE_PID}`, "u"));
    expect(signals).toEqual([]);
    expect(existsSync(stateFile)).toBe(true);
    // A holder record its writer left beside a lock no process holds proves nothing either.
    const free = withHolderStarted(machineWith(platform, signals, () => false), holderRecordWritten() - 1_000);
    expect(await lookupDaemon(home, free)).toEqual({ status: "unresponsive", state: state(platform) });
    // A pid started after daemon.json's record is another process's, whatever the holder record shows.
    const reused = withHolderStarted(machineWith(platform, signals, () => true, anHourLater), holderRecordWritten() + 3_600_000);
    expect(await lookupDaemon(home, reused)).toEqual({ status: "absent", notDaemon: LIVE_PID });
    expect(signals).toEqual([]);
  });

  // The holder record was written an hour ago and the process its pid names started half an hour ago, so before now: it
  // took a dead holder's pid since, and shows nothing of the lock. The start of the pid daemon.json names cannot be read.
  it.each(["linux", "win32"] as const)("keeps the record when the holder record's pid started after that record was written and before now, and its stop ends nothing and says what to check (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, OTHER_PID);
    const anHourAgo = new Date(Date.now() - 3_600_000);
    await utimes(join(home, LOCKS_DIR, "daemon.json"), anHourAgo, anHourAgo);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const machine = withHolderStarted(machineWith(platform, signals, () => true), Date.now() - 1_800_000);
    expect(await lookupDaemon(home, machine)).toEqual({ status: "unresponsive", state: state(platform) });
    expect(existsSync(stateFile)).toBe(true);
    const said = await stopDaemon(home, machine).then(String, (error: unknown) => String(error));
    expect(said).toMatch(new RegExp(`could not prove that pid ${LIVE_PID}, which .* names, is this home's daemon, so it ended nothing\\. See its command line with `, "u"));
    expect(said).toContain(platform === "win32" ? `ProcessId=${LIVE_PID}` : `ps -p ${LIVE_PID} -o command=`);
    expect(signals).toEqual([]);
    expect(existsSync(stateFile)).toBe(true);
  });

  // The home's daemon holds its lock and is busy, while daemon.json names a program a dead daemon's pid went to. A start
  // read takes time, as Windows PowerShell does, and the engine proof uses all of a lookup's limit, so `rerail status`
  // on Windows has none left to read it; a stop's lookup reads within the stop's own wait.
  it.each(["linux", "win32"] as const)("stops the daemon that holds the home when daemon.json names a later program instead (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const sent: Array<[number, string | number | undefined]> = [];
    let homeDaemonRuns = true;
    await holdHome(home, OTHER_PID);
    const written = holderRecordWritten();
    const started = (pid: number): number => (pid === OTHER_PID ? written - 1_000 : anHourLater());
    // The home's daemon listens on its database's engine pipe.
    const homePipe = enginePipeName(join(home, "rerail.sqlite"));
    const homeDaemon = stateFor({ pid: OTHER_PID, kernelSocketPath: platform === "win32" ? homePipe : "/nowhere/home.sock", accessToken: "capability" });
    const machine: DaemonMachine = {
      ...machineWith(platform, [], () => homeDaemonRuns),
      pipes: () => [pipeNameOf(PIPE), pipeNameOf(homePipe)],
      kill: (pid, signal) => {
        if (pid === OTHER_PID && !homeDaemonRuns) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        if (signal !== 0) sent.push([pid, signal]);
        if (pid === OTHER_PID && (signal === "SIGTERM" || signal === "SIGKILL")) homeDaemonRuns = false;
        return true;
      },
      capabilityProof: async (_socketPath, _accessToken, timeoutMs) => {
        await delay(timeoutMs);
        return "silent";
      },
      // The home's daemon writes daemon.json back once it finds it gone.
      homeDaemonLockHeld: () => {
        if (homeDaemonRuns && !existsSync(stateFile)) writeFileSync(stateFile, JSON.stringify(homeDaemon));
        return homeDaemonRuns;
      },
      processStartedAt: (pid, timeoutMs) => (platform === "win32" && timeoutMs < 200 ? undefined : started(pid)),
    };
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const status = await lookupDaemon(home, machine);
    expect(status).toEqual(platform === "win32" ? { status: "unresponsive", state: state(platform) } : { status: "absent", notDaemon: LIVE_PID });
    await writeFile(stateFile, JSON.stringify(state(platform)));
    expect(await stopDaemon(home, machine)).toBe("stopped");
    expect(sent).toEqual([[OTHER_PID, "SIGTERM"]]);
  }, 30_000);

  // The home's lock is held and its holder never writes daemon.json: a start or a stop waits its 15 s, then says what to
  // check. Here the holder started before its record, as the home's daemon does; a home copied with its timestamps beside
  // the daemon of the home it came from looks the same, so the record proves no process the one to end.
  it("names no process to end when the lock's holder never records itself, and gives the holder record's pid with the check of its command line", async () => {
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, OTHER_PID);
    const bounds: number[] = [];
    const machine = withHolderStarted(machineWith("linux", signals, () => true), holderRecordWritten() - 1_000, bounds);
    const said = await stopDaemon(home, machine).then(String, (error: unknown) => String(error));
    expect(said).toMatch(new RegExp(`A Rerail daemon holds .* names pid ${OTHER_PID}, which Rerail could not show is that daemon: see its command line with ps -p ${OTHER_PID} -o command=, and end it only if`, "u"));
    expect(said).not.toMatch(/end that process/u);
    expect(signals).toEqual([]);
    // No start is read for the message.
    expect(bounds).toEqual([]);
  }, 25_000);

  // A daemon of a release before the home's lock holds none; one whose lock file did not open holds none either; one
  // that could not write its holder record holds the lock unrecorded, beside the record an earlier holder left when it
  // could not remove that either.
  it.each([
    ["linux", "no lock", false, undefined],
    ["win32", "no lock", false, undefined],
    ["linux", "the lock, unrecorded", true, undefined],
    ["win32", "the lock, unrecorded", true, undefined],
    ["linux", "the lock, recorded by a holder that is gone", true, DEAD_PID],
    ["win32", "the lock, recorded by a holder that is gone", true, DEAD_PID],
  ] as const)("keeps a busy daemon that started before its record, and its stop ends it (%s, holding %s)", async (platform, _holds, lockHeld, holder) => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    if (holder !== undefined) await holdHome(home, holder);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const machine = (): DaemonMachine => machineWith(platform, signals, () => lockHeld, () => STARTED_AT - 500);
    expect(await lookupDaemon(home, machine())).toEqual({ status: "unresponsive", state: state(platform) });
    expect(existsSync(stateFile)).toBe(true);
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }, machine())).rejects.toThrow(/alive but not answering/u);
    expect(await stopDaemon(home, machine())).toBe("stopped");
    expect(signals).toEqual(["SIGTERM"]);
    expect(existsSync(stateFile)).toBe(false);
  });

  const unread = (): number | undefined => undefined;
  const withinTheSlack = (): number | undefined => STARTED_AT + 5_000;
  it.each([
    ["linux", "cannot be read", unread],
    ["win32", "cannot be read", unread],
    ["linux", "falls within the slack after the record", withinTheSlack],
    ["win32", "falls within the slack after the record", withinTheSlack],
  ] as const)("keeps a busy daemon nothing proves, and its stop ends nothing and says so (%s, its start %s)", async (platform, _start, started) => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const machine = (): DaemonMachine => machineWith(platform, signals, () => undefined, started);
    expect(await lookupDaemon(home, machine())).toEqual({ status: "unresponsive", state: state(platform) });
    await expect(stopDaemon(home, machine())).rejects.toThrow(new RegExp(`could not prove that pid ${LIVE_PID}, which .* names, is this home's daemon, so it ended nothing`, "u"));
    expect(signals).toEqual([]);
    expect(existsSync(stateFile)).toBe(true);
  });

  it.each(["linux", "win32"] as const)("keeps and stops a daemon that holds the home's lock and is not answering (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    expect(await lookupDaemon(home, machineWith(platform, signals, () => true))).toEqual({ status: "unresponsive", state: state(platform) });
    expect(existsSync(stateFile)).toBe(true);
    expect(await stopDaemon(home, machineWith(platform, signals, () => true))).toBe("stopped");
    expect(signals).toEqual(["SIGTERM"]);
    expect(existsSync(stateFile)).toBe(false);
  });

  // A start removed daemon.json to launch a daemon, which has taken the home's lock and records its own pid beside it
  // just as the stop asks the lock; or that daemon has recorded itself in daemon.json already.
  it.each([
    ["linux", "is removed", undefined],
    ["win32", "is removed", undefined],
    ["linux", "names another daemon", OTHER_PID],
    ["win32", "names another daemon", OTHER_PID],
  ] as const)("sends no signal once daemon.json names the pid no longer, and says it left that record as it is (%s, the record %s)", async (platform, _change, now) => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const machine = machineWith(platform, signals, () => true);
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the daemon that holds the home was not found");
    await rm(stateFile);
    const changed = now === undefined ? undefined : JSON.stringify({ ...state(platform), pid: now });
    if (changed !== undefined) await writeFile(stateFile, changed);
    const newDaemonRecords = (): boolean => {
      writeFileSync(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: OTHER_PID }));
      return true;
    };
    const result = await stopFoundDaemon(home, found, { ...machine, homeDaemonLockHeld: newDaemonRecords });
    expect(STOP_ANSWERS[result]).toBe("daemon.json changed while Rerail was stopping the daemon it named, so Rerail ended nothing and "
      + "left that record as it is; run rerail daemon stop again to stop the daemon it names now.\n");
    expect(signals).toEqual([]);
    expect(existsSync(stateFile) ? await readFile(stateFile, "utf8") : undefined).toBe(changed);
  });

  /** A change to the stop's world: to daemon.json, to the process daemon.json named, or to the home's lock. */
  type WorldChange =
    | "record-gone" | "record-other" | "record-dead" | "record-unreadable"
    | "exits" | "lets-go" | "reused" | "start-unread" | "another-holds";
  /**
   * One state a stop can meet between its proof and its answer: how it proved `LIVE_PID` the home's daemon, what changes
   * and when, what it sends, what it answers and what daemon.json holds after. The stop proved the daemon by its answer to
   * the health probe while it held the home's lock (`answer`), or, while it did not answer, by that lock recorded as its
   * own (`lock`) or by a start no later than its record (`start`). The world then changes before the stop decides its
   * SIGTERM (`before`), as it asks the home's lock (`ask`), as it asks that lock again (`again`: before its SIGTERM on
   * Linux, before its SIGKILL where a shutdown request was its SIGTERM), at the SIGTERM or, for a daemon that answers on
   * Windows, the shutdown request (`signal`), or during the wait that follows (`wait`). `sent` names the SIGTERM, which a
   * daemon that answers gets on Windows as the shutdown request. `rule` is the line of the rule the row checks:
   * 1. Before each signal the stop proves the process again from what the OS shows, never from daemon.json: the home's
   *    lock held as the pid recorded beside it, a start no later than daemon.json's `startedAt`, or, for a daemon that
   *    answered as itself while it held no lock, that it still runs.
   * 2. While the process lives and stays proven, the stop ends it as main does, whatever daemon.json says meanwhile.
   * 3. A process it does not prove gets nothing: once it has exited the stop says stopped; when the home's lock belongs
   *    to another live process and daemon.json names it no longer, it says the record changed; otherwise it says it
   *    could not prove whose process it is. It removes daemon.json only while that record names the process it stopped.
   */
  interface StopCell {
    cell: string;
    rule: 1 | 2 | 3;
    proof: "answer" | "lock" | "start";
    before?: WorldChange[];
    ask?: WorldChange[];
    again?: WorldChange[];
    signal?: WorldChange[];
    wait?: WorldChange[];
    sent: Array<"SIGTERM" | "SIGKILL">;
    answer: StopDaemonResult | "unproven";
    record: "the daemon's" | "another live process's" | "a gone process's" | "none" | "unreadable";
  }

  const STOP_CELLS: readonly StopCell[] = [
    { cell: "daemon.json names the daemon, which ends at its SIGTERM", rule: 2, proof: "lock",
      signal: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "daemon.json removed by another program while the daemon runs on", rule: 2, proof: "lock",
      before: ["record-gone"], signal: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "daemon.json removed by the daemon's own shutdown, which let go of the lock and runs on, as a second stop meets it",
      rule: 2, proof: "start", before: ["lets-go"], signal: ["exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "daemon.json unreadable", rule: 2, proof: "lock",
      before: ["record-unreadable"], signal: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "unreadable" },
    { cell: "daemon.json names another live process while the daemon holds the lock", rule: 2, proof: "lock",
      before: ["record-other"], signal: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "daemon.json names a process that is gone while the daemon holds the lock", rule: 2, proof: "lock",
      before: ["record-dead"], signal: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "daemon.json names a start's daemon, which holds the lock the daemon let go of while it runs on", rule: 3, proof: "lock",
      before: ["lets-go", "another-holds", "record-other"], sent: [], answer: "record-changed", record: "another live process's" },
    { cell: "daemon.json removed by a start, whose daemon takes the lock and records itself as the stop asks it", rule: 3,
      proof: "lock", before: ["record-gone"], ask: ["another-holds"], sent: [], answer: "record-changed", record: "none" },
    { cell: "the daemon exited just before the stop's look, its record gone with it", rule: 3, proof: "start",
      before: ["lets-go", "exits"], sent: [], answer: "stopped", record: "none" },
    { cell: "the pid is a process started after daemon.json, which still names it", rule: 1, proof: "start",
      before: ["reused"], sent: [], answer: "not-daemon", record: "none" },
    { cell: "a live pid nothing proves: no lock, no start read", rule: 1, proof: "start",
      before: ["start-unread"], sent: [], answer: "unproven", record: "the daemon's" },
    { cell: "the daemon outlives its SIGTERM holding the lock", rule: 2, proof: "lock",
      sent: ["SIGTERM", "SIGKILL"], answer: "stopped", record: "none" },
    { cell: "the daemon lets go of the lock at its SIGTERM and its exit runs on past the wait", rule: 2, proof: "answer",
      signal: ["lets-go"], sent: ["SIGTERM", "SIGKILL"], answer: "stopped", record: "none" },
    { cell: "the daemon lets go of the lock at its SIGTERM and its pid goes to a process started since", rule: 1, proof: "answer",
      signal: ["lets-go", "reused"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "a start's daemon takes the lock the daemon let go of at its SIGTERM and records itself while that exit runs on",
      rule: 3, proof: "lock", signal: ["lets-go"], wait: ["another-holds", "record-other"], sent: ["SIGTERM"], answer: "stopped",
      record: "another live process's" },
    { cell: "a start's daemon takes the lock the daemon let go of at its SIGTERM while the exit of that daemon, proven by its start, runs on",
      rule: 2, proof: "answer", signal: ["lets-go"], wait: ["another-holds", "record-other"], sent: ["SIGTERM", "SIGKILL"],
      answer: "stopped", record: "another live process's" },
    { cell: "the daemon exits during the wait", rule: 3, proof: "lock",
      wait: ["lets-go", "exits"], sent: ["SIGTERM"], answer: "stopped", record: "none" },
    { cell: "the daemon exits at its SIGTERM and a start's daemon records itself before the stop's cleanup", rule: 3, proof: "lock",
      signal: ["lets-go", "exits", "another-holds", "record-other"], sent: ["SIGTERM"], answer: "stopped", record: "another live process's" },
    { cell: "the daemon lets go of the lock at its SIGTERM, its start cannot be read, and its exit runs on past the wait",
      rule: 2, proof: "answer", before: ["start-unread"], signal: ["lets-go"], sent: ["SIGTERM", "SIGKILL"], answer: "stopped",
      record: "none" },
    { cell: "the daemon lets go of the lock after the stop proved it, its start cannot be read, and its exit runs on, as a second stop meets it",
      rule: 2, proof: "answer", before: ["start-unread"], again: ["lets-go"], sent: ["SIGTERM", "SIGKILL"], answer: "stopped",
      record: "none" },
  ];

  /** The process daemon.json named, its start, and who holds the home's lock, as a {@link StopCell} changes them. */
  interface StopWorld {
    running: boolean;
    started: number | undefined;
    holder: number | undefined;
  }

  /** Applies `change` to `world` and to the files in the home, as the process or program it stands for would. */
  function changeWorld(world: StopWorld, change: WorldChange, platform: NodeJS.Platform): void {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const named = (pid: number): void => writeFileSync(stateFile, JSON.stringify({ ...state(platform), pid }));
    const holdBy = (pid: number | undefined): void => {
      world.holder = pid;
      mkdirSync(join(home, LOCKS_DIR), { recursive: true });
      if (pid === undefined) rmSync(join(home, LOCKS_DIR, "daemon.json"), { force: true });
      else writeFileSync(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid }));
    };
    const changes: Record<WorldChange, () => void> = {
      "record-gone": () => rmSync(stateFile, { force: true }),
      "record-other": () => named(OTHER_PID),
      "record-dead": () => named(DEAD_PID),
      "record-unreadable": () => {
        rmSync(stateFile, { force: true });
        mkdirSync(stateFile);
      },
      exits: () => {
        world.running = false;
      },
      // The daemon's close: it removes daemon.json while it holds what it wrote, then its lock record, and lets go.
      "lets-go": () => {
        if (recordAfter(platform) === "the daemon's") rmSync(stateFile);
        if (world.holder === LIVE_PID) holdBy(undefined);
      },
      reused: () => {
        world.started = anHourLater();
      },
      "start-unread": () => {
        world.started = undefined;
      },
      "another-holds": () => holdBy(OTHER_PID),
    };
    changes[change]();
  }

  /**
   * A machine on `platform` for `row`: `LIVE_PID` runs until it exits or a SIGKILL reaches it, `OTHER_PID` runs, every
   * signal and shutdown request goes to `sent`, and each look at whether `LIVE_PID` runs moves the clock on 1 s, so the
   * stop's waits pass in a few polls.
   */
  function stopCellMachine(platform: NodeJS.Platform, row: StopCell, world: StopWorld, sent: string[]): DaemonMachine {
    const fired = new Set<string>();
    const at = (moment: "ask" | "again" | "signal" | "wait"): void => {
      if (fired.has(moment)) return;
      fired.add(moment);
      for (const change of row[moment] ?? []) changeWorld(world, change, platform);
    };
    let looksSinceSignal = 0;
    let asks = 0;
    const look = (): true => {
      vi.setSystemTime(Date.now() + 1_000);
      if (fired.has("signal") && ++looksSinceSignal === 3) at("wait");
      return true;
    };
    const signalled = (signal: string | number | undefined): true => {
      sent.push(String(signal));
      if (signal === "SIGKILL") world.running = false;
      else at("signal");
      return true;
    };
    vi.stubGlobal("fetch", vi.fn(async (url: URL) => {
      if (!String(url).endsWith("/native/shutdown")) throw new Error("connect ECONNREFUSED 127.0.0.1:1");
      sent.push("SIGTERM");
      at("signal");
      return Response.json({ stopping: true }, { status: 202 });
    }));
    return fakeMachine(platform, {
      env: { RERAIL_HOME: home },
      pipes: () => [pipeNameOf(PIPE)],
      capabilityProof: async () => (row.proof === "answer" ? "proved" : "silent"),
      homeDaemonLockHeld: () => {
        at((asks += 1) === 1 ? "ask" : "again");
        return world.holder !== undefined;
      },
      processStartedAt: pid => (pid === LIVE_PID && world.running ? world.started : undefined),
      kill: (pid, signal) => {
        if (pid === OTHER_PID && signal === 0) return true;
        if (pid !== LIVE_PID || !world.running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return signal === 0 ? look() : signalled(signal);
      },
    });
  }

  /** What daemon.json in the home holds after a stop, in the words of {@link StopCell}'s `record`. */
  function recordAfter(platform: NodeJS.Platform): StopCell["record"] {
    const stateFile = join(home, DAEMON_STATE_FILE);
    if (!existsSync(stateFile)) return "none";
    if (statSync(stateFile).isDirectory()) return "unreadable";
    const pid = (JSON.parse(readFileSync(stateFile, "utf8")) as { pid: number }).pid;
    if (readFileSync(stateFile, "utf8") === JSON.stringify(state(platform))) return "the daemon's";
    return pid === OTHER_PID ? "another live process's" : "a gone process's";
  }

  /**
   * The home, machine and daemon a stop finds for `row` on `platform`, as its lookup proved it: daemon.json names
   * `LIVE_PID`, which holds the home's lock as its recorded holder unless its start proves it.
   */
  async function stopCell(platform: NodeJS.Platform, row: StopCell): Promise<{ world: StopWorld; sent: string[]; machine: DaemonMachine; found: FoundDaemon }> {
    const world: StopWorld = { running: true, started: row.proof === "lock" ? undefined : STARTED_AT - 500, holder: undefined };
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(state(platform)));
    if (row.proof !== "start") {
      await holdHome(home, LIVE_PID);
      world.holder = LIVE_PID;
    }
    const sent: string[] = [];
    const daemon: DaemonState = { ...state(platform), nativeToken: "native" };
    const found: FoundDaemon = row.proof === "answer"
      ? { status: "running", state: daemon, health: { status: "ok", pid: LIVE_PID } as DaemonHealth }
      : { status: "unresponsive", state: state(platform) };
    return { world, sent, machine: stopCellMachine(platform, row, world, sent), found };
  }

  /** `unproven` for the stop's message that it could not prove `LIVE_PID` the home's daemon; any other error is thrown. */
  function unprovenOr(error: unknown): "unproven" {
    if (String(error).includes(`could not prove that pid ${LIVE_PID}`)) return "unproven";
    throw error;
  }

  describe.each(["linux", "win32"] as const)("one rule for a stop whose daemon.json or process changes under it (%s)", platform => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["Date"], now: Date.now() });
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    it.each(STOP_CELLS.map(row => [`line ${row.rule}: ${row.cell}`, row] as const))("%s", async (_name, row) => {
      const { world, sent, machine, found } = await stopCell(platform, row);
      for (const change of row.before ?? []) changeWorld(world, change, platform);
      const answer = await stopFoundDaemon(home, found, machine).catch((error: unknown) => unprovenOr(error));
      expect({ sent, answer, record: recordAfter(platform) }).toEqual({ sent: row.sent, answer: row.answer, record: row.record });
    });

    // Another program wrote daemon.json during the stop, naming a live program that started before the daemon. The stop
    // ended the daemon and removed that record, so a second stop finds none and ends nothing; a record kept would have
    // had it prove the program by its start and end it.
    it("a second stop ends nothing after daemon.json was made to name another live process during the first", async () => {
      const row = STOP_CELLS.find(cell => cell.before?.includes("record-other") === true && cell.rule === 2);
      if (row === undefined) throw new Error("the table holds no row of a record naming another live process");
      const { world, machine, found } = await stopCell(platform, row);
      for (const change of row.before ?? []) changeWorld(world, change, platform);
      expect(await stopFoundDaemon(home, found, machine)).toBe("stopped");
      vi.stubGlobal("fetch", vi.fn(async () => {
        throw new Error("connect ECONNREFUSED 127.0.0.1:1");
      }));
      const sent: string[] = [];
      let running = true;
      const second = fakeMachine(platform, {
        env: { RERAIL_HOME: home },
        pipes: () => [pipeNameOf(PIPE)],
        capabilityProof: async () => "silent",
        homeDaemonLockHeld: () => false,
        processStartedAt: pid => (pid === OTHER_PID ? STARTED_AT - 500 : undefined),
        kill: (pid, signal) => {
          if (pid !== OTHER_PID || !running) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
          vi.setSystemTime(Date.now() + 1_000);
          if (signal !== 0) sent.push(String(signal));
          running = signal === 0;
          return true;
        },
      });
      expect({ answer: await stopDaemon(home, second), sent }).toEqual({ answer: "not-running", sent: [] });
    });
  });

  // A daemon of a release before the home's lock answers its health probe as itself and holds none: its stop ends it
  // while its pid runs. Windows sends it the signal too when the record holds no token for a shutdown request.
  it.each(["linux", "win32"] as const)("ends a daemon that answers as itself and holds no lock of the home (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({ status: "ok", pid: LIVE_PID })));
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const machine: DaemonMachine = { ...machineWith(platform, signals, () => false), capabilityProof: async () => "proved" };
    expect(await lookupDaemon(home, machine)).toMatchObject({ status: "running" });
    expect(await stopDaemon(home, machine)).toBe("stopped");
    expect(signals).toEqual(["SIGTERM"]);
    expect(existsSync(stateFile)).toBe(false);
  });

  /** The home's lock as held for the first `asks` times it is asked, and free after: a daemon that ends meanwhile. */
  const heldFor = (asks: number): (() => boolean) => {
    let asked = 0;
    return () => (asked += 1) <= asks;
  };

  it.each(["linux", "win32"] as const)("ends nothing when the daemon a lookup found has let go of the home by the time the stop asks (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    // Its pid is another process's by then, started since.
    const machine = machineWith(platform, signals, heldFor(1), anHourLater);
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the daemon that holds the home was not found");
    expect(await stopFoundDaemon(home, found, machine)).toBe("not-daemon");
    expect(signals).toEqual([]);
    expect(existsSync(stateFile)).toBe(false);
  });

  it.each(["linux", "win32"] as const)("sends nothing to a daemon that ends after its lookup, as it lets go of the home, and says stopped (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    // The daemon ends as it lets go of the home, and its start can no longer be read.
    const lock = heldFor(1);
    let held = true;
    const machine: DaemonMachine = {
      ...machineWith(platform, signals, () => (held = lock())),
      kill: (_pid, signal) => {
        if (signal !== 0) signals.push(signal);
        if (!held) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
    };
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the daemon that holds the home was not found");
    expect(await stopFoundDaemon(home, found, machine)).toBe("stopped");
    expect(signals).toEqual([]);
  });

  // The stop waits its 10 s for the pid to end, as for any daemon that does not end at its SIGTERM.
  it.each(["linux", "win32"] as const)("sends no SIGKILL once a process started after the record has the pid of a daemon its start proved, after the SIGTERM (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await writeFile(stateFile, JSON.stringify(state(platform)));
    // No lock decides; the daemon ends at its SIGTERM, and a process started since takes its pid at once.
    const machine: DaemonMachine = {
      ...machineWith(platform, signals, () => false, () => (signals.includes("SIGTERM") ? anHourLater() : STARTED_AT - 500)),
      kill: (_pid, signal) => {
        if (signal !== 0) signals.push(signal);
        return true;
      },
    };
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the daemon its start proves was not found");
    expect(await stopFoundDaemon(home, found, machine)).toBe("stopped");
    expect(signals).toEqual(["SIGTERM"]);
  }, 20_000);

  /**
   * Has the clock read 5 s later at each look at whether `LIVE_PID` runs, so a stop's 10 s wait for it passes in two
   * polls, until the returned `undo`: a clock set after it would stay set for the tests that follow.
   */
  const clockOnEachLook = (machine: DaemonMachine): { machine: DaemonMachine; undo: () => void } => {
    vi.useFakeTimers({ toFake: ["Date"], now: Date.now() });
    let faked = true;
    return {
      machine: { ...machine, kill: (pid, signal) => {
        if (faked && signal === 0) vi.setSystemTime(Date.now() + 5_000);
        return machine.kill(pid, signal);
      } },
      undo: () => {
        faked = false;
        vi.useRealTimers();
      },
    };
  };

  // The stop's own read of the busy daemon's start proves it, and every read after that fails, as a Windows PowerShell
  // past its bound or refused does: the stop ends it then, and never says stopped while it runs. On Windows the record
  // holds the token of a shutdown request, which the daemon's silent engine keeps from going.
  it.each(["linux", "win32"] as const)("ends a busy daemon its stop proves when each read of its start after that fails (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    const recorded = { ...state(platform), nativeToken: "native" };
    await writeFile(stateFile, JSON.stringify(recorded));
    let reads = 0;
    const events: string[] = [];
    const base = machineWith(platform, signals, () => undefined, () => ((reads += 1) === 1 ? STARTED_AT - 500 : undefined));
    const { machine, undo } = clockOnEachLook({
      ...base,
      capabilityProof: async (socketPath, accessToken, timeoutMs) => {
        events.push("engine proof");
        return await base.capabilityProof(socketPath, accessToken, timeoutMs);
      },
      processStartedAt: (pid, timeoutMs) => {
        events.push("start read");
        return base.processStartedAt(pid, timeoutMs);
      },
      kill: (pid, signal) => {
        if (signal !== 0) events.push(String(signal));
        return base.kill(pid, signal);
      },
    });
    try {
      expect(await stopFoundDaemon(home, { status: "unresponsive", state: recorded }, machine)).toBe("stopped");
    } finally {
      undo();
    }
    expect(signals).toEqual(["SIGTERM"]);
    expect(processAlive(LIVE_PID, machine.kill)).toBe(false);
    // The one read of the start comes after the shutdown request's proof, with nothing between it and the signal.
    expect(events).toEqual(platform === "win32" ? ["engine proof", "start read", "SIGTERM"] : ["start read", "SIGTERM"]);
  });

  // The daemon outlives its SIGTERM, as a busy one on Linux does until its handler runs, and its start cannot be read
  // after that: the stop sends no SIGKILL, says so, and keeps every record of the daemon.
  it.each(["linux", "win32"] as const)("sends no SIGKILL, keeps every record and says so when the daemon outlives its SIGTERM and its start cannot be read again (%s)", async platform => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const sent: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state(platform)));
    const forgotten: number[] = [];
    const base = machineWith(platform, [], () => undefined, () => (sent.includes("SIGTERM") ? undefined : STARTED_AT - 500));
    const { machine, undo } = clockOnEachLook({
      ...base,
      kill: (pid, signal) => {
        if (signal !== 0) sent.push(signal);
        return signal === "SIGTERM" || base.kill(pid, signal);
      },
      forgetMainInstanceHolder: pid => forgotten.push(pid),
    });
    try {
      await expect(stopFoundDaemon(home, { status: "unresponsive", state: state(platform) }, machine)).rejects.toThrow(new RegExp(`could not prove that pid ${LIVE_PID}`, "u"));
    } finally {
      undo();
    }
    expect(sent).toEqual(["SIGTERM"]);
    expect(processAlive(LIVE_PID, machine.kill)).toBe(true);
    expect(JSON.parse(await readFile(stateFile, "utf8"))).toMatchObject({ pid: LIVE_PID });
    expect(JSON.parse(await readFile(join(home, LOCKS_DIR, "daemon.json"), "utf8"))).toMatchObject({ pid: LIVE_PID });
    expect(forgotten).toEqual([]);
  });

  /**
   * Stops the daemon a lookup found in `home`, which its start proves until its SIGTERM, which it outlives, as a busy one
   * on Linux does until its handler runs; its start cannot be read after that. The holder record beside the home's lock
   * names it, and the lock is asked of the files in `home` as Rerail asks it. Every signal sent goes to `sent`.
   */
  const stopOutlivingDaemon = async (platform: NodeJS.Platform, sent: Array<string | number | undefined>): Promise<StopDaemonResult> => {
    const lock = await vi.importActual<typeof import("../../src/engine/native/home-daemon-lock.js")>("../../src/engine/native/home-daemon-lock.js");
    await holdHome(home, LIVE_PID);
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(state(platform)));
    const base = machineWith(platform, [], () => undefined, () => (sent.includes("SIGTERM") ? undefined : STARTED_AT - 500));
    const { machine, undo } = clockOnEachLook({
      ...base,
      homeDaemonLockHeld: asked => lock.homeDaemonLockHeld(asked),
      kill: (pid, signal) => {
        if (signal !== 0) sent.push(signal);
        return signal === "SIGTERM" || base.kill(pid, signal);
      },
    });
    try {
      return await stopFoundDaemon(home, { status: "unresponsive", state: state(platform) }, machine);
    } finally {
      undo();
    }
  };

  // A lock file deleted under the daemon leaves it holding its lock all the same: a missing file is no lock let go.
  it.each(["linux", "win32"] as const)("sends no SIGKILL, keeps every record and says so when the home's lock file is missing and the daemon outlives its SIGTERM unproven (%s)", async platform => {
    const sent: Array<string | number | undefined> = [];
    await expect(stopOutlivingDaemon(platform, sent)).rejects.toThrow(new RegExp(`could not prove that pid ${LIVE_PID}`, "u"));
    expect(sent).toEqual(["SIGTERM"]);
    expect(JSON.parse(await readFile(join(home, DAEMON_STATE_FILE), "utf8"))).toMatchObject({ pid: LIVE_PID });
    expect(JSON.parse(await readFile(join(home, LOCKS_DIR, "daemon.json"), "utf8"))).toMatchObject({ pid: LIVE_PID });
  });

  // The daemon let go of the lock once it had closed, and its record beside the lock outlived it, as one another program
  // holds open does on Windows: the daemon is about to exit, and the next daemon may take the home.
  it.each(["linux", "win32"] as const)("sends no SIGKILL and says stopped when the daemon outlives its SIGTERM unproven after it let go of the home's lock file (%s)", async platform => {
    const sent: Array<string | number | undefined> = [];
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(join(home, LOCKS_DIR, "daemon.lock"), "");
    expect(await stopOutlivingDaemon(platform, sent)).toBe("stopped");
    expect(sent).toEqual(["SIGTERM"]);
    expect(existsSync(join(home, DAEMON_STATE_FILE))).toBe(false);
  });

  // A PowerShell that never answers stands in for the start's read on Windows, and the proof is silent at once, so the
  // read has every millisecond of each lookup's limit. The hook's wait is given as 8 s, where a hook's own is
  // HOOK_DAEMON_WAIT_MS: the first lookup's 2 s, with room for its stand-in to be ended and waited for on a loaded
  // machine before the second lookup starts.
  it("answers a hook within its wait for a busy daemon, the record kept, while the start's read never answers", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    const { spawnSync: runSync } = await vi.importActual<typeof childProcess>("node:child_process");
    const bounds: unknown[] = [];
    const neverAnswers = ((_program: string, _args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      bounds.push(options.timeout);
      return runSync(process.execPath, ["-e", "setTimeout(() => {}, 60_000)"], options);
    }) as unknown as typeof childProcess.spawnSync;
    vi.mocked(childProcess.spawnSync).mockImplementationOnce(neverAnswers).mockImplementationOnce(neverAnswers);
    await writeFile(stateFile, JSON.stringify(state("win32")));
    const machine: DaemonMachine = { ...machineWith("win32", signals, () => undefined), processStartedAt: (pid, timeoutMs) => processStartedAt(pid, timeoutMs, "win32") };
    const waitMs = 8_000;
    const arrived = Date.now();
    expect(await runningDaemonEngineEndpoint(arrived + waitMs, machine)).toEqual({ status: "unresponsive", state: state("win32") });
    // The first lookup's 2 s, then what is left of the hook's wait: no read is given time past it.
    expect(bounds).toHaveLength(2);
    expect(Number(bounds[0])).toBeLessThanOrEqual(2_000);
    expect(Number(bounds[0]) + Number(bounds[1])).toBeLessThanOrEqual(waitMs);
    // Each stand-in is ended at its bound and waited for, which takes a few seconds more on a loaded machine.
    expect(Date.now() - arrived).toBeLessThan(waitMs + 5_000);
    expect(existsSync(stateFile)).toBe(true);
    expect(signals).toEqual([]);
  }, 30_000);

  // The stop waits its 10 s for the pid to end, as for any daemon that does not end at its SIGTERM.
  it("sends no SIGKILL to a pid the daemon it stopped let go of, whoever has it now", { timeout: 20_000 }, async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const signals: Array<string | number | undefined> = [];
    await holdHome(home, LIVE_PID);
    await writeFile(stateFile, JSON.stringify(state("linux")));
    // The daemon ends at its SIGTERM and lets go of its lock, and another process takes its pid at once.
    const machine: DaemonMachine = {
      ...machineWith("linux", signals, () => !signals.includes("SIGTERM")),
      kill: (_pid, signal) => {
        if (signal !== 0) signals.push(signal);
        return true;
      },
    };
    const found = await lookupDaemon(home, machine);
    if (found.status === "absent") throw new Error("the daemon that holds the home was not found");
    expect(await stopFoundDaemon(home, found, machine)).toBe("stopped");
    expect(signals).toEqual(["SIGTERM"]);
  });
});

describe("the engine proof a lookup asks for", () => {
  // A busy daemon's engine is silent, not refusing: a lookup that read silence as a refusal dropped a daemon that runs.
  it("tells a listener that says nothing in time from one that hangs up or is not there", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-proof-"));
    const socketPath = process.platform === "win32"
      ? String.raw`\\.\pipe\rerail-proof-silent-${process.pid}-${Date.now().toString(16)}`
      : join(root, "engine.sock");
    let hangUp = false;
    const held: Socket[] = [];
    const listener = createServer((socket) => {
      if (hangUp) socket.destroy();
      else held.push(socket);
    });
    listener.listen(socketPath);
    await once(listener, "listening");
    try {
      expect(await engineCapabilityProof(socketPath, "capability", 100)).toBe("silent");
      hangUp = true;
      expect(await engineCapabilityProof(socketPath, "capability", 1_000)).toBe("refused");
    } finally {
      for (const socket of held) socket.destroy();
      const closed = Promise.withResolvers<void>();
      listener.close(() => closed.resolve());
      await closed.promise;
      await removeFolder(root);
    }
    expect(await engineCapabilityProof(socketPath, "capability", 1_000)).toBe("refused");
  });
});

describe("the engine endpoint of a busy daemon", () => {
  let home: string;

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-endpoint-"));
  });

  afterEach(async () => {
    vi.unstubAllGlobals();
    await removeFolder(home);
  });

  // RERAIL_AUTOSTART=1 starts a daemon, and the start finds this one alive; 0 starts none and looks it up.
  it.each(["1", "0"])("waits for a daemon that is alive but not answering once (RERAIL_AUTOSTART=%s)", async autostart => {
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4791/" })));
    const probes: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (url: URL) => {
      probes.push(String(url));
      throw new Error("connect ECONNREFUSED 127.0.0.1:4791");
    }));
    // A Linux daemon whose pid is alive and holds the home, and whose every health probe fails.
    await holdHome(home, DEAD_PID);
    const machine = fakeMachine("linux", { env: { RERAIL_HOME: home, RERAIL_AUTOSTART: autostart }, kill: () => true, homeDaemonLockHeld: () => true });
    await expect(daemonEngineEndpoint(true, undefined, machine)).rejects.toThrow(/alive but not answering/u);
    // The first probe and the busy wait's: against a daemon that answers late they wait up to 2 s and 10 s, and a
    // third probe would wait 10 s more.
    expect(probes).toEqual(["http://127.0.0.1:4791/healthz", "http://127.0.0.1:4791/healthz"]);
  });

  // With autostart on, a start leaves a busy daemon alone and waits for it. One that then answers from another package
  // takes no event, and the error says so without naming a setting the person did not set.
  it.each([undefined, "1"])("says a daemon of another package that answered late is running (RERAIL_AUTOSTART=%s)", async autostart => {
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4791/" })));
    let probes = 0;
    vi.stubGlobal("fetch", vi.fn(async () => {
      probes += 1;
      if (probes === 1) throw new Error("connect ECONNREFUSED 127.0.0.1:4791");
      return Response.json({ status: "ok", pid: DEAD_PID });
    }));
    const env = autostart === undefined ? { RERAIL_HOME: home } : { RERAIL_HOME: home, RERAIL_AUTOSTART: autostart };
    await holdHome(home, DEAD_PID);
    const machine = fakeMachine("linux", { env, kill: () => true, homeDaemonLockHeld: () => true });
    await expect(daemonEngineEndpoint(true, undefined, machine)).rejects.toThrow(
      new Error("the running Rerail daemon is from another Rerail package; run rerail daemon stop, then rerail daemon start"),
    );
    expect(probes).toBe(2);
  });
});

describe("launching the daemon", () => {
  let home: string;

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-launch-"));
  });

  afterEach(async () => {
    await removeFolder(home);
  });

  it("on Windows reads a launched process's exit code from the handle the native spawn kept, whoever has its pid now", () => {
    // A live pid, as when Windows has given a dead daemon's pid to another process during the start wait.
    const codes: Array<number | null> = [null, 3];
    let closed = false;
    const exitCode = (): number | null => {
      // As the native module does: it closes the handle once it has given the code, and a later read is an error.
      if (closed) throw new Error(`process ${process.pid} was not started by spawnDetached in this process`);
      const code = codes.shift() ?? null;
      closed = code !== null;
      return code;
    };
    const machine = fakeMachine("win32", { kill: () => true, spawnDetached: () => ({ pid: process.pid, exitCode }) });
    const daemon = launchDetached({ args: ["daemon.js"], cwd: undefined, env: {}, logFile: join(home, DAEMON_LOG_FILE) }, machine);
    expect(daemon.exitReason()).toBeUndefined();
    expect(daemon.exitReason()).toBe("exited with code 3");
    // Asked again once the handle is closed, it gives the same reason.
    expect(daemon.exitReason()).toBe("exited with code 3");
  });

  it("goes through the native spawn on Windows, whose hidden console the daemon's Git and npm share, never Node's detached spawn", () => {
    const launched: NativeLaunch[] = [];
    // This process's pid, which the machine reads as gone, from a binary that keeps no handle: the exit is the
    // machine's answer, not this OS's. Node's spawn is the fake machine's, which fails the test.
    const machine = fakeMachine("win32", {
      nodeExecutable: () => String.raw`C:\Program Files\nodejs\node.exe`,
      spawnDetached: launch => {
        launched.push(launch);
        return { pid: process.pid };
      },
    });
    const log = join(home, DAEMON_LOG_FILE);
    const daemon = launchDetached({ args: ["cli.js", "daemon", "start"], cwd: undefined, env: { A: "1", B: undefined }, logFile: log }, machine);
    expect(daemon.pid).toBe(process.pid);
    expect(daemon.exitReason()).toBe("exited");
    expect(launched).toEqual([{
      program: String.raw`C:\Program Files\nodejs\node.exe`, args: ["cli.js", "daemon", "start"], cwd: null, env: { A: "1" }, logFile: log,
    }]);
  });

  /** Starts the console probe as Rerail starts the daemon, through launchDetached, with its output in `log`. */
  const launchProbe = (log: string) => ({ args, cwd }: ProbeLaunch): StartedProbe => {
    const probe = launchDetached({ args, cwd, env: process.env, logFile: log });
    return {
      ended: () => probe.exitReason() !== undefined,
      kill: () => {
        if (probe.pid !== undefined) process.kill(probe.pid);
      },
      output: async () => await readFile(log, "utf8"),
    };
  };

  // What Windows did, read without asking Rerail's helper. A process started with no console, as Node's detached spawn
  // started Rerail 0.1.3's daemon, fails it twice: it has no console, and a Git it starts without windowsHide gets a
  // console of its own, with a window.
  it.runIf(process.platform === "win32")("on Windows starts a process with a console, which a Git started without windowsHide shares", async () => {
    const seen = await runConsoleProbe(home, launchProbe(join(home, "probe.log")));
    // windowsHide gives the second Git a console of its own with no window, so its console host shows the count is read.
    expect(seen).toEqual({ attached: true, consoleHosts: [0, 1] });
  }, 60_000);

  // The native spawn on Windows, Node's elsewhere: each platform's launch, on every OS.
  it.each(["linux", "win32"] as const)("starts the daemon in the Rerail home and names the caller's folder as its workspace root (%s)", async platform => {
    const launches: Array<{ args: readonly string[]; cwd: unknown }> = [];
    const machine = fakeMachine(platform, {
      spawn: (_command, args, options) => {
        launches.push({ args, cwd: options.cwd });
        const child = Object.assign(new EventEmitter(), { pid: DEAD_PID, unref() {} });
        process.nextTick(() => child.emit("exit", 1, null));
        return child;
      },
      spawnDetached: ({ args, cwd }) => {
        launches.push({ args, cwd });
        return { pid: DEAD_PID };
      },
    });
    const repository = join(home, "repository");
    await mkdir(repository);
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    await expect(startDaemon(
      { home, packageRoot, ensureSqlite: async root => ({ prefix: root, rebuilt: false }) },
      { dev: true, port: 0, cwd: repository },
      machine,
    )).rejects.toThrow(/exited/u);
    expect(launches).toHaveLength(1);
    expect(launches[0]!.cwd).toBe(home);
    const args = launches[0]!.args;
    expect(args[args.indexOf("--workspace-root") + 1]).toBe(resolve(repository));
  });

  // Every way Rerail starts the daemon but a hook's autostart (under "autostartDaemon"): `rerail daemon start` and
  // set-up's start (startDaemon), a hook's start before it sends its settings (startDaemonForPackage), and the
  // replacement a daemon starts near its heap limit (relaunchDaemon). Each must reach the native spawn, as the console
  // probe above is started. The fake machine's Node spawn fails the test: Node's detached gives the daemon no console.
  it.each(["startDaemon", "startDaemonForPackage", "relaunchDaemon"] as const)("on Windows starts the daemon through the native spawn, never Node's detached spawn (%s)", async start => {
    const launches: NativeLaunch[] = [];
    const machine = fakeMachine("win32", {
      env: { RERAIL_HOME: home },
      spawnDetached: launch => {
        launches.push(launch);
        return { pid: DEAD_PID };
      },
    });
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const control = { home, packageRoot, ensureSqlite: async (root: string) => ({ prefix: root, rebuilt: false }) };
    if (start === "relaunchDaemon") relaunchDaemon(["daemon.js"], join(home, DAEMON_LOG_FILE), machine);
    else await expect((start === "startDaemon" ? startDaemon : startDaemonForPackage)(control, { dev: true, port: 0 }, machine)).rejects.toThrow(/exited/u);
    expect(launches).toHaveLength(1);
  });
});

describe("autostartDaemon", () => {
  it("launches the daemon verb, never `rerail start`, in the Rerail home, and on Windows through the native spawn, never Node's detached one", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    const spawned: Array<{ args: readonly string[]; cwd: string | null }> = [];
    // Node's `detached: true` on Windows gives the launcher no console, so every console program it starts opens a
    // window. The fake machine's Node spawn fails the launch, which the autostart swallows: nothing would be spawned.
    const machine = fakeMachine("win32", {
      env: { RERAIL_HOME: home, RERAIL_AUTOSTART: "1" },
      spawnDetached: ({ args, cwd }) => {
        spawned.push({ args, cwd });
        return { pid: DEAD_PID };
      },
    });
    try {
      // Once per process: this is the only call this file makes.
      autostartDaemon(machine);
      expect(spawned).toHaveLength(1);
      // Windows holds a process's current folder open: a launcher started in the session's folder kept that checkout
      // from being deleted or renamed while it ran. The session's folder goes to it by name, as the daemon's workspace.
      expect(spawned[0]!.cwd).toBe(home);
      expect(spawned[0]!.args.slice(1)).toEqual(["daemon", "start", "--workspace-root", process.cwd()]);
      expect(spawned[0]!.args[0]).toMatch(/[\\/]dist[\\/]cli[\\/]cli\.js$/u);
    } finally {
      await removeFolder(home);
    }
  });

  const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const daemonControlModule = pathToFileURL(join(packageRoot, "src", "runtime", "daemon-control.ts")).href;

  /**
   * A process of its own, as every hook is, that finds no daemon in `home`: it prints `ready`, and on each line on stdin
   * calls autostartDaemon and prints how many processes it has launched, exiting after `calls` lines. Its machine counts
   * launches and starts nothing; the launcher it reports is `launcher`, by default the test's own process, which runs for
   * the whole test as a real launcher runs for its start, and null reports none. Its pid checks are real, but for
   * `handOverFrom`: the autostart that, as this hook looks at it, hands its claim to `launcher` and exits.
   */
  function hookProcess(home: string, { calls = 1, launcher = process.pid, handOverFrom }: { calls?: number; launcher?: number | null; handOverFrom?: number } = {}): {
    child: ChildProcess;
    ready: Promise<void>;
    go: () => void;
    /** Sends one line and resolves with the launches counted after it. */
    call: () => Promise<number>;
    launches: Promise<number>;
  } {
    const child = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `import { EventEmitter } from "node:events";
       import { writeFileSync } from "node:fs";
       import { join } from "node:path";
       import { createInterface } from "node:readline";
       const { AUTOSTART_CLAIM_FILE, autostartDaemon } = await import(${JSON.stringify(daemonControlModule)});
       let launches = 0;
       const launcher = ${launcher ?? "undefined"};
       const started = () => {
         launches += 1;
         return Object.assign(new EventEmitter(), { pid: launcher, unref() {} });
       };
       const handOverFrom = ${handOverFrom ?? "undefined"};
       const machine = {
         platform: process.platform,
         env: process.env,
         kill: (pid, signal) => {
           if (pid !== handOverFrom) return process.kill(pid, signal);
           writeFileSync(join(process.env.RERAIL_HOME, AUTOSTART_CLAIM_FILE), String(launcher));
           throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
         },
         pipes: () => [],
         holdsCapability: async () => false,
         forgetMainInstanceHolder: () => undefined,
         mainInstanceLockHeld: () => undefined,
         homeDaemonLockHeld: () => undefined,
         nodeExecutable: () => process.execPath,
         spawnDetached: () => { launches += 1; return { pid: launcher }; },
         spawn: started,
         spawnShown: started,
       };
       process.stdout.write("ready\\n");
       let calls = ${calls};
       for await (const _line of createInterface({ input: process.stdin })) {
         autostartDaemon(machine);
         process.stdout.write("launches " + launches + "\\n");
         if (--calls === 0) process.exit(0);
       }`,
    ], { cwd: packageRoot, env: { ...process.env, RERAIL_HOME: home, RERAIL_AUTOSTART: "1" }, stdio: ["pipe", "pipe", "pipe"] });
    let output = "";
    let errors = "";
    child.stdout!.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
    child.stderr!.setEncoding("utf8").on("data", (chunk: string) => { errors += chunk; });
    const exited = once(child, "exit");
    const counts = (): number[] => [...output.matchAll(/launches (\d+)/gu)].map(match => Number(match[1]));
    const ready = (async () => {
      while (!output.includes("ready\n")) {
        if (child.exitCode !== null) throw new Error(`hook process exited before it was ready: ${errors}`);
        await new Promise(resolve => setTimeout(resolve, 10));
      }
    })();
    let sent = 0;
    const call = async (): Promise<number> => {
      sent += 1;
      child.stdin!.write("go\n");
      while (counts().length < sent) {
        if (child.exitCode !== null) throw new Error(`hook process exited before it answered: ${errors}`);
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      return counts()[sent - 1]!;
    };
    const launches = exited.then(() => {
      const counted = counts().at(-1);
      if (counted === undefined) throw new Error(`hook process printed no count: ${errors}`);
      return counted;
    });
    launches.catch(() => undefined);
    return { child, ready, go: () => child.stdin!.write("go\n"), call, launches };
  }

  it("launches one `rerail daemon start` when several hooks find no daemon at the same moment", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    // After a daemon stops, every hook in flight finds it gone; each is its own process, so a flag in a process
    // cannot keep the others from launching.
    const hooks = Array.from({ length: 6 }, () => hookProcess(home));
    try {
      await Promise.all(hooks.map(async hook => await hook.ready));
      for (const hook of hooks) hook.go();
      const launches = await Promise.all(hooks.map(async hook => await hook.launches));
      expect(launches.reduce((sum, count) => sum + count, 0)).toBe(1);
      // The claim names the launcher, so it holds for as long as that launch runs and no longer.
      expect(await readFile(join(home, AUTOSTART_CLAIM_FILE), "utf8")).toBe(String(process.pid));
    } finally {
      for (const { child } of hooks) if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
      await removeFolder(home);
    }
  }, 60_000);

  it("leaves the start to a launcher that runs, and launches past a claim whose launcher is gone or no longer renews it", async () => {
    const claims: Array<{ content: string; at?: Date }> = [
      { content: String(process.pid) },
      // Just created, its launcher's pid not written yet.
      { content: "" },
      // Its launcher died, or was killed, without a daemon.
      { content: String(DEAD_PID) },
      // A launcher renews its claim every second while it runs (`keepAutostartClaim`): one killed 10 s ago renews it no
      // more, and the pid it names, alive now, can be another process the system gave a dead launcher's pid to.
      { content: String(process.pid), at: new Date(Date.now() - 10_000) },
      { content: String(process.pid), at: new Date(Date.now() - 60_000) },
      // Made before the clock moved back a minute: without the distance both ways it would hold until the clock caught up.
      { content: String(process.pid), at: new Date(Date.now() + 60_000) },
    ];
    const homes = await Promise.all(claims.map(async () => await mkdtemp(join(tmpdir(), "rerail-autostart-"))));
    const hooks = homes.map(home => hookProcess(home));
    try {
      await Promise.all(hooks.map(async hook => await hook.ready));
      // Written once every hook is up, just before they look: a claim holds for 5 s unrenewed, and six hooks starting
      // on a busy machine can take longer than that.
      for (const [index, { content, at }] of claims.entries()) {
        const claim = join(homes[index]!, AUTOSTART_CLAIM_FILE);
        await writeFile(claim, content);
        if (at !== undefined) await utimes(claim, at, at);
      }
      for (const hook of hooks) hook.go();
      expect(await Promise.all(hooks.map(async hook => await hook.launches))).toEqual([0, 0, 1, 1, 1, 1]);
    } finally {
      for (const { child } of hooks) if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
      await Promise.all(homes.map(async home => await removeFolder(home)));
    }
  }, 60_000);

  it("leaves the start to the launcher a claim is handed to while a hook finds the autostart that made it gone", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    const claim = join(home, AUTOSTART_CLAIM_FILE);
    // The autostart that made the claim hands it to its launcher, this test's process, then exits: a hook that read the
    // claim before the hand-over finds the autostart gone after it, and the claim names a launcher that runs.
    const autostart = DEAD_PID - 1;
    const hook = hookProcess(home, { handOverFrom: autostart });
    try {
      await hook.ready;
      await writeFile(claim, String(autostart));
      hook.go();
      expect(await hook.launches).toBe(0);
      expect(await readFile(claim, "utf8")).toBe(String(process.pid));
    } finally {
      if (hook.child.exitCode === null && hook.child.signalCode === null) hook.child.kill("SIGKILL");
      await removeFolder(home);
    }
  }, 60_000);

  it("renews the claim that names this process, its launcher, for the life of a start, and no other launcher's", async () => {
    const [own, other, late, hung] = await Promise.all([1, 2, 3, 4].map(async () => await mkdtemp(join(tmpdir(), "rerail-autostart-"))));
    const long = new Date(Date.now() - 4_000);
    const claims = {
      own: join(own!, AUTOSTART_CLAIM_FILE),
      other: join(other!, AUTOSTART_CLAIM_FILE),
      late: join(late!, AUTOSTART_CLAIM_FILE),
      hung: join(hung!, AUTOSTART_CLAIM_FILE),
    };
    await writeFile(claims.own, String(process.pid));
    await writeFile(claims.other, String(DEAD_PID));
    await writeFile(claims.late, String(process.pid));
    await writeFile(claims.hung, String(process.pid));
    for (const claim of Object.values(claims)) await utimes(claim, long, long);
    const renewed = async (claim: string): Promise<boolean> => (await stat(claim)).mtimeMs > long.getTime() + 1_000;
    // A start's waits for the start lock, a holder's record and the daemon run 15 s each: a launcher that started 31 s
    // ago may be in its last one and keeps its claim; one that started 46 s ago has outlived them all, and does not.
    const stops = [
      keepAutostartClaim(own!, Date.now()),
      keepAutostartClaim(other!, Date.now()),
      keepAutostartClaim(late!, Date.now() - 31_000),
      keepAutostartClaim(hung!, Date.now() - 46_000),
    ];
    try {
      await until(async () => await renewed(claims.own), 5_000);
      // Renewed again a second later, not only as it began: the lease of 5 s outlasts no start without it.
      const first = (await stat(claims.own)).mtimeMs;
      await delay(1_500);
      expect((await stat(claims.own)).mtimeMs).toBeGreaterThan(first);
      expect(await renewed(claims.late)).toBe(true);
      expect(await renewed(claims.other)).toBe(false);
      expect(await renewed(claims.hung)).toBe(false);
    } finally {
      for (const stop of stops) stop();
      await Promise.all([own, other, late, hung].map(async home => await removeFolder(home!)));
    }
  }, 30_000);

  it("launches at a later session start once the launch it left the start to has ended without a daemon", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    const claim = join(home, AUTOSTART_CLAIM_FILE);
    // A long-lived process, as a Pi session running Rerail in-process is, that asks once per session start.
    const session = hookProcess(home, { calls: 2 });
    try {
      await session.ready;
      await writeFile(claim, String(process.pid));
      expect(await session.call()).toBe(0);
      // That launch failed and ended its claim; the daemon is still down.
      await rm(claim);
      expect(await session.call()).toBe(1);
    } finally {
      if (session.child.exitCode === null && session.child.signalCode === null) session.child.kill("SIGKILL");
      await removeFolder(home);
    }
  }, 60_000);

  it("ends its claim when the launch starts no process, so the next process to find the daemon down launches", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    // A launch that could not start reports no pid (Node's spawn on ENOENT or EAGAIN), and its error comes later.
    const hook = hookProcess(home, { launcher: null });
    try {
      await hook.ready;
      hook.go();
      expect(await hook.launches).toBe(1);
      expect(existsSync(join(home, AUTOSTART_CLAIM_FILE))).toBe(false);
    } finally {
      if (hook.child.exitCode === null && hook.child.signalCode === null) hook.child.kill("SIGKILL");
      await removeFolder(home);
    }
  }, 60_000);
});
