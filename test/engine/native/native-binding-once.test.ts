import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { nativeGitAccess } from "../../../src/engine/native/git-access.js";
import { loadNativeBinding, nativeHealth } from "../../../src/engine/native/native.js";

vi.mock("node:fs", { spy: true });

const CALLS = 1_000;

/** The looks on the disk for an engine file (`existsSync` on a `.node` path) since the spy was last cleared. */
function looksSinceCleared(): number {
  return vi.mocked(existsSync).mock.calls.filter(([path]) => String(path).endsWith(".node")).length;
}

/** The looks for an engine file over `CALLS` calls of `call`, after its first. */
function engineFileLooks(call: () => unknown): number {
  call();
  vi.mocked(existsSync).mockClear();
  for (let made = 0; made < CALLS; made += 1) call();
  return looksSinceCleared();
}

afterEach(() => {
  vi.unstubAllEnvs();
});

// Every caller of the engine reaches it through loadNativeBinding: a walk asks it for each file and each ignore question.
describe("the engine's file", () => {
  it.each([
    ["loadNativeBinding()", () => loadNativeBinding()],
    ["nativeGitAccess()", () => nativeGitAccess()],
    ["nativeHealth()", () => nativeHealth()],
  ])("is not looked for on the disk at any call of %s after the first", (_name, call) => {
    expect(engineFileLooks(call)).toBe(0);
  });

  it("is looked for at each call while RERAIL_ENGINE_PATH names a missing file, and the engine loaded before is kept", () => {
    const binding = loadNativeBinding();
    vi.stubEnv("RERAIL_ENGINE_PATH", join(tmpdir(), "rerail-no-engine-here.node"));
    vi.mocked(existsSync).mockClear();
    expect(() => loadNativeBinding()).toThrow(/RERAIL_ENGINE_PATH does not exist/);
    expect(() => loadNativeBinding()).toThrow(/RERAIL_ENGINE_PATH does not exist/);
    expect(looksSinceCleared()).toBe(2);
    vi.unstubAllEnvs();
    vi.mocked(existsSync).mockClear();
    // Node's module cache hands back the same exports for the same path, so only the count shows the engine was kept.
    expect(loadNativeBinding()).toBe(binding);
    expect(looksSinceCleared()).toBe(0);
  });
});
