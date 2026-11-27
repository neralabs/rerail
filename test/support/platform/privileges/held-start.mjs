// Started in place of Windows PowerShell by backup-privileges.test.ts, as `node held-start.mjs <ms> <program> [args...]`:
// waits <ms>, as the first PowerShell start on a new machine waits while PowerShell prepares its modules, then runs
// <program> with <args> on this process's own input and outputs, and exits with its exit code.
import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";

const [ms, program, ...args] = process.argv.slice(2);
await delay(Number(ms));
const child = spawn(program ?? "", args, { stdio: "inherit", windowsHide: true });
child.once("error", error => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
child.once("exit", code => {
  process.exitCode = code ?? 1;
});
