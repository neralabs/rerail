import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi, type MockedFunction } from "vitest";

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/git.js")>();
  return { ...actual, runGitAsync: vi.fn(actual.runGitAsync) };
});

import { GitUnavailableError, gitExecutable, type GitResult, type RunGitOptions, type runGitAsync } from "../../../src/engine/git.js";
import { GIT_ASK_AGAIN_MS, gitAccessOwner, nativeGitAccess, readGitInstallation } from "../../../src/engine/native/git-access.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";

afterEach(() => {
  vi.unstubAllEnvs();
  vi.useRealTimers();
});

/**
 * The module afresh, with no process owner made yet, and the starter of its Git processes no thread waits on, which
 * runs Git until a case says otherwise.
 */
async function freshGitAccess(): Promise<typeof import("../../../src/engine/native/git-access.js") & { asks: MockedFunction<typeof runGitAsync> }> {
  vi.resetModules();
  const { runGitAsync } = await import("../../../src/engine/git.js");
  const asks = vi.mocked(runGitAsync);
  asks.mockReset();
  return { ...(await import("../../../src/engine/native/git-access.js")), asks };
}

/** Runs Git for an ask, and aborts `stopping` once that Git has started. */
async function abortedWhileAsking(stopping: AbortController, ...ask: Parameters<typeof runGitAsync>): Promise<GitResult> {
  const actual = await vi.importActual<typeof import("../../../src/engine/git.js")>("../../../src/engine/git.js");
  const asking = actual.runGitAsync(...ask);
  stopping.abort();
  return await asking;
}

/** The answers a Git that names `/opt/git/etc/gitconfig` gives the two asks. */
async function answered(_cwd: string, args: readonly string[]): Promise<GitResult> {
  return args.includes("var") ? { status: 0, stdout: "/opt/git/etc/gitconfig\n", stderr: "" } : { status: 0, stdout: LISTING, stderr: "" };
}

/** The lines `act` wrote on stderr, which it does not print. */
async function stderrOf(act: () => Promise<void>): Promise<string[]> {
  const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  try {
    await act();
    return stderr.mock.calls.map(([text]) => String(text));
  } finally {
    stderr.mockRestore();
  }
}

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
    const system = builtSystemFile() ?? skip("git var GIT_CONFIG_SYSTEM needs Git 2.42");
    expect(readGitInstallation().installation.systemConfig).toBe(system);
    expect(nativeGitAccess().systemConfig).toBe(system);
  });
});

/** The system file Rerail's Git was built to read, as `git var` names it with the variables that move it removed; undefined before Git 2.42. */
function builtSystemFile(): string | undefined {
  const env = { ...process.env };
  delete env.GIT_CONFIG_SYSTEM;
  delete env.GIT_CONFIG_NOSYSTEM;
  const version = execFileSync(gitExecutable(), ["version"], { encoding: "utf8" });
  const [, major, minor] = /(\d+)\.(\d+)/u.exec(version) ?? [];
  if (Number(major) * 100 + Number(minor) < 242) return undefined;
  return execFileSync(gitExecutable(), ["var", "GIT_CONFIG_SYSTEM"], { encoding: "utf8", env }).trim();
}

