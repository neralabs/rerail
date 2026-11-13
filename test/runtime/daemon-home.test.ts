import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { DAEMON_PORT_FILE } from "../../src/daemon/index.js";
import {
  type DaemonState,
  DAEMON_STATE_FILE,
  findRunningDaemon,
  processAlive,
  readDaemonState,
  startDaemon,
  stopDaemon,
} from "../../src/runtime/daemon-control.js";
import {
  daemonProcess,
  freePort,
  launchers,
  olderDaemon,
  packageRoot,
  recordedPid,
  startedDaemons,
  until,
  watchedEnvironment,
} from "../support/processes/daemon-home.js";
import { removeFolder, runningIn } from "../support/platform/remove-folder.js";

/** A pid no live process can hold on the platforms Rerail supports. */
const DEAD_PID = 2 ** 22 - 1;
const originalRerailHome = process.env.RERAIL_HOME;

/** The daemons started in `home` that still run. */
function liveDaemons(home: string): number[] {
  return startedDaemons(home).filter(pid => processAlive(pid));
}

/** Ends `child` if it still runs, and waits until it has exited, so the folder it ran in can go. */
async function ended(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  child.kill("SIGKILL");
  await exited;
}

/** Ends `pid`, which may have ended by itself since it was listed. */
function killGone(pid: number): void {
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    // Gone already.
  }
}

/**
 * What the starts, stops and crashes a Rerail home goes through leave in it: one daemon, which daemon.json names, or
 * none after a stop. Each test sees every daemon it started (`watchedEnvironment`), wherever it came from.
 */
