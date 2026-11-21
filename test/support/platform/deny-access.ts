/**
 * Taking a user's rights to a folder away, as a person's own security settings can, for the tests of what Rerail does
 * with a path it may not read.
 */
import { execFileSync, spawnSync } from "node:child_process";
import { closeSync, openSync, readdirSync } from "node:fs";
import { chmod } from "node:fs/promises";
import { join } from "node:path";

const WINDOWS = process.platform === "win32";
const systemTool = (name: string): string => join(process.env.SystemRoot ?? "C:\\Windows", "System32", name);
let sid: string | undefined;
function userSid(): string {
  sid ??= execFileSync(systemTool("whoami.exe"), ["/user", "/fo", "csv", "/nh"], { encoding: "utf8" }).trim().split(",").at(-1)?.replaceAll('"', "");
  if (sid === undefined || sid === "") throw new Error("whoami named no user");
  return sid;
}
function refused(look: () => void): boolean {
  try {
    look();
    return false;
  } catch {
    return true;
  }
}

/**
 * Takes this user's rights to the folder `path` away, or with `files` those to each file in it only (`probe` is one),
 * and says whether that holds for this process. On Windows through a deny rule, which a process holding the backup
 * privilege reads through; elsewhere through the folder's mode, which root reads through, and never `files`, which a mode
 * cannot do. `restoreAccess` gives the rights back, whether it held or not.
 */
export async function denyAccess(path: string, { files = false, probe = path }: { files?: boolean; probe?: string } = {}): Promise<boolean> {
  if (!WINDOWS) {
    if (files) return false;
    await chmod(path, 0o000);
    return refused(() => readdirSync(path));
  }
  execFileSync(systemTool("icacls.exe"), [path, "/deny", `*${userSid()}:${files ? "(OI)(IO)F" : "(OI)(CI)F"}`], { stdio: "ignore" });
  return files ? refused(() => closeSync(openSync(probe, "r"))) : refused(() => readdirSync(path));
}

/** Gives back what `denyAccess` took. */
export async function restoreAccess(path: string): Promise<void> {
  if (!WINDOWS) return void await chmod(path, 0o755).catch(() => undefined);
  spawnSync(systemTool("icacls.exe"), [path, "/remove:d", `*${userSid()}`], { stdio: "ignore" });
  spawnSync(systemTool("icacls.exe"), [path, "/reset", "/T", "/C", "/Q"], { stdio: "ignore" });
}
