import * as childProcess from "node:child_process";
import { EventEmitter, once } from "node:events";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, win32 } from "node:path";
import { PassThrough } from "node:stream";
import { promisify } from "node:util";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { assertProgramPath, endProcessTree, execFile, execFileSync, spawn, spawnShown, spawnSync, withHiddenWindow } from "../../src/engine/child-process.js";
import { processState } from "../../src/engine/process-liveness.js";
import { gitExecutable, GitReads, runGit } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

/** Whether a recorded child_process call passed `windowsHide: true` in its options. */
const hidesWindow = (call: readonly unknown[]): boolean =>
  call.some(argument => typeof argument === "object" && argument !== null && (argument as { windowsHide?: unknown }).windowsHide === true);

const roots: string[] = [];
afterEach(async () => {
  vi.clearAllMocks();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
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

  it("keeps the options after an argument list given as undefined or null, where Node reads them", () => {
    const callback = (): void => undefined;
    expect(withHiddenWindow(["git", undefined, { cwd: "/r" }])).toEqual(["git", undefined, { cwd: "/r", windowsHide: true }]);
    expect(withHiddenWindow(["git", null, { cwd: "/r" }, callback])).toEqual(["git", null, { cwd: "/r", windowsHide: true }, callback]);
    expect(withHiddenWindow(["git", undefined, callback])).toEqual(["git", undefined, { windowsHide: true }, callback]);
    // Read where Node reads it: a shell line after an undefined argument list is still a shell line.
    expect(() => assertProgramPath(["bash ~/.claude/statusline.sh", undefined, { shell: true }], "win32")).not.toThrow();
  });

  it("starts a child with the caller's options after an undefined argument list", () => {
    const result = spawnSync(process.execPath, undefined, { encoding: "utf8", input: "process.stdout.write(process.cwd())", cwd: tmpdir() });
    expect(result.stdout).toBe(tmpdir());
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

  it.runIf(process.platform === "win32")("refuses on Windows, through every starter, a program named without its folder, and starts nothing", async () => {
    // A name no folder holds, so a starter that let it through would start nothing either, and open no window.
    const bare = "rerail-test-no-such-program";
    const starters = [
      () => spawn(bare, []), () => spawnSync(bare, []), () => execFile(bare, []), () => execFileSync(bare, []),
      () => spawnShown(bare, [], { stdio: "ignore" }),
    ];
    for (const start of starters) expect(start).toThrow(/absolute path/u);
    await expect(promisify(execFile)(bare, [])).rejects.toThrow(/absolute path/u);
    for (const start of [childProcess.spawn, childProcess.spawnSync, childProcess.execFile, childProcess.execFileSync]) {
      expect(vi.mocked(start)).not.toHaveBeenCalled();
    }
  });
});

describe("endProcessTree", () => {
  it("on Windows ends the tree from the child while it runs, and kills the child alone only when that fails", () => {
    const events: string[] = [];
    const child = { pid: 4242, kill: () => events.push("kill child") > 0 };
    let failed: (() => void) | undefined;
    endProcessTree(child, {
      platform: "win32",
      killTree: (pid, onFailure) => {
        events.push(`taskkill /T /F /PID ${pid}`);
        failed = onFailure;
      },
    });
    // taskkill /T walks the tree from the pid it is given; once the child is gone it finds nothing to end.
    expect(events).toEqual(["taskkill /T /F /PID 4242"]);
    failed!();
    expect(events).toEqual(["taskkill /T /F /PID 4242", "kill child"]);
  });

  it("on Windows kills the child alone, and throws nothing, when taskkill cannot start", () => {
    // A spawn that throws, and one that answers with no child, as a test's stand-in for spawn can.
    const cannotStart = [() => { throw new Error("spawn EPERM"); }, () => undefined];
    for (const start of cannotStart) {
      // The stand-ins break spawn's promise of a child on purpose. Windows pids are multiples of 4, so 4242 names no process.
      vi.mocked(childProcess.spawn).mockImplementationOnce(start as unknown as typeof childProcess.spawn);
      const kills: string[] = [];
      expect(() => endProcessTree({ pid: 4242, kill: () => kills.push("kill child") > 0 }, { platform: "win32" })).not.toThrow();
      expect(kills).toEqual(["kill child"]);
    }
    expect(vi.mocked(childProcess.spawn).mock.calls.map(([program, args]) => [win32.basename(program), args]))
      .toEqual(Array.from(cannotStart, () => ["taskkill.exe", ["/T", "/F", "/PID", "4242"]]));
  });

  it("ends a child and the process it started, which holds the child's output, so that output closes", async () => {
    // The grandchild writes its pid to the output it shares with the child, then both wait; on macOS and Linux the
    // child leads a process group of its own (`detached`), which goes whole.
    const grandchild = "process.stdout.write(String(process.pid) + '\\n'); setInterval(() => {}, 1000)";
    const script = `require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(grandchild)}], { stdio: ['ignore', 'inherit', 'ignore'], windowsHide: true }); setInterval(() => {}, 1000)`;
    const child = spawn(process.execPath, ["-e", script], { stdio: ["ignore", "pipe", "ignore"], detached: process.platform !== "win32" });
    const closed = once(child, "close");
    const [line] = await once(child.stdout.setEncoding("utf8"), "data") as [string];
    const pid = Number.parseInt(line, 10);
    endProcessTree(child, { group: true });
    await Promise.race([closed, new Promise((_resolve, reject) => setTimeout(() => reject(new Error("the output stayed open")), 5_000))]);
    const deadline = Date.now() + 5_000;
    while (processState(pid) !== "gone" && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 50));
    const survived = processState(pid) !== "gone";
    if (survived) process.kill(pid);
    expect(survived).toBe(false);
  }, 15_000);
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

  it("in runGit and in GitReads' head, blobIds and blobs", async () => {
    const root = await repository();
    expect(runGit(root, ["status", "--porcelain"]).status).toBe(0);
    expect(await new GitReads().head(root)).toMatch(/^[0-9a-f]{40}/u);
    expect((await new GitReads().blobIds(root, ["HEAD:a.txt"])).size).toBe(1);
    expect((await new GitReads().blobs(root, ["HEAD:a.txt"], { maxBlobBytes: 1024 })).get("HEAD:a.txt")?.toString()).toBe("a\n");
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
    expect(await new GitReads().blobIds(root, ["HEAD:a.txt"])).toEqual(new Map([["HEAD:a.txt", "0123456789abcdef0123456789abcdef01234567"]]));
  }, 10_000);

  it("and a read whose launcher exits answers soon after, without waiting for the Git it left running", async () => {
    const root = await repository();
    vi.mocked(childProcess.spawn).mockImplementationOnce(launcherWhoseGitOutlivesIt).mockImplementationOnce(launcherWhoseGitOutlivesIt);
    const started = performance.now();
    expect((await new GitReads().blobs(root, ["HEAD:a.txt"], { maxBlobBytes: 1024 })).size).toBe(0);
    expect((await new GitReads().blobIds(root, ["HEAD:a.txt"])).size).toBe(0);
    expect(performance.now() - started).toBeLessThan(3_000);
  }, 10_000);
});
