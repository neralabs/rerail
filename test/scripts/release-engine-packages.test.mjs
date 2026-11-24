// scripts/release/engine/packages.mjs: the npm packages that carry the native engine, one per operating system and
// processor, the manifest of the packed ones, and core's pins on them.

import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { engineFileName } from "../../scripts/build/pull-engine.mjs";
import {
  absentEnginePackages,
  ENGINE_PACKAGES,
  ENGINE_PACKAGES_FILE,
  engineLockEntry,
  enginePackageJson,
  enginePins,
  engineTarballsProblem,
  fileIntegrity,
  packEnginePackages,
  readEngineManifest,
  withEnginePins,
  withoutEnginePins,
} from "../../scripts/release/engine/packages.mjs";
import { NATIVE_ENGINE_SYSTEMS, nativeEnginePackageName, nativePlatformSuffix } from "../../src/engine/index.ts";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const core = JSON.parse(readFileSync("package.json", "utf8"));
const [macArm, , , linuxX64] = ENGINE_PACKAGES;

/** A manifest entry as `pack` writes it, with an integrity no tarball has. */
const entry = ({ suffix, name }, version = "1.2.0") => ({ suffix, name, version, file: `${suffix}.tgz`, integrity: `sha512-${Buffer.alloc(64, suffix).toString("base64")}` });
const manifest = (packages = ENGINE_PACKAGES, version = "1.2.0") => ({ version, packages: packages.map((enginePackage) => entry(enginePackage, version)) });

describe("ENGINE_PACKAGES", () => {
  it("has one package for each engine the release builds, under the name and suffix the runtime loads the engine by", () => {
    expect(ENGINE_PACKAGES.map(({ target }) => target).sort()).toEqual([...core.napi.targets].sort());
    for (const { suffix, os, cpu, engine, name } of ENGINE_PACKAGES) {
      expect(nativePlatformSuffix(os, cpu), suffix).toBe(suffix);
      expect(nativeEnginePackageName(os, cpu), suffix).toBe(name);
      expect(engine, suffix).toBe(engineFileName(os, cpu));
    }
    expect(NATIVE_ENGINE_SYSTEMS).toEqual(ENGINE_PACKAGES.map(({ os, cpu }) => [os, cpu]));
  });

  // npm filters on `os`, `cpu` and `libc`, so an engine labelled for another platform is never installed where it runs.
  it("labels each package with the operating system, processor and C library of the target its engine is built for", () => {
    const processors = { aarch64: "arm64", x86_64: "x64" };
    const systems = { "apple-darwin": "darwin", "unknown-linux-gnu": "linux", "pc-windows-msvc": "win32" };
    for (const { target, os, cpu, libc } of ENGINE_PACKAGES) {
      const [processor, ...system] = target.split("-");
      expect({ os, cpu }, target).toEqual({ os: systems[system.join("-")], cpu: processors[processor] });
      expect(libc, target).toBe(os === "linux" ? "glibc" : undefined);
    }
  });

  it("names each package after core and its suffix, in a family of its own", () => {
    const names = ENGINE_PACKAGES.map(({ name }) => name);
    expect(names).toEqual(ENGINE_PACKAGES.map(({ suffix }) => `${core.name}-engine-${suffix}`));
    expect(new Set(names).size).toBe(6);
  });
});

describe("enginePackageJson", () => {
  it("is core's version, licence, repository and Node range, and what lets npm skip it on another platform", () => {
    expect(enginePackageJson(linuxX64, core)).toEqual({
      name: linuxX64.name,
      version: core.version,
      license: "Apache-2.0",
      repository: core.repository,
      os: ["linux"],
      cpu: ["x64"],
      libc: ["glibc"],
      engines: { node: core.engines.node },
      publishConfig: { access: "public" },
    });
    expect(enginePackageJson(macArm, core)).not.toHaveProperty("libc");
  });
});

