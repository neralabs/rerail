import { afterAll, beforeAll } from "vitest";
import { PrivilegeSwitch, type Program, WINDOWS_POWERSHELL } from "./privileges/privilege-switch.js";

/**
 * The Windows privileges that let a program open a file whatever its ACL says. Node opens every file with
 * FILE_FLAG_BACKUP_SEMANTICS, so while Backup is on a read passes a deny entry, and while Restore is on a write or a new
 * file does. An elevated administrator's token holds both, off; Git Bash turns them on in each process it starts, and
 * so in the Node that `npm test` starts under it, as on GitHub's Windows runner.
 */
const BACKUP_PRIVILEGES = ["SeBackupPrivilege", "SeRestorePrivilege"] as const;

/**
 * The bound on a describe block's wait for its PowerShell to say it is ready, and the only one. The first Windows
 * PowerShell start on a new GitHub Windows runner, which prepares its modules for first use, took 32.8 s; this allows
 * close to four times that, so a slow first start costs the set-up its time and never fails a case.
 */
export const PRIVILEGE_SWITCH_START_BOUND_MS = 120_000;

/** Runs `body` with this process's Backup and Restore privileges off. */
export type WithoutBackupPrivileges = <T>(body: () => Promise<T>) => Promise<T>;

/**
 * Keeps one Windows PowerShell for the describe block that calls this, which turns this process's Backup and Restore
 * privileges ({@link BACKUP_PRIVILEGES}) off and on when asked. The block's set-up starts it and waits for its line
 * that it is ready, bounded by {@link PRIVILEGE_SWITCH_START_BOUND_MS} alone, and its tear-down ends it, so neither the
 * start nor its compile falls in a case's time limit. When PowerShell ends before it is ready, the set-up fails once,
 * in PowerShell's own words, and vitest skips the block's cases.
 *
 * The privileges live in the token of the process that runs the test file: under `pool: "forks"` each file runs in a
 * child process of its own, one case at a time, so no other file's case shares the token while one has them off.
 *
 * Returns `withoutBackupPrivileges`, which runs `body` with both off and then turns on again those that were on, so an
 * ACL a test sets binds this process as it binds every account that holds neither on: a person's program, an elevated
 * one started from PowerShell or cmd included. Elsewhere than Windows it registers nothing and runs `body`.
 * `program` stands in for Windows PowerShell in a test of this helper.
 */
export function backupPrivilegeSwitch(program: Program = WINDOWS_POWERSHELL): { withoutBackupPrivileges: WithoutBackupPrivileges } {
  if (process.platform !== "win32") return { withoutBackupPrivileges: body => body() };
  let kept: PrivilegeSwitch | undefined;
  beforeAll(async () => {
    // Kept before the wait, so the tear-down ends it even when the wait is cut at its bound.
    kept = new PrivilegeSwitch(program);
    await kept.ready();
  }, PRIVILEGE_SWITCH_START_BOUND_MS);
  afterAll(async () => {
    await kept?.end();
  });
  return { withoutBackupPrivileges: body => withBackupPrivilegesOff(kept, body) };
}

/** Runs `body` with {@link BACKUP_PRIVILEGES} off through `kept`, then turns on again those that were on. */
async function withBackupPrivilegesOff<T>(kept: PrivilegeSwitch | undefined, body: () => Promise<T>): Promise<T> {
  if (kept === undefined) throw new Error("withoutBackupPrivileges runs only in a case of the describe block that called backupPrivilegeSwitch");
  const wereOn = await kept.set(BACKUP_PRIVILEGES, 0);
  try {
    return await body();
  } finally {
    if (wereOn.length > 0) await kept.set(wereOn, 2);
  }
}
