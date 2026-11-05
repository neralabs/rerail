import { execFileSync } from "node:child_process";

/**
 * Whether this Git honours `GIT_NO_LAZY_FETCH`: 2.45.0 on, and the 2024-04-19 security releases of 2.39 to 2.44.
 * An older Git ignores it, so it fetches what the partial-clone tests say it must not.
 */
export function honoursNoLazyFetch(): boolean {
  const match = /(\d+)\.(\d+)\.(\d+)/u.exec(execFileSync("git", ["--version"], { encoding: "utf8" }));
  if (match === null) return false;
  const [major, minor, patch] = match.slice(1).map(Number) as [number, number, number];
  const securityPatch: Record<number, number> = { 39: 4, 40: 2, 41: 1, 42: 2, 43: 4, 44: 1 };
  if (major !== 2) return major > 2;
  return minor >= 45 || (securityPatch[minor] !== undefined && patch >= securityPatch[minor]);
}

/** Whether the partial clone at `clone` still lacks object `oid`: asking for it with lazy fetch off fails. */
export function partialCloneLacks(clone: string, oid: string): boolean {
  try {
    execFileSync("git", ["cat-file", "-e", oid], { cwd: clone, env: { ...process.env, GIT_NO_LAZY_FETCH: "1" }, stdio: "ignore" });
    return false;
  } catch {
    return true;
  }
}