describe("packEnginePackages", () => {
  let folder;
  let native;
  let out;
  let packed;

  beforeAll(() => {
    folder = mkdtempSync(join(tmpdir(), "rerail-engine-packages-"));
    native = join(folder, "native");
    out = join(folder, "out");
    mkdirSync(native);
    for (const { engine } of [macArm, linuxX64]) writeFileSync(join(native, engine), `engine ${engine}`);
    packed = packEnginePackages({ root: process.cwd(), native, out });
  });
  afterAll(() => removeFolderSync(folder));

  it("packs the engines the folder holds, and says which it lacks", () => {
    expect(packed.manifest.packages.map(({ name }) => name)).toEqual([macArm.name, linuxX64.name]);
    expect(packed.absent).toEqual(ENGINE_PACKAGES.filter((enginePackage) => enginePackage !== macArm && enginePackage !== linuxX64));
    expect(absentEnginePackages(packed.manifest)).toEqual(packed.absent);
  });

  it("puts the engine, its package.json, a README and the licence and notices in each tarball, and nothing else", () => {
    const { file } = packed.manifest.packages[1];
    // Relative names: Git for Windows' GNU tar reads the `C:` of an absolute path as a remote host.
    const tar = (...args) => execFileSync("tar", [...args], { cwd: out, encoding: "utf8" });
    expect(tar("-tzf", file).trim().split(/\r?\n/u).sort()).toEqual(
      ["LICENSE", "NOTICE", "README.md", "THIRD_PARTY_NOTICES", "package.json", linuxX64.engine].map((name) => `package/${name}`).sort(),
    );
    expect(JSON.parse(tar("-xzOf", file, "package/package.json"))).toEqual(enginePackageJson(linuxX64, core));
    expect(tar("-xzOf", file, `package/${linuxX64.engine}`)).toBe(`engine ${linuxX64.engine}`);
    expect(tar("-xzOf", file, "package/README.md")).toContain(`# ${linuxX64.name}\n\nThe native engine of [Rerail](${core.homepage}) for Linux with glibc on x64.\n`);
  });

  it("writes a manifest of each tarball: its package, core's version, its file and the integrity npm's registry gives it", () => {
    expect(readEngineManifest(out)).toEqual(packed.manifest);
    expect(packed.manifest.version).toBe(core.version);
    expect(packed.manifest.packages[0]).toEqual({
      suffix: macArm.suffix,
      name: macArm.name,
      version: core.version,
      file: `neralabs-rerail-engine-${macArm.suffix}-${core.version}.tgz`,
      integrity: fileIntegrity(join(out, packed.manifest.packages[0].file)),
    });
    expect(packed.manifest.packages[0].integrity).toMatch(/^sha512-[A-Za-z0-9+/]{86}==$/u);
  });

  it("finds a tarball that is not the one the manifest lists, or is gone", () => {
    expect(engineTarballsProblem(out, packed.manifest)).toBeUndefined();
    const copy = join(folder, "copy");
    mkdirSync(copy);
    writeFileSync(join(copy, packed.manifest.packages[0].file), "another tarball");
    expect(engineTarballsProblem(copy, packed.manifest)).toContain(`is not the tarball ${ENGINE_PACKAGES_FILE} lists for ${macArm.name}`);
    rmSync(join(copy, packed.manifest.packages[0].file));
    expect(engineTarballsProblem(copy, packed.manifest)).toContain(`has no ${packed.manifest.packages[0].file}`);
  });

  it("refuses a release short of an engine, and a folder with none", () => {
    expect(() => packEnginePackages({ root: process.cwd(), native, out: join(folder, "all"), all: true })).toThrow(/lacks rerail_engine\.darwin-x64\.node, .*: a release packs all 6 engines/u);
    expect(() => packEnginePackages({ root: process.cwd(), native: join(folder, "none"), out: join(folder, "all") })).toThrow("holds no engine");
  });

  it("takes pack and a folder to write, and nothing it does not know", () => {
    const run = (...args) => spawnSync(process.execPath, ["scripts/release/engine/packages.mjs", ...args], { encoding: "utf8" });
    for (const args of [[], ["pack"], ["publish", "--out", out], ["pack", "--out", out, "--force"]]) {
      const refused = run(...args);
      expect(refused.status, args.join(" ")).toBe(2);
      expect(refused.stderr, args.join(" ")).toContain("usage: engine/packages.mjs pack --out <folder> [--native <folder>] [--all]");
    }
    const short = run("pack", "--out", join(folder, "all"), "--native", native, "--all");
    expect(short.status).toBe(1);
    expect(short.stderr).toContain("a release packs all 6 engines");
  });
});

