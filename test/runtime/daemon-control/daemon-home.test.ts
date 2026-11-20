import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { closeSync, existsSync, openSync } from "node:fs";
import type * as FileSystem from "node:fs/promises";
import { mkdir, mkdtemp, rename, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as DaemonWaits from "../../../src/runtime/daemon/daemon-waits.js";
import { DAEMON_PORT_FILE } from "../../../src/daemon/index.js";
import { LOCKS_DIR, defaultRerailDatabasePath } from "../../../src/engine/index.js";
import { enginePipeName } from "../../../src/runtime/daemon/engine-pipe.js";
import {
  type DaemonState,
  AUTOSTART_CLAIM_FILE,
  DAEMON_STATE_FILE,
  findRunningDaemon,
  processAlive,
  readDaemonState,
  startDaemon,
  stopDaemon,
  withStartLock,
} from "../../../src/runtime/daemon-control.js";
import {
  DEAD_PID,
  daemonProcess,
  exitMessage,
  freePort,
  holdEventLoop,
  launchers,
  notExited,
  olderDaemon,
  packageRoot,
  recordedPid,
  releaseSlowStart,
  until,
  watchedEnvironment,
} from "../../support/processes/daemon-home.js";
import { oneInstanceHolder } from "../../support/platform/one-instance-pipe.js";
import { removeFolder, runningIn } from "../../support/platform/remove-folder.js";

const execute = promisify(execFile);

/** How long a start waits (`START_TIMEOUT_MS`), read at each wait: the product's, unless a test sets its own. */
const waits = vi.hoisted(() => ({ startMs: undefined as number | undefined }));
vi.mock("../../../src/runtime/daemon/daemon-waits.js", async importOriginal => {
  const actual = await importOriginal<typeof DaemonWaits>();
  return { ...actual, get START_TIMEOUT_MS() { return waits.startMs ?? actual.START_TIMEOUT_MS; } };
});
/** Told of each file this process has read, once the read has ended, while a test listens. */
const reads = vi.hoisted(() => ({ ended: undefined as ((path: string) => void) | undefined }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  const readFile = (async (...args: Parameters<typeof actual.readFile>) => {
    try {
      return await actual.readFile(...args);
    } finally {
      reads.ended?.(String(args[0]));
    }
  }) as typeof actual.readFile;
  return { ...actual, readFile };
});

const originalRerailHome = process.env.RERAIL_HOME;
/** What a daemon that finds its home's lock held says as it exits (`HomeDaemonConflictError`). */
const HOME_TAKEN = /rerail daemon: Another Rerail daemon(?: \(pid \d+\))? runs in /u;

/** The daemons started in `home` that still run. */
function liveDaemons(home: string): number[] {
  return notExited(home, "daemons").filter(pid => processAlive(pid));
}

/** Whether every `rerail daemon start` launched in `home` has ended. */
function launchersEnded(home: string): boolean {
  return notExited(home, "launchers").every(pid => !processAlive(pid));
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

/** The leading words of `text` that can name a command: lowercase words, up to the first option or placeholder. */
function commandWords(text: string): string {
  const words = text.split(" ");
  const end = words.findIndex(word => !/^[a-z][a-z-]*$/u.test(word));
  return words.slice(0, end === -1 ? undefined : end).join(" ");
}

/**
 * The commands the first line of `message` names, `rerail` and the words after it, each cut where the longest of
 * `listed` that starts them ends; words no listed command starts stay whole, so a test names them.
 */
function namedCommands(message: string, listed: readonly string[]): string[] {
  const [line = ""] = message.split("\n");
  const longest = (words: string): string | undefined =>
    listed.filter(command => `${words} `.startsWith(`${command} `)).sort((a, b) => b.length - a.length)[0];
  return [...line.matchAll(/(?<![\w./\\-])rerail ([a-z][a-z-]*(?: [a-z][a-z-]*)*)/gu)].map(([, words = ""]) => longest(words) ?? words);
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
    waits.startMs = undefined;
    reads.ended = undefined;
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    await Promise.all(others.splice(0).map(async child => await ended(child)));
    const running = (): number[] => [...notExited(home, "daemons"), ...notExited(home, "launchers")].filter(pid => processAlive(pid));
    for (const pid of running()) killGone(pid);
    // Node marks a child it started as exited a moment after the OS has ended it.
    await until(() => running().length === 0 && runningIn(home).length === 0, 10_000, "every daemon and start of the test ended");
    await removeFolder(home);
  });

  const daemonControl = pathToFileURL(join(packageRoot, "src", "runtime", "daemon-control.ts")).href;

  /**
   * A process of its own, as every hook is: on `go` it looks for the daemon and, finding none, runs the real autostart,
   * as a hook does (`hook-host.ts`), then exits. A hook that comes after the daemon is up finds it and starts nothing.
   */
  function hookFindingNoDaemon(): { ready: Promise<void>; go: () => void; exited: Promise<unknown> } {
    const child = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `const { autostartDaemon, lookupDaemon } = await import(${JSON.stringify(daemonControl)});
       process.stdout.write("ready\\n");
       process.stdin.once("data", async () => {
         if ((await lookupDaemon(process.env.RERAIL_HOME)).status === "absent") autostartDaemon();
         process.exit(0);
       });`,
    ], { cwd: packageRoot, env: { ...env, RERAIL_AUTOSTART: "1" }, stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
    others.push(child);
    let errors = "";
    child.stderr!.setEncoding("utf8").on("data", (chunk: string) => { errors += chunk; });
    const exited = once(child, "exit");
    const { promise: ready, resolve, reject } = Promise.withResolvers<void>();
    child.stdout!.once("data", () => resolve());
    child.once("close", code => reject(new Error(`a hook process exited (${String(code)}) before it was ready: ${errors.slice(-600)}`)));
    return { ready, go: () => child.stdin!.write("go\n"), exited };
  }

  it.each([2, 5, 20])("%i hooks that find no daemon at the same moment launch one start, which leaves one daemon", async count => {
    // A start launched by an autostart takes the port the home remembers, 4737 unless one is recorded.
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const hooks = Array.from({ length: count }, () => hookFindingNoDaemon());
    await Promise.all(hooks.map(async hook => await hook.ready));
    for (const hook of hooks) hook.go();
    await Promise.all(hooks.map(async hook => await hook.exited));
    await until(async () => await findRunningDaemon(home) !== undefined && launchersEnded(home), 90_000, "a daemon running and every start ended");
    expect(launchers(home)).toHaveLength(1);
    expect(liveDaemons(home)).toEqual([await recordedPid(home)]);
  }, 120_000);

  it("keeps the claim of a launcher whose start waits past the claim's lease, so a hook after it launches nothing", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const first = hookFindingNoDaemon();
    const later = hookFindingNoDaemon();
    await Promise.all([first.ready, later.ready]);
    // A start the test holds the start lock against waits for it, as a launcher behind a slow start does.
    await withStartLock(home, async () => {
      first.go();
      await first.exited;
      await until(() => launchers(home).length === 1, 20_000);
      // Past the 5 s a claim holds unrenewed, within the 15 s a start waits for the start lock.
      await delay(7_000);
      later.go();
      await later.exited;
      expect(launchers(home)).toHaveLength(1);
    });
    await until(async () => await findRunningDaemon(home) !== undefined && launchersEnded(home), 60_000, "a daemon running and every start ended");
    expect(launchers(home)).toHaveLength(1);
    expect(liveDaemons(home)).toEqual([await recordedPid(home)]);
  }, 120_000);

  /** `rerail daemon <args>` from this checkout's build, in a process of its own, as a person or a launcher runs it. */
  async function cli(args: readonly string[]): Promise<void> {
    const child = spawn(process.execPath, [join(packageRoot, "dist", "cli", "cli.js"), "daemon", ...args], {
      cwd: packageRoot, env, stdio: "ignore", windowsHide: true,
    });
    others.push(child);
    await once(child, "exit");
  }

  /**
   * `rerail <args>` from this checkout's build, as a person runs it in a terminal in the home, with a home folder of its
   * own; what it printed, and its exit code.
   */
  async function rerail(args: readonly string[]): Promise<{ code: number; out: string }> {
    const user = join(home, "user");
    const own = { ...env, HOME: user, USERPROFILE: user, APPDATA: join(user, "AppData", "Roaming"), LOCALAPPDATA: join(user, "AppData", "Local") };
    const said = (out: { stdout?: string; stderr?: string }): string => `${out.stdout ?? ""}${out.stderr ?? ""}`;
    return await execute(process.execPath, [join(packageRoot, "dist", "cli", "cli.js"), ...args], { cwd: home, env: own, windowsHide: true })
      .then(out => ({ code: 0, out: said(out) }), (error: { code?: number; stdout?: string; stderr?: string }) => ({ code: error.code ?? 1, out: said(error) }));
  }

  /** The commands the built CLI's usage lists, each as its words before any option or placeholder: `daemon stop`. */
  async function listedCommands(): Promise<string[]> {
    const { out } = await rerail(["help"]);
    return out.split("\n").filter(line => line.startsWith("  rerail ")).map(line => commandWords(line.slice("  rerail ".length))).filter(Boolean);
  }

  /**
   * Runs, through the built CLI, every command `messages` name, each once, in the order named: each is one the CLI lists
   * and takes, never its usage error. What each printed, by command.
   */
  async function runNamedCommands(messages: readonly string[]): Promise<Map<string, string>> {
    const listed = await listedCommands();
    const said = new Map<string, string>();
    for (const command of new Set(messages.flatMap(message => namedCommands(message, listed)))) {
      expect(listed).toContain(command);
      const { out } = await rerail(command.split(" "));
      expect(out).not.toMatch(/^rerail: (?:Usage|Unknown command)/mu);
      said.set(command, out);
    }
    return said;
  }

  /** The daemons still running in the home a moment after the last start ended, and the pid daemon.json names. */
  async function settled(): Promise<{ live: number[]; recorded: number | undefined }> {
    await delay(2_500);
    return { live: liveDaemons(home), recorded: await recordedPid(home) };
  }

  it("leaves one daemon, which daemon.json names, after hooks, starts and daemon processes all come at once", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const hooks = Array.from({ length: 6 }, () => hookFindingNoDaemon());
    await Promise.all(hooks.map(async hook => await hook.ready));
    for (const hook of hooks) hook.go();
    const direct = Array.from({ length: 4 }, () => daemonProcess(home, env));
    others.push(...direct);
    await Promise.all(Array.from({ length: 4 }, async () => await cli(["start", "--dev"])));
    await Promise.all(hooks.map(async hook => await hook.exited));
    await until(() => launchersEnded(home), 90_000, "every start ended");
    await until(() => direct.every(daemon => daemon.exitCode !== null) || liveDaemons(home).length <= 1, 20_000).catch(() => undefined);
    const { live, recorded } = await settled();
    expect(live).toHaveLength(1);
    expect(recorded).toBe(live[0]);
    // Each daemon process that stopped did so on the home's lock. On macOS and Linux the engine socket's own check stops a
    // second daemon too, so without this the test could fail only on Windows.
    for (const daemon of direct.filter(daemon => daemon.exitCode !== null)) expect(await exitMessage(daemon)).toMatch(HOME_TAKEN);
  }, 180_000);

  it("leaves a start to the hooks that find a claim left from before a restart, and they leave one daemon", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    // The launcher it names is gone, and it was made ten minutes ago, past any start's time.
    const claim = join(home, AUTOSTART_CLAIM_FILE);
    await writeFile(claim, String(DEAD_PID));
    const long = new Date(Date.now() - 600_000);
    await utimes(claim, long, long);
    const hooks = Array.from({ length: 10 }, () => hookFindingNoDaemon());
    await Promise.all(hooks.map(async hook => await hook.ready));
    for (const hook of hooks) hook.go();
    await Promise.all(hooks.map(async hook => await hook.exited));
    await until(async () => await findRunningDaemon(home) !== undefined && launchersEnded(home), 90_000, "a daemon running and every start ended");
    const { live, recorded } = await settled();
    expect(launchers(home).length).toBeGreaterThanOrEqual(1);
    expect(live).toHaveLength(1);
    expect(recorded).toBe(live[0]);
  }, 150_000);

  it.each([2, 5, 20])("%i daemons started at the same moment leave one running, which daemon.json names", async count => {
    const daemons = Array.from({ length: count }, () => daemonProcess(home, env, ["--port", "0"]));
    others.push(...daemons);
    const running = (): ChildProcess[] => daemons.filter(daemon => daemon.exitCode === null && daemon.signalCode === null);
    await until(async () => await findRunningDaemon(home) !== undefined, 60_000, "a daemon running");
    // The others stop before they open anything; give every one the time a start takes.
    await until(() => running().length === 1, 20_000).catch(() => undefined);
    expect(running().map(daemon => daemon.pid)).toEqual([await recordedPid(home)]);
    const stopped = daemons.filter(daemon => daemon.exitCode !== null);
    expect(stopped.every(daemon => daemon.exitCode !== 0)).toBe(true);
    // Each stopped on the home's lock. On macOS and Linux the engine socket's own check stops a second daemon too.
    for (const daemon of stopped) expect(await exitMessage(daemon)).toMatch(HOME_TAKEN);
  }, 120_000);

  it("waits for a busy daemon whose daemon.json went missing, and uses it, where a start would launch a second", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const busy = join(home, "busy");
    env = watchedEnvironment(home, busy);
    const first = (await startDaemon(control(), { dev: true, env })).daemon.state;
    // Its event loop is held for 8 s, as a long check holds a busy daemon's; its socket and port stay bound.
    await holdEventLoop(busy);
    await rm(stateFile);
    const again = await startDaemon(control(), { dev: true, env });
    expect(again.started).toBe(false);
    expect(again.daemon.state.pid).toBe(first.pid);
    expect(liveDaemons(home)).toEqual([first.pid]);
  }, 60_000);

  it("leaves a busy daemon that daemon.json names to a start, which starts nothing beside it", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const busy = join(home, "busy");
    env = watchedEnvironment(home, busy);
    const first = (await startDaemon(control(), { dev: true, env })).daemon.state;
    await holdEventLoop(busy);
    // The start says the daemon is not answering (`LiveDaemonError`) unless it answers again first; either way it starts
    // nothing beside it.
    await cli(["start", "--dev"]);
    await until(() => !existsSync(`${busy}.held`), 20_000);
    const { live, recorded } = await settled();
    expect(live).toEqual([first.pid]);
    expect(recorded).toBe(first.pid);
  }, 90_000);

  it("says a daemon slow to come up may still be starting, and names the status that shows it once it answers; a start uses it, never a second", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const slowStart = join(home, "slow-start");
    env = watchedEnvironment(home, undefined, slowStart);
    // The daemon holds right after it first listens, before it can write daemon.json, until the test lets it go: past
    // the first start's whole wait for it, 3 s here rather than 15 s.
    await writeFile(slowStart, "");
    waits.startMs = 3_000;
    const first = await startDaemon(control(), { dev: true, env }).then(() => "", (error: unknown) => (error as Error).message);
    waits.startMs = undefined;
    await until(() => existsSync(`${slowStart}.held`), 30_000);
    const [launched] = liveDaemons(home);
    // The start names the daemon it leaves running and the command that shows it once it answers, never a failed start,
    // and no second start, which waits for a daemon only in some states.
    expect(first).toContain(`Rerail daemon (pid ${launched}) has not answered within 3s and may still be starting; this start leaves it running. `
      + "Once it answers, rerail status shows it running.");
    expect(first).not.toMatch(/did not become ready|end that process|start again/u);
    // A start finds the daemon holding the home and daemon.json not written yet; once it has looked for daemon.json,
    // the daemon goes on and writes it, and the start uses it.
    reads.ended = path => {
      if (path !== stateFile) return;
      reads.ended = undefined;
      releaseSlowStart(slowStart);
    };
    const second = await startDaemon(control(), { dev: true, env });
    expect(second).toMatchObject({ started: false, daemon: { state: { pid: launched } } });
    // The command the line names, run through the built CLI as the person reads it, shows that daemon running.
    const said = await runNamedCommands([first]);
    expect([...said.keys()]).toEqual(["status"]);
    expect(said.get("status")).toMatch(new RegExp(`Rerail daemon \\S+ running at \\S+ \\(pid ${launched},`, "u"));
    await until(() => !existsSync(`${slowStart}.held`), 40_000);
    const { live, recorded } = await settled();
    expect(live).toEqual([launched]);
    expect(recorded).toBe(live[0]);
  }, 120_000);

  // Two states of a start met in one home: another program holds the engine pipe's name with a pipe that allows one
  // instance, and the daemon is slow to come up. The start says the daemon may still be starting, by its pid, never
  // that it exited on the pipe or did not become ready, and the next start finds it serving on the fallback pipe.
  it.runIf(process.platform === "win32")("starts a daemon slow to come up beside a one-instance pipe on its engine pipe's name, and the next start uses it", async () => {
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port: await freePort() }));
    const slowStart = join(home, "slow-start");
    env = watchedEnvironment(home, undefined, slowStart);
    const pipe = enginePipeName(defaultRerailDatabasePath(env));
    const holder = await oneInstanceHolder(pipe);
    try {
      await writeFile(slowStart, "");
      waits.startMs = 3_000;
      const first = await startDaemon(control(), { dev: true, env }).then(() => "", (error: unknown) => (error as Error).message);
      waits.startMs = undefined;
      await until(() => existsSync(`${slowStart}.held`), 30_000);
      const [launched] = liveDaemons(home);
      expect(first).toContain(`Rerail daemon (pid ${launched}) has not answered within 3s and may still be starting`);
      expect(first).not.toMatch(/EBUSY|exited|did not become ready|end that process/u);
      releaseSlowStart(slowStart);
      const second = await startDaemon(control(), { dev: true, env });
      expect(second).toMatchObject({ started: false, daemon: { state: { pid: launched } } });
      expect(second.daemon.state.kernelSocketPath.startsWith(`${pipe}-`)).toBe(true);
      // Nothing took the holder's one instance: a client still opens it.
      closeSync(openSync(pipe, "r+"));
      const { live, recorded } = await settled();
      expect(live).toEqual([launched]);
      expect(recorded).toBe(launched);
    } finally {
      await holder.end();
    }
  }, 120_000);

  it("leaves a daemon started beside a busy one to exit, and keeps the record and the port with the busy one", async () => {
    // The port both daemons prefer, as 4737 is on a person's machine: the one the home remembers.
    const port = await freePort();
    await writeFile(join(home, DAEMON_PORT_FILE), JSON.stringify({ port }));
    const busy = join(home, "busy");
    env = watchedEnvironment(home, busy);
    const first = (await startDaemon(control(), { dev: true, env })).daemon.state;
    expect(first.port).toBe(port);
    await holdEventLoop(busy);
    // A second daemon process in the same home, as a start whose wait for a slow daemon ran out launches one.
    const second = daemonProcess(home, env);
    others.push(second);
    const ended = await Promise.race([once(second, "exit").then(([code]) => `exited ${String(code)}`), delay(7_000).then(() => "still running")]);
    expect(ended).toMatch(/^exited [1-9]/u);
    expect(await exitMessage(second)).toContain(`rerail daemon: Another Rerail daemon (pid ${first.pid}) runs in ${home}`);
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

  // Windows keeps a running daemon's current folder, its home, from being moved or removed.
  it.skipIf(process.platform === "win32")("writes no daemon.json into a home made again where its own was, once its own was moved away", async () => {
    const daemon = daemonProcess(home, env, ["--port", "0"]);
    others.push(daemon);
    await until(async () => await recordedPid(home) === daemon.pid, 30_000);
    const moved = `${home}-moved`;
    await rename(home, moved);
    try {
      await mkdir(home);
      // The daemon checks its record every second.
      await delay(3_000);
      expect(existsSync(stateFile)).toBe(false);
      await ended(daemon);
      expect(await exitMessage(daemon)).toContain(`no longer holds the Rerail home at ${home}`);
    } finally {
      await ended(daemon);
      await removeFolder(moved);
    }
  }, 60_000);

  /** What a daemon killed outright leaves, or what happens to it after: its lock released, its records left. */
  const afterKill: Array<[string, (other: number) => Promise<void>]> = [
    ["as the kill left it", async () => undefined],
    ["with its daemon.json gone", async () => await rm(stateFile, { force: true })],
    ["with its holder record naming another process that runs", async other => {
      await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: other }));
    }],
  ];

  it.each(afterKill)("gives way at once to the next start after its daemon was killed outright, %s", async (_case, after) => {
    const other = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e9)"], { stdio: "ignore", windowsHide: true });
    others.push(other);
    await once(other, "spawn");
    const first = (await startDaemon(control(), options())).daemon.state;
    process.kill(first.pid, "SIGKILL");
    await until(() => !processAlive(first.pid));
    await after(other.pid!);
    const asked = Date.now();
    expect(await stopDaemon(home)).toBe("not-running");
    const next = await startDaemon(control(), options());
    expect(next.started).toBe(true);
    // A home whose daemon runs without its record makes a stop or a start wait 15 s; nothing runs here. The start itself
    // launches a daemon, which takes seconds on a busy Windows machine.
    expect(Date.now() - asked).toBeLessThan(15_000);
    expect(liveDaemons(home)).toEqual([next.daemon.state.pid]);
  }, 60_000);

  /**
   * `rerail daemon stop` as the uninstall runs it, in a process of its own: what it answers, and the native addons it
   * loaded, by file name. The uninstall stops the daemon, then removes `versions/`, and Windows keeps a loaded DLL from
   * being deleted.
   */
  async function stopInItsOwnProcess(): Promise<{ result: string; loaded: string[] }> {
    const child = spawn(process.execPath, [
      "--import", "tsx", "--input-type=module", "-e",
      `import { createRequire } from "node:module";
       const cache = createRequire(import.meta.url).cache;
       const addons = () => Object.keys(cache).filter(path => path.endsWith(".node"));
       const { stopDaemon } = await import(${JSON.stringify(daemonControl)});
       const before = addons();
       const result = await stopDaemon(${JSON.stringify(home)});
       const loaded = addons().filter(path => !before.includes(path)).map(path => path.split(/[\\\\/]/u).pop());
       process.stdout.write(JSON.stringify({ result, loaded }));`,
    ], { cwd: packageRoot, env: { ...env, RERAIL_HOME: home }, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    others.push(child);
    let out = "";
    let errors = "";
    child.stdout!.setEncoding("utf8").on("data", (chunk: string) => { out += chunk; });
    child.stderr!.setEncoding("utf8").on("data", (chunk: string) => { errors += chunk; });
    await once(child, "close");
    if (!out.startsWith("{")) throw new Error(`the stop printed no answer: ${out} ${errors}`.slice(0, 600));
    return JSON.parse(out) as { result: string; loaded: string[] };
  }

  it("says no daemon runs, and loads no engine, once its killed daemon's pid went to another process", async () => {
    const first = (await startDaemon(control(), options())).daemon.state;
    process.kill(first.pid, "SIGKILL");
    await until(() => !processAlive(first.pid));
    // Windows gives a pid to another process soon after, and the record the killed holder left names it.
    const other = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e9)"], { stdio: "ignore", windowsHide: true });
    others.push(other);
    await once(other, "spawn");
    await writeFile(join(home, LOCKS_DIR, "daemon.json"), JSON.stringify({ pid: other.pid }));
    const stop = await stopInItsOwnProcess();
    expect(stop.result).toBe("not-running");
    // macOS and Linux delete a loaded library as any file.
    if (process.platform === "win32") expect(stop.loaded).toEqual([]);
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
