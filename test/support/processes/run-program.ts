/**
 * A program a case starts for its set-up, such as the Git that makes a test's repository, whose failure says what
 * happened. The error `execFileSync` throws with `stdio: "ignore"` has the message `Command failed: <command>`, and a
 * bare `execFileSync` adds what the program wrote to stderr: neither message holds the exit status, the signal or how
 * long the program ran, and a test report keeps only the message, so a Git that Windows could not initialise (status
 * 0xC0000142, with nothing written) reads there as any other failure.
 */
import { spawnSync } from "node:child_process";

/**
 * How many lines of each stream a failure's message shows, and how much of each line. A line longer than that shows
 * its first SHOWN_LINE_LENGTH characters and then CUT_MARK, so a reader tells a cut line from a whole one.
 */
const SHOWN_LINES = 20;
const SHOWN_LINE_LENGTH = 500;
const CUT_MARK = "…";

/** The folder and environment a program runs in; Node's defaults when left out. */
export interface ProgramPlace {
  readonly cwd?: string;
  readonly env?: NodeJS.ProcessEnv;
}

/** What a program that ran and failed did: its exit status or the signal that ended it, its time and its output. */
export interface FailedRun {
  readonly command: string;
  readonly cwd: string | undefined;
  readonly status: number | null;
  readonly signal: NodeJS.Signals | null;
  readonly milliseconds: number;
  readonly stdout: string;
  readonly stderr: string;
}

/** Thrown by {@link runProgram} for a program that started and ended with a status other than 0, or by a signal. */
export class ProgramFailed extends Error {
  constructor(readonly run: FailedRun) {
    super(failureMessage(run));
    this.name = "ProgramFailed";
  }
}

/**
 * Runs `file` with `args` and returns what it wrote to stdout. Its stdin is ignored, as `stdio: "ignore"` had it; its
 * stdout and stderr are kept whole, with no cap, so a program that writes much fails no differently; the other start
 * options (`windowsHide`, `shell`) are Node's defaults, as `execFileSync` had them. A start that fails (no such program,
 * or one Windows refuses to create) throws the start's own error; a program that ends with a status other than 0, or by
 * a signal, throws a {@link ProgramFailed}.
 */
export function runProgram(file: string, args: readonly string[], place: ProgramPlace = {}): string {
  const started = performance.now();
  const result = spawnSync(file, args, { ...place, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: Infinity });
  const milliseconds = Math.round(performance.now() - started);
  if (result.error !== undefined) throw result.error;
  if (result.status === 0) return result.stdout;
  const command = [file, ...args].map(word => (/^[^\s"']+$/u.test(word) ? word : JSON.stringify(word))).join(" ");
  throw new ProgramFailed({ command, cwd: place.cwd, status: result.status, signal: result.signal, milliseconds, stdout: result.stdout, stderr: result.stderr });
}

function failureMessage(run: FailedRun): string {
  // In hex a Windows status reads as Windows names it: 3221225794 is 0xC0000142.
  const status = run.status === null ? "none" : `${run.status} (0x${run.status.toString(16).toUpperCase().padStart(8, "0")})`;
  const place = run.cwd === undefined ? "" : ` in ${run.cwd}`;
  return [
    `${run.command} failed${place} after ${run.milliseconds} ms: exit status ${status}, signal ${run.signal ?? "none"}`,
    ...shown("stderr", run.stderr),
    ...shown("stdout", run.stdout),
  ].join("\n");
}

/** The first lines `text` holds under a heading that says how many it holds, or one line saying it is empty. */
function shown(stream: string, text: string): string[] {
  const lines = text.replace(/\r?\n$/u, "").split(/\r?\n/u);
  if (text.length === 0) return [`${stream}: nothing`];
  const heading = lines.length > SHOWN_LINES ? `${stream}, the first ${SHOWN_LINES} of ${lines.length} lines:` : `${stream}, ${lines.length} line${lines.length === 1 ? "" : "s"}:`;
  return [heading, ...lines.slice(0, SHOWN_LINES).map(line => `  ${shownLine(line)}`)];
}

/** `line` whole, or its first SHOWN_LINE_LENGTH characters and CUT_MARK when it is longer. */
function shownLine(line: string): string {
  return line.length > SHOWN_LINE_LENGTH ? `${line.slice(0, SHOWN_LINE_LENGTH)}${CUT_MARK}` : line;
}