describe("the daemons of a Rerail home", () => {
  let home: string;
  let stateFile: string;
  /** Every process a test starts gets it; written once, before any of them starts, since each reads it as it starts. */
  let env: NodeJS.ProcessEnv;
  const others: ChildProcess[] = [];
  const control = () => ({ home, packageRoot, ensureSqlite: async (root: string) => ({ prefix: root, rebuilt: false }) });
  const options = () => ({ dev: true, port: 0, env });

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-home-"));
    stateFile = join(home, DAEMON_STATE_FILE);
    env = watchedEnvironment(home);
    process.env.RERAIL_HOME = join(home, "catalog-home");
  });

  afterEach(async () => {
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await Promise.all(others.splice(0).map(async child => await ended(child)));
    const running = (): number[] => [...startedDaemons(home), ...launchers(home)].filter(pid => processAlive(pid));
    for (const pid of running()) killGone(pid);
    // Node marks a child it started as exited a moment after the OS has ended it.
    await until(() => running().length === 0 && runningIn(home).length === 0);
    await removeFolder(home);
  });

  /** A process of its own, as every hook is, that finds no daemon: on `go` it runs the real autostart and exits. */
  function hookFindingNoDaemon(): { ready: Promise<void>; go: () => void; exited: Promise<unknown> } {
    const daemonControl = pathToFileURL(join(packageRoot, "src", "runtime", "daemon-control.ts")).href;
    const child = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `const { autostartDaemon } = await import(${JSON.stringify(daemonControl)});
       process.stdout.write("ready\\n");
       process.stdin.once("data", () => { autostartDaemon(); process.exit(0); });`,
    ], { cwd: packageRoot, env: { ...env, RERAIL_AUTOSTART: "1" }, stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
    others.push(child);
    const exited = once(child, "exit");
    const ready = new Promise<void>((resolve, reject) => {
      child.stdout!.once("data", () => resolve());
      child.once("exit", code => reject(new Error(`a hook process exited (${String(code)}) before it was ready`)));
    });
    return { ready, go: () => child.stdin!.write("go\n"), exited };
  }

  it.each([2, 5, 20])("%i hooks that find no daemon at the same moment launch one start, which leaves one daemon", async count => {
    // A start launched by an autostart takes the port the home remembers, 4737 unless one is recorded.
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const hooks = Array.from({ length: count }, () => hookFindingNoDaemon());
    await Promise.all(hooks.map(async hook => await hook.ready));
    for (const hook of hooks) hook.go();
    await Promise.all(hooks.map(async hook => await hook.exited));
    await until(async () => await findRunningDaemon(home) !== undefined && launchers(home).every(pid => !processAlive(pid)), 90_000);
    expect(launchers(home)).toHaveLength(1);
    expect(liveDaemons(home)).toEqual([await recordedPid(home)]);
  }, 120_000);

  it.each([2, 5, 20])("%i daemons started at the same moment leave one running, which daemon.json names", async count => {
    const daemons = Array.from({ length: count }, () => daemonProcess(home, env, ["--port", "0"]));
    others.push(...daemons);
    const running = (): ChildProcess[] => daemons.filter(daemon => daemon.exitCode === null && daemon.signalCode === null);
    await until(async () => await findRunningDaemon(home) !== undefined, 60_000);
    // The others stop before they open anything; give every one the time a start takes.
    await until(() => running().length === 1, 20_000).catch(() => undefined);
    expect(running().map(daemon => daemon.pid)).toEqual([await recordedPid(home)]);
    expect(daemons.filter(daemon => daemon.exitCode !== null).every(daemon => daemon.exitCode !== 0)).toBe(true);
  }, 120_000);

  it("waits for a busy daemon whose daemon.json went missing, and uses it, where a start would launch a second", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const busy = join(home, "busy");
    env = watchedEnvironment(home, busy);
    const first = (await startDaemon(control(), { dev: true, env })).daemon.state;
    // Its event loop is held for 8 s, as a long check holds a busy daemon's; its socket and port stay bound.
    await writeFile(busy, "8000");
    await until(() => existsSync(`${busy}.held`));
    await rm(stateFile);
    const again = await startDaemon(control(), { dev: true, env });
    expect(again.started).toBe(false);
    expect(again.daemon.state.pid).toBe(first.pid);
    expect(liveDaemons(home)).toEqual([first.pid]);
  }, 60_000);

  it("leaves a daemon started beside a busy one to exit, and keeps the record and the port with the busy one", async () => {
    // The port both daemons prefer, as 4737 is on a person's machine: the one the home remembers.
    const port = await freePort();
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port }));
    const busy = join(home, "busy");
    env = watchedEnvironment(home, busy);
    const first = (await startDaemon(control(), { dev: true, env })).daemon.state;
    expect(first.port).toBe(port);
    await writeFile(busy, "8000");
    await until(() => existsSync(`${busy}.held`));
    // A second daemon process in the same home, as a start whose wait for a slow daemon ran out launches one.
    const second = daemonProcess(home, env);
    others.push(second);
    const ended = await Promise.race([once(second, "exit").then(([code]) => `exited ${String(code)}`), delay(7_000).then(() => "still running")]);
    expect(ended).toMatch(/^exited [1-9]/u);
    expect(await recordedPid(home)).toBe(first.pid);
    await until(() => !existsSync(`${busy}.held`));
    expect((await findRunningDaemon(home))?.state).toMatchObject({ pid: first.pid, port });
    expect(liveDaemons(home)).toEqual([first.pid]);
  }, 60_000);

  /** What a test writes over the running daemon's daemon.json: the record as something other than the daemon left it. */
  const spoiled: Array<[string, (state: DaemonState) => Promise<void>]> = [
    ["is gone", async () => await rm(stateFile)],
    ["names a process that is gone", async state => await writeFile(stateFile, JSON.stringify({ ...state, pid: DEAD_PID }))],
    ["names another process that runs", async state => {
      const other = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e9)"], { stdio: "ignore", windowsHide: true });
      others.push(other);
      await once(other, "spawn");
      await writeFile(stateFile, JSON.stringify({ ...state, pid: other.pid }));
    }],
  ];

  it.each(spoiled)("finds the running daemon when its daemon.json %s, to use it or stop it", async (_case, spoil) => {
    const first = (await startDaemon(control(), options())).daemon.state;
    await spoil(first);
    const again = await startDaemon(control(), options());
    expect(again.started).toBe(false);
    expect(again.daemon.state).toMatchObject({ pid: first.pid, url: first.url, kernelSocketPath: first.kernelSocketPath });
    await spoil(first);
    expect(await stopDaemon(home)).toBe("stopped");
    expect(processAlive(first.pid)).toBe(false);
    expect(liveDaemons(home)).toEqual([]);
  }, 60_000);

  it("gives way at once to the next start after its daemon was killed outright", async () => {
    const first = (await startDaemon(control(), options())).daemon.state;
    process.kill(first.pid, "SIGKILL");
    await until(() => !processAlive(first.pid));
    const asked = Date.now();
    expect(await stopDaemon(home)).toBe("not-running");
    const next = await startDaemon(control(), options());
    expect(next.started).toBe(true);
    // A home whose daemon runs without its record makes a start wait up to 15 s; nothing runs here.
    expect(Date.now() - asked).toBeLessThan(10_000);
    expect(liveDaemons(home)).toEqual([next.daemon.state.pid]);
  }, 60_000);

  it("ends a stop and a start at the same moment with at most one daemon, and the next start finds or starts one", async () => {
    for (let round = 0; round < 2; round += 1) {
      const first = (await startDaemon(control(), options())).daemon.state;
      const [stopped] = await Promise.allSettled([stopDaemon(home), startDaemon(control(), options())]);
      expect(stopped).toMatchObject({ status: "fulfilled", value: "stopped" });
      await until(() => !processAlive(first.pid), 20_000);
      expect(liveDaemons(home).length).toBeLessThanOrEqual(1);
      const after = await startDaemon(control(), options());
      expect(liveDaemons(home)).toEqual([after.daemon.state.pid]);
      expect(await recordedPid(home)).toBe(after.daemon.state.pid);
      expect(await stopDaemon(home)).toBe("stopped");
    }
  }, 120_000);

  it("keeps to the daemon of an older build, which takes no lock: a start uses it, one beside it exits, a stop ends it", async () => {
    const older = await olderDaemon(home, env);
    others.push(older);
    const found = await startDaemon(control(), options());
    expect(found).toMatchObject({ started: false, daemon: { state: { pid: older.pid } } });
    const beside = daemonProcess(home, env, ["--port", "0"]);
    others.push(beside);
    const [code] = await once(beside, "exit") as [number | null];
    expect(code).not.toBe(0);
    expect(liveDaemons(home)).toEqual([older.pid]);
    expect(await stopDaemon(home)).toBe("stopped");
    expect(liveDaemons(home)).toEqual([]);
  }, 60_000);

  it("runs nothing beside an older build's daemon whose daemon.json went missing: the start fails, naming it", async () => {
    const older = await olderDaemon(home, env);
    others.push(older);
    await rm(stateFile);
    await expect(startDaemon(control(), options())).rejects.toThrow(/in use by a running daemon/u);
    expect(liveDaemons(home)).toEqual([older.pid]);
    expect(await readDaemonState(stateFile)).toBeUndefined();
  }, 60_000);
});
