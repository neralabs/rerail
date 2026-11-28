/** What each command a person runs says of a daemon of Rerail 0.1.4 or earlier, which a Windows lookup finds by its start. */
import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DaemonHealth, DaemonState } from "../../../src/runtime/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { stubUserHome } from "../../support/user-home.js";

/** What a Windows lookup hands on for a daemon of 0.1.4 or earlier. */
interface OlderFound { status: "older"; state: DaemonState }

/** What a start that ended such a daemon and launched this release's answers. */
interface ReplacingStart { started: true; daemon: { state: DaemonState; health: DaemonHealth }; replaced: Pick<DaemonState, "pid" | "version"> }

// Each lookup and start answers what a case sets, as a Windows lookup beside a daemon of 0.1.4 answers, and so does
// the record beside the home's lock of a daemon in its start (`inStart`, its pid).
const answers = vi.hoisted(() => ({
  lookup: undefined as OlderFound | undefined, start: undefined as ReplacingStart | undefined, inStart: undefined as number | undefined,
}));
vi.mock("../../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../../src/runtime/daemon-control.js")>("../../../src/runtime/daemon-control.js");
  return {
    ...actual,
    lookupDaemon: async () => answers.lookup,
    findRunningDaemon: async () => answers.lookup,
    startDaemon: async () => answers.start,
    homeDaemonInStart: () => answers.inStart,
  };
});

import { runAuditCommand } from "../../../src/cli/audit-cli.js";
import { collectSupportBundle } from "../../../src/cli/support-bundle.js";
import { defaultRerailDatabasePath, PACKAGE_ROOT } from "../../../src/engine/index.js";

/** A daemon of 0.1.4 as a lookup hands it on: its state holds no capability. Port 1: nothing answers there. */
const OLDER: DaemonState = {
  pid: 4242, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "0.1.4", databasePath: "/nowhere/rerail.sqlite",
  kernelSocketPath: "/nowhere/engine.sock", ui: "bundled", startedAt: 0, packageRoot: "/nowhere/rerail",
};
const OLDER_FOUND: OlderFound = { status: "older", state: OLDER };
const OLDER_LINE = "Rerail daemon pid 4242 runs Rerail 0.1.4, a release before 0.1.5, which Rerail sends nothing; run rerail daemon start to end it and start this release's daemon.";

let root: string;

beforeEach(async () => {
  root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-older-daemon-")));
  vi.stubEnv("RERAIL_HOME", root);
  stubUserHome(root);
  answers.lookup = OLDER_FOUND;
  answers.inStart = undefined;
});

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  await removeFolder(root);
});

/** Runs the CLI in this process with `args`, as `rerail` would: what it wrote to stdout and its exit code. */
async function rerail(args: readonly string[]): Promise<{ stdout: string; exit: string | number | null | undefined }> {
  const written: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation(text => {
    written.push(String(text));
    return true;
  });
  const argv = process.argv;
  process.argv = [process.execPath, "rerail", "--rerail-managed-child", ...args];
  try {
    vi.resetModules();
    await import("../../../src/cli/cli.js");
    return { stdout: written.join(""), exit: process.exitCode };
  } finally {
    process.argv = argv;
    process.exitCode = undefined;
  }
}

describe("a daemon of Rerail 0.1.4 or earlier", () => {
  it("is named by rerail status, which exits 4", { timeout: 60_000 }, async () => {
    const { stdout, exit } = await rerail(["status"]);
    expect(stdout.split("\n")).toContain(OLDER_LINE);
    expect(exit).toBe(4);
  });

  // A release before 0.1.5 holds no lock, so a daemon of this release can hold the home in its start beside it.
  it("is named by rerail status while a daemon in its start holds the home, never as that one still starting", { timeout: 60_000 }, async () => {
    answers.inStart = 4343;
    const { stdout, exit } = await rerail(["status"]);
    expect(stdout.split("\n")).toContain(OLDER_LINE);
    expect(stdout).not.toContain("is still starting");
    expect(exit).toBe(4);
  });

  it("is named by rerail daemon start once it ended it and started this release's daemon", { timeout: 60_000 }, async () => {
    const state: DaemonState = { ...OLDER, pid: 4343, version: "0.2.0" };
    const health: DaemonHealth = {
      status: "ok", pid: 4343, version: "0.2.0", ui: "bundled", uptimeMs: 1, database: { path: "/nowhere/rerail.sqlite", exists: true },
      native: { ok: true, kernelVersion: "1.0.0" }, model: { ok: true, message: "ok" }, healthy: true, workspaceRoot: root, installedAt: null, experiments: [], update: null,
    };
    answers.start = { started: true, daemon: { state, health }, replaced: { pid: 4242, version: "0.1.4" } };
    const { stdout, exit } = await rerail(["daemon", "start"]);
    expect(stdout.split("\n")[0]).toBe("Ended Rerail daemon pid 4242, which ran Rerail 0.1.4, a release before 0.1.5, and started this release's daemon in its place.");
    expect(exit).toBe(0);
  });

  it("is refused by the audit, which sends it nothing", async () => {
    await expect(runAuditCommand("scan", ["--session", "s1"], { home: root })).rejects.toThrow(new Error(OLDER_LINE));
  });

  it("is reported by the support bundle by the release its record names", { timeout: 60_000 }, async () => {
    execFileSync("git", ["init", "-q", "-b", "main", "repo"], { cwd: root });
    const { bundle } = await collectSupportBundle({
      repo: join(root, "repo"), home: root, databasePath: defaultRerailDatabasePath(), sessionId: undefined, packageVersion: "9.9.9", packageRoot: PACKAGE_ROOT,
    });
    expect(bundle.daemon).toEqual({ status: "older", version: "0.1.4", ui: "bundled" });
  });
});
