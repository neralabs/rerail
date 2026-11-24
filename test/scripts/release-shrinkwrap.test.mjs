// scripts/release/shrinkwrap.mjs: the npm-shrinkwrap.json core's tarball ships, from package-lock.json and the packed
// engine packages. scripts/release/shrinkwrap-pack.mjs: what `npm pack` writes and takes out again (write, remove), and
// the release's check of the packed tarball (check).

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENGINE_PACKAGES, ENGINE_PACKAGES_ENV, ENGINE_PACKAGES_FILE, enginePins, fileIntegrity, withEnginePins } from "../../scripts/release/engine/packages.mjs";
import { enginePinsProblem, shrinkwrapFrom, shrinkwrapProblem, shrinkwrapText } from "../../scripts/release/shrinkwrap.mjs";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const manifest = {
  name: "@neralabs/rerail",
  version: "1.2.0",
  dependencies: { a: "^1.0.0", orm: "^2.0.0" },
  optionalDependencies: { fsevents: "^2.0.0" },
};

const [macArm, , , linuxX64, , windowsX64] = ENGINE_PACKAGES;

/** The manifest of packed engine packages (engine/packages.mjs pack), with an integrity no tarball has. */
function packedEngines(packages = ENGINE_PACKAGES, version = manifest.version) {
  return { version, packages: packages.map(({ suffix, name }) => ({ suffix, name, version, file: `${suffix}.tgz`, integrity: `sha512-${Buffer.alloc(64, suffix).toString("base64")}` })) };
}

