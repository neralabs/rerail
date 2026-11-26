import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { ProgramFailed, runProgram } from "./run-program.js";

/** What `run` threw; fails the case when it threw nothing. */
function caught(run: () => unknown): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  throw new Error("it threw nothing");
}

/** The ProgramFailed that running `script` in this Node threw, in `cwd` when given. */
function failedScript(script: string, cwd?: string): ProgramFailed {
  const failure = caught(() => runProgram(process.execPath, ["-e", script], cwd === undefined ? {} : { cwd }));
  expect(failure).toBeInstanceOf(ProgramFailed);
  return failure as ProgramFailed;
}

describe("a program a case's set-up starts", () => {
  it("gives what it wrote to stdout when it exits 0", () => {
    expect(runProgram(process.execPath, ["-e", "process.stdout.write('made\\n')"])).toBe("made\n");
  });

  it("fails, when it exits 3 after writing a line, with its command, folder, status in decimal and hex, signal, time and that line", () => {
    const folder = tmpdir();
    const script = "process.stderr.write('fatal: could not lock the index\\n'); process.exit(3)";
    const { run, message } = failedScript(script, folder);

    expect(run).toMatchObject({ cwd: folder, status: 3, signal: null, stdout: "", stderr: "fatal: could not lock the index\n" });
    expect(run.command.endsWith(` -e ${JSON.stringify(script)}`)).toBe(true);
    expect(Number.isInteger(run.milliseconds) && run.milliseconds >= 0).toBe(true);
    expect(message.split("\n")).toEqual([
      `${run.command} failed in ${folder} after ${run.milliseconds} ms: exit status 3 (0x00000003), signal none`,
      "stderr, 1 line:",
      "  fatal: could not lock the index",
      "stdout: nothing",
    ]);
  });

  it("shows the first 20 lines of a stream that wrote more, and how many it wrote", () => {
    const { message } = failedScript("for (let line = 1; line <= 25; line += 1) console.log('line ' + line); process.exit(1)");

    expect(message.split("\n").slice(1)).toEqual([
      "stderr: nothing",
      "stdout, the first 20 of 25 lines:",
      ...Array.from({ length: 20 }, (_, at) => `  line ${at + 1}`),
    ]);
  });

  it("marks a line over 500 characters as cut with a trailing …, and shows a line of exactly 500 whole", () => {
    const { message } = failedScript("console.log('a'.repeat(501)); console.log('b'.repeat(500)); process.exit(1)");

    expect(message.split("\n").slice(1)).toEqual(["stderr: nothing", "stdout, 2 lines:", `  ${"a".repeat(500)}…`, `  ${"b".repeat(500)}`]);
  });

  // Windows has no signals: a process that kills itself there exits with status 1.
  it.skipIf(process.platform === "win32")("fails with the signal that ended it, and no status", () => {
    const { run, message } = failedScript("process.kill(process.pid, 'SIGKILL')");

    expect([run.status, run.signal]).toEqual([null, "SIGKILL"]);
    expect(message.split("\n")[0]?.endsWith("exit status none, signal SIGKILL")).toBe(true);
  });

  it("throws the start's own error for a program that cannot be started", () => {
    const missing = join(tmpdir(), "rerail-no-such-program");
    const own = spawnSync(missing, ["init"]).error;
    expect(own, "the program is missing, so its start fails").toBeDefined();

    const failure = caught(() => runProgram(missing, ["init"]));

    expect(failure).not.toBeInstanceOf(ProgramFailed);
    expect(failure).toMatchObject({ ...own, message: own?.message, name: own?.name });
  });
});
