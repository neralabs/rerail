import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { runGitSync, type GitRunLimits, type GitRunResult } from "../../../src/engine/git-start.js";

/**
 * The blocking Git runs the engine must end as `spawnSync` ends them, each set read once in this process through
 * whichever start is on, or through the engine in a child process ({@link blockingRunsThroughEngine}). A run the
 * engine never returns blocks the thread that called it, which no test time limit can end, so the engine's side runs
 * where a bound can end it.
 */
export type BlockingRunSet = "limits" | "time-limit-held" | "output-limit-held" | "poll-fails";

/** The fields of a blocking run's error that `spawnSync` sets. */
function errorOf(run: { error?: Error }): unknown {
  const { message, code, errno, syscall, path, spawnargs } = (run.error ?? {}) as NodeJS.ErrnoException & { spawnargs?: string[] };
  return { message, code, errno, syscall, path, spawnargs };
}

/** How a blocking run ended, without what it printed. */
function ending(run: GitRunResult): unknown {
  return { status: run.status, signal: run.signal, error: errorOf(run) };
}

/** What a blocking run answered, its error as {@link errorOf} reads it. */
function answered(run: GitRunResult): unknown {
  return { ...run, error: errorOf(run) };
}

/**
 * A blocking run of `script` under `limits`, after the script has started `sleep 10` in the background, which holds
 * stdout and stderr for as long as it runs and writes its pid to `pidFile`: what the run answered, and whether that
 * sleep still ran when the run returned. The sleep is ended either way. A run through Node and one through the engine
 * get the same `pidFile`, so their arguments are the same.
 */
function heldRun(pidFile: string, script: string, limits: GitRunLimits): { answer: unknown; held: boolean } {
  const run = runGitSync("/bin/sh", ["-c", `sleep 10 & echo $! > "$1"; ${script}`, "sh", pidFile], process.env, limits);
  const holder = Number(readFileSync(pidFile, "utf8"));
  let held = true;
  try {
    process.kill(holder, "SIGKILL");
  } catch {
    held = false;
  }
  return { answer: answered(run), held };
}

/**
 * The time limit of a run whose script must reach a point before it: print, or set its TERM trap. At 300 ms a script
 * on a machine with a load of 18 on 16 cores once had printed nothing by its limit, in 1 of 20 runs; the run then
 * answers differently through each start by chance. A run that waits for its limit takes this long.
 */
const READY_LIMIT_MS = 1000;

/**
 * A blocking run, past its {@link READY_LIMIT_MS} time limit, of a script that takes the SIGTERM and then writes 1 MB
 * to its stdout: what the run answered, and how that write ended, as the script writes it to `statusFile`: 141 when
 * it met a closed output (EPIPE), 124 when it filled the buffers and waited until `timeout` ended it.
 */
function writtenAfterTerm(statusFile: string): { answer: unknown; write: string } {
  const script = `sleep 10 & s=$!; trap 'timeout 5 head -c 1000000 /dev/zero; echo $? > "$1"; kill $s; exit 0' TERM; echo started; wait`;
  const run = runGitSync("/bin/sh", ["-c", script, "sh", statusFile], process.env, { timeout: READY_LIMIT_MS });
  return { answer: answered(run), write: readFileSync(statusFile, "utf8") };
}

/** A run's output, its exit, and SIGTERM at its time limit or past its output limit, and an output exactly at it. */
function limits(): unknown {
  const late = runGitSync("/bin/sh", ["-c", "echo started; exec sleep 5"], process.env, { timeout: READY_LIMIT_MS });
  const long = runGitSync("/bin/sh", ["-c", "head -c 3000000 /dev/zero; exec sleep 5"], process.env, { maxBuffer: 1024 * 1024 });
  const plain = runGitSync("/bin/sh", ["-c", "read nothing || echo no-input; echo warned >&2; exit 5"], process.env);
  // Exactly the output limit: a run may reach it and not pass it.
  const exact = runGitSync("/bin/sh", ["-c", "head -c 1000 /dev/zero"], process.env, { maxBuffer: 1000 });
  return { late: answered(late), long: ending(long), plain, exact: ending(exact) };
}

