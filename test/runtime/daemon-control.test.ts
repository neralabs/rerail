import { spawn, type ChildProcess } from "node:child_process";
import type * as ChildProcessModule from "node:child_process";
import { EventEmitter, once } from "node:events";
import { createServer, type AddressInfo } from "node:net";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Tests that open pages install an interceptor; every other spawn reaches the real child_process. */
const opener = vi.hoisted(() => ({
  intercept: undefined as ((command: string, args: readonly string[], options: ChildProcessModule.SpawnOptions) => ChildProcess) | undefined,
}));
vi.mock("node:child_process", async () => {
  const actual = await vi.importActual<typeof ChildProcessModule>("node:child_process");
  return {
    ...actual,
    spawn: (command: string, args: readonly string[], options: ChildProcessModule.SpawnOptions) =>
      opener.intercept?.(command, args, options) ?? actual.spawn(command, args, options),
  };
});
/** Tests that launch on Windows install a native spawn; every other call reaches the real binding. */
const native = vi.hoisted(() => ({
  spawnDetached: undefined as ((program: string, args: string[], cwd: string | null, env: Record<string, string>, logPath: string) => number) | undefined,
  /** Absent, as in a binary built before it. */
  detachedExitCode: undefined as ((pid: number) => number | null) | undefined,
}));
vi.mock("../../src/engine/native/native.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/engine/native/native.js")>("../../src/engine/native/native.js");
  return {
    ...actual,
    loadNativeBinding: () => native.spawnDetached === undefined
      ? actual.loadNativeBinding()
      // Only the detached spawn and its exit code are read on this path; the cast stands in for the rest of the binding.
      : { spawnDetached: native.spawnDetached, detachedExitCode: native.detachedExitCode } as unknown as ReturnType<typeof actual.loadNativeBinding>,
  };
});
import { defaultEngineSocketPath, RerailDaemon } from "../../src/daemon/index.js";
import {
  type DaemonState,
  autostartDaemon,
  browserOpener,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  daemonMayHold,
  dashboardAccessLink,
  daemonUsesPackageRoot,
  findRunningDaemon,
  isUsableEngineSocketPath,
  launchDetached,
  lookupDaemon,
  openDashboard,
  ownProcessAlive,
  processAlive,
  readDaemonState,
  startDaemon,
  startDaemonForPackage,
  stillRunningMessage,
  stopDaemon,
  withStartLock,
} from "../../src/runtime/daemon-control.js";
import { otherUserPid } from "../support/other-user-pid.js";

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
});

