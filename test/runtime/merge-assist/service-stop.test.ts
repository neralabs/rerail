import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { access } from "node:fs/promises";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it, onTestFinished, vi } from "vitest";
import { mergeAssistState, type MergeAssistStatus, openRerailDatabase, recordCheckout, type RerailDatabase } from "../../../src/engine/index.js";
import { createMergeAssistService, type GithubClient, type MergeAssistRuntime, type PullRequestSummary } from "../../../src/runtime/index.js";
import { readConflicts, writeConflicts } from "../../../src/runtime/merge-assist/conflict-store.js";
import { withExperiments } from "../../support/environment/experiments.js";
import { git } from "../../support/merge-assist/git.js";
import { FakeGithub, mergeAssistWorkspaces, takeNotes, type Workspace } from "../../support/merge-assist/workspace.js";

// Every process Rerail starts goes through this module: the test stops the service as a pass starts a given Git read,
// and sees every Git process started after. On Windows the stop ends a running Git with taskkill, which is no Git read.
vi.mock("node:child_process", { spy: true });
// A pass checks that each worktree's folder is there with `access`: a test holds that call as a drive that stopped
// answering would.
vi.mock("node:fs/promises", { spy: true });

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

const workspace = mergeAssistWorkspaces();

/** How many branches the checkout's work conflicts with: a pass reads Git several times for each. */
const PEERS = 12;

/**
 * `origin/peer-<n>` for each of {@link PEERS}, one commit each off `main`, committed now, adding `src/api/orders.ts`
 * as the checkout's own commit does, with other text: each conflicts with the checkout's work. Answers their tips.
 */
function addPeers(ws: Workspace): string[] {
  return Array.from({ length: PEERS }, (_, index) => commitPeer(ws, index, `Orders, take ${index}`));
}

/**
 * Commits `origin/peer-<index>` anew off `main` with `subject`, its change to `file` as {@link addPeers} makes it to
 * `src/api/orders.ts`: by default the same conflict with another tip; in another file, no conflict. Answers the tip.
 */
function commitPeer(ws: Workspace, index: number, subject: string, file = "src/api/orders.ts"): string {
  const now = Math.floor(Date.now() / 1000);
  const data = (text: string): string => `data ${Buffer.byteLength(text)}\n${text}\n`;
  const commit = [
    `commit refs/remotes/origin/peer-${index}`,
    `committer Jo Barnes <jo@example.com> ${now} +0000`,
    `${data(subject)}from refs/heads/main^0`,
    `M 100644 inline ${file}`,
    data(`export const orders = () => client.get("/v${index + 2}/orders");\n`),
  ].join("\n");
  execFileSync("git", ["fast-import", "--quiet", "--force"], { cwd: ws.repo, input: commit, env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1" } });
  return git(ws.repo, "rev-parse", `refs/remotes/origin/peer-${index}`);
}

/** Commits every peer anew changing a file of its own: a pass finds each conflict gone. */
function peersElsewhere(ws: Workspace): void {
  for (let index = 0; index < PEERS; index += 1) commitPeer(ws, index, `Peer ${index}, elsewhere`, `src/peer-${index}.ts`);
}

/** Commits every peer anew with the change {@link addPeers} gave it: a pass finds each conflict again, the same one. */
function peersBack(ws: Workspace): void {
  for (let index = 0; index < PEERS; index += 1) commitPeer(ws, index, `Orders, take ${index}, again`);
}

/**
 * Every peer's conflict stored by a scan, then `origin/peer-0` committed anew: a later pass finds the same conflicts,
 * which tell the hook nothing new, and stores peer-0's new tip. Answers that tip.
 */
async function knownConflictsMoved(ws: Workspace): Promise<string> {
  addPeers(ws);
  const first = ws.service();
  await first.setEnabled(ws.repo, true);
  expect((await first.scan(ws.repo)).conflicts).toHaveLength(PEERS);
  return commitPeer(ws, 0, "Orders, take 0, again");
}

/** The tip of `origin/peer-0` the stored conflict with it names. */
const peerTip = (conflicts: MergeAssistStatus["conflicts"]): string | undefined =>
  conflicts.find(conflict => conflict.theirs.ref === "origin/peer-0")?.theirs.sha;

/** An event the hook command heard, with its conflict. */
interface Heard {
  readonly event: string;
  readonly id: string;
}

/** Sets a hook command on the repository that logs each event it hears; answers the events logged so far. */
async function loggedHook(ws: Workspace): Promise<() => Heard[]> {
  const log = join(ws.folder, "hook.log");
  const script = "require(\"node:fs\").appendFileSync(process.argv[1], process.env.RERAIL_CONFLICT_EVENT + \" \" + process.env.RERAIL_CONFLICT_ID + \"\\n\")";
  await ws.service().updateSettings(ws.repo, { hook: { command: [process.execPath, "-e", script, log], timeoutSeconds: 10 } });
  return () => (existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(line => line !== "") : []).map(line => {
    const [event = "", id = ""] = line.split(" ");
    return { event, id };
  });
}

/** The events the hook heard, each conflict's in the order it heard them, by conflict. */
function byConflict(heard: readonly Heard[]): Map<string, string[]> {
  const events = new Map<string, string[]>();
  for (const { event, id } of heard) events.set(id, [...(events.get(id) ?? []), event]);
  return events;
}

/** An open pull request from `peer-<index>`, its head at `sha`. */
function pullOf(sha: string, index: number): PullRequestSummary {
  return {
    number: index + 1, title: `Orders, take ${index}`, url: `https://github.com/acme/app/pull/${index + 1}`, headRefName: `peer-${index}`,
    headCommit: sha, baseRefName: "main", author: "jbarnes", isDraft: false, updatedAt: "2026-11-18T00:00:00Z", crossRepository: false, headOwner: "acme",
  };
}

/** A process the code under test started, as its command line. */
const commandLine = (args: readonly unknown[]): string => [args[0], ...(Array.isArray(args[1]) ? args[1] : [])].join(" ");

/** Whether a process started with these arguments is Git. */
const isGit = (args: readonly unknown[]): boolean => /(?:^|[\\/])git(?:\.exe)?$/iu.test(String(args[0]));

/** Whether a process has not exited yet. */
const running = (child: ChildProcess): boolean => child.exitCode === null && child.signalCode === null;

const { spawn: actualSpawn } = await vi.importActual<typeof import("node:child_process")>("node:child_process");

/** setTimeout's longest delay: a wait for the stop that a pass ends long before, by its own work. */
const UNREACHED_MS = 2 ** 31 - 1;

/**
 * Node waiting on a lock nothing releases: in place of a Git read, a process that ends only when it is ended, so the
 * pass waiting for that read still runs when the stop's wait is over, however fast the machine.
 */
const HELD_READ = ["-e", "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0)"];

/**
 * Where a test stops the service: as the work starts its first Git read whose arguments hold `at`, or when `trigger`
 * calls the stop it is handed, for a stop at something other than a Git read.
 */
interface StopPoint {
  readonly at?: string;
  readonly trigger?: (stop: () => void) => void;
  /** What the stop is given to wait for the passes running. */
  readonly waitMs: number;
  /** Whether {@link HELD_READ} stands in for the read at `at`. */
  readonly held: boolean;
  /** Runs once the stop has resolved and the Git reads still running are counted, before the work is awaited. */
  readonly afterStop?: () => void;
}

/**
 * Runs `work` and stops `service` at `point`. Answers, once the stop and the work are over, the Git reads the work
 * started that still ran when the stop resolved, and those started once the stop ended the held read or resolved.
 */
async function stopDuring(service: MergeAssistRuntime, work: () => Promise<unknown>, point: StopPoint): Promise<{ runningAtStop: string[]; startedAfter: string[] }> {
  const reads: Array<{ line: string; child: ChildProcess }> = [];
  const startedAfter: string[] = [];
  const stopping = Promise.withResolvers<{ stopped: Promise<void> }>();
  let state: "before" | "stopping" | "stopped" = "before";
  const stopped = (): void => {
    state = "stopped";
  };
  /** Starts the stop once the code that called this has set up what it is doing. */
  const beginStop = (): void => {
    state = "stopping";
    queueMicrotask(() => stopping.resolve({ stopped: service.stop(point.waitMs).then(stopped) }));
  };
  /** The read at `point`: {@link HELD_READ} in its place when `held`. */
  const stopAt = (args: Parameters<typeof spawn>): ChildProcess => {
    beginStop();
    const child = point.held ? actualSpawn(process.execPath, HELD_READ, { stdio: ["pipe", "pipe", "pipe"] }) : actualSpawn(...args);
    // Once the held read has ended, which the stop does at once and a read the stop missed only at its time limit, no
    // Git may start.
    if (point.held) child.once("exit", stopped);
    return child;
  };
  point.trigger?.(beginStop);
  vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
    const line = commandLine(args);
    const child = state === "before" && point.at !== undefined && line.includes(point.at) ? stopAt(args) : actualSpawn(...args);
    if (isGit(args)) {
      reads.push({ line, child });
      if (state === "stopped") startedAfter.push(line);
    }
    return child;
  });
  try {
    const done = work();
    await (await stopping.promise).stopped;
    const left = reads.filter(read => running(read.child));
    // A held read the stop left running would hold the work for ever: it ends here, once counted.
    for (const { child } of left) child.kill("SIGKILL");
    point.afterStop?.();
    await done;
    return { runningAtStop: left.map(({ line }) => line), startedAfter };
  } finally {
    vi.mocked(spawn).mockImplementation(actualSpawn);
  }
}

