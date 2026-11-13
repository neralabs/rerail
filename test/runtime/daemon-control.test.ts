import { spawn, type ChildProcess } from "node:child_process";
import { EventEmitter, once } from "node:events";
import { createServer, type AddressInfo, type Socket } from "node:net";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defaultEngineSocketPath, RerailDaemon } from "../../src/daemon/index.js";
import {
  type DaemonMachine,
  type DaemonState,
  type NativeLaunch,
  type StartedChild,
  AUTOSTART_CLAIM_FILE,
  AutostartOffError,
  autostartDaemon,
  autostartOff,
  browserOpener,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  daemonEngineEndpoint,
  daemonMayHold,
  daemonNotRunningMessage,
  dashboardAccessLink,
  daemonUsesPackageRoot,
  findRunningDaemon,
  isUsableEngineSocketPath,
  launchDetached,
  lookupDaemon,
  openDashboard,
  openInBrowser,
  processAlive,
  readDaemonState,
  recordedDaemonAlive,
  runningDaemonEngineEndpoint,
  startDaemon,
  startDaemonForPackage,
  stillRunningMessage,
  stopDaemon,
  stopFoundDaemon,
  withStartLock,
} from "../../src/runtime/daemon-control.js";
import { LOCKS_DIR } from "../../src/engine/index.js";
import { enginePipeName, fallbackPipe, isEnginePipe, pipeNameOf } from "../../src/runtime/engine-pipe.js";
import { engineCapabilityProof } from "../../src/runtime/engine-proof.js";
import { olderDaemon, until } from "../support/processes/daemon-home.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { otherUserPid } from "../support/other-user-pid.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** A pid no live process can hold on the platforms Rerail supports. */
const DEAD_PID = 2 ** 22 - 1;

