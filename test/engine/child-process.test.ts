import * as childProcess from "node:child_process";
import { EventEmitter, once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { promisify } from "node:util";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { assertProgramPath, execFile, execFileSync, spawn, spawnShown, spawnSync, withHiddenWindow } from "../../src/engine/child-process.js";
import { gitExecutable, gitHead, readGitBlobIds, readGitBlobs, runGit } from "../../src/engine/git.js";

/** Whether a recorded child_process call passed `windowsHide: true` in its options. */
const hidesWindow = (call: readonly unknown[]): boolean =>
  call.some(argument => typeof argument === "object" && argument !== null && (argument as { windowsHide?: unknown }).windowsHide === true);

const roots: string[] = [];
afterEach(async () => {
  vi.clearAllMocks();
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

describe("withHiddenWindow", () => {
  it("sets windowsHide in the options after the argument array, after the command, or ahead of a callback", () => {
    const callback = (): void => undefined;
    expect(withHiddenWindow(["git", ["status"], { cwd: "/r" }])).toEqual(["git", ["status"], { cwd: "/r", windowsHide: true }]);
    expect(withHiddenWindow(["git", ["status"]])).toEqual(["git", ["status"], { windowsHide: true }]);
    expect(withHiddenWindow(["git", { shell: true }])).toEqual(["git", { shell: true, windowsHide: true }]);
    expect(withHiddenWindow(["git", ["status"], callback])).toEqual(["git", ["status"], { windowsHide: true }, callback]);
    expect(withHiddenWindow(["git", ["status"], undefined, callback])).toEqual(["git", ["status"], { windowsHide: true }, callback]);
    expect(withHiddenWindow(["git", ["status"], { windowsHide: false }])).toEqual(["git", ["status"], { windowsHide: true }]);
  });
});

describe("assertProgramPath", () => {
  it("refuses on Windows a program the current folder could supply, and allows an absolute path or a shell line", () => {
    for (const command of ["git", "node.exe", String.raw`.\node.exe`, String.raw`bin\git.exe`, "rundll32"]) {
      expect(() => assertProgramPath([command, []], "win32"), command).toThrow(/absolute path/u);
    }
    expect(() => assertProgramPath([String.raw`C:\Program Files\Git\cmd\git.exe`, ["status"]], "win32")).not.toThrow();
    expect(() => assertProgramPath([String.raw`\\server\share\tool.exe`], "win32")).not.toThrow();
    expect(() => assertProgramPath(["bash ~/.claude/statusline.sh", { shell: true }], "win32")).not.toThrow();
    // macOS and Linux look a bare name up on PATH alone.
    expect(() => assertProgramPath(["xdg-open", ["https://example.test"]], "linux")).not.toThrow();
  });
});

describe("the child-process helper", () => {
  it("starts every child with windowsHide, and promisified execFile still answers { stdout, stderr }", async () => {
    spawnSync(process.execPath, ["-e", ""]);
    execFileSync(process.execPath, ["-e", ""]);
    await once(spawn(process.execPath, ["-e", ""], { stdio: "ignore" }), "exit");
    const { stdout, stderr } = await promisify(execFile)(process.execPath, ["-e", "process.stdout.write('out'); process.stderr.write('err')"]);
    expect({ stdout, stderr }).toEqual({ stdout: "out", stderr: "err" });
    await expect(promisify(execFile)(process.execPath, ["-e", "process.stdout.write('half'); process.exit(3)"]))
      .rejects.toMatchObject({ code: 3, stdout: "half" });
    for (const start of [childProcess.spawnSync, childProcess.execFileSync, childProcess.spawn, childProcess.execFile]) {
      const calls = vi.mocked(start).mock.calls as unknown as unknown[][];
      expect(calls.length).toBeGreaterThan(0);
      expect(calls.every(hidesWindow)).toBe(true);
    }
  });
});

describe("spawnShown", () => {
  it("starts the browser opener with its window shown, whatever the caller passed", async () => {
    const child = spawnShown(process.execPath, ["-e", ""], { stdio: "ignore", windowsHide: true });
    await once(child, "exit");
    expect(vi.mocked(childProcess.spawn).mock.calls.at(-1)?.[2]).toMatchObject({ stdio: "ignore", windowsHide: false });
  });
});

describe("Git runs with its window hidden", () => {
  async function repository(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-hidden-git-"));
    roots.push(root);
    const git = (...args: string[]): void => {
      childProcess.execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd: root });
    };
    git("init", "--quiet");
    await writeFile(join(root, "a.txt"), "a\n");
    git("add", ".");
    git("commit", "--quiet", "-m", "seed");
    vi.clearAllMocks();
    return root;
  }

  /** The recorded calls of `start` that ran Rerail's Git. */
  const gitCalls = (start: (...args: never[]) => unknown): unknown[][] =>
    (vi.mocked(start).mock.calls as unknown as unknown[][]).filter(call => call[0] === gitExecutable());

  it("in runGit, gitHead, readGitBlobIds and readGitBlobs", async () => {
    const root = await repository();
    expect(runGit(root, ["status", "--porcelain"]).status).toBe(0);
    expect(await gitHead(root)).toMatch(/^[0-9a-f]{40}/u);
    expect((await readGitBlobIds(root, ["HEAD:a.txt"])).size).toBe(1);
    expect((await readGitBlobs(root, ["HEAD:a.txt"], { maxBlobBytes: 1024 })).get("HEAD:a.txt")?.toString()).toBe("a\n");
    const calls = [childProcess.spawnSync, childProcess.execFile, childProcess.spawn].map(gitCalls);
    expect(calls.map(list => list.length)).toEqual([1, 1, 2]);
    expect(calls.flat().every(hidesWindow)).toBe(true);
  });

  /**
   * Git for Windows' `cmd\git.exe` starts the real Git as its own child. A timeout kills only the launcher, and the
   * real Git keeps the stdout pipe open, so `close` comes only when it exits: a child that exits and never closes.
   */
  function launcherWhoseGitOutlivesIt(): childProcess.ChildProcess {
    const child = Object.assign(new EventEmitter(), { stdin: new PassThrough(), stdout: new PassThrough(), stderr: null });
    setTimeout(() => child.emit("exit", null, "SIGTERM"), 10);
    return child as unknown as childProcess.ChildProcess;
  }

  /** A Git that exits by itself, with output still to read: `close` comes after the last of it. */
  function gitWhoseOutputTrailsItsExit(): childProcess.ChildProcess {
    const stdout = new PassThrough();
    const child = Object.assign(new EventEmitter(), { stdin: new PassThrough(), stdout, stderr: null });
    setTimeout(() => child.emit("exit", 0, null), 10);
    setTimeout(() => {
      stdout.once("end", () => setImmediate(() => child.emit("close", 0, null)));
      stdout.end("0123456789abcdef0123456789abcdef01234567 blob\n");
      stdout.resume();
    }, 800);
    return child as unknown as childProcess.ChildProcess;
  }

  it("and a read whose Git exits by itself waits for the rest of its output", async () => {
    const root = await repository();
    vi.mocked(childProcess.spawn).mockImplementationOnce(gitWhoseOutputTrailsItsExit);
    expect(await readGitBlobIds(root, ["HEAD:a.txt"])).toEqual(new Map([["HEAD:a.txt", "0123456789abcdef0123456789abcdef01234567"]]));
  }, 10_000);

  it("and a read whose launcher exits answers soon after, without waiting for the Git it left running", async () => {
    const root = await repository();
    vi.mocked(childProcess.spawn).mockImplementationOnce(launcherWhoseGitOutlivesIt).mockImplementationOnce(launcherWhoseGitOutlivesIt);
    const started = performance.now();
    expect((await readGitBlobs(root, ["HEAD:a.txt"], { maxBlobBytes: 1024 })).size).toBe(0);
    expect((await readGitBlobIds(root, ["HEAD:a.txt"])).size).toBe(0);
    expect(performance.now() - started).toBeLessThan(3_000);
  }, 10_000);
});