/**
 * A Merge Assist over a connection of its own to the workspace's database, and the closing of that connection, as a
 * runtime closes its database once its Merge Assist has stopped. Closed after the test at the latest.
 */
function ownConnection(ws: Workspace, github: GithubClient): { service: MergeAssistRuntime; close: () => void; database: RerailDatabase } {
  const database = openRerailDatabase(ws.databasePath);
  let open = true;
  const close = (): void => {
    if (open) database.$client.close();
    open = false;
  };
  onTestFinished(close);
  return { service: createMergeAssistService({ database, github }), close, database };
}

/** The SQLite driver Rerail's database runs on, for another process to open the workspace's database with. */
const SQLITE_DRIVER = createRequire(import.meta.url).resolve("better-sqlite3");

/**
 * The longest {@link holdWriteLock}'s process holds the write lock: its bound, so a test that dies leaves no holder.
 * It is within the five seconds a Rerail connection waits for a lock, so a release that waits for the lock, which
 * holds the test's thread and with it any file the test would write, gets the lock once the bound is over.
 */
const LOCK_HELD_MS = 2_000;

/** A process holding a database's write lock ({@link holdWriteLock}). */
interface LockHolder {
  readonly child: ChildProcess;
  /** Resolves once the process holds the lock. */
  readonly held: Promise<void>;
  /** Writes the file that tells the process to give the lock back; resolves once it has exited. */
  readonly release: () => Promise<void>;
}

/**
 * Another process holding the write lock of the database at `path` from its start. It gives the lock back when the
 * test writes a file in a folder of its own under `folder` ({@link LockHolder.release}), when it is ended, or once
 * {@link LOCK_HELD_MS} is over. It writes nothing on its standard output, so it can stand in for a Git read.
 */
