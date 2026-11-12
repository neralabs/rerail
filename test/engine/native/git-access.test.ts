import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";
import { GitUnavailableError, gitExecutable, type GitResult, type RunGitOptions } from "../../../src/engine/git.js";
import { nativeGitAccess, readGitInstallation } from "../../../src/engine/native/git-access.js";

interface Ask {
  args: readonly string[];
  env: RunGitOptions["env"];
}

/** A Git starter that answers `version` with `version` and records every ask. */
function recordingGit(version: string): { run: (cwd: string, args: readonly string[], options?: RunGitOptions) => GitResult; asks: Ask[] } {
  const asks: Ask[] = [];
  const run = (_cwd: string, args: readonly string[], options?: RunGitOptions): GitResult => {
    asks.push({ args, env: options?.env });
    if (args.includes("version")) return { status: 0, stdout: `git version ${version}\n`, stderr: "" };
    if (args.includes("var")) return { status: 0, stdout: "/opt/git/etc/gitconfig\n", stderr: "" };
    return { status: 0, stdout: "system\0file:/opt/git/etc/gitconfig\0core.autocrlf\0", stderr: "" };
  };
  return { run, asks };
}

describe("the engine's Git installation", () => {
  it("asks Rerail's Git three things, outside any repository, with the system file's variables removed", () => {
    const { run, asks } = recordingGit("2.53.0");
    expect(readGitInstallation(run)).toEqual({
      version: "git version 2.53.0",
      systemConfig: "/opt/git/etc/gitconfig",
      configOrigins: "system\0file:/opt/git/etc/gitconfig\0core.autocrlf\0",
    });
    expect(asks.map(ask => ask.args.filter(arg => !arg.startsWith("--git-dir="))[0])).toEqual(["version", "var", "config"]);
    for (const ask of asks) {
      expect(ask.env).toHaveProperty("GIT_CONFIG_SYSTEM", undefined);
      expect(ask.env).toHaveProperty("GIT_CONFIG_NOSYSTEM", undefined);
    }
    expect(asks.slice(1).every(ask => ask.args.some(arg => arg.startsWith("--git-dir=")))).toBe(true);
  });

  it("asks a Git before 2.42, which has no git var GIT_CONFIG_SYSTEM, twice", () => {
    const { run, asks } = recordingGit("2.39.5");
    expect(readGitInstallation(run).systemConfig).toBeNull();
    expect(asks).toHaveLength(2);
  });

  it("reports nothing without Git", () => {
    let asked = 0;
    const missing = (): GitResult => {
      asked += 1;
      throw new GitUnavailableError(["git"]);
    };
    expect(readGitInstallation(missing)).toEqual({ version: "" });
    expect(asked).toBe(1);
  });

  it("names the system file Rerail's Git was built to read, as the engine's owner takes it", () => {
    const env = { ...process.env };
    delete env.GIT_CONFIG_SYSTEM;
    delete env.GIT_CONFIG_NOSYSTEM;
    const version = execFileSync(gitExecutable(), ["version"], { encoding: "utf8" });
    const [, major, minor] = /(\d+)\.(\d+)/u.exec(version) ?? [];
    if (Number(major) * 100 + Number(minor) < 242) return;
    const system = execFileSync(gitExecutable(), ["var", "GIT_CONFIG_SYSTEM"], { encoding: "utf8", env }).trim();
    expect(readGitInstallation().systemConfig).toBe(system);
    expect(nativeGitAccess().systemConfig).toBe(system);
  });
});
