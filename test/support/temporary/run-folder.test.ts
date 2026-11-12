import { spawn, spawnSync, type ChildProcessByStdio } from "node:child_process";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import type { Readable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

/** This checkout, whose vitest.config.ts every run below uses. */
const CHECKOUT = fileURLToPath(new URL("../../../", import.meta.url));
const VITEST = join(CHECKOUT, "node_modules", "vitest", "vitest.mjs");
const DAY_MS = 24 * 60 * 60 * 1000;
/** What the `stuck` test file writes next to itself: the pid of the process in its folder, or the folder it locked. */
const STUCK_NOTE = "stuck.txt";
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
  // Leaves a folder no removal takes until releaseStuck lets it go, and writes next to itself what that takes: Windows
  // removes no folder a process runs in, and elsewhere no file goes from a folder its owner may not write to. The
  // process takes the folder as it starts, after spawn returns, so the test waits for the file it writes there.
  stuck: `${IMPORTS}
import { spawn } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
it("leaves a folder its run cannot remove", async () => {
  const stuck = mkdtempSync(join(tmpdir(), "stuck-"));
  const note = new URL("${STUCK_NOTE}", import.meta.url);
  if (process.platform === "win32") {
    const holder = spawn(process.execPath, ["-e", "require('node:fs').writeFileSync('running', ''); setTimeout(() => {}, 600000)"], { cwd: stuck, detached: true, stdio: "ignore", windowsHide: true });
    holder.unref();
    writeFileSync(note, String(holder.pid));
    while (!existsSync(join(stuck, "running"))) await new Promise(resolve => setTimeout(resolve, 20));
  } else {
    mkdirSync(join(stuck, "locked"));
    writeFileSync(join(stuck, "locked", "file"), "");
    chmodSync(join(stuck, "locked"), 0o500);
    writeFileSync(note, join(stuck, "locked"));
  }
}, 60_000);
`,
  // Listens on a Unix socket as far below the temp folder as engine-service-socket.test.ts's, the deepest a test binds.
  socket: `${IMPORTS}
import { mkdirSync } from "node:fs";
import { createServer } from "node:net";
it("listens on a socket 54 bytes below the temp folder", async () => {
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
  for (const root of scratches) await releaseStuck(join(root, "tests"));
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

/** This process's environment with every temp folder variable naming `temp`, and none of this vitest worker's own. */
function runEnvironment(temp: string): NodeJS.ProcessEnv {
  const environment = Object.fromEntries(Object.entries(process.env).filter(([name]) => !/^(?:TMPDIR|TEMP|TMP|VITEST.*)$/iu.test(name)));
  return { ...environment, TMPDIR: temp, TEMP: temp, TMP: temp };
}

/** `vitest run` on this checkout's config over the test files in `directory`, with `temp` as its temp folder. */
function startRun(directory: string, temp: string): Run {
  const child = spawn(process.execPath, [VITEST, "run", "--root", CHECKOUT, "--dir", directory], {
    cwd: CHECKOUT,
    env: runEnvironment(temp),
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

/** The pid of a process that has ended. */
function endedPid(): number {
  return spawnSync(process.execPath, ["-e", ""]).pid;
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
 * Lets go the folder the `stuck` test file in `tests` left, once: ends the process running in it on Windows, and
 * elsewhere makes the folder it locked writable again. Nothing to do when that test file has not run.
 */
async function releaseStuck(tests: string): Promise<void> {
  const note = join(tests, "stuck", STUCK_NOTE);
  const text = await readFile(note, "utf8").catch(() => undefined);
  if (text === undefined) return;
  // Gone before the release, so a second call cannot signal a pid that another process has since been given.
  await rm(note);
  if (process.platform === "win32") {
    kill(Number(text));
    await processEnded(Number(text));
  } else {
    await chmod(text, 0o700);
  }
}

describe("a test run's temporary folders", () => {
  it("are all gone after the run, with the folders of ended runs, and a running run's folder stays", async () => {
    const { temp, tests } = await scratch();
    const endedRun = `rerail-vitest-${endedPid()}-a1b2c3`;
    const runningRun = `rerail-vitest-${process.pid}-d4e5f6`;
    // Each test file's home and store from before the run folder, made straight in the temp folder.
    const olderRunsDayOld = "rerail-vitest-home-g7h8i9";
    const olderRunsRecent = "rerail-vitest-store-j1k2l3";
    // A running Rerail's own folder, of a family the sweep never removes.
    const notATestFolder = "rerail-clock-m4n5o6";
    for (const name of [endedRun, runningRun, olderRunsDayOld, olderRunsRecent, notATestFolder]) {
      await mkdir(join(temp, name, "inside"), { recursive: true });
    }
    const twoDaysAgo = (Date.now() - 2 * DAY_MS) / 1000;
    for (const name of [olderRunsDayOld, notATestFolder]) await utimes(join(temp, name), twoDaysAgo, twoDaysAgo);

    const run = await startRun(join(tests, "leaves"), temp).ended;

    expect(run.code, run.output).toBe(0);
    expect((await readdir(temp)).sort()).toEqual([notATestFolder, olderRunsRecent, runningRun].sort());
  }, 120_000);

  it("left by a run killed before its teardown are gone after the next run", async () => {
    const { temp, tests } = await scratch();
    const killed = startRun(join(tests, "holds"), temp);
    const held = await waitForFolder(temp, "held-", killed);
    killRun(killed, Number(held.split("-")[1]));
    await killed.ended;
    expect(await readdir(temp)).not.toEqual([]);

    const next = await startRun(join(tests, "leaves"), temp).ended;

    expect(next.code, next.output).toBe(0);
    expect(await readdir(temp)).toEqual([]);
  }, 120_000);

  it("that a run cannot remove stay, the run says so and does not fail, and a later run removes them", async () => {
    const { temp, tests } = await scratch();
    const stuck = await startRun(join(tests, "stuck"), temp).ended;
    const afterStuck = await readdir(temp);
    // The next run finds the stuck run ended but cannot remove its folder either.
    const blocked = await startRun(join(tests, "leaves"), temp).ended;
    const afterBlocked = await readdir(temp);
    await releaseStuck(tests);

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

  it.skipIf(process.platform === "win32")("go in /tmp when the deepest socket a test binds would not fit below a run folder in the temp folder", async () => {
    const { tests } = await scratch();
    // Straight in /tmp, the temp folder is 32 bytes, as a TMPDIR set by hand may be: a socket 54 bytes below it fits,
    // and one below a run folder there too does not. Named rerail-vitest-*, it goes with a later run a day after a kill
    // left it.
    const temp = await mkdtemp("/tmp/rerail-vitest-socket-");
    scratches.push(temp);
    const run = startRun(join(tests, "socket"), temp);

    const result = await run.ended;

    expect(result.code, result.output).toBe(0);
    expect(await readdir(temp)).toEqual([]);
    expect((await readdir("/tmp")).filter(name => name.startsWith(`rerail-vitest-${run.child.pid}-`))).toEqual([]);
  }, 120_000);
});
