/**
 * A named pipe (`mkfifo`, Linux and macOS) put where a test expects a file: a read that opens it as a file waits for a
 * writer, and with none it would hold the test's process for good.
 */
import { execFileSync, spawn } from "node:child_process";
import { rmSync } from "node:fs";

/**
 * Opens the pipe its argument names for writing 3 s after it starts, once a reader waits there, and ends: the reader then
 * reads nothing. It gives up 10 s after it starts.
 */
const LATE_WRITER = [
  "const fs = require('node:fs');",
  "const until = Date.now() + 10000;",
  "const open = () => { try { fs.closeSync(fs.openSync(process.argv[1], fs.constants.O_WRONLY | fs.constants.O_NONBLOCK)); } catch { if (Date.now() < until) setTimeout(open, 20); } };",
  "setTimeout(open, 3000);",
].join("\n");

/** A pipe made by {@link pipeAt}; `end` stops its writer. */
export interface PlantedPipe {
  end(): Promise<void>;
}

/**
 * A named pipe at `path` in place of what was there, and a writer that opens it 3 s later: a read that waits on the pipe
 * ends then, so code that waits on it fails its test on time instead of holding the test's process for good.
 */
export function pipeAt(path: string): PlantedPipe {
  rmSync(path, { force: true });
  execFileSync("mkfifo", [path]);
  const writer = spawn(process.execPath, ["-e", LATE_WRITER, path], { stdio: "ignore" });
  const exited = new Promise<void>(resolve => writer.once("exit", () => resolve()));
  return {
    end: async () => {
      if (writer.exitCode === null && writer.signalCode === null) writer.kill();
      await exited;
    },
  };
}
