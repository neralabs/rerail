import { spawn, type ChildProcessByStdio } from "node:child_process";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import type { Readable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { MODULE_COPIES_LINE, OWNER_FILE, thisPlace } from "./run-folder.js";

/** This checkout, whose vitest.config.ts every run below uses. */
const CHECKOUT = fileURLToPath(new URL("../../../", import.meta.url));
const VITEST = join(CHECKOUT, "node_modules", "vitest", "vitest.mjs");
const MINUTE_MS = 60 * 1000;
const DAY_MS = 24 * 60 * MINUTE_MS;
/** A pid no process has: above Linux's highest pid, and not one Windows gives. */
const NO_PROCESS = 2147483644;
/** The name vitest gives the folder it copies a run's transformed modules into. */
const MODULE_COPIES = /^[A-Za-z0-9_-]{21}$/u;
/** A run folder's name: `rv-`, the pid of the run's vitest process, then mkdtemp's six characters. */
const RUN_FOLDER = /^rv-\d+-[A-Za-z0-9]{6}$/u;
/** The longest Unix socket path in bytes, the terminating NUL aside: 107 on Linux, 103 on macOS. */
const SOCKET_PATH_BYTES = process.platform === "linux" ? 107 : 103;
/** How far below its temp folder the `socket` test file's socket ends, the deepest a test binds. */
const DEEPEST_SOCKET_BYTES = "/rerail-socket-XXXXXX/rerail-socket/nested/engine.sock".length;
/** The longest temp folder with room for that socket below a run folder, whose pid Linux gives at most 7 digits. */
const ROOMY_TEMP_BYTES = SOCKET_PATH_BYTES - DEEPEST_SOCKET_BYTES - "/rv-1234567-XXXXXX".length;
/** The shortest temp folder with no room for that socket below a run folder, whatever its pid. */
const CRAMPED_TEMP_BYTES = SOCKET_PATH_BYTES - DEEPEST_SOCKET_BYTES - "/rv-1-XXXXXX".length + 1;
/** Where this worker's pid means something, as a run folder's owner file names it: the runs below share it. */
const HERE = thisPlace() ?? "";
/**
 * What the `stuck`, `locked` and `socket` test files write next to themselves: the pid of the process in the folder,
 * the folder, or the temp folder the test file ran with.
 */
const NOTE = "note.txt";
const IMPORTS = [
  `import { mkdtempSync } from "node:fs";`,
  `import { tmpdir } from "node:os";`,
  `import { join } from "node:path";`,
  `import { it } from "vitest";`,
].join("\n");
/** The test files the runs below run, each in a folder of its own: a run's `--dir` takes every test file in it. */
const TEST_FILES = {
  leaves: `${IMPORTS}\nit("leaves a folder behind", () => { mkdtempSync(join(tmpdir(), "left-")); });\n`,
  // The folder's name carries the worker's pid, which killRun ends after vitest's own process.
  holds: `${IMPORTS}\nit("holds a folder until killed", async () => { mkdtempSync(join(tmpdir(), "held-" + process.pid + "-")); await new Promise(() => {}); }, 600_000);\n`,
  // On Windows, leaves a folder no removal takes until releaseHeld ends the process that runs in it, whose pid it writes
  // next to itself. The process takes the folder as it starts, after spawn returns, so the test waits for the file it
  // writes there.
  stuck: `${IMPORTS}
import { spawn } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
it("leaves a folder its run cannot remove", async () => {
  const stuck = mkdtempSync(join(tmpdir(), "stuck-"));
  const holder = spawn(process.execPath, ["-e", "require('node:fs').writeFileSync('running', ''); setTimeout(() => {}, 600000)"], { cwd: stuck, detached: true, stdio: "ignore", windowsHide: true });
  holder.unref();
  writeFileSync(new URL("${NOTE}", import.meta.url), String(holder.pid));
  while (!existsSync(join(stuck, "running"))) await new Promise(resolve => setTimeout(resolve, 20));
}, 60_000);
`,
  // Leaves a folder and a file in it read-only, as a test killed before it gives a folder back its write permission
  // does, and writes the folder's path next to itself for releaseHeld.
  locked: `${IMPORTS}
import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
it("leaves a folder and a file it made read-only", () => {
  const locked = join(mkdtempSync(join(tmpdir(), "locked-")), "locked");
  mkdirSync(locked);
  writeFileSync(join(locked, "file"), "");
  chmodSync(join(locked, "file"), 0o400);
  chmodSync(locked, 0o500);
  writeFileSync(new URL("${NOTE}", import.meta.url), locked);
});
`,
  // Listens on a Unix socket as far below the temp folder as engine-service-socket.test.ts's, the deepest a test binds,
  // and writes its temp folder next to itself.
  socket: `${IMPORTS}
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
it("listens on a socket 54 bytes below the temp folder", async () => {
  writeFileSync(new URL("${NOTE}", import.meta.url), tmpdir());
  const folder = join(mkdtempSync(join(tmpdir(), "rerail-socket-")), "rerail-socket", "nested");
  mkdirSync(folder, { recursive: true });
  const server = createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(join(folder, "engine.sock"), resolve); });
  await new Promise(resolve => server.close(resolve));
});
`,
};

interface Run {
  readonly child: ChildProcessByStdio<null, Readable, Readable>;
  readonly ended: Promise<{ code: number | null; output: string }>;
}

const scratches: string[] = [];
const runs: Run[] = [];

afterEach(async () => {
  // A case that failed part way may leave a run going; it ends before its folders go. Its workers end with it, as a
  // vitest worker does once its parent is gone. Only a run still going is signalled: an ended one's pid may be reused.
  for (const run of runs.splice(0)) {
    if (run.child.exitCode === null && run.child.signalCode === null) kill(run.child.pid);
    await run.ended.catch(() => undefined);
  }
  for (const root of scratches) await releaseHeld(join(root, "tests"));
  await Promise.all(scratches.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** A folder for one case: `temp` is the empty temp folder its runs get, `tests/<name>/` holds each of TEST_FILES. */
async function scratch(): Promise<{ temp: string; tests: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-run-folder-"));
  scratches.push(root);
  await mkdir(join(root, "temp"));
  for (const [name, text] of Object.entries(TEST_FILES)) {
    await mkdir(join(root, "tests", name), { recursive: true });
    await writeFile(join(root, "tests", name, `${name}.test.ts`), text);
  }
  return { temp: join(root, "temp"), tests: join(root, "tests") };
}

/**
 * An empty temp folder `bytes` long, outside this run's folder, as a TMPDIR set by hand may be, so a run there picks
 * its run folder's place as such a run does. Its parent in /tmp is named rerail-vitest-*, so a later run removes it a
 * day after a kill left it.
 */
async function tempOfLength(bytes: number): Promise<string> {
  const root = await mkdtemp("/tmp/rerail-vitest-k-");
  scratches.push(root);
  const temp = join(root, "t".repeat(bytes - root.length - 1));
  await mkdir(temp);
  return temp;
}

/**
 * This process's environment with every temp folder variable naming `temp`, the variables in `extra` added, and none
 * of this vitest worker's own nor a short temp folder the person set.
 */
function runEnvironment(temp: string, extra: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const environment = Object.fromEntries(
    Object.entries(process.env).filter(([name]) => !/^(?:TMPDIR|TEMP|TMP|VITEST.*|RERAIL_TEST_SHORT_TEMP)$/iu.test(name)),
  );
  return { ...environment, TMPDIR: temp, TEMP: temp, TMP: temp, ...extra };
}

/**
 * `vitest run` on this checkout's config over the test files in `directory`, with `temp` as its temp folder and the
 * variables in `extra`. Its module cache is off, as it is wherever node_modules cannot hold the cache, so vitest copies
 * each transformed module into a folder of its own in `temp`, which a run killed before vitest closes leaves.
 */
function startRun(directory: string, temp: string, extra: NodeJS.ProcessEnv = {}): Run {
  const child = spawn(process.execPath, [VITEST, "run", "--root", CHECKOUT, "--dir", directory, "--no-fsModuleCache"], {
    cwd: CHECKOUT,
    env: runEnvironment(temp, extra),
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let output = "";
  child.stdout.on("data", (chunk: Buffer) => { output += chunk.toString(); });
  child.stderr.on("data", (chunk: Buffer) => { output += chunk.toString(); });
  const ended = new Promise<{ code: number | null; output: string }>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", code => resolve({ code, output }));
  });
  const run: Run = { child, ended };
  runs.push(run);
  return run;
}

/** Sends SIGKILL to `pid` (TerminateProcess on Windows); a process already gone is no error. */
function kill(pid: number | undefined): void {
  if (pid === undefined) return;
  try {
    process.kill(pid, "SIGKILL");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
}

/**
 * Ends `run` as a tool call's time limit or a closed terminal does, so no teardown runs: vitest's own process first,
 * then `worker`. The other order lets vitest see its worker end and wind the run down, teardown included, before its
 * own end comes, which `taskkill /T` on Windows did.
 */
function killRun(run: Run, worker: number): void {
  kill(run.child.pid);
  kill(worker);
}

/**
 * The name of the first folder named `prefix…` somewhere in `folder`, once there is one; fails with the run's output if
 * the run ends first.
 */
async function waitForFolder(folder: string, prefix: string, run: Run): Promise<string> {
  let over: { code: number | null; output: string } | undefined;
  void run.ended.then(result => { over = result; }, (error: unknown) => { over = { code: null, output: String(error) }; });
  for (;;) {
    // The run makes and removes folders while this reads; a folder gone mid-read only means another look.
    const names = await readdir(folder, { recursive: true }).catch(() => []);
    const found = names.map(name => basename(name)).find(name => name.startsWith(prefix));
    if (found !== undefined) return found;
    if (over !== undefined) throw new Error(`The run ended (${over.code}) before it made ${prefix}*:\n${over.output}`);
    await delay(50);
  }
}

/**
 * Makes a run folder at `path` with a folder inside and an owner file that names `place` and the folders of module
 * copies in `copies`, last touched `agoMs` ago, as a run there left it.
 */
async function ownedFolder(path: string, owner: { place: string; agoMs: number; copies?: string[] }): Promise<void> {
  await mkdir(join(path, "inside"), { recursive: true });
  const copies = (owner.copies ?? []).map(copy => `${MODULE_COPIES_LINE}${copy}\n`).join("");
  await writeFile(join(path, OWNER_FILE), owner.place + copies);
  await backdate(join(path, OWNER_FILE), owner.agoMs);
}

/** Sets the times of the file or folder at `path` to `agoMs` ago. */
async function backdate(path: string, agoMs: number): Promise<void> {
  const then = (Date.now() - agoMs) / 1000;
  await utimes(path, then, then);
}

/** Resolves once no process has `pid`; signal 0 only checks. */
async function processEnded(pid: number): Promise<void> {
  for (;;) {
    try {
      process.kill(pid, 0);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ESRCH") return;
      throw error;
    }
    await delay(50);
  }
}

/**
 * Lets go what the `stuck` and `locked` test files in `tests` left, once each: ends the process running in the stuck
 * folder, and gives the locked folder back its write permission, when a run has not removed it. Nothing to do for a
 * test file that has not run.
 */
async function releaseHeld(tests: string): Promise<void> {
  const pid = await takeNote(join(tests, "stuck", NOTE));
  if (pid !== undefined) {
    kill(Number(pid));
    await processEnded(Number(pid));
  }
  const locked = await takeNote(join(tests, "locked", NOTE));
  if (locked !== undefined) await chmod(locked, 0o700).catch(() => undefined);
}

/**
 * The text of the note at `path`, or undefined when there is none. The note goes as it is read, so a second call
 * cannot signal a pid that another process has since been given.
 */
async function takeNote(path: string): Promise<string | undefined> {
  const text = await readFile(path, "utf8").catch(() => undefined);
  if (text !== undefined) await rm(path);
  return text;
}

describe("a test run's temporary folders", () => {
  it("are all gone after the run, with the folders of ended runs, and a running run's folder stays", async () => {
    const { temp, tests } = await scratch();
    const endedRun = `rv-${NO_PROCESS}-a1b2c3`;
    const runningRun = `rv-${process.pid}-d4e5f6`;
    // Each test file's home and store from before the run folder, made straight in the temp folder.
    const olderRunsDayOld = "rerail-vitest-home-g7h8i9";
    const olderRunsRecent = "rerail-vitest-store-j1k2l3";
    // A running Rerail's own folder, of a family the sweep never removes.
    const notATestFolder = "rerail-clock-m4n5o6";
    // Folders of module copies that vitest named for the ended run and for the running one.
    const endedRunCopies = "AbCdEfGhIjKlMnOpQrStU";
    const runningRunCopies = "VwXyZ0123456789_-abcd";
    // The ended run's owner file also names a folder that is not one of vitest's, which no owner file can have removed.
    const endedCopies = [endedRunCopies, notATestFolder].map(name => join(temp, name));
    await ownedFolder(join(temp, endedRun), { place: HERE, agoMs: 0, copies: endedCopies });
    await ownedFolder(join(temp, runningRun), { place: HERE, agoMs: 0, copies: [join(temp, runningRunCopies)] });
    for (const name of [olderRunsDayOld, olderRunsRecent, notATestFolder, endedRunCopies, runningRunCopies]) {
      await mkdir(join(temp, name, "inside"), { recursive: true });
    }
    for (const name of [olderRunsDayOld, notATestFolder]) await backdate(join(temp, name), 2 * DAY_MS);

    const run = await startRun(join(tests, "leaves"), temp).ended;

    expect(run.code, run.output).toBe(0);
    const left = [notATestFolder, olderRunsRecent, runningRun, runningRunCopies];
    expect((await readdir(temp)).sort()).toEqual(left.sort());
  }, 120_000);

  it("of a run from another place stay until it stops touching its owner file, and any run's go a day after it stopped", async () => {
    const { temp, tests } = await scratch();
    // A run in a container or sandbox with a pid namespace of its own, or on another host, that shares this temp
    // folder: its pid names no process here, so only its owner file tells whether it still runs.
    const elsewhereRunning = `rv-${NO_PROCESS}-p1q2r3`;
    const elsewhereQuiet = `rv-${NO_PROCESS}-s4t5u6`;
    await ownedFolder(join(temp, elsewhereRunning), { place: "host elsewhere\n", agoMs: 9 * MINUTE_MS });
    await ownedFolder(join(temp, elsewhereQuiet), { place: "host elsewhere\n", agoMs: 11 * MINUTE_MS });
    // A run from here that stopped a day ago, whose pid a running process has since been given.
    const pidTakenSince = `rv-${process.pid}-v7w8x9`;
    await ownedFolder(join(temp, pidTakenSince), { place: HERE, agoMs: 2 * DAY_MS });
    // A run between making its folder and writing its owner file, or killed in between: its age decides.
    const noOwnerYet = `rv-${NO_PROCESS}-y1z2a3`;
    const noOwnerDayOld = `rv-${NO_PROCESS}-b4c5d6`;
    for (const name of [noOwnerYet, noOwnerDayOld]) await mkdir(join(temp, name, "inside"), { recursive: true });
    await backdate(join(temp, noOwnerDayOld), 2 * DAY_MS);

    const run = await startRun(join(tests, "leaves"), temp).ended;

    expect(run.code, run.output).toBe(0);
    expect((await readdir(temp)).sort()).toEqual([elsewhereRunning, noOwnerYet].sort());
  }, 120_000);

  it("left by a run killed before its teardown are gone after the next run", async () => {
    const { temp, tests } = await scratch();
    const killed = startRun(join(tests, "holds"), temp);
    const held = await waitForFolder(temp, "held-", killed);
    killRun(killed, Number(held.split("-")[1]));
    await killed.ended;
    // The killed run's folder, and the folder vitest copied its modules into.
    const afterKill = await readdir(temp);
    expect(afterKill.filter(name => RUN_FOLDER.test(name))).toHaveLength(1);
    expect(afterKill.filter(name => MODULE_COPIES.test(name))).toHaveLength(1);

    const next = await startRun(join(tests, "leaves"), temp).ended;

    expect(next.code, next.output).toBe(0);
    expect(await readdir(temp)).toEqual([]);
  }, 120_000);

  // Only Windows refuses this user the removal of a folder it made, while a process runs in it: elsewhere a folder stays
  // only when it holds another user's file, which takes an administrator to make.
  it.skipIf(process.platform !== "win32")("that a run cannot remove stay, the run says so and does not fail, and a later run removes them", async () => {
    const { temp, tests } = await scratch();
    const stuck = await startRun(join(tests, "stuck"), temp).ended;
    const afterStuck = await readdir(temp);
    // The removal that left the folder kept its owner file, which says whose the folder is. Backdated a day, it ends
    // the stuck run for the runs below whatever process has its pid by then.
    const leftInside = await readdir(join(temp, afterStuck[0] ?? ""));
    expect(leftInside).toContain(OWNER_FILE);
    await backdate(join(temp, afterStuck[0] ?? "", OWNER_FILE), 2 * DAY_MS);
    // The next run finds the stuck run ended but cannot remove its folder either.
    const blocked = await startRun(join(tests, "leaves"), temp).ended;
    const afterBlocked = await readdir(temp);
    await releaseHeld(tests);

    const next = await startRun(join(tests, "leaves"), temp).ended;

    expect(stuck.code, stuck.output).toBe(0);
    expect(stuck.output).toContain("The test run could not remove its temporary folder; the next run tries again");
    expect(afterStuck).toHaveLength(1);
    expect(blocked.code, blocked.output).toBe(0);
    expect(blocked.output).toContain("The test run could not remove the folders of ended test runs; the next run tries again");
    expect(afterBlocked).toEqual(afterStuck);
    expect(next.code, next.output).toBe(0);
    expect(await readdir(temp)).toEqual([]);
  }, 180_000);

  it("that a test left read-only are gone after the run", async () => {
    const { temp, tests } = await scratch();

    const run = await startRun(join(tests, "locked"), temp).ended;

    expect(run.code, run.output).toBe(0);
    expect(run.output).not.toContain("could not remove");
    expect(await readdir(temp)).toEqual([]);
  }, 120_000);

  it.skipIf(process.platform === "win32")("stay on the temp folder's filesystem when the deepest socket a test binds fits below a run folder there", async () => {
    const { tests } = await scratch();
    const temp = await tempOfLength(ROOMY_TEMP_BYTES);

    const run = await startRun(join(tests, "socket"), temp).ended;

    expect(run.code, run.output).toBe(0);
    // The socket test file's temp folder was a run folder in `temp`.
    const used = await readFile(join(tests, "socket", NOTE), "utf8");
    expect([dirname(used), basename(used)]).toEqual([temp, expect.stringMatching(RUN_FOLDER)]);
    expect(await readdir(temp)).toEqual([]);
  }, 120_000);

  it.skipIf(process.platform === "win32")("go in /tmp when the deepest socket a test binds would not fit below a run folder in the temp folder", async () => {
    const { tests } = await scratch();
    const temp = await tempOfLength(CRAMPED_TEMP_BYTES);
    const run = startRun(join(tests, "socket"), temp);

    const result = await run.ended;

    expect(result.code, result.output).toBe(0);
    const used = await readFile(join(tests, "socket", NOTE), "utf8");
    expect([dirname(used), basename(used)]).toEqual(["/tmp", expect.stringMatching(RUN_FOLDER)]);
    expect(await readdir(temp)).toEqual([]);
    expect((await readdir("/tmp")).filter(name => name.startsWith(`rv-${run.child.pid}-`))).toEqual([]);
  }, 120_000);

  // A read-only folder stops only a user other than root.
  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("are made as before there was a run folder when the short folder refuses one", async () => {
    const { tests } = await scratch();
    // Outside this run's folder, as the socket case's temp folder is, and too long for a socket 54 bytes below a run
    // folder in it. The short folder in its place is read-only, as /tmp is in some sandboxes.
    const root = await mkdtemp("/tmp/rerail-vitest-long-");
    scratches.push(root);
    const temp = join(root, "a-temp-folder-too-long-for-the-socket-tests");
    const readOnly = join(root, "read-only");
    await mkdir(temp);
    await mkdir(readOnly);
    await chmod(readOnly, 0o500);

    const run = await startRun(join(tests, "leaves"), temp, { RERAIL_TEST_SHORT_TEMP: readOnly }).ended;

    expect(run.code, run.output).toBe(0);
    expect(run.output).toContain(`The test run could not make its folder in ${readOnly}`);
    expect((await readdir(temp)).map(name => name.slice(0, "left-".length))).toEqual(["left-"]);
    expect(await readdir(readOnly)).toEqual([]);
  }, 120_000);
});