describe("the engine's Git installation learnt without blocking", () => {
  it("names the system file Rerail's Git was built to read, whatever GIT_CONFIG_SYSTEM says", async ({ skip }) => {
    const system = builtSystemFile() ?? skip("git var GIT_CONFIG_SYSTEM needs Git 2.42");
    vi.stubEnv("GIT_CONFIG_SYSTEM", join(tmpdir(), "elsewhere.gitconfig"));
    const { learnGitAccess, standingGitAccess } = await freshGitAccess();
    await learnGitAccess();
    expect(standingGitAccess()?.systemConfig).toBe(system);
  });

  it("shares one ask among the callers meanwhile, and asks nothing while its answer stands", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    asks.mockImplementation(answered);
    await Promise.all([learnGitAccess(), learnGitAccess()]);
    await learnGitAccess();
    expect({ asks: asks.mock.calls.length, system: standingGitAccess()?.systemConfig }).toEqual({ asks: 2, system: "/opt/git/etc/gitconfig" });
  });

  it("makes no owner and says nothing when its signal is aborted during the first ask", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    const stopping = new AbortController();
    asks.mockImplementationOnce(async (...ask) => await abortedWhileAsking(stopping, ...ask));
    expect(await stderrOf(() => learnGitAccess({ signal: stopping.signal }))).toEqual([]);
    expect(standingGitAccess()).toBeUndefined();
  });

  it("asks again for a caller whose signal is live when another caller's signal stopped the ask they shared", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    const first = new AbortController();
    asks.mockImplementationOnce(async (...ask) => await abortedWhileAsking(first, ...ask));
    await Promise.all([learnGitAccess({ signal: first.signal }), learnGitAccess({ signal: new AbortController().signal })]);
    expect(standingGitAccess()).toBeDefined();
  });

  it("returns at once for a caller whose own signal is aborted, while the ask it shares with another caller goes on", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    const held = Promise.withResolvers<void>();
    asks.mockImplementation(async (cwd, args) => {
      await held.promise;
      return await answered(cwd, args);
    });
    const stopping = new AbortController();
    let asking = true;
    const starter = learnGitAccess();
    const joiner = learnGitAccess({ signal: stopping.signal }).then(() => asking);
    stopping.abort();
    // The shared asks answer at the next turn of the event loop.
    setImmediate(() => {
      asking = false;
      held.resolve();
    });
    expect(await joiner).toBe(true);
    await starter;
    expect(standingGitAccess()?.systemConfig).toBe("/opt/git/etc/gitconfig");
  });

  it("returns at once for a caller whose own signal was aborted before it joined an ask another caller started", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    const held = Promise.withResolvers<void>();
    asks.mockImplementation(async (cwd, args) => {
      await held.promise;
      return await answered(cwd, args);
    });
    const starter = learnGitAccess();
    // Its abort event fired before it joined, so none reaches it while it waits.
    const stopping = new AbortController();
    stopping.abort();
    let asking = true;
    const joiner = learnGitAccess({ signal: stopping.signal }).then(() => asking);
    setImmediate(() => {
      asking = false;
      held.resolve();
    });
    expect(await joiner).toBe(true);
    await starter;
    expect(standingGitAccess()?.systemConfig).toBe("/opt/git/etc/gitconfig");
  });

  it("asks nothing again for a caller whose shared ask was stopped once a blocking ask made the owner meanwhile", async () => {
    const { learnGitAccess, nativeGitAccess: blockingAccess, standingGitAccess, asks } = await freshGitAccess();
    const first = new AbortController();
    asks.mockImplementationOnce(async (...ask) => {
      // Once the other caller has joined, the engine's thread asks Git and makes the owner; then this ask's caller stops.
      await new Promise(resolve => setImmediate(resolve));
      blockingAccess();
      return await abortedWhileAsking(first, ...ask);
    });
    const before = asks.mock.calls.length;
    await Promise.all([learnGitAccess({ signal: first.signal }), learnGitAccess({ signal: new AbortController().signal })]);
    // The stopped ask's two: the live caller, asking again, finds the owner standing and asks nothing.
    expect({ asks: asks.mock.calls.length - before, standing: standingGitAccess() !== undefined }).toEqual({ asks: 2, standing: true });
  });

  it("keeps the owner a blocking ask made while its own asks ran", async () => {
    const { learnGitAccess, nativeGitAccess: blockingAccess, standingGitAccess, asks } = await freshGitAccess();
    const held = Promise.withResolvers<void>();
    asks.mockImplementation(async (cwd, args) => {
      await held.promise;
      return await answered(cwd, args);
    });
    const learning = learnGitAccess();
    const blocking = blockingAccess();
    held.resolve();
    await learning;
    expect(standingGitAccess()).toBe(blocking);
  });

  it("asks again once an answer that did not settle is due, and keeps the one Git gives then", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    let timesOut = true;
    asks.mockImplementation(async (cwd, args) => (timesOut ? { status: null, stdout: "", stderr: "", error: new Error("Git did not answer in time") } : await answered(cwd, args)));
    const told = await stderrOf(async () => {
      await learnGitAccess();
      const fallback = standingGitAccess();
      expect(fallback?.systemConfig).toBeNull();
      vi.setSystemTime(Date.now() + GIT_ASK_AGAIN_MS - 1);
      await learnGitAccess();
      expect(standingGitAccess()).toBe(fallback);
      expect(asks).toHaveBeenCalledTimes(2);
      vi.setSystemTime(Date.now() + 1);
      expect(standingGitAccess()).toBeUndefined();
      timesOut = false;
      await learnGitAccess();
      vi.setSystemTime(Date.now() + 10 * GIT_ASK_AGAIN_MS);
      expect({ asks: asks.mock.calls.length, system: standingGitAccess()?.systemConfig }).toEqual({ asks: 4, system: "/opt/git/etc/gitconfig" });
    });
    expect(told).toEqual([expect.stringContaining("Git did not say which system config file it reads (Git did not answer in time)")]);
  });

  it("never fails, and makes nothing, where Git is missing", async () => {
    const { learnGitAccess, standingGitAccess, asks } = await freshGitAccess();
    asks.mockRejectedValue(new GitUnavailableError(["git"]));
    await expect(learnGitAccess()).resolves.toBeUndefined();
    expect(standingGitAccess()).toBeUndefined();
  });
});
