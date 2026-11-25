import { existsSync, mkdtempSync, statSync, type PathLike, type StatSyncOptions } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { packCompileChecks } from "../../../src/engine/catalog/pack-compile-checks.js";
import { nativeGitAccess } from "../../../src/engine/native/git-access.js";
import { isWorkspaceBusy, loadNativeBinding, nativeHealth, nativePlatformSuffix, resolveNativeBindingPath } from "../../../src/engine/native/native.js";
import { PACKAGE_ROOT } from "../../../src/engine/package-root.js";
import { linkFolder } from "../../support/platform/links.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

vi.mock("node:fs", { spy: true });

const actual = await vi.importActual<typeof import("node:fs")>("node:fs");

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

const homes: string[] = [];

afterEach(() => {
  vi.unstubAllEnvs();
  vi.mocked(existsSync).mockReset();
  vi.mocked(statSync).mockReset();
  for (const home of homes.splice(0)) removeFolderSync(home);
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

  it("is not looked for where a retry asks whether its error was the busy refusal, though RERAIL_ENGINE_PATH has changed", () => {
    const busy = loadNativeBinding().WORKSPACE_BUSY;
    vi.stubEnv("RERAIL_ENGINE_PATH", join(tmpdir(), "rerail-no-engine-here.node"));
    vi.mocked(existsSync).mockClear();
    expect(isWorkspaceBusy(new Error(busy))).toBe(true);
    expect(looksSinceCleared()).toBe(0);
  });
});

const PACK = "a pack";
const VERDICT = { problem: null, notRun: [] };

/** A Rerail home that keeps a verdict on `PACK` under the engine build of the engine this process has loaded. */
function homeWithVerdict(): string {
  loadNativeBinding();
  const home = mkdtempSync(join(tmpdir(), "rerail-engine-stamp-"));
  homes.push(home);
  const checks = packCompileChecks(home);
  checks.record(PACK, VERDICT);
  checks.save();
  return home;
}

/**
 * The builds `rebuiltAt` has faked. Each reads as a file of its own: a loader that read one into its record in a case
 * that failed must not find that time and size again in a later case.
 */
let builds = 0;

/** `statSync`, but reading `path` as a file a build wrote after this process loaded `held`: a later time, another size. */
function rebuiltAt(path: string, held: string): typeof statSync {
  builds += 1;
  const rebuilt = actual.statSync(held);
  rebuilt.mtimeMs += 1_000 * builds;
  rebuilt.size += builds;
  return ((file: PathLike, options?: StatSyncOptions) => (String(file) === path ? rebuilt : actual.statSync(file, options))) as typeof statSync;
}

// The catalog keeps the engine's verdicts on each pack under a stamp of the engine's file (pack-compile-checks.ts): a
// stamp of a file other than the engine this process runs would hand that engine's verdicts to another engine's processes.
describe("the catalog's engine stamp", () => {
  it("names the file this process loaded though a look on the disk now finds another one first", () => {
    const home = homeWithVerdict();
    const held = resolveNativeBindingPath();
    const named = join(PACKAGE_ROOT, "native", `rerail_engine.${nativePlatformSuffix()}.node`);
    // A checkout's build writes the name of its platform, first in the look; where that is the file loaded, the look
    // finds the other name of native/ instead.
    const found = held === named ? join(PACKAGE_ROOT, "native", "rerail_engine.node") : named;
    vi.mocked(existsSync).mockImplementation(path => String(path) === found || (String(path) !== held && actual.existsSync(path)));
    vi.mocked(statSync).mockImplementation(rebuiltAt(found, held));
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });

  it("names the file as this process loaded it though a build has replaced it at its path since", () => {
    const home = homeWithVerdict();
    const held = resolveNativeBindingPath();
    vi.mocked(statSync).mockImplementation(rebuiltAt(held, held));
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });

  it("moves with the engine where RERAIL_ENGINE_PATH changes, and comes back with it", () => {
    const home = homeWithVerdict();
    vi.stubEnv("RERAIL_ENGINE_PATH", join(tmpdir(), "rerail-no-engine-here.node"));
    expect(() => loadNativeBinding()).toThrow(/RERAIL_ENGINE_PATH does not exist/);
    expect(packCompileChecks(home).verdict(PACK)).toBeUndefined();
    vi.unstubAllEnvs();
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });

  // Node loads a file under one name once, and hands back that engine at every later load of the name.
  it("names the file as this process first loaded it where an engine loaded through RERAIL_ENGINE_PATH goes, though a build has replaced the file", () => {
    const home = homeWithVerdict();
    const held = resolveNativeBindingPath();
    vi.stubEnv("RERAIL_ENGINE_PATH", held);
    loadNativeBinding();
    vi.mocked(statSync).mockImplementation(rebuiltAt(held, held));
    vi.unstubAllEnvs();
    loadNativeBinding();
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });

  it("names the file as this process first loaded it while an engine loaded through RERAIL_ENGINE_PATH is held and the variable is gone", () => {
    const home = homeWithVerdict();
    const held = resolveNativeBindingPath();
    vi.stubEnv("RERAIL_ENGINE_PATH", held);
    loadNativeBinding();
    vi.mocked(statSync).mockImplementation(rebuiltAt(held, held));
    vi.unstubAllEnvs();
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });

  it("names the file as this process first loaded it where RERAIL_ENGINE_PATH reaches it through a link, though a build has replaced it", () => {
    const home = homeWithVerdict();
    const binding = loadNativeBinding();
    const held = resolveNativeBindingPath();
    linkFolder(dirname(held), join(home, "engine"));
    const linked = join(home, "engine", basename(held));
    vi.stubEnv("RERAIL_ENGINE_PATH", linked);
    vi.mocked(statSync).mockImplementation(rebuiltAt(linked, held));
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
    // Node names a module by its file with every link resolved, so the engine it loaded first comes back.
    expect(loadNativeBinding()).toBe(binding);
    expect(packCompileChecks(home).verdict(PACK)).toEqual(VERDICT);
  });
});
