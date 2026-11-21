import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MAX_HOOK_STATUS, runHookCommand, stopCommand, type HookAnswer, type HookPayload } from "../../../src/runtime/merge-assist/hook-command.js";
import { sampleConflict } from "../../support/merge-assist/status.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { fromDriveRoot } from "../../support/platform/windows-paths.js";

const payload: HookPayload = {
  version: 1, event: "detected", conflict: sampleConflict(), checkout: { path: tmpdir(), branch: "mine", base: "main" }, session: null,
};

/** The person's command answering `answer` on stdout. */
async function answering(answer: unknown): Promise<HookAnswer> {
  const script = `process.stdout.write(${JSON.stringify(JSON.stringify(answer))})`;
  return await runHookCommand({ command: [process.execPath, "-e", script], timeoutSeconds: 10 }, payload, { cwd: tmpdir(), env: process.env });
}

const drives: string[] = [];
const driveRoots: string[] = [];
afterEach(async () => {
  // A drive goes before its folder: Windows keeps a folder a drive maps.
  for (const drive of drives.splice(0)) execFileSync("subst", [drive, "/d"], { stdio: "ignore" });
  await Promise.all(driveRoots.splice(0).map(async root => await removeFolder(root)));
});

/**
 * A free drive mapped to a folder of this test, holding `folder` (if given) at the path it has on its own drive,
 * through a junction. Vitest runs test files at once and every file shares the user's drive letters, so this file takes
 * only N to P (the PATH search's tests take R to T, the start state's U to W, the status line's X to Z, and the
 * registry's tests count on Q being free), and the next one where `subst` refuses a letter.
 */
async function driveHolding(folder?: string): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-hook-drive-"));
  driveRoots.push(root);
  if (folder !== undefined) {
    const link = join(root, fromDriveRoot(folder));
    await mkdir(dirname(link), { recursive: true });
    await symlink(folder, link, "junction");
  }
  for (const name of "NOP") {
    const drive = `${name}:`;
    if (existsSync(`${drive}\\`)) continue;
    try {
      execFileSync("subst", [drive, root], { stdio: "ignore" });
      drives.push(drive);
      return drive;
    } catch {
      // Taken since the check above: the next one.
    }
  }
  throw new Error("no free drive letter from N to P");
}

describe("the conflict hook command's answer", () => {
  it("reads a null note or status as none, as most serializers write an absent field", async () => {
    expect(await answering({ note: null, status: "Sam is on it" })).toEqual({ ok: true, error: null, note: null, status: "Sam is on it" });
    expect(await answering({ note: "Ask Jordan first.", status: null })).toEqual({ ok: true, error: null, note: "Ask Jordan first.", status: null });
    expect(await answering({ note: 1 })).toMatchObject({ ok: false });
  });

  it("cuts a long status between characters, never inside one", async () => {
    const status = `${"a".repeat(MAX_HOOK_STATUS - 2)}🔥🔥🔥`;
    const answer = await answering({ status });
    expect([...answer.status!]).toHaveLength(MAX_HOOK_STATUS);
    expect(answer.status).toBe(`${"a".repeat(MAX_HOOK_STATUS - 2)}🔥…`);
  });
});

describe("starting the conflict hook command", () => {
  const run = (program: string) => runHookCommand({ command: [program], timeoutSeconds: 10 }, payload, { cwd: tmpdir(), env: process.env });

  it("records a program PATH does not find, or one Node refuses to start, as the command's failure", async () => {
    expect(await run("rerail-no-such-hook-program")).toEqual({
      ok: false, error: "could not start rerail-no-such-hook-program: no such program on PATH", note: null, status: null,
    });
    // A path Node refuses before starting anything throws rather than emitting `error`.
    expect(await run("./bad\u0000name")).toMatchObject({ ok: false, error: expect.stringMatching(/^could not start \.\/bad\u0000name: /u) });
  });

  it("starts a program by a path from the checkout's root", async () => {
    const answer = await runHookCommand({ command: [`./${basename(process.execPath)}`, "-e", "process.stdout.write('{}')"], timeoutSeconds: 10 }, payload, { cwd: dirname(process.execPath), env: process.env });
    expect(answer).toEqual({ ok: true, error: null, note: null, status: null });
  });

  it.runIf(process.platform === "win32")("starts a program named from the root of the checkout's drive by its full path on that drive", async () => {
    // Node itself, named without its drive (`\Program Files\nodejs\node.exe`), answering with the name it was started by.
    // Each checkout sits on a drive of its own, not this process's: the answer shows which drive the name was read on.
    const program = fromDriveRoot(process.execPath);
    const command = [program, "-e", "process.stdout.write(JSON.stringify({ note: process.argv0 }))"];
    const from = (checkout: string) => runHookCommand({ command, timeoutSeconds: 10 }, payload, { cwd: checkout, env: process.env });
    const holding = await driveHolding(dirname(process.execPath));
    expect(await from(`${holding}\\`)).toEqual({ ok: true, error: null, note: `${holding}${program}`, status: null });
    const empty = await driveHolding();
    expect(await from(`${empty}\\`)).toMatchObject({ ok: false, error: expect.stringContaining(`spawn ${empty}${program} ENOENT`) });
  });
});

describe("stopping the conflict hook command", () => {
  /** A command as pid 4242, running unless `exitCode` says it exited, recording the signals it gets itself. */
  const command = (exitCode: number | null = null) => {
    const signals: string[] = [];
    const kill = (signal?: NodeJS.Signals | number): boolean => {
      signals.push(String(signal));
      return true;
    };
    return { signals, child: { pid: 4242, exitCode, signalCode: null, kill } };
  };

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("on Windows ends the command's process tree at once, both asking and forcing, and leaves an exited one alone", () => {
    const killTree = vi.fn();
    const { child, signals } = command();
    stopCommand(child, "SIGTERM", { platform: "win32", killTree });
    stopCommand(child, "SIGKILL", { platform: "win32", killTree });
    expect(killTree.mock.calls.map(([pid]) => pid)).toEqual([4242, 4242]);
    expect(signals).toEqual([]);
    const exited = command(0);
    stopCommand(exited.child, "SIGKILL", { platform: "win32", killTree });
    expect(killTree).toHaveBeenCalledTimes(2);
  });

  it("on macOS and Linux signals the command's process group, and the command alone when it has none", () => {
    const kill = vi.spyOn(process, "kill").mockReturnValue(true);
    const { child, signals } = command();
    stopCommand(child, "SIGTERM", { platform: "linux" });
    stopCommand(child, "SIGKILL", { platform: "linux" });
    expect(kill.mock.calls).toEqual([[-4242, "SIGTERM"], [-4242, "SIGKILL"]]);
    expect(signals).toEqual([]);
    kill.mockImplementation(() => {
      throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
    });
    stopCommand(child, "SIGTERM", { platform: "linux" });
    stopCommand(child, "SIGKILL", { platform: "linux" });
    expect(signals).toEqual(["SIGTERM", "SIGKILL"]);
  });
});
