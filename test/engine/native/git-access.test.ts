import { execFileSync } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import { GitUnavailableError, gitExecutable, type GitResult, type RunGitOptions } from "../../../src/engine/git.js";
import { GIT_ASK_AGAIN_MS, gitAccessOwner, nativeGitAccess, readGitInstallation } from "../../../src/engine/native/git-access.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";

interface Ask {
  args: readonly string[];
  env: RunGitOptions["env"];
}

/** A Git starter that answers `var` as a Git with or without `git var GIT_CONFIG_SYSTEM` (2.42) does, and records every ask. */
function recordingGit(hasVar: boolean): { run: (cwd: string, args: readonly string[], options?: RunGitOptions) => GitResult; asks: Ask[] } {
  const asks: Ask[] = [];
  const run = (_cwd: string, args: readonly string[], options?: RunGitOptions): GitResult => {
    asks.push({ args, env: options?.env });
    if (args.includes("var")) return hasVar ? { status: 0, stdout: "/opt/git/etc/gitconfig\n", stderr: "" } : { status: 129, stdout: "", stderr: "usage: git var (-l | <variable>)\n" };
    return { status: 0, stdout: "system\0file:/opt/git/etc/gitconfig\0core.autocrlf\0", stderr: "" };
  };
  return { run, asks };
}

const LISTING = "system\0file:/opt/git/etc/gitconfig\0core.autocrlf\0";
const missingGit = (): GitResult => {
  throw new GitUnavailableError(["git"]);
};

describe("the engine's Git installation", () => {
  it("asks Rerail's Git two things, outside any repository, with the system file's variables removed", () => {
    const { run, asks } = recordingGit(true);
    expect(readGitInstallation(run)).toEqual({ installation: { systemConfig: "/opt/git/etc/gitconfig", configOrigins: LISTING }, settled: true });
    expect(asks.map(ask => ask.args.filter(arg => !arg.startsWith("--git-dir="))[0])).toEqual(["var", "config"]);
    for (const ask of asks) {
      expect(ask.env).toHaveProperty("GIT_CONFIG_SYSTEM", undefined);
      expect(ask.env).toHaveProperty("GIT_CONFIG_NOSYSTEM", undefined);
      expect(ask.args.some(arg => arg.startsWith("--git-dir="))).toBe(true);
    }
  });

  it("leaves the system file to the listing on a Git before 2.42, which has no git var GIT_CONFIG_SYSTEM", () => {
    const { run, asks } = recordingGit(false);
    expect(readGitInstallation(run)).toEqual({ installation: { configOrigins: LISTING }, settled: true });
    expect(asks).toHaveLength(2);
  });

  it("reports nothing without Git, settled, and why", () => {
    expect(readGitInstallation(missingGit)).toEqual({ installation: {}, settled: true, problem: expect.stringContaining("git") });
  });

  it("hands the engine what a Git before 2.42, or no Git, reports", () => {
    const { GitAccess } = loadNativeBinding();
    expect(new GitAccess(readGitInstallation(recordingGit(false).run).installation).systemConfig).toBe("/opt/git/etc/gitconfig");
    const none = new GitAccess(readGitInstallation(missingGit).installation);
    expect([none.systemConfig, none.installationConfig]).toEqual([null, null]);
  });

  it("asks Git again after asks that did not finish, keeps the owner while the answer is the same, and keeps the first answer Git gives", () => {
    let clock = 0;
    let timesOut = true;
    let asks = 0;
    const run = (_cwd: string, args: readonly string[]): GitResult => {
      asks += 1;
      if (timesOut) return { status: null, stdout: "", stderr: "", error: new Error("spawnSync git ETIMEDOUT") };
      return args.includes("var") ? { status: 0, stdout: "/opt/git/etc/gitconfig\n", stderr: "" } : { status: 0, stdout: LISTING, stderr: "" };
    };
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      const owner = gitAccessOwner(loadNativeBinding(), run, () => clock);
      const fallback = owner();
      expect(fallback.systemConfig).toBeNull();
      clock += GIT_ASK_AGAIN_MS - 1;
      expect(owner()).toBe(fallback);
      expect(asks).toBe(2);
      clock += 1;
      // Asked again, and Git still did not answer: the same fallback, so readers keep what they opened.
      expect(owner()).toBe(fallback);
      expect(asks).toBe(4);
      timesOut = false;
      clock += GIT_ASK_AGAIN_MS;
      const answered = owner();
      expect(answered).not.toBe(fallback);
      expect(answered.systemConfig).toBe("/opt/git/etc/gitconfig");
      clock += 10 * GIT_ASK_AGAIN_MS;
      expect(owner()).toBe(answered);
      expect(asks).toBe(6);
      expect(stderr.mock.calls.map(([text]) => String(text))).toEqual([expect.stringMatching(/Git did not say which system config file it reads \(spawnSync git ETIMEDOUT\); the engine's Git reads take (\/etc\/gitconfig|no system config file)\n$/u)]);
    } finally {
      stderr.mockRestore();
    }
  });

  it("names the system file Rerail's Git was built to read, as the engine's owner takes it", ({ skip }) => {
    const env = { ...process.env };
    delete env.GIT_CONFIG_SYSTEM;
    delete env.GIT_CONFIG_NOSYSTEM;
    const version = execFileSync(gitExecutable(), ["version"], { encoding: "utf8" });
    const [, major, minor] = /(\d+)\.(\d+)/u.exec(version) ?? [];
    if (Number(major) * 100 + Number(minor) < 242) skip("git var GIT_CONFIG_SYSTEM needs Git 2.42");
    const system = execFileSync(gitExecutable(), ["var", "GIT_CONFIG_SYSTEM"], { encoding: "utf8", env }).trim();
    expect(readGitInstallation().installation.systemConfig).toBe(system);
    expect(nativeGitAccess().systemConfig).toBe(system);
  });
});