/** A workspace lockfile: core's dependencies, a dev tool, a pack workspace, a nested copy and an optional peer. */
function lockfile(rootDependencies = manifest.dependencies, rootOptionalDependencies = manifest.optionalDependencies) {
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
        optionalDependencies: rootOptionalDependencies,
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
    expect(shrinkwrap.packages["node_modules/a/node_modules/b"]).toEqual({ version: "2.0.0", resolved: "https://r/b2.tgz", integrity: "sha512-b2", gypfile: false });
    expect(shrinkwrap.packages["node_modules/fsevents"].optional).toBe(true);
    expect(shrinkwrap.packages["node_modules/a"]).not.toHaveProperty("optional");
  });

  // npm reads a shrinkwrapped dependency from its entry, and the entries npm writes have no `gypfile`. Left that way,
  // npm runs `node-gyp rebuild` for better-sqlite3, which ships prebuilt binaries and sets `gypfile: false` in its own
  // package.json, and the build fails on an image with no compiler, such as node:24-slim.
  it("tells npm to build no dependency from source, as the managed install runs no install script", () => {
    const shrinkwrap = shrinkwrapFrom(manifest, lockfile());
    for (const [location, entry] of Object.entries(shrinkwrap.packages)) {
      if (location === "") expect(entry).not.toHaveProperty("gypfile");
      else expect(entry.gypfile, location).toBe(false);
    }
    const repository = JSON.parse(shrinkwrapText(process.cwd()));
    expect(repository.packages["node_modules/better-sqlite3"].gypfile).toBe(false);
  });

  it("refuses a lockfile that is not package.json's, or lacks a package a dependency needs", () => {
    expect(() => shrinkwrapFrom(manifest, lockfile({ a: "^1.0.0" }))).toThrow("run npm install");
    const missing = lockfile();
    delete missing.packages["node_modules/a/node_modules/b"];
    delete missing.packages["node_modules/b"];
    expect(() => shrinkwrapFrom(manifest, missing)).toThrow("resolves no b for node_modules/a");
  });

  // package-lock.json cannot hold the engine packages: the release that packs core builds them. npm installs a
  // shrinkwrapped package's dependencies from its shrinkwrap alone, so an engine package it lacks is never installed.
  it("lists each packed engine package: an optional dependency of the root at its exact version, and its entry", () => {
    const engines = packedEngines([macArm, linuxX64]);
    const shrinkwrap = shrinkwrapFrom(manifest, lockfile(), engines);
    expect(shrinkwrap.packages[""].optionalDependencies).toEqual({ fsevents: "^2.0.0", [macArm.name]: "1.2.0", [linuxX64.name]: "1.2.0" });
    expect(shrinkwrap.packages[`node_modules/${linuxX64.name}`]).toEqual({
      version: "1.2.0", integrity: engines.packages[1].integrity, os: ["linux"], cpu: ["x64"], libc: ["glibc"], optional: true, gypfile: false,
    });
    expect(shrinkwrap.packages[`node_modules/${macArm.name}`]).toEqual({
      version: "1.2.0", integrity: engines.packages[0].integrity, os: ["darwin"], cpu: ["arm64"], optional: true, gypfile: false,
    });
    expect(Object.keys(shrinkwrap.packages)).toHaveLength(Object.keys(shrinkwrapFrom(manifest, lockfile()).packages).length + 2);
  });

  // With a `resolved` URL npm asks that URL's registry for the tarball, not the registry the scope is mapped to, so a
  // mirror or a local registry would install no engine, with exit code 0.
  it("gives an engine package's entry no resolved URL, so npm fetches it from the registry of core's scope", () => {
    const shrinkwrap = shrinkwrapFrom(manifest, lockfile(), packedEngines());
    for (const { name } of ENGINE_PACKAGES) expect(shrinkwrap.packages[`node_modules/${name}`], name).not.toHaveProperty("resolved");
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

describe("enginePinsProblem", () => {
  const core = { name: manifest.name, version: manifest.version, dependencies: manifest.dependencies };
  /** A packed core's shrinkwrap and package.json, parsed, pinning `engines`. */
  const packed = (engines = packedEngines(), lock = lockfile(core.dependencies, {})) => ({
    shrinkwrap: shrinkwrapFrom(core, lock, engines),
    packageJson: withEnginePins(core, engines),
  });
  const problem = ({ shrinkwrap, packageJson }, engines = packedEngines()) => enginePinsProblem(shrinkwrap, packageJson, engines);
  const located = ({ name }) => `node_modules/${name}`;

  it("passes a core that pins all six engine packages as they were packed", () => {
    expect(problem(packed())).toBeUndefined();
  });

  // The writer pins what the manifest lists, so a core packed in a checkout with one engine agrees with its manifest.
  it("refuses a core short of an engine, though it pins every package its manifest lists", () => {
    const five = packedEngines(ENGINE_PACKAGES.filter((enginePackage) => enginePackage !== windowsX64));
    expect(problem(packed(five), five)).toBe(`the engine packages lack ${windowsX64.name}: a release packs all 6`);
    expect(problem(packed(packedEngines([])), packedEngines([]))).toContain("a release packs all 6");
  });

  it("refuses engine packages of another version than the tarball's", () => {
    expect(problem(packed(), packedEngines(ENGINE_PACKAGES, "1.1.0"))).toBe("it is 1.2.0, and the engine packages are 1.1.0");
  });

  it("refuses a package.json that does not pin exactly the six, each at core's version", () => {
    const refusal = "its package.json's optionalDependencies are not the 6 engine packages at 1.2.0";
    const changed = (change) => {
      const core = packed();
      core.packageJson = { ...core.packageJson, optionalDependencies: change({ ...core.packageJson.optionalDependencies }) };
      return problem(core);
    };
    expect(changed(({ [windowsX64.name]: _dropped, ...pins }) => pins)).toBe(refusal);
    expect(changed((pins) => ({ ...pins, [macArm.name]: "^1.2.0" }))).toBe(refusal);
    expect(changed((pins) => ({ ...pins, fsevents: "^2.0.0" }))).toBe(refusal);
    expect(changed(() => undefined)).toBe(refusal);
  });

  it("refuses a shrinkwrap whose root does not name the same six", () => {
    const core = packed();
    delete core.shrinkwrap.packages[""].optionalDependencies[linuxX64.name];
    expect(problem(core)).toBe("its npm-shrinkwrap.json's root optionalDependencies are not the 6 engine packages at 1.2.0");
  });

  it("refuses an engine package's entry that is absent or differs in any field from the packed one, a resolved URL included", () => {
    const changed = (change) => {
      const core = packed();
      change(core.shrinkwrap.packages, located(linuxX64));
      return problem(core);
    };
    const refusal = `its npm-shrinkwrap.json does not pin ${linuxX64.name} as packed: version, integrity, os, cpu, libc, optional, and no resolved`;
    expect(changed((packages, location) => { delete packages[location]; })).toBe(refusal);
    expect(changed((packages, location) => { packages[location].resolved = `https://registry.npmjs.org/${linuxX64.name}/-/x.tgz`; })).toBe(refusal);
    expect(changed((packages, location) => { packages[location].version = "1.1.0"; })).toBe(refusal);
    expect(changed((packages, location) => { packages[location].integrity = packages[located(macArm)].integrity; })).toBe(refusal);
    expect(changed((packages, location) => { packages[location].os = ["darwin"]; })).toBe(refusal);
    expect(changed((packages, location) => { packages[location].cpu = ["arm64"]; })).toBe(refusal);
    expect(changed((packages, location) => { delete packages[location].libc; })).toBe(refusal);
    expect(changed((packages, location) => { delete packages[location].optional; })).toBe(refusal);
  });

  // The managed install passes --omit=optional and names its own engine package, so npm leaves out every other
  // optional package: nothing core needs may be one.
  it("refuses any other optional package, and an engine package anywhere but beside core's dependencies", () => {
    const lock = lockfile(core.dependencies, {});
    lock.packages["node_modules/a"].optionalDependencies = { fsevents: "^2.0.0" };
    expect(problem(packed(packedEngines(), lock))).toContain("has optional packages besides the engine packages (node_modules/fsevents)");
    const nested = packed();
    nested.shrinkwrap.packages[`node_modules/a/${located(macArm)}`] = { version: "1.2.0" };
    expect(problem(nested)).toContain(`(node_modules/a/${located(macArm)})`);
  });
});

/** Runs shrinkwrap-pack.mjs with RERAIL_ENGINE_PACKAGES naming `engines`, or unset. */
function shrinkwrapPack(args, engines) {
  const { [ENGINE_PACKAGES_ENV]: _inherited, ...environment } = process.env;
  const env = engines === undefined ? environment : { ...environment, [ENGINE_PACKAGES_ENV]: engines };
  return spawnSync(process.execPath, ["scripts/release/shrinkwrap-pack.mjs", ...args], { encoding: "utf8", env });
}

describe("shrinkwrap-pack.mjs write and remove", () => {
  const packageJson = `${JSON.stringify({ name: manifest.name, version: manifest.version, files: ["dist"], dependencies: {}, engines: { node: ">=24" } }, null, 2)}\n`;
  let root;
  let engines;

  /** Writes the manifest of packed engine packages the environment names. */
  const pack = (packages, version) => writeFileSync(join(engines, ENGINE_PACKAGES_FILE), JSON.stringify(packedEngines(packages, version)));
  const read = (file) => (existsSync(join(root, file)) ? readFileSync(join(root, file), "utf8") : undefined);

  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "rerail-shrinkwrap-pack-"));
    engines = join(root, "engines");
    mkdirSync(engines);
    writeFileSync(join(root, "package.json"), packageJson);
    writeFileSync(join(root, "package-lock.json"), JSON.stringify({ name: manifest.name, version: manifest.version, lockfileVersion: 3, requires: true, packages: { "": { name: manifest.name, version: manifest.version } } }));
  });
  afterEach(() => removeFolderSync(root));

  it("pins the packed engine packages in package.json and the shrinkwrap, and leaves the checkout as it was", () => {
    pack();
    const written = shrinkwrapPack(["write", root], engines);
    expect(written.stderr).toBe("");
    expect(written.status).toBe(0);
    const pins = enginePins(packedEngines());
    expect(JSON.parse(read("package.json")).optionalDependencies).toEqual(pins);
    const shrinkwrap = JSON.parse(read("npm-shrinkwrap.json"));
    expect(shrinkwrap.packages[""].optionalDependencies).toEqual(pins);
    expect(Object.keys(shrinkwrap.packages)).toEqual(["", ...ENGINE_PACKAGES.map(({ name }) => `node_modules/${name}`)]);

    expect(shrinkwrapPack(["remove", root]).status).toBe(0);
    expect(read("package.json")).toBe(packageJson);
    expect(read("npm-shrinkwrap.json")).toBeUndefined();
    // Twice, as after a pack that never wrote: nothing to remove is no failure.
    expect(shrinkwrapPack(["remove", root]).status).toBe(0);
    expect(read("package.json")).toBe(packageJson);
  });

  it("packs a checkout with no engine packages named, saying its install has no engine", () => {
    const written = shrinkwrapPack(["write", root]);
    expect(written.status).toBe(0);
    expect(written.stderr).toContain(`Packing core with no engine package (${ENGINE_PACKAGES_ENV} names no folder): its install has no engine, and the release's check refuses it.`);
    expect(read("package.json")).toBe(packageJson);
    expect(Object.keys(JSON.parse(read("npm-shrinkwrap.json")).packages)).toEqual([""]);
  });

  it("pins the engine packages a checkout packed, saying which platforms its install has no engine on", () => {
    pack([macArm]);
    const written = shrinkwrapPack(["write", root], engines);
    expect(written.status).toBe(0);
    expect(written.stderr).toContain("Packing core with 1 of 6 engine packages");
    expect(written.stderr).toContain(`lacks ${ENGINE_PACKAGES.slice(1).map(({ suffix }) => suffix).join(", ")}`);
    expect(JSON.parse(read("package.json")).optionalDependencies).toEqual({ [macArm.name]: "1.2.0" });
  });

  // A pack that fails after prepack runs no postpack, so the next one starts from a stamped package.json.
  it("writes again over the pins a failed pack left, and takes out the ones it no longer has", () => {
    pack();
    expect(shrinkwrapPack(["write", root], engines).status).toBe(0);
    pack([macArm]);
    expect(shrinkwrapPack(["write", root], engines).status).toBe(0);
    expect(JSON.parse(read("package.json")).optionalDependencies).toEqual({ [macArm.name]: "1.2.0" });
    expect(shrinkwrapPack(["write", root]).status).toBe(0);
    expect(read("package.json")).toBe(packageJson);
  });

  it("refuses engine packages of another version, or a folder without their manifest, and writes nothing", () => {
    pack(ENGINE_PACKAGES, "1.1.0");
    const stale = shrinkwrapPack(["write", root], engines);
    expect(stale.status).not.toBe(0);
    expect(stale.stderr).toContain("holds the engine packages of 1.1.0, and package.json is 1.2.0: pack them again");
    const empty = shrinkwrapPack(["write", root], join(root, "elsewhere"));
    expect(empty.status).not.toBe(0);
    expect(empty.stderr).toContain(`has no ${ENGINE_PACKAGES_FILE}`);
    expect(read("package.json")).toBe(packageJson);
    expect(read("npm-shrinkwrap.json")).toBeUndefined();
  });
});

