import { execFileSync } from "node:child_process";
import { appendFile, mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IgnoreRuleFiles } from "../../../src/engine/code-rule/ignore-rule-files.js";

let home: string;

beforeEach(async () => {
  home = await realpath(await mkdtemp(join(tmpdir(), "rerail-ignore-rule-files-")));
  vi.stubEnv("HOME", home);
  vi.stubEnv("XDG_CONFIG_HOME", "");
  vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "no-gitconfig"));
  vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
});
afterEach(async () => {
  vi.unstubAllEnvs();
  await rm(home, { recursive: true, force: true });
});

async function checkout(path: string): Promise<string> {
  await mkdir(path, { recursive: true });
  execFileSync("git", ["init", "--quiet"], { cwd: path });
  return path;
}

describe("IgnoreRuleFiles", () => {
  it("reports a changed exclude file once, a nested checkout's too, and each .gitignore a tool edit names", async () => {
    const root = await checkout(join(home, "repo"));
    const nested = await checkout(join(root, "vendor", "lib"));
    const files = await IgnoreRuleFiles.open(root, [root, nested]);

    await appendFile(join(nested, ".git", "info", "exclude"), "scratch/\n");
    expect(await files.changes([], [root, nested])).toEqual([{ path: "vendor/lib/.git/info/exclude", via: "observed" }]);
    expect(await files.changes([], [root, nested])).toEqual([]);

    await appendFile(join(root, ".git", "info", "exclude"), "out/\n");
    expect(await files.changes([join(root, ".git", "info", "exclude"), "api/.gitignore", "src/a.ts"], [root, nested])).toEqual([
      { path: ".git/info/exclude", via: "tool" },
      { path: "api/.gitignore", via: "tool" },
    ]);
  });

  it("watches a checkout that appears later from what its exclude file holds then", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    const later = await checkout(join(root, "later"));
    await appendFile(join(later, ".git", "info", "exclude"), "before-watched/\n");
    expect(await files.changes([], [root, later])).toEqual([]);
    await appendFile(join(later, ".git", "info", "exclude"), "after-watched/\n");
    expect(await files.changes([], [root, later])).toEqual([{ path: "later/.git/info/exclude", via: "observed" }]);
  });

  it("reports Git's default excludes file, outside the session folder, by its absolute path", async () => {
    const root = await checkout(join(home, "repo"));
    const files = await IgnoreRuleFiles.open(root, [root]);
    await mkdir(join(home, ".config", "git"), { recursive: true });
    await appendFile(join(home, ".config", "git", "ignore"), "*.local\n");
    expect(await files.changes([], [root])).toEqual([{ path: join(home, ".config", "git", "ignore"), via: "observed" }]);
  });
});
