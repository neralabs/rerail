import { readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer as createHttpServer, type IncomingMessage, type ServerResponse } from "node:http";
import { createServer as createNetServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

const autostart = vi.hoisted(() => ({ calls: 0 }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return { ...actual, autostartDaemon: () => { autostart.calls += 1; } };
});

import { HOOK_FAILURES_FILE, hookFailureLines, readHookFailures, recordHookFailure, type HookFailure } from "../../src/harness/hook-failures.js";
import { runHostHook } from "../../src/harness/hook-host.js";

const homes: string[] = [];
async function home(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-hook-failures-"));
  homes.push(path);
  return path;
}

afterEach(async () => {
  vi.unstubAllEnvs();
  autostart.calls = 0;
  await Promise.all(homes.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

const failure = (at: number, event = "MessageDisplay"): HookFailure => ({
  at, host: "claude-code", event, agent: "claude-code_2-1-311_harness", rerail: "0.1.3", reason: "Claude hook omitted index",
});

describe("what rerail status says of hook failures", () => {
  it("says hooks run but nothing was recorded when every failure is newer than the last recorded event", () => {
    expect(hookFailureLines([failure(2_000), failure(3_000)], 1_000)).toEqual([
      "Hooks run but nothing was recorded since 1970-01-01T00:00:02.000Z: 2 hook events failed; the last, MessageDisplay from "
        + "claude-code_2-1-311_harness with Rerail 0.1.3 at 1970-01-01T00:00:03.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines([failure(2_000)], null)[0]).toMatch(/^Hooks run but nothing was recorded since /u);
  });

  it("names the failures without that lead when events were recorded after some of them, and says nothing without any", () => {
    expect(hookFailureLines([failure(2_000), failure(4_000, "Stop")], 3_000)).toEqual([
      "Hooks: 2 hook events could not be recorded in the last 24 hours; the last, Stop from claude-code_2-1-311_harness "
        + "with Rerail 0.1.3 at 1970-01-01T00:00:04.000Z: Claude hook omitted index.",
    ]);
    expect(hookFailureLines([], 3_000)).toEqual([]);
  });
});

describe("the hook failure log", () => {
  it("keeps each failure with the agent's own name and version, and reads back only those since the given time", async () => {
    const rerailHome = await home();
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "old" }, { home: rerailHome, now: 1_000, env: {} });
    recordHookFailure({ host: "claude-code", event: "MessageDisplay", reason: "new" }, { home: rerailHome, now: 5_000, env: { AI_AGENT: "claude-code_2-1-311_harness" } });
    expect(readHookFailures(rerailHome, 2_000)).toEqual([
      expect.objectContaining({ at: 5_000, host: "claude-code", event: "MessageDisplay", agent: "claude-code_2-1-311_harness", reason: "new" }),
    ]);
  });

  it("moves a full log aside instead of growing without end", async () => {
    const rerailHome = await home();
    writeFileSync(join(rerailHome, HOOK_FAILURES_FILE), `${"x".repeat(300 * 1024)}\n`);
    recordHookFailure({ host: "claude-code", event: "Stop", reason: "after" }, { home: rerailHome, now: 1, env: {} });
    expect(readFileSync(join(rerailHome, `${HOOK_FAILURES_FILE}.1`), "utf8").length).toBeGreaterThan(300 * 1024);
    expect(readHookFailures(rerailHome, 0).map(entry => entry.reason)).toEqual(["after"]);
  });
});

describe("a hook that cannot record its event", () => {
  it("logs the event as lost and starts a daemon in the background when none is running", async () => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    const handle = vi.fn(async () => ({}));
    expect(await runHostHook("claude-code", rerailHome, "s", false, { storageRoot: join(rerailHome, "sessions") }, handle, "PostToolUse")).toEqual({});
    expect(handle).not.toHaveBeenCalled();
    expect(autostart.calls).toBe(1);
    expect(readHookFailures(rerailHome, 0)).toEqual([
      expect.objectContaining({ host: "claude-code", event: "PostToolUse", reason: "no Rerail daemon answered, so this event was not recorded" }),
    ]);
  });

  it.each([
    {
      daemon: "refuses it",
      answer: (_request: IncomingMessage, response: ServerResponse) => { response.writeHead(401).end(); },
      reason: "refused the access capability in daemon.json",
    },
    // A daemon whose one event loop a walk or a long garbage collection holds.
    { daemon: "does not answer", answer: () => undefined, reason: "is running but did not answer within 12 s" },
  ])("logs the event as lost and starts no daemon when the recorded daemon is alive but $daemon", async ({ answer, reason }) => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    // A live daemon: this process, holding the engine socket daemon.json names, with `answer` at the URL it records.
    const front = createHttpServer(answer);
    const engine = createNetServer();
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-alive-${process.pid}-${Date.now()}` : join(rerailHome, "engine.sock");
    await new Promise<void>(resolve => front.listen(0, "127.0.0.1", resolve));
    await new Promise<void>(resolve => engine.listen(socketPath, resolve));
    try {
      const port = (front.address() as AddressInfo).port;
      writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({
        pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test", ui: "bundled",
        startedAt: 1, packageRoot: rerailHome, databasePath: join(rerailHome, "rerail.sqlite"), kernelSocketPath: socketPath, accessToken: "token",
      }));
      const handle = vi.fn(async () => ({}));
      // A deadline already reached: the hook gives up after the first probe's 2 s instead of waiting 10 s more.
      const options = { storageRoot: join(rerailHome, "sessions"), daemonDeadline: Date.now() };
      expect(await runHostHook("claude-code", rerailHome, "s", false, options, handle, "PreToolUse")).toEqual({});
      expect(handle).not.toHaveBeenCalled();
      // A start finds the same daemon and refuses to replace it: the background start would cost a process and do nothing.
      expect(autostart.calls).toBe(0);
      expect(readHookFailures(rerailHome, 0)).toEqual([expect.objectContaining({
        event: "PreToolUse", reason: `the Rerail daemon (pid ${process.pid}) ${reason}, so this event was not recorded`,
      })]);
    } finally {
      front.closeAllConnections();
      await Promise.all([front, engine].map(server => new Promise(resolve => server.close(resolve))));
    }
  });

  it("logs a failure inside the hook under the event's own name, then fails as before", async () => {
    const rerailHome = await home();
    vi.stubEnv("RERAIL_HOME", rerailHome);
    const options = { storageRoot: join(rerailHome, "sessions"), databasePath: join(rerailHome, "rerail.sqlite"), embedder: { fingerprint: () => ({ modelId: "t", revision: "1", dimensions: 2, weightsSha256: "t", tokenizerSha256: "t", kernelFormatVersion: 1 }), embed: async (texts: readonly string[]) => texts.map(() => new Float32Array([1, 0])) } };
    await expect(runHostHook("claude-code", rerailHome, "s", false, options, async () => { throw new Error("Claude hook omitted index"); }, "MessageDisplay"))
      .rejects.toThrow("Claude hook omitted index");
    expect(readHookFailures(rerailHome, 0)).toEqual([expect.objectContaining({ event: "MessageDisplay", reason: "Claude hook omitted index" })]);
  });
});
