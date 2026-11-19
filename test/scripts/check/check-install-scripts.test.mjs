// scripts/check/check-install-scripts.mjs: a dependency that ships its addon prebuilt has its install scripts denied,
// for the version installed, so `npm ci` in a checkout compiles nothing.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { installedAddons, installScriptProblems } from "../../../scripts/check/check-install-scripts.mjs";

/** A lockfile with `sqlite` (prebuilt, gypfile false), `native` (built from source) and `plain` (no addon). */
const lock = {
  lockfileVersion: 3,
  packages: {
    "": { name: "app" },
    "node_modules/sqlite": { version: "13.0.3" },
    "node_modules/native": { version: "2.0.0" },
    "node_modules/plain": { version: "1.0.0" },
    "node_modules/pack": { resolved: "packs/x", link: true },
  },
};

/** What each location holds on disk: its own package.json, a binding.gyp, a prebuilds folder. */
function installed(overrides = {}) {
  const packages = {
    "node_modules/sqlite": { manifest: { name: "sqlite", version: "13.0.3", gypfile: false }, bindingGyp: true, prebuilds: true },
    "node_modules/native": { manifest: { name: "native", version: "2.0.0", scripts: { install: "node-gyp rebuild" } }, bindingGyp: true, prebuilds: false },
    "node_modules/plain": { manifest: { name: "plain", version: "1.0.0", gypfile: false }, bindingGyp: false, prebuilds: false },
    ...overrides,
  };
  return location => packages[location];
}

describe("installScriptProblems", () => {
  it("passes a deny keyed to the installed version of each prebuilt addon, and leaves other packages alone", () => {
    expect(installScriptProblems({ "sqlite@13.0.3": false, native: true }, lock, installed())).toEqual([]);
  });

  it("fails an approval, which lets npm run node-gyp rebuild for the binding.gyp", () => {
    for (const allowScripts of [{ sqlite: true }, { "sqlite@13.0.3": true }]) {
      expect(installScriptProblems(allowScripts, lock, installed())).toEqual([
        expect.stringMatching(/allowScripts approves sqlite@13\.0\.3.*"sqlite@13\.0\.3": false/su),
      ]);
    }
  });

  it("fails a prebuilt addon allowScripts leaves out, or denies at another version, as after a bump", () => {
    expect(installScriptProblems({}, lock, installed())).toEqual([expect.stringContaining('"sqlite@13.0.3": false')]);
    expect(installScriptProblems({ "sqlite@13.0.2": false }, lock, installed())).toEqual([expect.stringContaining('"sqlite@13.0.3": false')]);
  });

  it("counts a package without prebuilds, without a binding.gyp, or not installed as no prebuilt addon", () => {
    const none = installed({
      "node_modules/sqlite": { manifest: { name: "sqlite", version: "13.0.3", gypfile: false }, bindingGyp: true, prebuilds: false },
    });
    expect(installedAddons(lock, none)).toEqual([]);
    expect(installedAddons(lock, installed({ "node_modules/sqlite": undefined }))).toEqual([]);
    expect(installedAddons(lock, installed())).toEqual([{ name: "sqlite", version: "13.0.3" }]);
  });

  it("passes this repository's package.json and installed node_modules", () => {
    const manifest = JSON.parse(readFileSync("package.json", "utf8"));
    const repositoryLock = JSON.parse(readFileSync("package-lock.json", "utf8"));
    expect(installScriptProblems(manifest.allowScripts ?? {}, repositoryLock)).toEqual([]);
  });
});
