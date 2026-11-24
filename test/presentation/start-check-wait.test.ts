import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readRepositoryState, readRerailSummary } from "../../src/presentation/session-presentation.js";
import { testEngineEndpoint } from "../support/platform/engine-endpoint.js";
import { removeFolder } from "../support/platform/remove-folder.js";

// Each read of the recorded daemon answers silent, as a busy daemon's engine does; a case reads which read was asked.
const reads = vi.hoisted(() => ({ command: vi.fn(async () => "silent"), event: vi.fn(async () => "silent") }));
vi.mock("../../src/runtime/daemon-control.js", async importOriginal => ({
  ...(await importOriginal<Record<string, unknown>>()),
  recordedDaemonStatus: reads.command,
  recordedDaemonStatusForEvent: reads.event,
}));

let home: string;

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), "rerail-start-check-"));
  await mkdir(join(home, "workspace"), { recursive: true });
  await writeFile(join(home, "daemon.json"), JSON.stringify({
    pid: process.pid, host: "127.0.0.1", port: 4737, url: "http://127.0.0.1:4737/", version: "test", databasePath: join(home, "rerail.sqlite"),
    kernelSocketPath: testEngineEndpoint(home), ui: "bundled", startedAt: 1, packageRoot: home,
  }));
  vi.stubEnv("RERAIL_HOME", home);
});

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.clearAllMocks();
  await removeFolder(home);
});

describe("the start check a hook makes before its event", () => {
  // On Windows a hook's read waits for a free instance of a busy daemon's engine pipe as main's hook did, so its event
  // reaches a daemon that frees one; `/rerail` and the status line read as a command, within the proof's limit.
  it("reads the recorded daemon as a hook does, where /rerail reads it as a command", async () => {
    const cwd = join(home, "workspace");
    expect(await readRepositoryState(cwd, "claude-code")).toBeNull();
    expect(reads.event).toHaveBeenCalledTimes(1);
    expect(reads.command).not.toHaveBeenCalled();
    // Cursor names no session: `/rerail` reads the repository's state.
    const summary = await readRerailSummary({ host: "cursor", sessionId: undefined, cwd, startState: () => "started" });
    expect(summary.repositoryState).toBe("not-answering");
    expect(reads.command).toHaveBeenCalledTimes(1);
    expect(reads.event).toHaveBeenCalledTimes(1);
  });
});
