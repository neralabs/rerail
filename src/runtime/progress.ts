/**
 * Progress for a long command made of steps, such as `rerail update`: a line as each step starts, a sign it is still
 * running, and a warning once it has run long enough to look stuck. A command whose steps run npm says nothing for
 * minutes otherwise, and a person cannot tell a slow download from a hung one.
 */

export interface ProgressStream {
  write(text: string): unknown;
  readonly isTTY?: boolean;
}

/** When a step has run long enough to look stuck, and what the person should check. */
export interface StallWarning {
  readonly afterMs: number;
  /** One or two sentences after "<step> has taken <time>.": what may be wrong and what to check. */
  readonly hint: string;
}

export interface Progress {
  /** Runs `run` as the step `label`, reporting it as it starts and while it runs; resolves or rejects as `run` does. */
  step<T>(label: string, run: () => Promise<T>, stall?: StallWarning): Promise<T>;
}

/** Reports nothing: for callers with no one to tell, such as tests. */
export const NO_PROGRESS: Progress = { step: async (_label, run) => await run() };

/** `ms` as a person reads it: `12 s`, `2 min`, `2 min 5 s`. */
export function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1_000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

export interface StreamProgressOptions {
  /** Off a terminal, how often a step that is still running says so. */
  readonly heartbeatMs?: number;
}

/** On a terminal, how often the running step's elapsed time is redrawn. */
const TERMINAL_TICK_MS = 1_000;
/** Off a terminal (a coding agent, CI, a log), how often a running step says so: often enough to tell slow from hung. */
const DEFAULT_HEARTBEAT_MS = 30_000;
const CLEAR_LINE = "\r\u001B[2K";

/**
 * Progress written to `stream`. On a terminal a step is one line whose elapsed time counts up in place, ended when the
 * step ends. Elsewhere every write is a whole line: the step as it starts, then "still running" every `heartbeatMs`,
 * so a caller reading the stream line by line sees each as it comes.
 */
export function streamProgress(stream: ProgressStream, options: StreamProgressOptions = {}): Progress {
  const terminal = stream.isTTY === true;
  return {
    async step(label, run, stall) {
      const started = Date.now();
      const elapsed = (): string => formatDuration(Date.now() - started);
      stream.write(terminal ? `${label}…` : `${label}…\n`);
      const timers = [
        setInterval(() => {
          stream.write(terminal ? `${CLEAR_LINE}${label}… ${elapsed()}` : `${label}… still running after ${elapsed()}\n`);
        }, terminal ? TERMINAL_TICK_MS : options.heartbeatMs ?? DEFAULT_HEARTBEAT_MS),
      ];
      if (stall !== undefined) {
        timers.push(setTimeout(() => {
          const warning = `${label} has taken ${formatDuration(stall.afterMs)}. ${stall.hint}\n`;
          // On a terminal the warning takes a line of its own, and the step's line is drawn again below it.
          stream.write(terminal ? `\n${warning}${label}… ${elapsed()}` : warning);
        }, stall.afterMs));
      }
      try {
        return await run();
      } finally {
        for (const timer of timers) clearTimeout(timer);
        if (terminal) stream.write("\n");
      }
    },
  };
}
