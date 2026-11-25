// Loaded with `node --import` into a Rerail command a case runs, this stands in for a machine where Windows PowerShell
// is slow to start or blocked: every synchronous start of a program named powershell.exe first takes
// RERAIL_TEST_POWERSHELL_MS of this process's time, and is ended at its own timeout as Node ends one that runs past it;
// with RERAIL_TEST_POWERSHELL=blocked it fails to start, as a policy that blocks the program makes it fail. Rerail's own
// `child-process.ts` reads node:child_process at each call, so the replaced function is the one it calls.
import childProcess from "node:child_process";
import { syncBuiltinESMExports } from "node:module";
import { win32 } from "node:path";

const realSpawnSync = childProcess.spawnSync;
const delayMs = Number(process.env.RERAIL_TEST_POWERSHELL_MS ?? 0);
const blocked = process.env.RERAIL_TEST_POWERSHELL === "blocked";

/** Holds this thread for `ms`, as a PowerShell start that takes that long holds a synchronous caller. */
function hold(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

/** What Node's spawnSync gives for a start that failed with `code`, or that ran to its timeout (`ETIMEDOUT`). */
function failedStart(program, code) {
  const error = Object.assign(new Error(`spawnSync ${program} ${code}`), { code, syscall: `spawnSync ${program}`, path: program });
  return { pid: 0, output: [null, "", ""], stdout: "", stderr: "", status: null, signal: code === "ETIMEDOUT" ? "SIGTERM" : null, error };
}

/** Whether `program` names Windows PowerShell's executable. */
function isPowerShell(program) {
  return typeof program === "string" && win32.basename(program).toLowerCase() === "powershell.exe";
}

/** The timeout a spawnSync call's options give, in ms; 0 for none. */
function timeoutOf(options) {
  return Number(options?.timeout ?? 0);
}

/** A synchronous start of PowerShell as this stand-in machine makes it: refused, or held first and ended at its timeout. */
function slowStart(program, args, options) {
  if (blocked) return failedStart(program, "EACCES");
  const timeout = timeoutOf(options);
  if (timeout > 0 && delayMs >= timeout) {
    hold(timeout);
    return failedStart(program, "ETIMEDOUT");
  }
  hold(delayMs);
  return realSpawnSync(program, args, timeout > 0 ? { ...options, timeout: timeout - delayMs } : options);
}

childProcess.spawnSync = (program, args, options) => (isPowerShell(program) ? slowStart(program, args, options) : realSpawnSync(program, args, options));
syncBuiltinESMExports();