const originalRerailHome = process.env.RERAIL_HOME;

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

  it("keeps the state file when a live pid stops answering, and refuses to start over it", async () => {
    // A closed port with a live pid: this process is alive, nothing listens on the recorded URL.
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: process.pid, url: "http://127.0.0.1:1/" });
    await writeFile(stateFile, JSON.stringify(state));
    expect(await lookupDaemon(home, posix)).toEqual({ status: "unresponsive", state });
    expect(existsSync(stateFile)).toBe(true);
    expect(await findRunningDaemon(home, posix)).toBeUndefined();
    expect(existsSync(stateFile)).toBe(true);
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }, posix))
      .rejects.toThrow(/alive but not answering/);
    expect(await withStartLock(home, async () => "the start lock is free")).toBe("the start lock is free");
  });

  it("stops an unresponsive live daemon pid and clears its state file", async () => {
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

      expect(await stopDaemon(home, posix)).toBe("stopped");
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
      recordCheckMs: 20,
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
      "The Rerail daemon is not running, and RERAIL_AUTOSTART=0 keeps hooks from starting it, so nothing is recorded. Start it with rerail daemon start.",
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
    // Elevated or not, a daemon whose one event loop is in an audit answers no proof within the wait.
    for (const kill of [denied, () => true]) {
      await writeFile(stateFile, JSON.stringify(state));
      waits.length = 0;
      const busy: DaemonMachine = {
        ...windows([PIPE_NAME]),
        env: { RERAIL_HOME: home },
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

    // A daemon whose pipe is there but which does not answer is still the daemon, and start refuses to go over it.
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new Error("connect ECONNREFUSED 127.0.0.1:1");
    }));
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    expect(await lookupDaemon(home, windows([PIPE_NAME]))).toMatchObject({ status: "unresponsive" });
  });

  it("finds the engine pipe whatever its case, and leaves a daemon whose pipes cannot be listed to the health probe", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    vi.stubGlobal("fetch", vi.fn(async () => {
      throw new Error("connect ECONNREFUSED 127.0.0.1:1");
    }));
    await writeFile(stateFile, JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    expect(await lookupDaemon(home, windows(["other", PIPE_NAME.toUpperCase()]))).toMatchObject({ status: "unresponsive" });
    const unlisted = {
      ...windows([]),
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

  it("forgets the home lock's record of the daemon a stop ended, which Windows ends with no exit handler", async () => {
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE })));
    const holder = join(home, LOCKS_DIR, "daemon.json");
    await mkdir(join(home, LOCKS_DIR), { recursive: true });
    await writeFile(holder, JSON.stringify({ pid: DEAD_PID }));
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
    // Left there, it names a pid Windows soon reuses, and a later stop would ask the lock, loading the engine.
    expect(existsSync(holder)).toBe(false);
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
    // A Linux daemon whose pid is alive and whose every health probe fails.
    const machine = fakeMachine("linux", { env: { RERAIL_HOME: home, RERAIL_AUTOSTART: autostart }, kill: () => true });
    await expect(daemonEngineEndpoint(true, undefined, machine)).rejects.toThrow(/alive but not answering/u);
    // The first probe and the busy wait's: against a daemon that answers late they wait up to 2 s and 10 s, and a
    // third probe would wait 10 s more.
    expect(probes).toEqual(["http://127.0.0.1:4791/healthz", "http://127.0.0.1:4791/healthz"]);
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
    const machine = fakeMachine("win32", {
      kill: () => true,
      spawnDetached: () => ({ pid: process.pid, exitCode: () => codes.shift() ?? null }),
    });
    const daemon = launchDetached({ args: ["daemon.js"], cwd: undefined, env: {}, logFile: join(home, DAEMON_LOG_FILE) }, machine);
    expect(daemon.exitReason()).toBeUndefined();
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
   * the whole test as a real launcher runs for its start, and null reports none. Its pid checks are real.
   */
  function hookProcess(home: string, { calls = 1, launcher = process.pid }: { calls?: number; launcher?: number | null } = {}): {
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
       import { createInterface } from "node:readline";
       const { autostartDaemon } = await import(${JSON.stringify(daemonControlModule)});
       let launches = 0;
       const launcher = ${launcher ?? "undefined"};
       const started = () => {
         launches += 1;
         return Object.assign(new EventEmitter(), { pid: launcher, unref() {} });
       };
       const machine = {
         platform: process.platform,
         env: process.env,
         kill: (pid, signal) => process.kill(pid, signal),
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

  it("leaves the start to a launcher that runs, and launches past a claim whose launcher is gone or that is more than 30 s from now", async () => {
    const claims: Array<{ content: string; at?: Date }> = [
      { content: String(process.pid) },
      // Just created, its launcher's pid not written yet.
      { content: "" },
      // Its launcher died, or was killed, without a daemon.
      { content: String(DEAD_PID) },
      // A pid alive now can be another process Windows gave a dead launcher's pid to.
      { content: String(process.pid), at: new Date(Date.now() - 60_000) },
      // Made before the clock moved back a minute: without the distance both ways it would hold until the clock caught up.
      { content: String(process.pid), at: new Date(Date.now() + 60_000) },
    ];
    const homes = await Promise.all(claims.map(async () => await mkdtemp(join(tmpdir(), "rerail-autostart-"))));
    for (const [index, { content, at }] of claims.entries()) {
      const claim = join(homes[index]!, AUTOSTART_CLAIM_FILE);
      await writeFile(claim, content);
      if (at !== undefined) await utimes(claim, at, at);
    }
    const hooks = homes.map(home => hookProcess(home));
    try {
      await Promise.all(hooks.map(async hook => await hook.ready));
      for (const hook of hooks) hook.go();
      expect(await Promise.all(hooks.map(async hook => await hook.launches))).toEqual([0, 0, 1, 1, 1]);
    } finally {
      for (const { child } of hooks) if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
      await Promise.all(homes.map(async home => await removeFolder(home)));
    }
  }, 60_000);

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
