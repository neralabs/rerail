import { readFileSync } from "node:fs";
import { join } from "node:path";
import { vi } from "vitest";

/** The coding agent settings files Rerail's registration writes in a user home. */
export const AGENT_SETTINGS_FILES: readonly string[] = [
  join(".claude", "settings.json"),
  join(".claude", "settings.local.json"),
  join(".codex", "hooks.json"),
  join(".cursor", "hooks.json"),
  join(".cursor", "cli-config.json"),
  join(".omp", "agent", "mcp.json"),
  join(".pi", "agent", "settings.json"),
];

/** How often `text` names a path under `scratch` (the temp folder), in either slash style or JSON's escaped backslashes. */
function scratchMentions(text: string, scratch: string): number {
  const forms = new Set([scratch, scratch.replaceAll("\\", "/"), JSON.stringify(scratch).slice(1, -1)]);
  let count = 0;
  for (const form of forms) if (form.length > 3) count += text.split(form).length - 1;
  return count;
}

/**
 * A check of the person's real agent settings for test leaks: it notes how often each file in `realHome` names a path
 * under `scratch` now, and the function it returns names every file that names more such paths since, which only a
 * test writing its temp folders into the real profile does. Other edits to those files (the person's own, another
 * program's) are not leaks and are not reported.
 */
export function watchRealAgentSettings(realHome: string, scratch: string): () => string[] {
  const read = (file: string): number => {
    try {
      return scratchMentions(readFileSync(join(realHome, file), "utf8"), scratch);
    } catch {
      return 0;
    }
  };
  const before = new Map(AGENT_SETTINGS_FILES.map(file => [file, read(file)]));
  return () => AGENT_SETTINGS_FILES.filter(file => read(file) > before.get(file)!).map(file => join(realHome, file));
}

/**
 * Points the user home at `path` for the current test: HOME, which os.homedir() reads on POSIX, and USERPROFILE, which
 * it reads on Windows. Stubbing HOME alone leaves Windows on the person's real profile, where a test that registers
 * hooks would rewrite their own ~/.claude/settings.json. `vi.unstubAllEnvs()` restores both.
 */
export function stubUserHome(path: string): void {
  vi.stubEnv("HOME", path);
  vi.stubEnv("USERPROFILE", path);
}
