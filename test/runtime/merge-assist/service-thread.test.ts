import { execFileSync, spawnSync } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import { withExperiments } from "../../support/environment/experiments.js";
import { mergeAssistWorkspaces } from "../../support/merge-assist/workspace.js";

// Every process Rerail starts goes through these; one started with spawnSync or execFileSync holds the thread until it
// exits, and a daemon answers no hook meanwhile.
vi.mock("node:child_process", { spy: true });

/** Set while a case runs as on a machine with no Git installed: every Git read off the thread finds none. */
const git = vi.hoisted(() => ({ missing: false }));
vi.mock("../../../src/engine/git.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/engine/git.js")>("../../../src/engine/git.js");
  const runGitAsync: typeof actual.runGitAsync = async (...args) => {
    if (git.missing) throw new actual.GitUnavailableError([]);
    return await actual.runGitAsync(...args);
  };
  return { ...actual, runGitAsync };
});

// Merge Assist is experimental: every test here runs with it on, as a daemon started with the variable does.
withExperiments("MERGE_ASSIST");

const workspace = mergeAssistWorkspaces();

/** The processes `read` started that held the thread until they exited, each as its command line. */
async function heldWhile(read: () => Promise<unknown>): Promise<string[]> {
  const starters = [vi.mocked(spawnSync), vi.mocked(execFileSync)];
  for (const starter of starters) starter.mockClear();
  await read();
  return starters.flatMap(starter => starter.mock.calls.map(([command, args]) => [command, ...(Array.isArray(args) ? args : [])].join(" ")));
}

describe("Merge Assist reads a checkout without holding the thread", { timeout: 30_000 }, () => {
  it("starts no process that holds the thread for a status read or a settings read, as the dashboard's page makes them", async () => {
    const ws = await workspace();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);

    expect(await heldWhile(() => service.status(ws.repo))).toEqual([]);
    expect(await heldWhile(() => service.settings(ws.repo))).toEqual([]);
  });

  it("starts no process that holds the thread for a session's start or an edit's detection, where Merge Assist is on", async () => {
    const ws = await workspace();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);

    expect(await heldWhile(() => service.openSession(ws.repo, "claude-code", "session-1"))).toEqual([]);
    expect(await heldWhile(() => service.detectAfterEdit(ws.repo, "session-1", "claude-code"))).toEqual([]);
  });

  it("scans nothing at a session's start where Git is not installed, as before", async () => {
    const ws = await workspace();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);

    git.missing = true;
    try {
      await service.openSession(ws.repo, "claude-code", "session-1");
    } finally {
      git.missing = false;
    }

    const status = await service.status(ws.repo);
    expect({ lastScan: status.lastScan, scanning: status.scanning }).toEqual({ lastScan: null, scanning: false });
  });
});
