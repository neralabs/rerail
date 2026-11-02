import type { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { chmodSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { afterEach } from "vitest";

/** What one fake agent CLI run does once it has read its whole stdin. */
export interface FakeRun {
  readonly stdout?: string | Buffer;
  readonly stderr?: string;
  readonly code?: number;
  /** Written to the file after `-o` before the run exits, as Codex does. */
  readonly answer?: string;
  /** Keeps running until killed; with `ignoreTerm`, only SIGKILL stops it. */
  readonly hang?: boolean;
  readonly ignoreTerm?: boolean;
  /** Fails to start with this error instead of running. */
  readonly spawnError?: string;
}

export interface FakeCall {
  readonly command: string;
  readonly args: string[];
  readonly options: { cwd?: string; env?: NodeJS.ProcessEnv; shell?: boolean; stdio?: unknown };
  stdin: string;
  readonly signals: NodeJS.Signals[];
  /** The working directory's permission bits while the run was going. */
  cwdMode: number | null;
  /** The text of the `--output-schema` file while the run was going. */
  schemaText: string | null;
}

class FakeChild extends EventEmitter {
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  #run: FakeRun = {};
  #exited = false;

  constructor(private readonly call: FakeCall) {
    super();
  }

  start(run: FakeRun): void {
    this.#run = run;
  }

  kill(signal: NodeJS.Signals = "SIGTERM"): boolean {
    this.call.signals.push(signal);
    if (signal === "SIGKILL" || this.#run.ignoreTerm !== true) this.exit(null, signal);
    return true;
  }

  exit(code: number | null, signal: NodeJS.Signals | null): void {
    if (this.#exited) return;
    this.#exited = true;
    this.exitCode = code;
    this.signalCode = signal;
    this.stdout.end();
    this.stderr.end();
    setImmediate(() => {
      this.emit("exit", code, signal);
      this.emit("close", code, signal);
    });
  }
}

function argAfter(args: readonly string[], flag: string): string | undefined {
  const index = args.indexOf(flag);
  return index === -1 ? undefined : args[index + 1];
}

/** An injected `spawn` that records each call and answers it with `behave`'s run; it never starts a process. */
export function fakeAgentCli(behave: (call: FakeCall) => FakeRun): { spawn: typeof spawn; calls: FakeCall[] } {
  const calls: FakeCall[] = [];
  const fake = (command: string, args: readonly string[], options: FakeCall["options"]) => {
    const call: FakeCall = { command, args: [...args], options, stdin: "", signals: [], cwdMode: null, schemaText: null };
    calls.push(call);
    const child = new FakeChild(call);
    child.stdin.setEncoding("utf8").on("data", (chunk: string) => { call.stdin += chunk; });
    child.stdin.on("end", () => {
      if (options.cwd !== undefined) call.cwdMode = statSync(options.cwd).mode & 0o777;
      const schemaPath = argAfter(call.args, "--output-schema");
      if (schemaPath !== undefined) call.schemaText = readFileSync(schemaPath, "utf8");
      const run = behave(call);
      child.start(run);
      if (run.spawnError !== undefined) {
        child.emit("error", Object.assign(new Error(run.spawnError), { code: "ENOENT" }));
        return;
      }
      if (run.stderr !== undefined) child.stderr.write(run.stderr);
      const answerPath = argAfter(call.args, "-o");
      if (run.answer !== undefined && answerPath !== undefined) writeFileSync(answerPath, run.answer);
      if (run.stdout !== undefined) {
        const bytes = Buffer.isBuffer(run.stdout) ? run.stdout : Buffer.from(run.stdout);
        for (let start = 0; start < bytes.length; start += 64 * 1024) child.stdout.write(bytes.subarray(start, start + 64 * 1024));
      }
      if (run.hang !== true) child.exit(run.code ?? 0, null);
    });
    return child;
  };
  return { spawn: fake as unknown as typeof spawn, calls };
}

/** A directory holding executable stand-ins for `names`, for a PATH that finds exactly those CLIs; removed after each test. */
export function agentBinDirectories(): (names: readonly string[]) => string {
  const made: string[] = [];
  afterEach(() => {
    for (const directory of made.splice(0)) rmSync(directory, { recursive: true, force: true });
  });
  return names => {
    const directory = mkdtempSync(join(tmpdir(), "rerail-judge-bin-"));
    made.push(directory);
    for (const name of names) {
      const path = join(directory, name);
      writeFileSync(path, "#!/bin/sh\nexit 1\n");
      chmodSync(path, 0o755);
    }
    return directory;
  };
}
