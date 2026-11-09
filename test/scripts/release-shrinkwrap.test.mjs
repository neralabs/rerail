// scripts/release/shrinkwrap.mjs: the npm-shrinkwrap.json core's tarball ships, from package-lock.json, and the
// release's check of the packed one.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { shrinkwrapFrom, shrinkwrapProblem, shrinkwrapText } from "../../scripts/release/shrinkwrap.mjs";

const manifest = {
  name: "@neralabs/rerail",
  version: "1.2.0",
  dependencies: { a: "^1.0.0", orm: "^2.0.0" },
  optionalDependencies: { fsevents: "^2.0.0" },
};

/** A workspace lockfile: core's dependencies, a dev tool, a pack workspace, a nested copy and an optional peer. */
function lockfile(rootDependencies = manifest.dependencies) {
  return {
    name: "@neralabs/rerail",
    version: "1.1.0",
    lockfileVersion: 3,
    requires: true,
    packages: {
      "": {
        name: "@neralabs/rerail",
        version: "1.1.0",
        workspaces: ["packs/*"],
        dependencies: rootDependencies,
        optionalDependencies: manifest.optionalDependencies,
        devDependencies: { tool: "^1.0.0" },
      },
      "node_modules/@neralabs/rerail-pack-x": { resolved: "packs/x", link: true },
      "node_modules/a": { version: "1.0.0", resolved: "https://r/a.tgz", integrity: "sha512-a", dependencies: { b: "^2.0.0" } },
      "node_modules/a/node_modules/b": { version: "2.0.0", resolved: "https://r/b2.tgz", integrity: "sha512-b2" },
      "node_modules/b": { version: "1.0.0", resolved: "https://r/b1.tgz", integrity: "sha512-b1", dev: true },
      "node_modules/fsevents": { version: "2.3.0", resolved: "https://r/f.tgz", integrity: "sha512-f", optional: true, os: ["darwin"] },
      "node_modules/orm": {
        version: "2.0.0", resolved: "https://r/o.tgz", integrity: "sha512-o",
        peerDependencies: { types: "*" }, peerDependenciesMeta: { types: { optional: true } },
      },
      "node_modules/tool": { version: "1.0.0", resolved: "https://r/t.tgz", integrity: "sha512-t", dev: true, dependencies: { b: "^1.0.0" } },
      "node_modules/types": { version: "1.0.0", resolved: "https://r/ty.tgz", integrity: "sha512-ty", devOptional: true },
      "packs/x": { name: "@neralabs/rerail-pack-x", version: "0.1.0", dependencies: { a: "^1.0.0" } },
    },
  };
}

describe("shrinkwrapFrom", () => {
  it("keeps what core's dependencies reach, as npm resolves them, and nothing the workspace adds", () => {
    const shrinkwrap = shrinkwrapFrom(manifest, lockfile());
    expect(shrinkwrap).toMatchObject({ name: "@neralabs/rerail", version: "1.2.0", lockfileVersion: 3 });
    expect(Object.keys(shrinkwrap.packages).sort()).toEqual(["", "node_modules/a", "node_modules/a/node_modules/b", "node_modules/fsevents", "node_modules/orm"]);
    expect(shrinkwrap.packages[""]).not.toHaveProperty("devDependencies");
    expect(shrinkwrap.packages[""]).not.toHaveProperty("workspaces");
    expect(shrinkwrap.packages[""].version).toBe("1.2.0");
    expect(shrinkwrap.packages["node_modules/a/node_modules/b"]).toEqual({ version: "2.0.0", resolved: "https://r/b2.tgz", integrity: "sha512-b2" });
    expect(shrinkwrap.packages["node_modules/fsevents"].optional).toBe(true);
    expect(shrinkwrap.packages["node_modules/a"]).not.toHaveProperty("optional");
  });

  it("refuses a lockfile that is not package.json's, or lacks a package a dependency needs", () => {
    expect(() => shrinkwrapFrom(manifest, lockfile({ a: "^1.0.0" }))).toThrow("run npm install");
    const missing = lockfile();
    delete missing.packages["node_modules/a/node_modules/b"];
    delete missing.packages["node_modules/b"];
    expect(() => shrinkwrapFrom(manifest, missing)).toThrow("resolves no b for node_modules/a");
  });

  it("gives the repository's lockfile's production packages, every one and no other", () => {
    const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
    const shrinkwrap = JSON.parse(shrinkwrapText(process.cwd()));
    const production = Object.entries(lock.packages)
      .filter(([location, entry]) => location.startsWith("node_modules/") && !entry.dev && !entry.devOptional && !entry.link)
      .map(([location]) => location);
    expect(Object.keys(shrinkwrap.packages).filter(location => location !== "").sort()).toEqual(production.sort());
  });
});

describe("shrinkwrapProblem", () => {
  const expected = `${JSON.stringify(shrinkwrapFrom(manifest, lockfile()), null, 2)}\n`;

  it("passes the shrinkwrap package-lock.json gives", () => {
    expect(shrinkwrapProblem(expected, expected)).toBeUndefined();
  });

  it("fails a tarball without one, and names the packages a stale one gets wrong", () => {
    expect(shrinkwrapProblem(undefined, expected)).toContain("has no npm-shrinkwrap.json");
    const stale = JSON.parse(expected);
    stale.packages["node_modules/a"].version = "0.9.0";
    expect(shrinkwrapProblem(JSON.stringify(stale), expected)).toContain("node_modules/a");
    expect(shrinkwrapProblem("not json", expected)).toBeDefined();
  });
});
