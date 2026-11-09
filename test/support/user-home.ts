import { vi } from "vitest";

/**
 * Points the user home at `path` for the current test: HOME, which os.homedir() reads on POSIX, and USERPROFILE, which
 * it reads on Windows. Stubbing HOME alone leaves Windows on the person's real profile, where a test that registers
 * hooks would rewrite their own ~/.claude/settings.json. `vi.unstubAllEnvs()` restores both.
 */
export function stubUserHome(path: string): void {
  vi.stubEnv("HOME", path);
  vi.stubEnv("USERPROFILE", path);
}