function holdWriteLock(path: string, folder: string): LockHolder {
  const signals = join(folder, "write-lock");
  mkdirSync(signals, { recursive: true });
  const released = join(signals, "released");
  const script = [
    "const { existsSync, watch } = require(\"node:fs\");",
    "const [driver, path, folder, released, boundMs] = process.argv.slice(1);",
    "const database = new (require(driver))(path);",
    "database.exec(\"BEGIN IMMEDIATE\");",
    "const end = () => { database.exec(\"ROLLBACK\"); database.close(); process.exit(0); };",
    "watch(folder, () => { if (existsSync(released)) end(); });",
    "if (existsSync(released)) end();",
    "setTimeout(end, Number(boundMs));",
    "process.stderr.write(\"held\\n\");",
  ].join(" ");
  const child = actualSpawn(process.execPath, ["-e", script, SQLITE_DRIVER, path, signals, released, String(LOCK_HELD_MS)], { stdio: ["pipe", "pipe", "pipe"] });
  const exited = new Promise<void>(done => child.once("exit", () => done()));
  // The workspace's folder goes after the test: Windows refuses to remove a database a process holds.
  onTestFinished(async () => {
    if (running(child)) child.kill("SIGKILL");
    await exited;
  });
  child.stdin.on("error", () => undefined);
  let said = "";
  const held = new Promise<void>((resolveHeld, failed) => {
    child.stderr.on("data", (chunk: Buffer) => {
      said += String(chunk);
      if (said.includes("held\n")) resolveHeld();
    });
    child.once("exit", code => failed(new Error(`The process holding the lock exited with ${String(code)} before it held it: ${said}`)));
  });
  return {
    child,
    held,
    release: async () => {
      if (running(child)) writeFileSync(released, "");
      await exited;
    },
  };
}

const { access: actualAccess } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

/**
 * Holds the first `access` call for `folder` as a drive that stopped answering would: it answers only once `release`
 * is called, and `onHeld` runs as it is held.
 */
function holdAccessTo(folder: string, onHeld: () => void): { release: () => void } {
  const held = Promise.withResolvers<void>();
  const same = (path: string): boolean => resolve(path).toLowerCase() === resolve(folder).toLowerCase();
  let holding = false;
  vi.mocked(access).mockImplementation(async (path, mode) => {
    if (holding || !same(String(path))) return await actualAccess(path, mode);
    holding = true;
    onHeld();
    await held.promise;
    return await actualAccess(path, mode);
  });
  onTestFinished(() => {
    held.resolve();
    vi.mocked(access).mockImplementation(actualAccess);
  });
  return { release: () => held.resolve() };
}

/** `gh` whose listing of open pull requests answers only once `release` is called; `onHeld` runs as it is asked. */
class HeldGithub extends FakeGithub {
  readonly #held = Promise.withResolvers<void>();
  onHeld: () => void = () => undefined;

  override async openPullRequests(): Promise<{ pulls: PullRequestSummary[]; truncated: boolean }> {
    this.onHeld();
    await this.#held.promise;
    return await super.openPullRequests();
  }

  release(): void {
    this.#held.resolve();
  }
}

/**
 * Merge Assist on, and every peer conflicting with the checkout's work, none stored yet: a pass finds each conflict new.
 * Answers the open pull requests from the peers.
 */
async function conflictsNew(ws: Workspace): Promise<PullRequestSummary[]> {
  const pullRequests = addPeers(ws).map(pullOf);
  await ws.service().setEnabled(ws.repo, true);
  return pullRequests;
}

/**
 * Every peer's conflict stored by a scan, then each peer committed anew changing a file of its own: a pass finds every
 * conflict gone. Answers the open pull requests from the new tips.
 */
async function conflictsGone(ws: Workspace): Promise<PullRequestSummary[]> {
  const first = ws.service({ github: new FakeGithub({ pullRequests: addPeers(ws).map(pullOf) }) });
  await first.setEnabled(ws.repo, true);
  expect((await first.scan(ws.repo)).conflicts).toHaveLength(PEERS);
  await first.settled();
  return Array.from({ length: PEERS }, (_, index) => pullOf(commitPeer(ws, index, `Peer ${index}, elsewhere`, `src/peer-${index}.ts`), index));
}

const CHANGES = { new: conflictsNew, gone: conflictsGone };

/** Merge Assist on, and every peer's conflict stored by a scan. Answers the open pull requests and the conflicts' ids. */
async function conflictsStored(ws: Workspace): Promise<{ pullRequests: PullRequestSummary[]; ids: string[] }> {
  const pullRequests = addPeers(ws).map(pullOf);
  const first = ws.service({ github: new FakeGithub({ pullRequests }) });
  await first.setEnabled(ws.repo, true);
  const ids = (await first.scan(ws.repo)).conflicts.map(conflict => conflict.id);
  expect(ids).toHaveLength(PEERS);
  await first.settled();
  return { pullRequests, ids };
}

/**
 * A session's scan of `checkout` in a new service, stopped as it is `at` a point: "gh", its listing of open pull
 * requests, which answers once the stop has begun; else its first Git read whose arguments hold `at`. The stop's wait
 * lasts until the scan ends. Answers the Git reads left running at the stop and started after it.
 */
async function scanEndingWithinStop(ws: Workspace, at: string, pullRequests: PullRequestSummary[], checkout = ws.repo): Promise<{ runningAtStop: string[]; startedAfter: string[] }> {
  const github = new HeldGithub({ pullRequests });
  if (at !== "gh") github.release();
  const service = ws.service({ github });
  return await stopDuring(service, () => service.openSession(checkout, "claude-code", "session-1"), {
    ...(at === "gh" ? {} : { at }),
    trigger: stop => {
      if (at !== "gh") return;
      github.onHeld = () => {
        stop();
        queueMicrotask(() => github.release());
      };
    },
    waitMs: UNREACHED_MS,
    held: false,
  });
}

/** A detection after an edit in `checkout` in a new service, and its hook's handlers settled. */
async function nextPass(ws: Workspace, pullRequests: PullRequestSummary[], checkout = ws.repo): Promise<void> {
  const next = ws.service({ github: new FakeGithub({ pullRequests }) });
  expect(await next.detectAfterEdit(checkout)).toBe(true);
  await next.settled();
}

/** The open conflicts of the checkout as a new service reads them. */
async function openConflicts(ws: Workspace): Promise<number> {
  return (await stored(ws)).conflicts.filter(conflict => conflict.status === "open").length;
}

