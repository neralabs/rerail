import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { nativeEnginePackageName, nativeTrayFileName, resolveNativeTrayPath, type NativeEngineSite } from "../../../src/engine/native/native.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

const VERSION = "1.2.0";
type System = readonly [platform: NodeJS.Platform, architecture: string];
const LINUX: System = ["linux", "x64"];
const on = (packageRoot: string, [platform, architecture]: System): NativeEngineSite => ({ packageRoot, platform, architecture });

/** Rerail installed at `<prefix>/node_modules/@neralabs/rerail`, at VERSION. */
function installedRerail(): { prefix: string; packageRoot: string } {
  // macOS keeps its temporary folder behind a symlink; the paths compared are the real ones.
  const prefix = realpathSync(mkdtempSync(join(tmpdir(), "rerail-tray-path-")));
  roots.push(prefix);
  const packageRoot = join(prefix, "node_modules", "@neralabs", "rerail");
  mkdirSync(packageRoot, { recursive: true });
  writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: VERSION }));
  return { prefix, packageRoot };
}

/** The engine package of `system` beside Rerail at `version`, with its tray program unless `tray` is false; returns the tray's path. */
function enginePackage(prefix: string, system: System, { version = VERSION, tray = true } = {}): string {
  const root = join(prefix, "node_modules", ...nativeEnginePackageName(...system).split("/"));
  mkdirSync(root, { recursive: true });
  writeFileSync(join(root, "package.json"), JSON.stringify({ name: nativeEnginePackageName(...system), version }));
  if (tray) writeFileSync(join(root, nativeTrayFileName(...system)), "");
  return join(root, nativeTrayFileName(...system));
}

describe("nativeTrayFileName", () => {
  it("is the engine's platform suffix, with .exe on Windows", () => {
    expect(nativeTrayFileName("win32", "x64")).toBe("rerail-tray.win32-x64-msvc.exe");
    expect(nativeTrayFileName("darwin", "arm64")).toBe("rerail-tray.darwin-arm64");
    expect(nativeTrayFileName("linux", "x64")).toBe("rerail-tray.linux-x64-gnu");
  });
});

describe("the tray program Rerail runs", () => {
  it("is the one in the package's own native/ before the engine package's, as a checkout's build is", () => {
    const { prefix, packageRoot } = installedRerail();
    enginePackage(prefix, LINUX);
    mkdirSync(join(packageRoot, "native"));
    writeFileSync(join(packageRoot, "native", nativeTrayFileName(...LINUX)), "");
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBe(join(packageRoot, "native", nativeTrayFileName(...LINUX)));
  });

  it("is the one in the system's engine package, where native/ has none", () => {
    const { prefix, packageRoot } = installedRerail();
    const tray = enginePackage(prefix, LINUX);
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBe(tray);
  });

  it("is none, never an error, where neither has one, or the engine package is another version's", () => {
    const { prefix, packageRoot } = installedRerail();
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBeUndefined();
    enginePackage(prefix, LINUX, { tray: false });
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBeUndefined();
    enginePackage(prefix, LINUX, { version: "9.9.9" });
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBeUndefined();
    // A Rerail whose package.json does not parse, beside an engine package of its version.
    enginePackage(prefix, LINUX);
    writeFileSync(join(packageRoot, "package.json"), "{");
    expect(resolveNativeTrayPath(on(packageRoot, LINUX))).toBeUndefined();
  });
});
