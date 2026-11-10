import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubUserHome, watchRealAgentSettings } from "./user-home.js";

const under = (path: string, root: string): boolean => realpathSync(path).startsWith(realpathSync(root));

describe("the user home a test sees", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("is a folder of the test file's own under the temp folder, so no test writes the person's real profile", () => {
    // os.homedir() reads HOME on POSIX and USERPROFILE on Windows; both must point away from the real profile.
    expect(under(homedir(), tmpdir())).toBe(true);
    expect(process.env.HOME).toBe(homedir());
    expect(process.env.USERPROFILE).toBe(homedir());
  });

  it("moves with stubUserHome on every platform, not only where os.homedir() reads HOME", () => {
    const moved = join(homedir(), "moved");
    stubUserHome(moved);
    expect(homedir()).toBe(moved);
    expect(process.env.HOME).toBe(moved);
  });
});

describe("the Git configuration a test repository sees", () => {
  it("is its own and the test home's global file, never the machine's or the person's, so Git writes files the same everywhere", () => {
    const root = mkdtempSync(join(tmpdir(), "rerail-git-config-"));
    try {
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      const scopes = execFileSync("git", ["config", "--list", "--show-scope"], { cwd: root, encoding: "utf8" })
        .split("\n").filter(line => line !== "").map(line => line.split("\t")[0]);
      expect(new Set(scopes)).toEqual(new Set(["global", "local"]));
      expect(execFileSync("git", ["config", "--global", "--list"], { cwd: root, encoding: "utf8" }).split("\n").filter(line => line !== "").sort())
        .toEqual(["user.email=test@rerail.invalid", "user.name=Rerail test"]);
      // Git for Windows' installer sets core.autocrlf=true in the machine's file; a test repository never reads it.
      expect(spawnSync("git", ["config", "--get", "core.autocrlf"], { cwd: root }).status).toBe(1);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe("the guard on the person's real agent settings", () => {
  it("names a settings file a test wrote its temp folders into, and not one edited otherwise", () => {
    const realHome = mkdtempSync(join(tmpdir(), "rerail-real-home-"));
    const scratch = join(tmpdir(), "rerail-scratch-root");
    try {
      mkdirSync(join(realHome, ".claude"));
      mkdirSync(join(realHome, ".cursor"));
      const claude = join(realHome, ".claude", "settings.json");
      const cursor = join(realHome, ".cursor", "hooks.json");
      writeFileSync(claude, JSON.stringify({ hooks: { Stop: [{ command: "node", args: ["/opt/rerail/dist/hosts/claude/claude-hook.js"] }] } }));
      writeFileSync(cursor, "{}");
      const leaked = watchRealAgentSettings(realHome, scratch);
      expect(leaked()).toEqual([]);
      // The person's own edit: no temp folder in it.
      writeFileSync(cursor, JSON.stringify({ version: 1 }));
      // What a test registering into the real profile leaves: its temp install, JSON-escaped on Windows.
      writeFileSync(claude, JSON.stringify({ hooks: { Stop: [{ command: "node", args: [join(scratch, "rerail-registration-x", "home", ".rerail", "current", "dist", "harness", "hook-launcher.js")] }] } }));
      expect(leaked()).toEqual([claude]);
    } finally {
      rmSync(realHome, { recursive: true, force: true });
    }
  });
});