describe("readEngineManifest", () => {
  let folder;
  beforeAll(() => {
    folder = mkdtempSync(join(tmpdir(), "rerail-engine-manifest-"));
  });
  afterAll(() => removeFolderSync(folder));

  const reading = (written) => {
    writeFileSync(join(folder, ENGINE_PACKAGES_FILE), JSON.stringify(written));
    return () => readEngineManifest(folder);
  };

  it("reads the manifest pack wrote, complete or not", () => {
    expect(reading(manifest())()).toEqual(manifest());
    expect(reading(manifest([linuxX64]))().packages).toHaveLength(1);
  });

  it("refuses a folder without one, naming the command that writes it", () => {
    expect(() => readEngineManifest(join(folder, "empty"))).toThrow(/has no engine-packages\.json: node scripts\/release\/engine\/packages\.mjs pack --out/u);
  });

  it("refuses a manifest pack does not write", () => {
    const changed = (change) => {
      const written = manifest();
      change(written.packages);
      return reading(written);
    };
    expect(reading({ packages: [] })).toThrow("no version or no packages");
    expect(changed((packages) => { packages[0].name = "@neralabs/rerail-pack-react"; })).toThrow("@neralabs/rerail-pack-react is not an engine package");
    expect(changed((packages) => { packages[1] = packages[0]; })).toThrow(`lists ${macArm.name} twice`);
    expect(changed((packages) => { packages[0].suffix = linuxX64.suffix; })).toThrow(`${macArm.name}'s suffix is ${linuxX64.suffix}, not ${macArm.suffix}`);
    expect(changed((packages) => { packages[0].version = "1.1.0"; })).toThrow(`${macArm.name} is 1.1.0, not 1.2.0`);
    expect(changed((packages) => { packages[0].file = "../elsewhere.tgz"; })).toThrow("names no file in the folder");
    expect(changed((packages) => { packages[0].integrity = "sha1-abc"; })).toThrow("has no sha512 integrity");
  });
});

describe("core's pins on the engine packages", () => {
  const packageJson = { name: "@neralabs/rerail", version: "1.2.0", files: ["dist"], dependencies: { a: "^1.0.0" }, engines: { node: ">=24" } };

  it("pins each packed engine package at its exact version, as optional dependencies after the dependencies", () => {
    const pinned = withEnginePins(packageJson, manifest([macArm, linuxX64]));
    expect(pinned.optionalDependencies).toEqual({ [macArm.name]: "1.2.0", [linuxX64.name]: "1.2.0" });
    expect(Object.keys(pinned)).toEqual(["name", "version", "files", "dependencies", "optionalDependencies", "engines"]);
    expect(enginePins(manifest())).toEqual(Object.fromEntries(ENGINE_PACKAGES.map(({ name }) => [name, "1.2.0"])));
  });

  it("takes the pins out again and leaves the package.json it started from, another optional dependency included", () => {
    expect(JSON.stringify(withoutEnginePins(withEnginePins(packageJson, manifest())))).toBe(JSON.stringify(packageJson));
    expect(withoutEnginePins(packageJson)).toBe(packageJson);
    const withOther = { ...packageJson, optionalDependencies: { fsevents: "^2.0.0" } };
    expect(withEnginePins(withOther, manifest([macArm])).optionalDependencies).toEqual({ fsevents: "^2.0.0", [macArm.name]: "1.2.0" });
    expect(JSON.stringify(withoutEnginePins(withEnginePins(withOther, manifest())))).toBe(JSON.stringify(withOther));
    // Packed again against other engine packages, as after a pack that failed before restoring: no pin is left over.
    expect(withEnginePins(withEnginePins(packageJson, manifest()), manifest([macArm])).optionalDependencies).toEqual({ [macArm.name]: "1.2.0" });
  });

  it("gives each a shrinkwrap entry npm filters by platform and fetches from the scope's registry", () => {
    expect(engineLockEntry(entry(linuxX64))).toEqual({ version: "1.2.0", integrity: entry(linuxX64).integrity, os: ["linux"], cpu: ["x64"], libc: ["glibc"], optional: true });
    expect(engineLockEntry(entry(macArm))).toEqual({ version: "1.2.0", integrity: entry(macArm).integrity, os: ["darwin"], cpu: ["arm64"], optional: true });
  });

  // Packing writes package.json again from its parsed form to stamp the pins and to take them out. Written as npm
  // writes it, the file comes back byte for byte, and the checkout shows no change after a pack.
  it("leaves the repository's package.json as it is: no pin committed, no engine among its files, and npm's formatting", () => {
    const text = readFileSync("package.json", "utf8");
    expect(`${JSON.stringify(JSON.parse(text), null, 2)}\n`).toBe(text.replaceAll("\r\n", "\n"));
    expect(withoutEnginePins(core)).toBe(core);
    expect(core.files.filter((path) => path.endsWith(".node"))).toEqual([]);
    expect(core.files).toContain("native/src/code_rule_dsl/capabilities.json");
  });
});
