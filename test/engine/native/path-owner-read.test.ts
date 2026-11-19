// The engine's owner read lets a repository start place each worktree without Git on Windows. An engine built before it,
// or a read that throws, sends every placement back to Git: the first such failure is written to the daemon's log.
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NativeBinding } from "../../../src/engine/native/native.js";

const platform = Object.getOwnPropertyDescriptor(process, "platform")!;

afterEach(() => {
  Object.defineProperty(process, "platform", platform);
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
});

type OwnerRead = NativeBinding["pathsOwnedByCurrentUser"];

/**
 * `pathsOwnedByCurrentUser` of a fresh copy of the module, as a new process has it, asked as on Windows with the loaded
 * engine's owner read replaced by `ownerRead` (absent when undefined); returns each answer and what it wrote to stderr.
 */
async function askedOnWindows(ownerRead: OwnerRead, asks: ReadonlyArray<readonly string[]>): Promise<{ answers: Array<boolean | undefined>; reports: string[] }> {
  vi.resetModules();
  const native = await import("../../../src/engine/native/native.js");
  // The engine of this machine stays the one loaded while the platform reads as Windows.
  vi.stubEnv("RERAIL_ENGINE_PATH", native.resolveNativeBindingPath());
  const binding = native.loadNativeBinding();
  const had = Object.getOwnPropertyDescriptor(binding, "pathsOwnedByCurrentUser");
  const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  Object.defineProperty(binding, "pathsOwnedByCurrentUser", { value: ownerRead, configurable: true, writable: true });
  Object.defineProperty(process, "platform", { ...platform, value: "win32" });
  try {
    const answers = asks.map(paths => native.pathsOwnedByCurrentUser(paths));
    return { answers, reports: written.mock.calls.map(([text]) => String(text)) };
  } finally {
    Object.defineProperty(process, "platform", platform);
    if (had === undefined) Reflect.deleteProperty(binding, "pathsOwnedByCurrentUser");
    else Object.defineProperty(binding, "pathsOwnedByCurrentUser", had);
  }
}

const REPOSITORY = ["C:\\repo", "C:\\repo\\.git"];

describe("the engine's owner read", () => {
  it("leaves the owners to Git with an engine built before the read, and says so once", async () => {
    const { answers, reports } = await askedOnWindows(undefined, [REPOSITORY, REPOSITORY]);
    expect(answers).toEqual([undefined, undefined]);
    expect(reports).toEqual([expect.stringContaining("built before it could read a path's owner")]);
  });

  it("leaves the owners to Git when the read throws, and says why once", async () => {
    const read: OwnerRead = () => {
      throw new Error("the owner read failed");
    };
    const { answers, reports } = await askedOnWindows(read, [REPOSITORY, REPOSITORY]);
    expect(answers).toEqual([undefined, undefined]);
    expect(reports).toEqual([expect.stringContaining("the owner read failed")]);
  });

  it("gives the engine's answer, and says nothing, when the read answers", async () => {
    const { answers, reports } = await askedOnWindows(paths => paths.length === 2, [REPOSITORY, ["C:\\repo"]]);
    expect({ answers, reports }).toEqual({ answers: [true, false], reports: [] });
  });
});