describe("Rerail daemon control", () => {
  let home: string;

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-home-"));
    process.env.RERAIL_HOME = join(home, "catalog-home");
  });

  afterEach(async () => {
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await rm(home, { recursive: true, force: true });
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
    expect(await findRunningDaemon(home)).toBeUndefined();
    expect(existsSync(stateFile)).toBe(false);
    expect(await stopDaemon(home)).toBe("not-running");
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
      expect(await findRunningDaemon(home)).toBeUndefined();
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
    expect(await lookupDaemon(home)).toEqual({ status: "unresponsive", state });
    expect(existsSync(stateFile)).toBe(true);
    expect(await findRunningDaemon(home)).toBeUndefined();
    expect(existsSync(stateFile)).toBe(true);
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }))
      .rejects.toThrow(/alive but not answering/);
    expect(existsSync(join(home, "start.lock"))).toBe(false);
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
            { detached: true, stdio: "ignore" },
          );
          daemon.unref();
          process.stdout.write(String(daemon.pid));
        `],
        { stdio: ["ignore", "pipe", "ignore"] },
      );
      if (launcher.stdout === null) throw new Error("test launcher did not start");
      const [pidOutput] = await once(launcher.stdout, "data") as [Buffer];
      await once(launcher, "close");
      daemonPid = Number(pidOutput.toString());
      if (!Number.isInteger(daemonPid)) throw new Error("test daemon did not receive a pid");
      await writeFile(stateFile, JSON.stringify(stateFor({ pid: daemonPid, url: "http://127.0.0.1:1/" })));

      expect(await stopDaemon(home)).toBe("stopped");
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

  it("reclaims a start lock whose owner is dead or whose lease has expired", async () => {
    const lockPath = join(home, "start.lock");
    await mkdir(lockPath, { recursive: true });
    await writeFile(join(lockPath, "lease.json"), JSON.stringify({ pid: DEAD_PID, expiresAt: Date.now() + 60_000 }));
    // Dead owner: the lock is reclaimed and the (unresponsive) lookup still runs to its refusal.
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ pid: process.pid, url: "http://127.0.0.1:1/" })));
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }))
      .rejects.toThrow(/alive but not answering/);
    expect(existsSync(lockPath)).toBe(false);

    // Live owner but expired lease: also reclaimed, so a crashed-but-still-running starter cannot wedge the home.
    await mkdir(lockPath, { recursive: true });
    await writeFile(join(lockPath, "lease.json"), JSON.stringify({ pid: process.pid, expiresAt: Date.now() - 1 }));
    await expect(startDaemon({ home, packageRoot: "/nowhere/rerail" }, { dev: true }))
      .rejects.toThrow(/alive but not answering/);
    expect(existsSync(lockPath)).toBe(false);
  });

  it("renews its start lease while the operation runs, and only removes a lease it still owns", async () => {
    const leasePath = join(home, "start.lock", "lease.json");
    const readLease = async (): Promise<{ pid: number; expiresAt: number }> =>
      JSON.parse(await readFile(leasePath, "utf8")) as { pid: number; expiresAt: number };

    // A rebuild longer than the lease: the expiry keeps moving forward, so no other starter reclaims the lock.
    const timing = { leaseMs: 200, renewMs: 40 };
    const seen: number[] = [];
    await withStartLock(home, async () => {
      seen.push((await readLease()).expiresAt);
      await new Promise(resolve => setTimeout(resolve, 300));
      const lease = await readLease();
      seen.push(lease.expiresAt);
      expect(lease.pid).toBe(process.pid);
      expect(lease.expiresAt).toBeGreaterThan(Date.now());
    }, timing);
    const [first, renewed] = seen;
    expect(first).toBeDefined();
    expect(renewed).toBeGreaterThan(first ?? Number.POSITIVE_INFINITY);
    expect(existsSync(leasePath)).toBe(false);

    // Another live starter took the lock while we ran (it judged our lease stale): leave its lease in place.
    const other = { pid: process.ppid, expiresAt: Date.now() + 60_000 };
    await withStartLock(home, async () => {
      await writeFile(leasePath, JSON.stringify(other));
      await new Promise(resolve => setTimeout(resolve, 100));
    }, timing);
    expect(await readLease()).toEqual(other);
    await rm(join(home, "start.lock"), { recursive: true, force: true });
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
    } finally {
      await stopDaemon(home);
    }

    const failure = new Error("better-sqlite3 in /nowhere cannot load under Node 25.2.1 (ABI 141) and npm rebuild did not fix it.");
    await expect(startDaemon(
      { home, packageRoot, ensureSqlite: async () => { throw failure; } },
      { dev: true, port: 0 },
    )).rejects.toBe(failure);
    expect(existsSync(join(home, DAEMON_STATE_FILE))).toBe(false);
    expect(existsSync(join(home, "start.lock"))).toBe(false);
  });

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
    const first = await startDaemon({ home, packageRoot }, { dev: true, port: 0 });
    try {
      await writeFile(
        join(home, DAEMON_STATE_FILE),
        JSON.stringify({ ...first.daemon.state, packageRoot: join(home, "versions", "0.0.0-old") }),
      );

      const replacement = await startDaemonForPackage(
        { home, packageRoot },
        { dev: true, port: 0 },
      );

      expect(replacement.started).toBe(true);
      expect(replacement.daemon.state.pid).not.toBe(first.daemon.state.pid);
      expect(await daemonUsesPackageRoot(replacement.daemon.state, packageRoot)).toBe(true);
    } finally {
      await stopDaemon(home);
    }
  }, 15_000);
});

describe("openDashboard", () => {
  const url = "http://127.0.0.1:4737/#rules";
  const desktopOpener = browserOpener(url).command;
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

  /** Records every spawn; `orca` children settle per `settle`, desktop openers are inert. */
  function interceptSpawns(settle: (child: EventEmitter) => void, opened = url): string[] {
    const commands: string[] = [];
    opener.intercept = (command, args) => {
      commands.push(command);
      const child = Object.assign(new EventEmitter(), { unref() {} }) as unknown as ChildProcess;
      if (command === orca) {
        expect(args).toEqual(["tab", "create", "--url", opened]);
        process.nextTick(() => settle(child));
      }
      return child;
    };
    return commands;
  }

  afterEach(async () => {
    opener.intercept = undefined;
    await rm(scratch, { recursive: true, force: true });
  });

  it("opens the Windows browser through System32's rundll32 from the home folder, never a rundll32 the repository holds", () => {
    const opener = browserOpener(url, "win32", { SystemRoot: String.raw`D:\Windows` });
    expect(opener).toEqual({ command: String.raw`D:\Windows\System32\rundll32.exe`, args: ["url.dll,FileProtocolHandler", url], cwd: homedir() });
    expect(browserOpener(url, "darwin")).toEqual({ command: "open", args: [url] });
    expect(browserOpener(url, "linux")).toEqual({ command: "xdg-open", args: [url] });
  });

  it("uses the desktop browser outside Orca", async () => {
    const commands = interceptSpawns(() => { throw new Error("orca must not be spawned"); });
    expect(await openDashboard(url, outsideOrca, candidates)).toBe("system-browser");
    expect(commands).toEqual([desktopOpener]);
  });

  it.skipIf(process.platform === "win32")("opens a tab in the Orca window through its installed absolute path", async () => {
    const commands = interceptSpawns(child => child.emit("exit", 0, null));
    expect(await openDashboard(url, underOrca, candidates)).toBe("orca-tab");
    expect(commands).toEqual([orca]);
  });

  it("uses the desktop browser without consulting PATH when no Orca CLI is installed", async () => {
    const commands = interceptSpawns(() => { throw new Error("orca must not be spawned"); });
    expect(await openDashboard(url, underOrca, [join(scratch, "missing", "orca")])).toBe("system-browser");
    expect(commands).toEqual([desktopOpener]);
  });

  it.skipIf(process.platform === "win32")("falls back to the desktop browser when orca cannot be spawned", async () => {
    const commands = interceptSpawns(child => child.emit("error", Object.assign(new Error("spawn orca EACCES"), { code: "EACCES" })));
    expect(await openDashboard(url, underOrca, candidates)).toBe("system-browser");
    expect(commands).toEqual([orca, desktopOpener]);
  });

  it.skipIf(process.platform === "win32")("falls back to the desktop browser when orca fails", async () => {
    const commands = interceptSpawns(child => child.emit("exit", 1, null));
    expect(await openDashboard(url, underOrca, candidates)).toBe("system-browser");
    expect(commands).toEqual([orca, desktopOpener]);
  });

  it.skipIf(process.platform === "win32")("hands the home daemon's access capability to the browser, only for that daemon's own pages", async () => {
    const home = outsideOrca.RERAIL_HOME!;
    await mkdir(home, { recursive: true });
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4737/", accessToken: "cap" })));
    const commands = interceptSpawns(child => child.emit("exit", 0, null), "http://127.0.0.1:4737/?rerail-access=cap#rules");
    expect(await openDashboard(url, underOrca, candidates)).toBe("orca-tab");
    expect(commands).toEqual([orca]);
    // Another origin is not this daemon, and a loopback-only daemon has nothing to hand over.
    expect(await dashboardAccessLink("http://127.0.0.1:1/#rules", home)).toBe("http://127.0.0.1:1/#rules");
    await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stateFor({ url: "http://127.0.0.1:4737/" })));
    expect(await dashboardAccessLink(url, home)).toBe(url);
  });
});

describe("stopping the daemon", () => {
  it("names a check that runs where the person is when the daemon outlives the kill", () => {
    expect(stillRunningMessage(123, "win32")).toContain(`tasklist /FI "PID eq 123"`);
    expect(stillRunningMessage(123, "win32")).not.toContain("ps -p");
    expect(stillRunningMessage(123, "linux")).toContain("ps -p 123");
  });
});

describe("a state file Windows left behind", () => {
  const PIPE = String.raw`\\.\pipe\rerail-0123456789abcdef01234567`;
  let home: string;
  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-stale-"));
  });
  afterEach(async () => {
    vi.unstubAllGlobals();
    await rm(home, { recursive: true, force: true });
  });

  /**
   * A Windows machine where every pid, even one no process can hold, is a live process of this user, with `pipes`, where
   * no engine proves it holds a capability, and whose main-instance records forgotten go to `forgotten`.
   */
  function windows(pipes: readonly string[], signals: Array<string | number | undefined> = [], forgotten: number[] = []) {
    return {
      platform: "win32" as const,
      kill: (_pid: number, signal?: string | number) => {
        signals.push(signal);
        return true;
      },
      pipes: () => pipes,
      holdsCapability: async () => false,
      forgetMainInstanceHolder: (pid: number) => {
        forgotten.push(pid);
      },
    };
  }

  const denied = (): boolean => {
    throw Object.assign(new Error("kill EPERM"), { code: "EPERM" });
  };

  it("counts an EPERM from a pid as not this user's on Windows, so its URL never gets the capability", () => {
    expect(processAlive(4, denied)).toBe(true);
    expect(ownProcessAlive(4, denied)).toBe(false);
  });

  it("keeps the state of a daemon this user started elevated, found by its engine's proof, and drops one nothing proves", async () => {
    const stateFile = join(home, DAEMON_STATE_FILE);
    const state = stateFor({ pid: DEAD_PID, kernelSocketPath: PIPE, accessToken: "capability" });
    const probe = vi.fn(async () => Response.json({ status: "ok", pid: DEAD_PID }));
    vi.stubGlobal("fetch", probe);
    const asked: Array<[string, string]> = [];
    // An elevated process answers a medium-integrity `kill(pid, 0)` with EPERM, as another user's does.
    const elevated = {
      ...windows(["rerail-0123456789abcdef01234567"]),
      kill: denied,
      holdsCapability: async (socketPath: string, accessToken: string) => {
        asked.push([socketPath, accessToken]);
        return true;
      },
    };
    await writeFile(stateFile, JSON.stringify(state));
    expect(await lookupDaemon(home, elevated)).toMatchObject({ status: "running", state: { pid: DEAD_PID } });
    expect(asked).toEqual([[PIPE, "capability"]]);
    expect(existsSync(stateFile)).toBe(true);

    // Another user's process on a reused pid holds no such engine: the state is stale, and its URL is never probed.
    probe.mockClear();
    expect(await lookupDaemon(home, { ...elevated, holdsCapability: async () => false })).toEqual({ status: "absent" });
    expect(probe).not.toHaveBeenCalled();
    expect(existsSync(stateFile)).toBe(false);

    // On macOS and Linux an EPERM is another user's process, and nothing is asked.
    await writeFile(stateFile, JSON.stringify(state));
    asked.length = 0;
    expect(await lookupDaemon(home, { ...elevated, platform: "linux" })).toEqual({ status: "absent" });
    expect(asked).toEqual([]);
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
    expect(await lookupDaemon(home, windows(["rerail-0123456789abcdef01234567"]))).toMatchObject({ status: "unresponsive" });
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
      ...windows(["rerail-0123456789abcdef01234567"], [], forgotten),
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

  it("counts a recorded cache holder only while a Rerail engine pipe exists on Windows", () => {
    expect(daemonMayHold(DEAD_PID, windows([]))).toBe(false);
    expect(daemonMayHold(DEAD_PID, windows(["other-pipe"]))).toBe(false);
    expect(daemonMayHold(DEAD_PID, windows(["rerail-0123456789abcdef01234567"]))).toBe(true);
    expect(daemonMayHold(DEAD_PID, { ...windows([]), platform: "linux" })).toBe(true);
  });
});

describe("launching the daemon", () => {
  let home: string;

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-launch-"));
  });

  afterEach(async () => {
    opener.intercept = undefined;
    native.spawnDetached = undefined;
    native.detachedExitCode = undefined;
    await rm(home, { recursive: true, force: true });
  });

  it("on Windows reads a launched process's exit code from the handle the native spawn kept, whoever has its pid now", () => {
    // A live pid, as when Windows has given a dead daemon's pid to another process during the start wait.
    native.spawnDetached = () => process.pid;
    const codes: Array<number | null> = [null, 3];
    const asked: number[] = [];
    native.detachedExitCode = (pid) => {
      asked.push(pid);
      return codes.shift() ?? null;
    };
    const daemon = launchDetached({ args: ["daemon.js"], cwd: undefined, env: {}, logFile: join(home, DAEMON_LOG_FILE) }, "win32");
    expect(daemon.exitReason()).toBeUndefined();
    expect(daemon.exitReason()).toBe("exited with code 3");
    expect(asked).toEqual([process.pid, process.pid]);
  });

  it("goes through the native spawn on Windows, whose hidden console the daemon's Git and npm share, never Node's detached spawn", () => {
    const launched: Array<{ args: string[]; cwd: string | null; logPath: string }> = [];
    native.spawnDetached = (_program, args, cwd, _env, logPath) => {
      launched.push({ args, cwd, logPath });
      return DEAD_PID;
    };
    opener.intercept = () => {
      throw new Error("Node's spawn must not start a detached process on Windows");
    };
    const log = join(home, DAEMON_LOG_FILE);
    const daemon = launchDetached({ args: ["cli.js", "daemon", "start"], cwd: undefined, env: { A: "1", B: undefined }, logFile: log }, "win32");
    expect(daemon.pid).toBe(DEAD_PID);
    expect(daemon.exitReason()).toBe("exited");
    expect(launched).toEqual([{ args: ["cli.js", "daemon", "start"], cwd: null, logPath: log }]);
  });

  it("starts the daemon in the Rerail home and names the caller's folder as its workspace root", async () => {
    const launches: Array<{ args: readonly string[]; cwd: unknown }> = [];
    opener.intercept = (_command, args, options) => {
      launches.push({ args, cwd: options.cwd });
      const child = Object.assign(new EventEmitter(), { pid: DEAD_PID, unref() {} });
      process.nextTick(() => child.emit("exit", 1, null));
      return child as unknown as ChildProcess;
    };
    native.spawnDetached = (_program, args, cwd) => {
      launches.push({ args, cwd });
      return DEAD_PID;
    };
    const repository = join(home, "repository");
    await mkdir(repository);
    const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
    await expect(startDaemon(
      { home, packageRoot, ensureSqlite: async root => ({ prefix: root, rebuilt: false }) },
      { dev: true, port: 0, cwd: repository },
    )).rejects.toThrow(/exited/u);
    expect(launches).toHaveLength(1);
    expect(launches[0]!.cwd).toBe(home);
    const args = launches[0]!.args;
    expect(args[args.indexOf("--workspace-root") + 1]).toBe(resolve(repository));
  });
});

describe("autostartDaemon", () => {
  afterEach(() => {
    opener.intercept = undefined;
    native.spawnDetached = undefined;
    vi.unstubAllEnvs();
  });

  it("launches the daemon verb, never `rerail start`, and on Windows through the native spawn, never Node's detached one", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    vi.stubEnv("RERAIL_HOME", home);
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    const spawned: Array<readonly string[]> = [];
    // Node's `detached: true` on Windows gives the launcher no console, so every console program it starts opens a window.
    const detachedByNode: Array<readonly string[]> = [];
    opener.intercept = (_command, args) => {
      detachedByNode.push(args);
      return Object.assign(new EventEmitter(), { unref() {} }) as unknown as ChildProcess;
    };
    native.spawnDetached = (_program, args) => {
      spawned.push(args);
      return DEAD_PID;
    };
    try {
      // Once per process: this is the only call this file makes.
      autostartDaemon("win32");
      expect(detachedByNode).toEqual([]);
      expect(spawned).toHaveLength(1);
      expect(spawned[0]!.slice(1)).toEqual(["daemon", "start"]);
      expect(spawned[0]![0]).toMatch(/[\\/]dist[\\/]cli[\\/]cli\.js$/u);
    } finally {
      await rm(home, { recursive: true, force: true });
    }
  });
});
