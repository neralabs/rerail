import { tmpdir } from "node:os";
import { basename, dirname } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MAX_HOOK_STATUS, runHookCommand, stopCommand, type HookAnswer, type HookPayload } from "../../../src/runtime/merge-assist/hook-command.js";
import { sampleConflict } from "../../support/merge-assist/status.js";
import { fromDriveRoot } from "../../support/platform/windows-paths.js";

const payload: HookPayload = {
  version: 1, event: "detected", conflict: sampleConflict(), checkout: { path: tmpdir(), branch: "mine", base: "main" }, session: null,
};

/** The person's command answering `answer` on stdout. */
async function answering(answer: unknown): Promise<HookAnswer> {
  const script = `process.stdout.write(${JSON.stringify(JSON.stringify(answer))})`;
  return await runHookCommand({ command: [process.execPath, "-e", script], timeoutSeconds: 10 }, payload, { cwd: tmpdir(), env: process.env });
}

describe("the conflict hook command's answer", () => {
  it("reads a null note or status as none, as most serializers write an absent field", async () => {
    expect(await answering({ note: null, status: "Tim is on it" })).toEqual({ ok: true, error: null, note: null, status: "Tim is on it" });
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
    const command = [fromDriveRoot(process.execPath), "-e", "process.stdout.write(JSON.stringify({ note: process.argv0 }))"];
    const answer = await runHookCommand({ command, timeoutSeconds: 10 }, payload, { cwd: dirname(process.execPath), env: process.env });
    expect(answer).toEqual({ ok: true, error: null, note: process.execPath, status: null });
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
