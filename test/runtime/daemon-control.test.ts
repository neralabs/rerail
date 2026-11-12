import { spawn, type ChildProcess } from "node:child_process";
import type * as ChildProcessModule from "node:child_process";
import { EventEmitter, once } from "node:events";
import { connect, createServer, type AddressInfo } from "node:net";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Tests that open pages install an interceptor; every other spawn reaches the real child_process. */
const opener = vi.hoisted(() => ({
  intercept: undefined as ((command: string, args: readonly string[]) => ChildProcess) | undefined,
}));
vi.mock("node:child_process", async () => {
  const actual = await vi.importActual<typeof ChildProcessModule>("node:child_process");
  return {
    ...actual,
    spawn: (command: string, args: readonly string[], options: object) =>
      opener.intercept?.(command, args) ?? actual.spawn(command, args, options),
  };
});
import { defaultEngineSocketPath, RerailDaemon } from "../../src/daemon/index.js";
import {
  type DaemonState,
  AUTOSTART_CLAIM_FILE,
  autostartDaemon,
  DAEMON_ACCESS_HEADER,
  DAEMON_LOG_FILE,
  DAEMON_STATE_FILE,
  dashboardAccessLink,
  daemonUsesPackageRoot,
  findRunningDaemon,
  isUsableEngineSocketPath,
  lookupDaemon,
  openDashboard,
  processAlive,
  readDaemonState,
  startDaemon,
  startDaemonForPackage,
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
    await once(hook, "connect");
    hook.write(`POST /events HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n${DAEMON_ACCESS_HEADER}: ${state.accessToken ?? ""}\r\n\r\n{`);
    const kill = vi.spyOn(process, "kill");
    try {
      expect(await stopDaemon(home)).toBe("stopped");
      // The daemon ended itself; before, it waited on the hook and stop killed it once its 10 s were up.
      expect(kill.mock.calls.filter(([, signal]) => signal === "SIGKILL")).toEqual([]);
      expect(processAlive(state.pid)).toBe(false);
    } finally {
      kill.mockRestore();
      hook.destroy();
      if (processAlive(state.pid)) process.kill(state.pid, "SIGKILL");
    }
  }, 30_000);

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
  const desktopOpener = process.platform === "darwin" ? "open" : process.platform === "win32" ? "rundll32" : "xdg-open";
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

describe("autostartDaemon", () => {
  afterEach(() => {
    opener.intercept = undefined;
    vi.unstubAllEnvs();
  });

  it("launches the daemon verb, never `rerail start`, which would start the spawning repository", async () => {
    vi.stubEnv("RERAIL_HOME", await mkdtemp(join(tmpdir(), "rerail-autostart-")));
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    const spawned: Array<readonly string[]> = [];
    opener.intercept = (_command, args) => {
      spawned.push(args);
      return Object.assign(new EventEmitter(), { unref() {} }) as unknown as ChildProcess;
    };
    autostartDaemon();
    expect(spawned).toHaveLength(1);
    expect(spawned[0]!.slice(1)).toEqual(["daemon", "start"]);
    expect(spawned[0]![0]).toMatch(/[\\/]dist[\\/]cli[\\/]cli\.js$/u);
  });

  const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const daemonControlModule = pathToFileURL(join(packageRoot, "src", "runtime", "daemon-control.ts")).href;

  /**
   * A process of its own, as every hook is, that finds no daemon in `home`: it prints `ready`, and on each line on stdin
   * calls autostartDaemon and prints how many processes it has launched, exiting after `calls` lines. Its Node spawn
   * counts launches and starts nothing; the launcher it reports is `launcher`, by default the test's own process, which
   * runs for the whole test as a real launcher runs for its start, and null reports none.
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
      `import childProcess from "node:child_process";
       import { EventEmitter } from "node:events";
       import { syncBuiltinESMExports } from "node:module";
       import { createInterface } from "node:readline";
       let launches = 0;
       childProcess.spawn = () => {
         launches += 1;
         return Object.assign(new EventEmitter(), { pid: ${launcher ?? "undefined"}, unref() {} });
       };
       syncBuiltinESMExports();
       const { autostartDaemon } = await import(${JSON.stringify(daemonControlModule)});
       process.stdout.write("ready\\n");
       let calls = ${calls};
       for await (const _line of createInterface({ input: process.stdin })) {
         autostartDaemon();
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
      await rm(home, { recursive: true, force: true });
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
      await Promise.all(homes.map(async home => await rm(home, { recursive: true, force: true })));
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
      await rm(home, { recursive: true, force: true });
    }
  }, 60_000);

  it("ends its claim when the launch starts no process, so the next process to find the daemon down launches", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-autostart-"));
    // Node's spawn reports a launch that could not start (ENOENT, EAGAIN) with no pid, and the error comes later.
    const hook = hookProcess(home, { launcher: null });
    try {
      await hook.ready;
      hook.go();
      expect(await hook.launches).toBe(1);
      expect(existsSync(join(home, AUTOSTART_CLAIM_FILE))).toBe(false);
    } finally {
      if (hook.child.exitCode === null && hook.child.signalCode === null) hook.child.kill("SIGKILL");
      await rm(home, { recursive: true, force: true });
    }
  }, 60_000);
});