/** Runs past their time limit while a process Git started holds the output, has closed it, or prints. */
function timeLimitHeld(files: string): unknown {
  return {
    running: heldRun(join(files, "running.pid"), "echo started; exec sleep 10", { timeout: READY_LIMIT_MS }),
    exited: heldRun(join(files, "exited.pid"), "echo started; exit 0", { timeout: READY_LIMIT_MS }),
    closed: answered(runGitSync("/bin/sh", ["-c", "echo started; exec > /dev/null 2>&1; exec sleep 10"], process.env, { timeout: READY_LIMIT_MS })),
    printing: ending(runGitSync("/bin/sh", ["-c", "while :; do printf x; done"], process.env, { timeout: 300, maxBuffer: 8 << 20 })),
    writing: writtenAfterTerm(join(files, "writing.status")),
  };
}

/** The exit status of a {@link pollFails} script whose PATH holds no prlimit. */
const NO_PRLIMIT = 97;

/**
 * A run whose script lowers this process's open-file limit to 2, below the three descriptors the engine's wait polls
 * (both outputs and the exit pipe), then prints and sleeps: each poll after that fails with EINVAL, and the sleep lasts
 * until that failed poll ends it, whenever this process polls again within the run's 3 s time limit, which only bounds
 * a run whose poll does not fail. The script exits {@link NO_PRLIMIT} when prlimit is not on PATH, and 98 when prlimit
 * fails. Run it only in a process of its own.
 */
function pollFails(): unknown {
  const script = `command -v prlimit > /dev/null || exit ${NO_PRLIMIT}; prlimit --nofile=2: --pid "$PPID" || exit 98; echo lowered; exec sleep 10`;
  return answered(runGitSync("/bin/sh", ["-c", script], process.env, { timeout: 3000 }));
}

/** Whether the "poll-fails" run's answer says its PATH holds no prlimit (util-linux), the tool it lowers the limit with. */
export function lacksPrlimit(answer: unknown): boolean {
  return typeof answer === "object" && answer !== null && "status" in answer && answer.status === NO_PRLIMIT;
}

/** What each set of runs answers in this process, `files` holding the files the scripts write. */
export function blockingRuns(set: BlockingRunSet, files: string): unknown {
  if (set === "limits") return limits();
  if (set === "time-limit-held") return timeLimitHeld(files);
  if (set === "poll-fails") return pollFails();
  return heldRun(join(files, "holder.pid"), "head -c 1001 /dev/zero; exec sleep 10", { maxBuffer: 1000 });
}

/** How long the child may take: several times the runs' limits of 300 ms to 1 s added up, and the child's start. */
const ENGINE_RUNS_BOUND_MS = 30_000;

const sourceUrl = (path: string): string => pathToFileURL(resolve(import.meta.dirname, path)).href;

/**
 * What {@link blockingRuns} answers in a child process whose Git starts through the engine, as JSON. A child that has
 * not answered within {@link ENGINE_RUNS_BOUND_MS} is killed and the call throws, so a run that never returns fails
 * its test in seconds.
 */
export function blockingRunsThroughEngine(set: BlockingRunSet, files: string): unknown {
  const program = `import { blockingRuns } from ${JSON.stringify(sourceUrl("blocking-git-runs.ts"))};
import { startGitWithoutCopying } from ${JSON.stringify(sourceUrl("../../../src/engine/git-start.ts"))};
import { loadNativeBinding } from ${JSON.stringify(sourceUrl("../../../src/engine/native/native.ts"))};
if (!startGitWithoutCopying(loadNativeBinding)) throw new Error("the engine cannot start Git here");
process.stdout.write(JSON.stringify(blockingRuns(${JSON.stringify(set)}, ${JSON.stringify(files)})));`;
  const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", program], {
    cwd: resolve(import.meta.dirname, "..", "..", ".."),
    encoding: "utf8",
    env: process.env,
    killSignal: "SIGKILL",
    timeout: ENGINE_RUNS_BOUND_MS,
  });
  if (child.error !== undefined) throw new Error(`the engine's ${set} runs gave no answer within ${ENGINE_RUNS_BOUND_MS} ms: ${child.error.message}`);
  if (child.status !== 0) throw new Error(`the engine's ${set} runs failed (${child.status ?? child.signal}): ${child.stderr}`);
  return JSON.parse(child.stdout) as unknown;
}