describe("shrinkwrap-pack.mjs check", () => {
  const repository = JSON.parse(readFileSync("package.json", "utf8"));
  let folder;
  let engines;
  let tarball;

  /** Packs engine packages as `engine/packages.mjs pack` leaves them: a file each, and the manifest of their integrity. */
  function packEngines(packages = ENGINE_PACKAGES) {
    const entries = packages.map(({ suffix, name }) => {
      writeFileSync(join(engines, `${suffix}.tgz`), `engine ${suffix}`);
      return { suffix, name, version: repository.version, file: `${suffix}.tgz`, integrity: fileIntegrity(join(engines, `${suffix}.tgz`)) };
    });
    const packedManifest = { version: repository.version, packages: entries };
    writeFileSync(join(engines, ENGINE_PACKAGES_FILE), JSON.stringify(packedManifest));
    return packedManifest;
  }

  /** Packs the repository's core as `npm pack` does against `pinned`: its shrinkwrap and package.json alone. */
  function packCore(pinned) {
    writeFileSync(join(folder, "package", "npm-shrinkwrap.json"), shrinkwrapText(process.cwd(), pinned));
    writeFileSync(join(folder, "package", "package.json"), JSON.stringify(pinned === undefined ? repository : withEnginePins(repository, pinned)));
    // Relative names, so the tar that packs it reads no colon whatever the platform.
    execFileSync("tar", ["-czf", "rerail.tgz", "package"], { cwd: folder, stdio: "ignore" });
  }

  beforeEach(() => {
    folder = mkdtempSync(join(tmpdir(), "rerail-shrinkwrap-check-"));
    engines = join(folder, "engines");
    tarball = join(folder, "rerail.tgz");
    mkdirSync(engines);
    mkdirSync(join(folder, "package"));
  });
  afterEach(() => removeFolderSync(folder));

  it("passes a tarball named by its absolute path, a drive letter's colon and all, that pins the six packed engine packages", () => {
    packCore(packEngines());

    const check = shrinkwrapPack(["check", tarball], engines);

    expect(check.stderr).toBe("");
    expect(check.stdout).toBe(`${tarball} ships the npm-shrinkwrap.json package-lock.json gives, and pins the 6 engine packages in ${engines}.\n`);
    expect(check.status).toBe(0);
  });

  // The pins are one half of the check. The other holds every other entry to package-lock.json, as before the engine
  // packages: a dependency core ships at another version than the lockfile gives is refused with the pins right.
  it("fails a tarball that pins the six and ships another shrinkwrap than package-lock.json gives", () => {
    const pinned = packEngines();
    packCore(pinned);
    const shrinkwrap = JSON.parse(shrinkwrapText(process.cwd(), pinned));
    const dependency = Object.keys(shrinkwrap.packages).find((location) => location !== "" && shrinkwrap.packages[location].optional !== true);
    shrinkwrap.packages[dependency].version = "0.0.0-stale";
    writeFileSync(join(folder, "package", "npm-shrinkwrap.json"), JSON.stringify(shrinkwrap));
    execFileSync("tar", ["-czf", "rerail.tgz", "package"], { cwd: folder, stdio: "ignore" });

    const check = shrinkwrapPack(["check", tarball], engines);

    expect(check.status).toBe(1);
    expect(check.stderr).toContain("is not what package-lock.json gives");
  });

  it("fails a tarball packed in a checkout: with no engine package, or with only the ones the checkout built", () => {
    const all = packEngines();
    packCore(undefined);
    const none = shrinkwrapPack(["check", tarball], engines);
    expect(none.status).toBe(1);
    expect(none.stderr).toBe(`${tarball}: its package.json's optionalDependencies are not the 6 engine packages at ${repository.version}\n`);

    packCore({ ...all, packages: all.packages.slice(0, 1) });
    expect(shrinkwrapPack(["check", tarball], engines).status).toBe(1);

    packCore(packEngines(ENGINE_PACKAGES.slice(0, 5)));
    const five = shrinkwrapPack(["check", tarball], engines);
    expect(five.status).toBe(1);
    expect(five.stderr).toBe(`${tarball}: the engine packages lack ${windowsX64.name}: a release packs all 6\n`);
  });

  it("fails when no folder of engine packages is named, the folder has no manifest, or an engine package is not the tarball core pins", () => {
    packCore(packEngines());
    const unnamed = shrinkwrapPack(["check", tarball]);
    expect(unnamed.status).toBe(1);
    expect(unnamed.stderr).toContain(`${ENGINE_PACKAGES_ENV} names no folder`);
    const empty = shrinkwrapPack(["check", tarball], folder);
    expect(empty.status).toBe(1);
    expect(empty.stderr).toBe(`${tarball}: ${folder} has no ${ENGINE_PACKAGES_FILE}: node scripts/release/engine/packages.mjs pack --out ${folder} writes it\n`);

    writeFileSync(join(engines, `${linuxX64.suffix}.tgz`), "an engine built again");
    const rebuilt = shrinkwrapPack(["check", tarball], engines);
    expect(rebuilt.status).toBe(1);
    expect(rebuilt.stderr).toContain(`is not the tarball ${ENGINE_PACKAGES_FILE} lists for ${linuxX64.name}`);
  });
});
