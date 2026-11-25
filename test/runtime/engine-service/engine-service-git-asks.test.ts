/**
 * The engine service's two asks of Git's config, which it starts without waiting as it starts (`askGitWithoutBlocking`),
 * once its close or its start fails. Here each ask runs until its signal is aborted, as a Git slow to answer does; a
 * real ask ends its Git as its signal is aborted and settles once that Git has exited (`runGitAsync`), so an ask whose
 * run has ended leaves no Git running.
 */
import { mkdtemp } from "node:fs/promises";
import { Server } from "node:http";
import { Server as NetServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** A Git run's answer, as `runGitAsync` gives it. */
interface GitAnswer {
  status: number | null;
  stdout: string;
  stderr: string;
  error?: Error;
}
type RunGitAsync = (cwd: string, args: readonly string[], options?: { signal?: AbortSignal }) => Promise<GitAnswer>;

/** The owner lock, as `holdOwnerLock` gives it. */
interface OwnerLock {
  readonly id: string | undefined;
  release(): void;
}

/** How many asks began and how many runs of them ended; `failRelease` has the owner lock's release throw once it let go. */
const asks = vi.hoisted(() => ({ begun: 0, ended: 0, failRelease: false }));

vi.mock("../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<{ runGitAsync: RunGitAsync }>();
  return {
    ...actual,
    runGitAsync: async (...ask: Parameters<RunGitAsync>): Promise<GitAnswer> => {
      const [, args, options] = ask;
      // The asks run outside any repository: `--git-dir=` the null device.
      if (!args.some(arg => arg.startsWith("--git-dir="))) return await actual.runGitAsync(...ask);
      const signal = options?.signal;
      // As a real ask: one whose signal is aborted starts no Git.
      if (signal?.aborted !== false) return { status: null, stdout: "", stderr: "", error: new Error("Git was not started: the read was stopped") };
      asks.begun += 1;
      await new Promise(ended => signal.addEventListener("abort", ended, { once: true }));
      asks.ended += 1;
      return { status: null, stdout: "", stderr: "", error: new Error("Git was ended: the read was stopped") };
    },
  };
});

vi.mock("../../../src/engine/audit/owner-lock.js", async importOriginal => {
  const actual = await importOriginal<{ holdOwnerLock: (databasePath: string) => OwnerLock }>();
  return {
    ...actual,
    holdOwnerLock: (databasePath: string): OwnerLock => {
      const lock = actual.holdOwnerLock(databasePath);
      return {
        id: lock.id,
        release: () => {
          lock.release();
          if (asks.failRelease) throw new Error("the owner lock's file could not be let go");
        },
      };
    },
  };
});

const roots: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  Object.assign(asks, { begun: 0, ended: 0, failRelease: false });
  await Promise.all(roots.splice(0).map(async root => await removeFolder(root)));
});

/** The engine service and Git's asks afresh, with no ask under way, and options for a service in a folder of its own. */
async function fresh() {
  vi.resetModules();
  const root = await mkdtemp(join(tmpdir(), "rerail-git-asks-"));
  roots.push(root);
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-git-asks-${process.pid}-${roots.length}-${Date.now()}` : join(root, "engine.sock");
  const { startEngineService } = await import("../../../src/runtime/engine-service.js");
  return { startEngineService, options: { socketPath, storageRoot: join(root, "state"), databasePath: join(root, "rerail.sqlite"), embedder: testEmbedder } };
}

describe("the engine service's Git asks, when its close or its start fails", () => {
  it("ends them though its server's close failed", async () => {
    const { startEngineService, options } = await fresh();
    const service = await startEngineService(options);
    expect(asks).toMatchObject({ begun: 1, ended: 0 });
    const closeOf = Server.prototype.close;
    vi.spyOn(Server.prototype, "close").mockImplementationOnce(function (this: Server, closed?: (error?: Error) => void) {
      return closeOf.call(this, () => closed?.(new Error("the server's close failed")));
    });
    await expect(service.close()).rejects.toThrow("the server's close failed");
    expect(asks).toMatchObject({ begun: 1, ended: 1 });
  });

  it("ends them though its listen failed and so did what its start undoes after it", async () => {
    const { startEngineService, options } = await fresh();
    vi.spyOn(NetServer.prototype, "listen").mockImplementationOnce(function (this: NetServer) {
      process.nextTick(() => this.emit("error", Object.assign(new Error("listen EACCES: permission denied"), { code: "EACCES" })));
      return this;
    });
    asks.failRelease = true;
    await expect(startEngineService(options)).rejects.toThrow("the owner lock's file could not be let go");
    expect(asks).toMatchObject({ begun: 1, ended: 1 });
  });
});