/**
 * A session's scan in a new service over `github`, waiting for its listing of open pull requests, and that service's
 * stop begun with a wait the scan ends within once `github` answers. Answers the service, the scan and the stop.
 */
async function stopWhileScanWaits(ws: Workspace, github: HeldGithub): Promise<{ service: MergeAssistRuntime; scan: Promise<void>; stopped: Promise<void> }> {
  const asked = Promise.withResolvers<void>();
  github.onHeld = () => asked.resolve();
  const service = ws.service({ github });
  const scan = service.openSession(ws.repo, "claude-code", "session-1");
  await asked.promise;
  return { service, scan, stopped: service.stop(UNREACHED_MS) };
}

/** How many rows hold the kept conflict events of `checkout`: none once a change has taken them. */
const keptRows = (ws: Workspace, checkout = ws.repo): number =>
  ws.database.select().from(mergeAssistState).where(eq(mergeAssistState.key, `unheard:${checkout}`)).all().length;

/** The checkout's kept conflict events as stored, each entry as it is. */
function keptEntries(ws: Workspace): Array<{ event?: unknown; conflict?: unknown }> {
  const row = ws.database.select().from(mergeAssistState).where(eq(mergeAssistState.key, `unheard:${ws.repo}`)).get();
  return row === undefined ? [] : (JSON.parse(row.valueJson) as Array<{ event?: unknown; conflict?: unknown }>);
}

/**
 * Adds to the checkout's kept events one this build cannot read, an event of a kind it does not know, as a later
 * build might keep one; `alone`, in their place.
 */
function keepUnreadable(ws: Workspace, alone = false): void {
  const kept = keptEntries(ws);
  const unreadable = { event: "reopened", conflict: kept[0]?.conflict };
  const valueJson = JSON.stringify(alone ? [unreadable] : [...kept, unreadable]);
  ws.database.update(mergeAssistState).set({ valueJson }).where(eq(mergeAssistState.key, `unheard:${ws.repo}`)).run();
}

/** `count` times `event`. */
const times = (count: number, event: string): string[] => Array.from({ length: count }, () => event);

/** A checkout of the repository at `<folder>/<name>` on a new branch from `from`, recorded as a session start does. */
function addCheckout(ws: Workspace, name: string, from: string): string {
  git(ws.repo, "worktree", "add", "-q", "-b", name, join(ws.folder, name), from);
  return recordCheckout(ws.database, join(ws.folder, name)).checkoutPath;
}

/** The checkout's last scan, scan mark and conflicts as a new service reads them. */
async function stored(ws: Workspace): Promise<Pick<MergeAssistStatus, "lastScan" | "scanning" | "conflicts">> {
  const { lastScan, scanning, conflicts } = await ws.service().status(ws.repo);
  return { lastScan, scanning, conflicts };
}

