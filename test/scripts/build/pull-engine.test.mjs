// scripts/build/pull-engine.mjs: a released native engine for a checkout without Rust, the package it is fetched from,
// and the warning when the engine's sources changed since that release.

import { describe, expect, it } from "vitest";
import { ENGINE_SOURCES, engineFileName, engineSource, staleWarning } from "../../../scripts/build/pull-engine.mjs";
import { ENGINE_PACKAGES } from "../../../scripts/release/engine/packages.mjs";

describe("engineFileName", () => {
  it("names the file the release packs for each supported platform (package.yml's list)", () => {
    const pairs = [["darwin", "arm64"], ["darwin", "x64"], ["linux", "x64"], ["linux", "arm64"], ["win32", "x64"], ["win32", "arm64"]];
    expect(pairs.map(([platform, arch]) => engineFileName(platform, arch))).toEqual([
      "rerail_engine.darwin-arm64.node",
      "rerail_engine.darwin-x64.node",
      "rerail_engine.linux-x64-gnu.node",
      "rerail_engine.linux-arm64-gnu.node",
      "rerail_engine.win32-x64-msvc.node",
      "rerail_engine.win32-arm64-msvc.node",
    ]);
  });
});

describe("engineSource", () => {
  /** A core's optional dependencies as its release pins them: every engine package at core's own version. */
  const pins = (version) => Object.fromEntries(ENGINE_PACKAGES.map(({ name }) => [name, version]));

  it("is the platform's engine package, at the version core pins, for a core that names its engine packages", () => {
    for (const { os, cpu, name, engine } of ENGINE_PACKAGES) {
      expect(engineSource("0.2.1", pins("0.2.1"), os, cpu), name).toEqual({ spec: `${name}@0.2.1`, path: `package/${engine}` });
    }
  });

  it("is core's own tarball for a release from before the engine packages, which names none", () => {
    expect(engineSource("0.1.5", {}, "linux", "x64")).toEqual({ spec: "@neralabs/rerail@0.1.5", path: "package/native/rerail_engine.linux-x64-gnu.node" });
    expect(engineSource("0.1.5", {}, "win32", "arm64")).toEqual({ spec: "@neralabs/rerail@0.1.5", path: "package/native/rerail_engine.win32-arm64-msvc.node" });
  });
});

describe("staleWarning", () => {
  it("says nothing when the engine's sources are the release's", () => {
    expect(staleWarning("0.1.3", "")).toBeUndefined();
  });

  it("names the release and the changed sources, and how to build the checkout's own engine", () => {
    const stat = " native/src/process.rs | 101 ++++++++++++++++++++++++++++++++++++++++++++++++++++---\n 1 file changed, 96 insertions(+), 5 deletions(-)\n";
    const warning = staleWarning("0.1.3", stat);
    expect(warning).toContain("since v0.1.3");
    expect(warning).toContain("native/src/process.rs");
    expect(warning).toContain("npm run build:native");
  });

  it("compares everything the engine is built from", () => {
    expect(ENGINE_SOURCES).toEqual(["native/", "Cargo.toml", "Cargo.lock", "languages/"]);
  });
});
