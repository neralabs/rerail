/**
 * How the conformance checks and the derivation (scripts/pack-data/) run other programs (git; the pinned runtimes):
 * through a runner, so a test can stand one in.
 */
import { spawn } from "node:child_process";

export interface CommandResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

export interface CommandOptions {
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  input?: string;
  timeoutMs?: number;
}

export interface CommandRunner {
  run(argv: readonly string[], options?: CommandOptions): Promise<CommandResult>;
}

export class ProcessCommandRunner implements CommandRunner {
  async run(argv: readonly string[], options: CommandOptions = {}): Promise<CommandResult> {
    if (argv.length === 0) throw new Error("cannot execute an empty command");
    return await new Promise<CommandResult>((resolve, reject) => {
      const child = spawn(argv[0]!, argv.slice(1), {
        cwd: options.cwd,
        env: options.env,
        stdio: [options.input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
      });
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      child.stdout!.on("data", (chunk: Buffer) => stdout.push(chunk));
      child.stderr!.on("data", (chunk: Buffer) => stderr.push(chunk));
      child.on("error", reject);
      let timer: NodeJS.Timeout | undefined;
      if (options.timeoutMs !== undefined) {
        timer = setTimeout(() => child.kill("SIGKILL"), options.timeoutMs);
      }
      child.on("close", (code) => {
        clearTimeout(timer);
        resolve({
          exitCode: code ?? 1,
          stdout: Buffer.concat(stdout).toString("utf8"),
          stderr: Buffer.concat(stderr).toString("utf8"),
        });
      });
      if (options.input !== undefined) {
        // A child that exits before reading its input fails the write with EPIPE; uncaught, that would end the process.
        child.stdin!.once("error", reject);
        child.stdin!.end(options.input);
      }
    });
  }
}

/** Run a command and return its stdout, failing with its stderr when it exits non-zero. */
export async function runChecked(runner: CommandRunner, argv: readonly string[], options: CommandOptions = {}): Promise<string> {
  const result = await runner.run(argv, options);
  if (result.exitCode !== 0) {
    const detail = (result.stderr.trim() || result.stdout.trim()).split("\n").slice(-20).join("\n");
    throw new Error(`${argv.join(" ")} exited ${result.exitCode}${detail === "" ? "" : `:\n${detail}`}`);
  }
  return result.stdout;
}