describe("Merge Assist's stop and a detection pass in flight", { timeout: 60_000 }, () => {
  it.each([
    ["reads the checkout's own branch and base", "refs/heads/master", false],
    ["compares the checkout's work with other branches", "--name-only", false],
    ["checks which open pull requests are work of their own", "--is-ancestor", true],
  ])("ends a scan that %s, still running when the stop's wait is over, at its next Git read: no Git runs once the stop resolves, and what was found stays", async (_, at, pulls) => {
    const ws = await workspace(pulls ? { remote: "https://github.com/acme/app.git" } : {});
    const tips = addPeers(ws);
    const github = new FakeGithub(pulls ? { pullRequests: tips.map(pullOf) } : {});
    const first = ws.service({ github });
    await first.setEnabled(ws.repo, true);
    const found = await first.scan(ws.repo);
    expect(found.conflicts).toHaveLength(PEERS);

    const service = ws.service({ github });
    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at, waitMs: 0, held: true });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: found.lastScan, scanning: false, conflicts: found.conflicts });
  });

  it("lets a scan that ends within the stop's wait store what it found, as a scan that ends before the stop does", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at: "--name-only", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const scan = await stored(ws);
    expect({ peers: scan.lastScan?.peers, scanning: scan.scanning, conflicts: scan.conflicts.length, tip: peerTip(scan.conflicts) })
      .toEqual({ peers: { branches: PEERS, pullRequests: 0 }, scanning: false, conflicts: PEERS, tip });
  });

  it("lets the detection after an edit that ends within the stop's wait store what it found", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.detectAfterEdit(ws.repo, "session-1", "claude-code"), { at: "--name-only", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const { conflicts } = await stored(ws);
    expect({ conflicts: conflicts.length, tip: peerTip(conflicts) }).toEqual({ conflicts: PEERS, tip });
  });

  it("lets a session's scan whose checkout Git was still locating when the stop came store what it found within the stop's wait", async () => {
    const ws = await workspace();
    const tip = await knownConflictsMoved(ws);
    const service = ws.service();

    const { runningAtStop } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), { at: "--show-toplevel", waitMs: UNREACHED_MS, held: false });

    expect(runningAtStop).toEqual([]);
    const scan = await stored(ws);
    expect({ peers: scan.lastScan?.peers, scanning: scan.scanning, conflicts: scan.conflicts.length, tip: peerTip(scan.conflicts) })
      .toEqual({ peers: { branches: PEERS, pullRequests: 0 }, scanning: false, conflicts: PEERS, tip });
  });

  it.each([
    ["whose `gh` listing answers within the stop's wait", "new", "gh"],
    ["already comparing the checkout's work with other branches when the stop came", "new", "--name-only"],
    ["whose `gh` listing answers within the stop's wait", "gone", "gh"],
    ["already comparing the checkout's work with other branches when the stop came", "gone", "--name-only"],
  ] as const)("stores what a scan %s finds %s, as a scan that ends before the stop does, and a service opened after hands its hook each event the stopped hook did not hear", async (_, change, at) => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const pullRequests = await CHANGES[change](ws);
    const heard = await loggedHook(ws);

    const { runningAtStop, startedAfter } = await scanEndingWithinStop(ws, at, pullRequests);

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(heard()).toEqual([]);
    const scan = await stored(ws);
    expect({ scanned: scan.lastScan !== null, scanning: scan.scanning, open: await openConflicts(ws) })
      .toEqual({ scanned: true, scanning: false, open: change === "new" ? PEERS : 0 });
    await nextPass(ws, pullRequests);
    expect(heard().map(({ event }) => event)).toEqual(Array.from({ length: PEERS }, () => (change === "new" ? "detected" : "closed")));
  });

  it.each(["new", "gone"] as const)("with no hook command, stores what a scan already comparing when the stop came finds %s, and a service opened after shows it before any pass: the agents' notes and the page's conflicts", async change => {
    const ws = await workspace();
    const pullRequests = await CHANGES[change](ws);

    await scanEndingWithinStop(ws, "--name-only", pullRequests);

    const next = ws.service();
    expect({ open: await openConflicts(ws), notes: takeNotes(next, "session-2", ws.repo).length })
      .toEqual(change === "new" ? { open: PEERS, notes: PEERS } : { open: 0, notes: 0 });
  });

  it("hands the kept events to the hook once when two services pass at once", async () => {
    const ws = await workspace();
    await conflictsNew(ws);
    const heard = await loggedHook(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    expect(await openConflicts(ws)).toBe(PEERS);

    const own = [ownConnection(ws, new FakeGithub()), ownConnection(ws, new FakeGithub())];
    const passes = own.map(({ service }) => service);
    expect(await Promise.all(passes.map(async pass => await pass.detectAfterEdit(ws.repo)))).toEqual([true, true]);
    await Promise.all(passes.map(async pass => await pass.settled()));
    // Closed now, before the workspace's folder is removed: Windows refuses to remove a database a connection holds.
    for (const { close } of own) close();

    expect(heard().map(({ event }) => event)).toEqual(Array.from({ length: PEERS }, () => "detected"));
  });

  it("hands a kept `detected` before the next pass's `closed` of the same conflict when its conflict went before that pass", async () => {
    const ws = await workspace();
    await conflictsNew(ws);
    const heard = await loggedHook(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    for (let index = 0; index < PEERS; index += 1) commitPeer(ws, index, `Peer ${index}, elsewhere`, `src/peer-${index}.ts`);

    await nextPass(ws, []);

    const events = byConflict(heard());
    expect({ conflicts: events.size, open: await openConflicts(ws) }).toEqual({ conflicts: PEERS, open: 0 });
    for (const sequence of events.values()) expect(sequence).toEqual(["detected", "closed"]);
  });

  it("keeps nothing of a conflict one stopped scan found and a later stopped scan found gone: the hook never heard it open, and it is not open", async () => {
    const ws = await workspace();
    await conflictsNew(ws);
    const heard = await loggedHook(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    for (let index = 0; index < PEERS; index += 1) commitPeer(ws, index, `Peer ${index}, elsewhere`, `src/peer-${index}.ts`);
    await scanEndingWithinStop(ws, "--name-only", []);
    expect(await openConflicts(ws)).toBe(0);

    await nextPass(ws, []);

    expect(heard()).toEqual([]);
  });

  it("hands a conflict that three stopped scans found gone, found again and found gone as `closed`, then the `detected` of the pass that finds it once more: never one event twice in a row", async () => {
    const ws = await workspace();
    const heard = await loggedHook(ws);
    await conflictsStored(ws);
    for (const move of [peersElsewhere, peersBack, peersElsewhere]) {
      move(ws);
      await scanEndingWithinStop(ws, "--name-only", []);
    }
    expect(await openConflicts(ws)).toBe(0);
    peersBack(ws);

    await nextPass(ws, []);

    const events = byConflict(heard());
    expect({ conflicts: events.size, open: await openConflicts(ws) }).toEqual({ conflicts: PEERS, open: PEERS });
    for (const sequence of events.values()) expect(sequence).toEqual(["detected", "closed", "detected"]);
  });

  it("judges a conflict by its last kept event: one stopped scan found it gone and the next found it again, so a pass that finds it still open hands the kept `closed` then `detected`", async () => {
    const ws = await workspace();
    const heard = await loggedHook(ws);
    await conflictsStored(ws);
    peersElsewhere(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    peersBack(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    expect(await openConflicts(ws)).toBe(PEERS);

    await nextPass(ws, []);

    const events = byConflict(heard());
    expect({ conflicts: events.size, open: await openConflicts(ws), kept: keptRows(ws) }).toEqual({ conflicts: PEERS, open: PEERS, kept: 0 });
    for (const sequence of events.values()) expect(sequence).toEqual(["detected", "closed", "detected"]);
  });

  it.each([
    ["beside the kept events it can read", false],
    ["alone", true],
  ] as const)("hands the hook every kept event it can read and removes the key at the next pass when one kept entry has a shape this build does not know, %s", async (_, alone) => {
    const ws = await workspace();
    await conflictsNew(ws);
    const heard = await loggedHook(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    keepUnreadable(ws, alone);

    await nextPass(ws, []);

    expect({ heard: heard().map(({ event }) => event), kept: keptRows(ws) }).toEqual({ heard: alone ? [] : times(PEERS, "detected"), kept: 0 });
  });

  it("keeps the kept events it can read beside the events of a scan that ends within the stop's wait when one kept entry has a shape this build does not know, and leaves that entry out", async () => {
    const ws = await workspace();
    const heard = await loggedHook(ws);
    await conflictsStored(ws);
    peersElsewhere(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    keepUnreadable(ws);
    peersBack(ws);

    await scanEndingWithinStop(ws, "--name-only", []);

    expect(keptEntries(ws).map(({ event }) => event)).toEqual([...times(PEERS, "closed"), ...times(PEERS, "detected")]);
    await nextPass(ws, []);
    const events = byConflict(heard());
    expect({ conflicts: events.size, kept: keptRows(ws) }).toEqual({ conflicts: PEERS, kept: 0 });
    for (const sequence of events.values()) expect(sequence).toEqual(["detected", "closed", "detected"]);
  });

  it.each([
    ["new", "gone", peersElsewhere],
    ["gone", "open again", peersBack],
  ] as const)("hands the hook no kept event the store contradicts: a scan that ends within the stop's wait finds the conflicts %s, then a build without the kept events' key stores them %s", async (change, _, undo) => {
    const ws = await workspace();
    const pullRequests = await CHANGES[change](ws);
    const heard = await loggedHook(ws);
    const before = readConflicts(ws.database, ws.repo);
    await scanEndingWithinStop(ws, "--name-only", pullRequests);
    // As a pass of a build without the key: it finds the conflicts as they were before the scan and stores them,
    // neither reading the key nor clearing it.
    undo(ws);
    writeConflicts(ws.database, ws.repo, before, Date.now());

    await nextPass(ws, pullRequests);

    expect({ heard: heard(), open: await openConflicts(ws), kept: keptRows(ws) }).toEqual({ heard: [], open: change === "new" ? 0 : PEERS, kept: 0 });
  });

  it("keeps the `closed` of a conflict dismissed during the stop's wait, and a service opened after hands it to its hook", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const { pullRequests, ids } = await conflictsStored(ws);
    const heard = await loggedHook(ws);
    const github = new HeldGithub({ pullRequests });
    const { service, scan, stopped } = await stopWhileScanWaits(ws, github);

    await service.dismissConflict(ws.repo, ids[0] ?? "");
    github.release();
    await scan;
    await stopped;

    expect(heard()).toEqual([]);
    await nextPass(ws, pullRequests);
    expect(heard()).toEqual([{ event: "closed", id: ids[0] }]);
  });

  it("keeps the `closed` of each conflict open when Merge Assist is turned off during the stop's wait, and the first pass once it is on again hands each to its hook before its own `detected`", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    const { pullRequests } = await conflictsStored(ws);
    const heard = await loggedHook(ws);
    const github = new HeldGithub({ pullRequests });
    const { service, scan, stopped } = await stopWhileScanWaits(ws, github);

    await service.setEnabled(ws.repo, false);
    github.release();
    await scan;
    await stopped;

    expect(heard()).toEqual([]);
    await ws.service().setEnabled(ws.repo, true);
    await nextPass(ws, pullRequests);
    const events = byConflict(heard());
    expect(events.size).toBe(PEERS);
    for (const sequence of events.values()) expect(sequence).toEqual(["closed", "detected"]);
  });

  it.each(["dismissal", "turn-off"] as const)("hands the kept `detected` of a scan that ended within the stop's wait before the `closed` of a %s in a service opened after", async change => {
    const ws = await workspace();
    await conflictsNew(ws);
    const heard = await loggedHook(ws);
    await scanEndingWithinStop(ws, "--name-only", []);
    const next = ws.service();
    const [dismissed = ""] = (await next.status(ws.repo)).conflicts.map(conflict => conflict.id);

    await (change === "dismissal" ? next.dismissConflict(ws.repo, dismissed) : next.setEnabled(ws.repo, false));
    await next.settled();

    const events = byConflict(heard());
    expect(events.size).toBe(PEERS);
    for (const [id, sequence] of events) expect(sequence).toEqual(change === "turn-off" || id === dismissed ? ["detected", "closed"] : ["detected"]);
  });

  it.each([
    ["the `closed` events of scans that found the conflicts gone within the stop's wait", "with no wait", true, 0],
    ["the `closed` of each conflict still open", "with a wait", false, UNREACHED_MS],
  ] as const)("keeps in two checkouts %s when a stop %s lands while turning Merge Assist off reads the first one's branches, and the next turn-off hands each to the hook once", async (_, _wait, gone, waitMs) => {
    const ws = await workspace();
    // A second checkout with the main checkout's work: every peer conflicts with it too.
    const checkouts = [ws.repo, addCheckout(ws, "mine-too", "mine")];
    const heard = await loggedHook(ws);
    addPeers(ws);
    const first = ws.service();
    await first.setEnabled(ws.repo, true);
    for (const checkout of checkouts) expect((await first.scan(checkout)).conflicts).toHaveLength(PEERS);
    await first.settled();
    if (gone) {
      peersElsewhere(ws);
      for (const checkout of checkouts) await scanEndingWithinStop(ws, "--name-only", [], checkout);
    }
    const { service, close } = ownConnection(ws, new FakeGithub());
    // Once the stop has resolved: a second stop with no wait, as a runtime's stores give as they close, and the close.
    const stopAgainAndClose = (): void => {
      void service.stop(0);
      close();
    };

    // The first branch read answers only once the database has closed. The turn-off's answer, read after, meets the
    // closed database, as before.
    const turnOff = (): Promise<unknown> => service.setEnabled(ws.repo, false).catch(() => undefined);
    await stopDuring(service, turnOff, { at: "symbolic-ref", waitMs, held: true, afterStop: stopAgainAndClose });

    const kept = (): number[] => checkouts.map(checkout => keptRows(ws, checkout));
    expect({ heard: heard().length, kept: kept() }).toEqual({ heard: 2 * PEERS, kept: [1, 1] });
    const next = ws.service();
    await next.setEnabled(ws.repo, true);
    await next.setEnabled(ws.repo, false);
    await next.settled();
    await next.stop();
    expect({ closed: heard().filter(({ event }) => event === "closed").length, kept: kept() }).toEqual({ closed: 2 * PEERS, kept: [0, 0] });
  });

  it("goes on with a stop with no wait that meets another process's write lock as it keeps the events turning Merge Assist off has not handed yet, and says they are not kept", async () => {
    const ws = await workspace();
    const heard = await loggedHook(ws);
    await conflictsStored(ws);
    const { service, close } = ownConnection(ws, new FakeGithub());
    const reading = Promise.withResolvers<ChildProcess>();
    let held = false;
    // The turn-off's first branch read is held until the test ends it.
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (held || !commandLine(args).includes("symbolic-ref")) return actualSpawn(...args);
      held = true;
      const child = actualSpawn(process.execPath, HELD_READ, { stdio: ["pipe", "pipe", "pipe"] });
      reading.resolve(child);
      return child;
    });
    try {
      const turnOff = service.setEnabled(ws.repo, false).catch(() => undefined);
      const read = await reading.promise;
      const holder = holdWriteLock(ws.databasePath, ws.folder);
      await holder.held;
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      // As `DatabaseApiStores.close()` does. A stop that waited for the lock would hold this thread until the other
      // process's bound is over, then get the lock and keep the events.
      const stopped = service.stop(0);
      close();
      const written = stderr.mock.calls.map(([text]) => String(text));
      stderr.mockRestore();
      await holder.release();
      read.kill("SIGKILL");
      await turnOff;

      await expect(stopped).resolves.toBeUndefined();
      const lost = "rerail: Merge Assist could not keep the conflict events it had not handed: database is locked\n";
      expect({ written, kept: keptRows(ws), closed: heard().filter(({ event }) => event === "closed") }).toEqual({ written: [lost], kept: 0, closed: [] });
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }
  });

  it("keeps in two checkouts the events turning Merge Assist off has not handed yet when a stop with a wait meets another process's write lock: it waits for the lock, which frees within Rerail's wait for one", async () => {
    const ws = await workspace();
    // A second checkout with the main checkout's work: every peer conflicts with it too.
    const checkouts = [ws.repo, addCheckout(ws, "mine-too", "mine")];
    addPeers(ws);
    const first = ws.service();
    await first.setEnabled(ws.repo, true);
    for (const checkout of checkouts) expect((await first.scan(checkout)).conflicts).toHaveLength(PEERS);
    // Rerail's own connection, which waits five seconds for a lock.
    const { service } = ownConnection(ws, new FakeGithub());
    const reading = Promise.withResolvers<ChildProcess>();
    let held = false;
    // The turn-off's first branch read is held until the test ends it.
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (held || !commandLine(args).includes("symbolic-ref")) return actualSpawn(...args);
      held = true;
      const child = actualSpawn(process.execPath, HELD_READ, { stdio: ["pipe", "pipe", "pipe"] });
      reading.resolve(child);
      return child;
    });
    let written: string[] = [];
    try {
      const turnOff = service.setEnabled(ws.repo, false).catch(() => undefined);
      const read = await reading.promise;
      const holder = holdWriteLock(ws.databasePath, ws.folder);
      await holder.held;
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      // The other process gives the lock back at its bound, within the five seconds: a stop that waits for the lock holds
      // this thread until then and keeps the events; one that did not wait would meet the lock held and lose them.
      try {
        await service.stop(UNREACHED_MS);
      } finally {
        written = stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes("Merge Assist"));
        stderr.mockRestore();
      }
      await holder.release();
      read.kill("SIGKILL");
      await turnOff;
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }

    expect({ written, kept: checkouts.map(checkout => keptRows(ws, checkout)) }).toEqual({ written: [], kept: [1, 1] });
  });

  it("hands the `closed` of conflicts another pass stores in a checkout as Merge Assist is being turned off, as it hands those of the other checkouts", async () => {
    const ws = await workspace();
    const heard = await loggedHook(ws);
    await conflictsStored(ws);
    const other = addCheckout(ws, "other", "main");
    const service = ws.service();
    // As another process's pass would, while the turn-off locates its checkout, before it changes the store.
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (commandLine(args).includes("--show-toplevel") && readConflicts(ws.database, ws.repo).length > 0 && readConflicts(ws.database, other).length === 0) {
        writeConflicts(ws.database, other, readConflicts(ws.database, ws.repo), Date.now());
      }
      return actualSpawn(...args);
    });
    try {
      await service.setEnabled(ws.repo, false);
      await service.settled();
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }

    expect({ closed: heard().filter(({ event }) => event === "closed").length, other: readConflicts(ws.database, other), kept: keptRows(ws, other) })
      .toEqual({ closed: 2 * PEERS, other: [], kept: 0 });
  });

  it("reads the branches only of the checkouts it hands events in when Merge Assist is turned on or off, none of those passes scanned and found nothing in", async () => {
    const ws = await workspace();
    await ws.service().setEnabled(ws.repo, true);
    // Scanned before any peer is there: each pass finds nothing, and stores that it found nothing.
    const idle = ["idle-1", "idle-2", "idle-3", "idle-4"].map(name => addCheckout(ws, name, "main"));
    for (const checkout of idle) expect((await ws.service().scan(checkout)).conflicts).toEqual([]);
    const found = ws.database.select().from(mergeAssistState).all().filter(({ key }) => idle.some(checkout => key === `conflicts:${checkout}`));
    expect(found.map(({ valueJson }) => valueJson)).toEqual(times(idle.length, "[]"));
    await conflictsStored(ws);
    const reads = { on: [] as string[], off: [] as string[] };
    let turn: "on" | "off" = "on";
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (isGit(args)) reads[turn].push(commandLine(args));
      return actualSpawn(...args);
    });
    try {
      // Already on, asked from a checkout with nothing found: no event to hand.
      await ws.service().setEnabled(idle[0] ?? "", true);
      turn = "off";
      await ws.service().setEnabled(ws.repo, false);
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }

    const idleReads = reads.off.filter(line => idle.some(checkout => line.includes(checkout)));
    expect({ on: reads.on.filter(line => line.includes(ws.repo)), off: idleReads }).toEqual({ on: [], off: [] });
  });

  it("resolves a stop with no wait whose release of a scan's mark meets another process's write lock without waiting for the lock, and leaves the mark to go stale, as when its process ends", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    // Rerail's own connection, which waits five seconds for a lock.
    const { service, close, database } = ownConnection(ws, github);
    const busyTimeout: unknown = database.$client.pragma("busy_timeout", { simple: true });
    const asked = Promise.withResolvers<void>();
    github.onHeld = () => asked.resolve();
    const scan = service.openSession(ws.repo, "claude-code", "session-1");
    await asked.promise;
    const holder = holdWriteLock(ws.databasePath, ws.folder);
    await holder.held;
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    // As `DatabaseApiStores.close()` does: the stop, then the database closed in the same call. The other process gives
    // the lock back only once the test writes its file, after the close. A release that waited for the lock would hold
    // this thread until the other process's bound is over, then get the lock and remove the mark.
    const stopped = service.stop(0);
    const busyTimeoutAfter: unknown = database.$client.pragma("busy_timeout", { simple: true });
    close();
    const written = stderr.mock.calls.map(([text]) => String(text));
    stderr.mockRestore();
    await holder.release();
    github.release();
    await scan;

    await expect(stopped).resolves.toBeUndefined();
    expect({ written, busyTimeoutAfter }).toEqual({ written: [`rerail: Merge Assist could not give up its scan of ${ws.repo}: database is locked\n`], busyTimeoutAfter: busyTimeout });
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: true, conflicts: [] });
  });

  it("resolves a stop with no wait whose caller keeps the database open until it resolves without waiting for another process's write lock at either release of a scan's mark", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    const { service, close } = ownConnection(ws, github);
    const asked = Promise.withResolvers<void>();
    github.onHeld = () => asked.resolve();
    const scan = service.openSession(ws.repo, "claude-code", "session-1");
    await asked.promise;
    const holder = holdWriteLock(ws.databasePath, ws.folder);
    await holder.held;
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    // The other process gives the lock back only once the stop has resolved and the database has closed. A release that
    // waited for the lock would hold this thread until the other process's bound is over, then get it and remove the mark.
    await service.stop(0);
    const written = stderr.mock.calls.map(([text]) => String(text));
    stderr.mockRestore();
    close();
    await holder.release();
    github.release();
    await scan;

    const busy = `rerail: Merge Assist could not give up its scan of ${ws.repo}: database is locked\n`;
    expect({ written, stored: await stored(ws) }).toEqual({ written: [busy, busy], stored: { lastScan: null, scanning: true, conflicts: [] } });
  });

  it("gives up the mark of a scan still running when a stop's wait is over once another process's write lock frees, as before", async () => {
    const ws = await workspace();
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const { service, close } = ownConnection(ws, new FakeGithub());
    // In place of the scan's first comparing read runs another process holding the write lock. Once the stop's wait is
    // over, the stop ends every read still running, this one with it, right before it gives up the scan's mark on this
    // thread: the lock frees while that release waits for it, and a release that did not wait would meet it held.
    const holding = Promise.withResolvers<LockHolder>();
    let holder: LockHolder | undefined;
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (holder !== undefined || !commandLine(args).includes("--name-only")) return actualSpawn(...args);
      holder = holdWriteLock(ws.databasePath, ws.folder);
      holding.resolve(holder);
      return holder.child;
    });
    let written: string[] = [];
    try {
      const scan = service.openSession(ws.repo, "claude-code", "session-1");
      await (await holding.promise).held;
      const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
      try {
        await service.stop(10);
      } finally {
        written = stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes("Merge Assist"));
        stderr.mockRestore();
      }
      await scan;
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }
    close();

    expect(written).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("returns once the Git it ended has exited while a scan waits for a file call that never answers, and the scan then writes nothing to the closed database and starts no Git", async () => {
    const ws = await workspace();
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const { service, close } = ownConnection(ws, new FakeGithub());
    let release = (): void => undefined;

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      trigger: stop => {
        release = holdAccessTo(ws.repo, stop).release;
      },
      waitMs: 0,
      held: false,
      // The call answers only once the stop has resolved and the database has closed.
      afterStop: () => {
        close();
        release();
      },
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("lets a scan that `gh` answers once the stop has resolved and the database has closed write nothing", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    const { service, close } = ownConnection(ws, github);

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      trigger: stop => {
        github.onHeld = stop;
      },
      waitMs: 0,
      held: false,
      afterStop: () => {
        close();
        github.release();
      },
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("lets a scan that `gh` answers right after a stop with no wait, whose caller closed the database without awaiting it, write nothing and start no Git", async () => {
    const ws = await workspace({ remote: "https://github.com/acme/app.git" });
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const github = new HeldGithub();
    const { service, close } = ownConnection(ws, github);
    const asked = Promise.withResolvers<void>();
    github.onHeld = () => asked.resolve();
    const scan = service.openSession(ws.repo, "claude-code", "session-1");
    await asked.promise;
    const started: string[] = [];
    vi.mocked(spawn).mockImplementation((...args: Parameters<typeof spawn>) => {
      if (isGit(args)) started.push(commandLine(args));
      return actualSpawn(...args);
    });
    try {
      // As `DatabaseApiStores.close()` does: the stop, then the database closed in the same call.
      const stopped = service.stop(0);
      close();
      github.release();
      await scan;
      await stopped;
    } finally {
      vi.mocked(spawn).mockImplementation(actualSpawn);
    }

    expect(started).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });

  it("ends a session's locating of its checkout still running when the stop's wait is over: no Git runs once the stop resolves, and nothing is written", async () => {
    const ws = await workspace();
    addPeers(ws);
    await ws.service().setEnabled(ws.repo, true);
    const { service, close } = ownConnection(ws, new FakeGithub());

    const { runningAtStop, startedAfter } = await stopDuring(service, () => service.openSession(ws.repo, "claude-code", "session-1"), {
      at: "--show-toplevel", waitMs: 0, held: true, afterStop: close,
    });

    expect(startedAfter).toEqual([]);
    expect(runningAtStop).toEqual([]);
    expect(await stored(ws)).toEqual({ lastScan: null, scanning: false, conflicts: [] });
  });
});
