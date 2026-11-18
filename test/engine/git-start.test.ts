import { execFileSync } from "node:child_process";
import { once } from "node:events";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../../src/engine/child-process.js", { spy: true });

import { spawn, spawnSync } from "../../src/engine/child-process.js";
import { gitCrlfPaths, gitHead, gitModifiedOrUntrackedPaths, readGitBlobIds, readGitBlobs, runGit, runGitAsync } from "../../src/engine/git.js";
import { runGitSync, startGit, startGitThroughNode, startGitWithoutCopying, type GitProcess, type GitRunLimits, type GitRunResult } from "../../src/engine/git-start.js";
import { loadNativeBinding } from "../../src/engine/native/native.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { daemonProcess, packageRoot, until, watchedEnvironment } from "../support/processes/daemon-home.js";

const LINUX = process.platform === "linux";
const folders: string[] = [];

afterEach(() => {
  startGitThroughNode();
  vi.mocked(spawn).mockClear();
  vi.mocked(spawnSync).mockClear();
});

afterAll(async () => {
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

async function folder(name: string): Promise<string> {
  const made = await mkdtemp(join(tmpdir(), `rerail-git-start-${name}-`));
  folders.push(made);
  return made;
}

/** A repository whose one commit holds a text file, a CRLF file by attribute, and a binary file; one file is changed after. */
async function repository(): Promise<string> {
  const root = await folder("repo");
  const git = (...args: string[]): string => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" });
  git("init", "--quiet");
  git("config", "user.email", "test@example.invalid");
  git("config", "user.name", "test");
  await writeFile(join(root, ".gitattributes"), "*.bat text eol=crlf\n");
  await writeFile(join(root, "a.py"), "print(1)\n");
  await writeFile(join(root, "run.bat"), "echo 1\n");
  await writeFile(join(root, "blob.bin"), Buffer.from([0, 1, 2, 255, 0, 10, 13]));
  git("add", ".");
  git("commit", "--quiet", "-m", "seed");
  await writeFile(join(root, "a.py"), "print(2)\n");
  await writeFile(join(root, "new.py"), "print(3)\n");
  return root;
}

/** Every answer a walk and a boundary ask of Git, read once. */
async function answers(root: string): Promise<unknown> {
  const revisions = ["HEAD:a.py", "HEAD:blob.bin", ":run.bat", "HEAD:missing.py"];
  return {
    head: await gitHead(root),
    blobs: Object.fromEntries([...(await readGitBlobs(root, revisions, { maxBlobBytes: 1 << 20 }))].map(([at, blob]) => [at, blob.toString("hex")])),
    ids: Object.fromEntries(await readGitBlobIds(root, revisions)),
    crlf: await gitCrlfPaths(root, ["run.bat", "a.py"]),
    changed: await gitModifiedOrUntrackedPaths(root),
    hashed: await runGitAsync(root, ["hash-object", "--stdin"], { input: "some text\n" }),
    failed: await runGitAsync(root, ["rev-parse", "--verify", "no-such-ref"]),
    ran: runGit(root, ["cat-file", "-p", "HEAD:blob.bin"]),
    ranAndFailed: runGit(root, ["rev-parse", "--verify", "no-such-ref"]),
    listed: runGit(root, ["ls-files", "-z"], { maxBuffer: 64 }),
  };
}

/** What `child` printed, once it has closed, with how it ended. */
async function ended(child: GitProcess): Promise<{ stdout: string; code: number | null; signal: NodeJS.Signals | null }> {
  let stdout = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
  const [code, signal] = (await once(child, "close")) as [number | null, NodeJS.Signals | null];
  return { stdout, code, signal };
}

/** The fields of a blocking run's error that `spawnSync` sets. */
function errorOf(run: { error?: Error }): unknown {
  const { message, code, errno, syscall, path, spawnargs } = (run.error ?? {}) as NodeJS.ErrnoException & { spawnargs?: string[] };
  return { message, code, errno, syscall, path, spawnargs };
}

/** How a blocking run ended, without what it printed. */
function ending(run: GitRunResult): unknown {
  return { status: run.status, signal: run.signal, error: errorOf(run) };
}

/** What a blocking run answered, its error as {@link errorOf} reads it. */
function answered(run: GitRunResult): unknown {
  return { ...run, error: errorOf(run) };
}

/**
 * A blocking run of `script` under `limits`, after the script has started `sleep 10` in the background, which holds
 * stdout and stderr for as long as it runs: what the run answered, and whether that sleep still ran when the run
 * returned. The sleep is ended either way.
 */
async function heldRun(script: string, limits: GitRunLimits): Promise<{ answer: unknown; held: boolean }> {
  const pidFile = join(await folder("held"), "holder.pid");
  const run = runGitSync("/bin/sh", ["-c", `sleep 10 & echo $! > "$1"; ${script}`, "sh", pidFile], process.env, limits);
  const holder = Number(readFileSync(pidFile, "utf8"));
  let held = true;
  try {
    process.kill(holder, "SIGKILL");
  } catch {
    held = false;
  }
  return { answer: answered(run), held };
}

/**
 * A blocking run, past its 300 ms time limit, of a script that takes the SIGTERM and then writes 1 MB to its stdout:
 * what the run answered, and how that write ended: 141 when it met a closed output (EPIPE), 124 when it filled the
 * buffers and waited until `timeout` ended it.
 */
async function writtenAfterTerm(): Promise<{ answer: unknown; write: string }> {
  const statusFile = join(await folder("write"), "status");
  const script = `sleep 10 & s=$!; trap 'timeout 5 head -c 1000000 /dev/zero; echo $? > "$1"; kill $s; exit 0' TERM; echo started; wait`;
  const run = runGitSync("/bin/sh", ["-c", script, "sh", statusFile], process.env, { timeout: 300 });
  return { answer: answered(run), write: readFileSync(statusFile, "utf8") };
}

/** The children of this process that have exited and are not reaped. */
function zombies(): number {
  return readdirSync("/proc").filter(name => /^\d+$/u.test(name)).filter(pid => {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const [state, parent] = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      return state === "Z" && Number(parent) === process.pid;
    } catch {
      return false;
    }
  }).length;
}

describe.runIf(LINUX)("Git started through the engine, on Linux", () => {
  it("answers every read a walk and a boundary make as Git started through Node answers it", async () => {
    const root = await repository();
    const throughNode = await answers(root);
    vi.mocked(spawn).mockClear();
    vi.mocked(spawnSync).mockClear();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await answers(root);
    expect(throughEngine).toEqual(throughNode);
    // Node started none of them.
    expect([vi.mocked(spawn).mock.calls.length, vi.mocked(spawnSync).mock.calls.length]).toEqual([0, 0]);
  }, 30_000);

  it("gives Git the signal state, descriptors, environment and folder Node gives it", async () => {
    const script = "grep -E '^Sig(Blk|Ign|Cgt)' /proc/self/status; ls /proc/self/fd | wc -l; env | sort; pwd; test -t 0 || echo not-a-terminal";
    const env = { PATH: process.env.PATH, LANG: "C", RERAIL_GIT_START: "a b=c" };
    const node = await ended(startGit("/bin/sh", ["-c", script], env, "ignore"));
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const engine = await ended(startGit("/bin/sh", ["-c", script], env, "ignore"));
    expect(engine).toEqual(node);
    expect(engine.stdout).toContain("RERAIL_GIT_START=a b=c");
  });

  it("ends a Git with the signal a read's time limit or stop sends, and kills nothing once it is reaped", async () => {
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const child = startGit("/bin/sh", ["-c", "read line"], process.env, "pipe");
    expect(child.kill("SIGKILL")).toBe(true);
    expect(await ended(child)).toEqual({ stdout: "", code: null, signal: "SIGKILL" });
    expect([child.exitCode, child.signalCode, child.kill("SIGTERM")]).toEqual([null, "SIGKILL", false]);
  });

  it("writes Git's input, closes it, and reads its output and its stderr to their ends", async () => {
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const child = startGit("/bin/sh", ["-c", "cat; echo warned >&2; exit 3"], process.env, "pipe");
    let stderr = "";
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
    child.stdin.end("x".repeat(300_000));
    const exited = once(child, "exit");
    const answer = await ended(child);
    expect([answer.stdout.length, answer.code, answer.signal, stderr]).toEqual([300_000, 3, null, "warned\n"]);
    expect(await exited).toEqual([3, null]);
  });

  it("leaves no child unreaped and no descriptor open after its reads", async () => {
    const root = await repository();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    await gitHead(root);
    const open = readdirSync("/proc/self/fd").length;
    for (let read = 0; read < 20; read += 1) await Promise.all([gitHead(root), readGitBlobs(root, ["HEAD:a.py"], { maxBlobBytes: 1 << 20 })]);
    expect([zombies(), readdirSync("/proc/self/fd").length]).toEqual([0, open]);
  }, 30_000);

  it("runs a blocking Git as spawnSync does: its output, its exit, and SIGTERM at its time limit or past its output limit", () => {
    const runs = (): unknown => {
      const late = runGitSync("/bin/sh", ["-c", "echo started; exec sleep 5"], process.env, { timeout: 200 });
      const long = runGitSync("/bin/sh", ["-c", "head -c 3000000 /dev/zero; exec sleep 5"], process.env, { maxBuffer: 1024 * 1024 });
      const plain = runGitSync("/bin/sh", ["-c", "read nothing || echo no-input; echo warned >&2; exit 5"], process.env);
      return { late: answered(late), long: ending(long), plain };
    };
    const throughNode = runs();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = runs();
    expect(throughEngine).toEqual(throughNode);
    expect(throughEngine).toMatchObject({ late: { signal: "SIGTERM", error: { code: "ETIMEDOUT" } }, long: { signal: "SIGTERM", error: { code: "ENOBUFS" } }, plain: { status: 5, stdout: "no-input\n" } });
  }, 30_000);

  it("ends a blocking Git at its time limit as spawnSync does while a process Git started still holds its output", async () => {
    const runs = async (): Promise<unknown> => ({
      running: await heldRun("echo started; exec sleep 10", { timeout: 300 }),
      exited: await heldRun("echo started; exit 0", { timeout: 300 }),
      closed: answered(runGitSync("/bin/sh", ["-c", "echo started; exec > /dev/null 2>&1; exec sleep 10"], process.env, { timeout: 300 })),
      printing: ending(runGitSync("/bin/sh", ["-c", "while :; do printf x; done"], process.env, { timeout: 300, maxBuffer: 8 << 20 })),
      writing: await writtenAfterTerm(),
    });
    const throughNode = await runs();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await runs();
    expect(throughEngine).toEqual(throughNode);
    const timedOut = { stdout: "started\n", error: { code: "ETIMEDOUT" } };
    expect(throughEngine).toMatchObject({
      running: { held: true, answer: { ...timedOut, status: null, signal: "SIGTERM" } },
      exited: { held: true, answer: { ...timedOut, status: 0, signal: null } },
      closed: { ...timedOut, status: null, signal: "SIGTERM" },
      printing: { status: null, signal: "SIGTERM", error: { code: "ETIMEDOUT" } },
      writing: { answer: { ...timedOut, status: 0, signal: null }, write: "141\n" },
    });
  }, 120_000);

  it("ends a blocking Git past its output limit as spawnSync does while a process Git started still holds its output", async () => {
    const run = async (): Promise<unknown> => await heldRun("head -c 1001 /dev/zero; exec sleep 10", { maxBuffer: 1000 });
    const throughNode = await run();
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    const throughEngine = await run();
    expect(throughEngine).toEqual(throughNode);
    expect(throughEngine).toMatchObject({ held: true, answer: { stdout: "\0".repeat(1001), status: null, signal: "SIGTERM", error: { code: "ENOBUFS" } } });
  }, 60_000);

  it("starts through Node a start the engine fails and a program named without a folder", async () => {
    const root = await repository();
    const refuse = (): never => { throw new Error("posix_spawn: Resource temporarily unavailable"); };
    const refusing = { startGit: refuse, reapGit: () => ({}), runGit: refuse };
    expect(startGitWithoutCopying(() => refusing)).toBe(true);
    expect(await gitHead(root)).toBe(execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim());
    expect(spawn).toHaveBeenCalledTimes(1);
    expect(startGitWithoutCopying(loadNativeBinding)).toBe(true);
    expect((await ended(startGit("sh", ["-c", "echo bare"], process.env, "ignore"))).stdout).toBe("bare\n");
    expect(spawn).toHaveBeenCalledTimes(2);
  }, 30_000);
});

describe("Git started through Node, off Linux or without the engine", () => {
  it("is what a process gets off Linux, and in a process whose engine cannot be loaded", () => {
    expect(startGitWithoutCopying(loadNativeBinding, "win32")).toBe(false);
    expect(startGitWithoutCopying(loadNativeBinding, "darwin")).toBe(false);
    expect(startGitWithoutCopying(() => { throw new Error("Unsupported platform or missing native addon"); }, "linux")).toBe(false);
    expect(startGitWithoutCopying(() => ({}), "linux")).toBe(false);
  });
});

/**
 * Has the daemon, started from its built entry as `rerail daemon start` starts it, grow by 640 MB and read HEAD 20
 * times through its own Git module and 20 times through Node's spawn, and say what each cost it.
 */
async function daemonReads(root: string): Promise<{ head: string | undefined; nodeStarts: number; moduleSystemMs: number; nodeSystemMs: number }> {
  const home = await folder("home");
  const out = join(home, "reads.json");
  const preload = join(home, "reads.cjs");
  const gitModule = pathToFileURL(join(packageRoot, "dist", "engine", "git.js")).href;
  await writeFile(preload, `const cp = require("node:child_process");
const fs = require("node:fs");
let nodeStarts = 0;
const spawnOf = cp.ChildProcess.prototype.spawn;
cp.ChildProcess.prototype.spawn = function (options) { if (/git/u.test(String(options.file))) nodeStarts += 1; return spawnOf.call(this, options); };
const held = [];
setTimeout(async () => {
  for (let i = 0; i < 10; i += 1) held.push(Buffer.alloc(64 << 20, i + 1));
  const { gitHead, gitExecutable } = await import(${JSON.stringify(gitModule)});
  const system = async read => { const before = process.cpuUsage().system; for (let i = 0; i < 20; i += 1) await read(); return (process.cpuUsage().system - before) / 20000; };
  nodeStarts = 0;
  let head;
  const moduleSystemMs = await system(async () => { head = await gitHead(${JSON.stringify(root)}); });
  const starts = nodeStarts;
  const nodeSystemMs = await system(async () => { const c = cp.spawn(gitExecutable(), ["-C", ${JSON.stringify(root)}, "rev-parse", "HEAD"], { stdio: ["pipe", "pipe", "ignore"] }); c.stdout.resume(); c.stdin.end(); await require("node:events").once(c, "close"); });
  fs.writeFileSync(${JSON.stringify(out)}, JSON.stringify({ head, nodeStarts: starts, moduleSystemMs, nodeSystemMs }));
}, 500);
`);
  const env = watchedEnvironment(home);
  env.NODE_OPTIONS = `${env.NODE_OPTIONS ?? ""} --require ${JSON.stringify(preload)}`;
  const daemon = daemonProcess(home, env);
  try {
    await until(() => existsSync(out), 60_000, "the daemon's reads");
    return JSON.parse(await readFile(out, "utf8")) as Awaited<ReturnType<typeof daemonReads>>;
  } finally {
    daemon.kill("SIGKILL");
    await once(daemon, "close");
  }
}

describe.runIf(LINUX)("the daemon's process entry", () => {
  it("starts the daemon's Git without copying the daemon: no fork through Node, and a third of a fork's system time or less", async () => {
    const root = await repository();
    const reads = await daemonReads(root);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    // The figures go in the message, so a failure says what each read cost.
    const seen = { head: reads.head, nodeStarts: reads.nodeStarts, underAThird: reads.moduleSystemMs < reads.nodeSystemMs / 3 };
    expect(seen, JSON.stringify(reads)).toEqual({ head, nodeStarts: 0, underAThird: true });
  }, 90_000);
});
