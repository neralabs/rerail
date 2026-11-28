// scripts/release/engine/release.mjs: the engine packages' part of a core release. Which core versions have engine
// packages, the six published and read back before core, the names the dist-tag steps move with core, the refusal of
// a retry that would build the engines again, and the tarballs an install check serves.

import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { ENGINE_PACKAGES, ENGINE_PACKAGES_FILE, fileIntegrity } from "../../scripts/release/engine/packages.mjs";
import {
  ENGINE_PACKAGES_SINCE,
  enginePackageFor,
  engineSpecs,
  hasEnginePackages,
  publishedEnginePackages,
  publishEnginePackages,
  rebuildRefusal,
  releaseTarballs,
} from "../../scripts/release/engine/release.mjs";
import { linkFolder } from "../support/platform/links.ts";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const SCRIPT = join("scripts", "release", "engine", "release.mjs");
const [macArm, macIntel] = ENGINE_PACKAGES;

const folders = [];
afterEach(() => {
  for (const folder of folders.splice(0)) removeFolderSync(folder);
});
const temporary = () => {
  const folder = mkdtempSync(join(tmpdir(), "rerail-engine-release-"));
  folders.push(folder);
  return folder;
};

/** A manifest entry as `pack` writes it, with an integrity no tarball has. */
const entry = ({ suffix, name }, version) => ({ suffix, name, version, file: `${suffix}.tgz`, integrity: `sha512-${Buffer.alloc(64, suffix).toString("base64")}` });
const manifestOf = (packages = ENGINE_PACKAGES, version = "1.2.0") => ({ version, packages: packages.map((enginePackage) => entry(enginePackage, version)) });

/** A folder as `pack` leaves it: a stand-in tarball per package, the manifest giving each one's real integrity. */
function packedFolder(packages = ENGINE_PACKAGES, version = "1.2.0") {
  const folder = temporary();
  const manifest = manifestOf(packages, version);
  for (const packed of manifest.packages) {
    writeFileSync(join(folder, packed.file), `the engine for ${packed.suffix}`);
    packed.integrity = fileIntegrity(join(folder, packed.file));
  }
  writeFileSync(join(folder, ENGINE_PACKAGES_FILE), JSON.stringify(manifest));
  return { folder, manifest };
}

describe("hasEnginePackages", () => {
  it("is true from the first release with engine packages on, its previews included, and false for every version before", () => {
    expect(ENGINE_PACKAGES_SINCE).toBe("0.2.1");
    for (const version of ["0.2.1", "0.2.1-next.1", "0.2.1-beta.1", "0.2.2", "0.3.0-next.1", "1.0.0"]) expect(hasEnginePackages(version), version).toBe(true);
    for (const version of ["0.2.0", "0.2.0-next.4", "0.1.5", "0.1.0"]) expect(hasEnginePackages(version), version).toBe(false);
  });
});

describe("engineSpecs", () => {
  it("names the six engine packages at core's version, under a dist-tag when one is given", () => {
    expect(engineSpecs("0.2.1")).toEqual(ENGINE_PACKAGES.map(({ name }) => `${name}@0.2.1`));
    expect(engineSpecs("0.2.1-next.1", "next")).toEqual(ENGINE_PACKAGES.map(({ name }) => `${name}@0.2.1-next.1:next`));
  });

  it("names none for a version from before the engine packages, so a put-back to one moves core alone", () => {
    expect(engineSpecs("0.1.5")).toEqual([]);
    expect(engineSpecs("0.2.0", "latest")).toEqual([]);
  });

  it("prints them on one line for a workflow step, and an empty line for a version with none", () => {
    const specs = (...args) => spawnSync(process.execPath, [SCRIPT, "specs", ...args], { encoding: "utf8" });
    expect(specs("0.2.1", "next").stdout).toBe(`${ENGINE_PACKAGES.map(({ name }) => `${name}@0.2.1:next`).join(" ")}\n`);
    expect(specs("0.1.5")).toMatchObject({ status: 0, stdout: "\n" });
    expect(spawnSync(process.execPath, [SCRIPT, "specs"], { encoding: "utf8" }).status).toBe(2);
  });
});

describe("enginePackageFor", () => {
  it("is the package whose engine a machine loads, and none where Rerail has no engine", () => {
    for (const enginePackage of ENGINE_PACKAGES) expect(enginePackageFor(enginePackage.os, enginePackage.cpu)).toBe(enginePackage);
    expect(enginePackageFor("freebsd", "x64")).toBeUndefined();
    expect(enginePackageFor("linux", "riscv64")).toBeUndefined();
  });
});

describe("releaseTarballs", () => {
  it("lists the engine packages packed beside core's tarball, then core, which pins them", () => {
    const { folder, manifest } = packedFolder([macArm, macIntel]);
    const core = join(folder, "neralabs-rerail-1.2.0.tgz");
    expect(releaseTarballs(core)).toEqual([...manifest.packages.map(({ file }) => join(folder, file)), core]);
  });

  it("refuses a folder with no manifest, and one whose tarball is absent or is not the one packed", () => {
    expect(() => releaseTarballs(join(temporary(), "neralabs-rerail-1.2.0.tgz"))).toThrow(`has no ${ENGINE_PACKAGES_FILE}`);
    const { folder, manifest } = packedFolder([macArm, macIntel]);
    writeFileSync(join(folder, manifest.packages[1].file), "another build's engine");
    expect(() => releaseTarballs(join(folder, "neralabs-rerail-1.2.0.tgz"))).toThrow(`is not the tarball ${ENGINE_PACKAGES_FILE} lists for ${macIntel.name}`);
  });
});

describe("publishedEnginePackages and rebuildRefusal", () => {
  it("counts a version a dist-tag points at as on npm without reading it, since the dist-tags see a publish first", async () => {
    const asked = [];
    const read = {
      distTags: async (name) => (name === macArm.name ? { latest: "0.0.0", next: "1.2.0" } : { latest: "0.0.0" }),
      manifest: async (name) => {
        asked.push(name);
        return name === macIntel.name ? { name } : null;
      },
    };
    expect(await publishedEnginePackages("1.2.0", read)).toEqual([macArm.name, macIntel.name]);
    expect(asked).not.toContain(macArm.name);
    expect(asked).toHaveLength(5);
  });

  it("finds none where npm has no such package or no such version, and stops on a registry it could not read", async () => {
    expect(await publishedEnginePackages("1.2.0", { distTags: async () => null, manifest: async () => null })).toEqual([]);
    const unreadable = { distTags: async () => ({}), manifest: async () => Promise.reject(new Error("answers an anonymous request with 503")) };
    await expect(publishedEnginePackages("1.2.0", unreadable)).rejects.toThrow("503");
  });

  it("refuses a retry while npm has an engine package of the version, saying what to do instead", () => {
    expect(rebuildRefusal("1.2.0", [])).toBeNull();
    const refusal = rebuildRefusal("1.2.0", [macArm.name, macIntel.name]);
    expect(refusal).toContain(`npm already has ${macArm.name}, ${macIntel.name} at 1.2.0`);
    expect(refusal).toContain("re-run the failed publish job of the run that published them");
    expect(refusal).toContain("release the next version");
  });
});

describe("publishEnginePackages", () => {
  const CONFLICT = { ok: false, output: "npm error 403 You cannot publish over the previously published versions: 1.2.0." };
  const packedAs = (manifest, name) => manifest.packages.find((packed) => packed.name === name).integrity;

  /**
   * Steps that record what happens in order, over a registry that serves a version once it is published: `onNpm` is
   * what it holds beforehand, by package name. `publish(file)` and `served(name, reading)` (the integrity npm serves at
   * that reading of the package, null for none yet) take the registry's place where a case needs another answer.
   */
  function steps({ unknown = [], onNpm = {}, publish, served, manifest }) {
    const events = [];
    const readings = new Map();
    const registry = new Map(Object.entries(onNpm));
    return {
      events,
      exists: async (name) => {
        events.push(`exists ${name}`);
        return !unknown.includes(name);
      },
      publish: (file, distTag) => {
        events.push(`publish ${file} ${distTag}`);
        if (publish !== undefined) return publish(file);
        const { name, integrity } = manifest.packages.find((packed) => packed.file === file);
        if (registry.has(name)) return CONFLICT;
        registry.set(name, integrity);
        return { ok: true, output: "" };
      },
      readIntegrity: async (name, version) => {
        const reading = (readings.get(name) ?? 0) + 1;
        readings.set(name, reading);
        events.push(`read ${name}@${version}`);
        return served === undefined ? (registry.get(name) ?? null) : served(name, reading, manifest);
      },
      wait: async () => {
        events.push("wait");
      },
      log: () => {},
    };
  }
  const kinds = (events) => events.map((event) => event.split(" ")[0]);

  it("asks npm for every name, reads what it has, publishes the six under core's dist-tag, and returns only once each is read back as packed", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest });
    await publishEnginePackages(manifest, "next", run);
    expect(kinds(run.events)).toEqual([...Array(6).fill("exists"), ...Array(6).fill("read"), ...Array(6).fill("publish"), ...Array(6).fill("read")]);
    expect(run.events.filter((event) => event.startsWith("publish"))).toEqual(manifest.packages.map(({ file }) => `publish ${file} next`));
    expect(run.events.filter((event) => event.startsWith("read")).slice(6)).toEqual(ENGINE_PACKAGES.map(({ name }) => `read ${name}@1.2.0`));
  });

  it("publishes nothing when the manifest lacks an engine package: core would install with no engine there", async () => {
    const manifest = manifestOf(ENGINE_PACKAGES.slice(1));
    const run = steps({ manifest });
    await expect(publishEnginePackages(manifest, "next", run)).rejects.toThrow(`the manifest lacks ${macArm.name}`);
    expect(run.events).toEqual([]);
  });

  it("publishes nothing when npm has never had one of the names, and names each for its first publish by hand", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest, unknown: [macIntel.name, ENGINE_PACKAGES[5].name] });
    await expect(publishEnginePackages(manifest, "next", run)).rejects.toThrow(`First publish needed: npm has no public package named ${macIntel.name}, ${ENGINE_PACKAGES[5].name}.`);
    expect(kinds(run.events)).toEqual(Array(6).fill("exists"));
  });

  it("stops at a publish that fails, before the packages after it and before any read-back", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest, served: () => null, publish: (file) => (file === manifest.packages[2].file ? { ok: false, output: "npm error code E403" } : { ok: true, output: "" }) });
    await expect(publishEnginePackages(manifest, "next", run)).rejects.toThrow(`npm publish ${manifest.packages[2].name}@1.2.0 failed`);
    expect(kinds(run.events)).toEqual([...Array(6).fill("exists"), ...Array(6).fill("read"), ...Array(3).fill("publish")]);
  });

  it("publishes, on a re-run, only the versions npm does not have, and reads all six back", async () => {
    const manifest = manifestOf();
    const earlier = manifest.packages.slice(0, 3);
    const run = steps({ manifest, onNpm: Object.fromEntries(earlier.map(({ name, integrity }) => [name, integrity])) });
    await publishEnginePackages(manifest, "next", run);
    expect(run.events.filter((event) => event.startsWith("publish"))).toEqual(manifest.packages.slice(3).map(({ file }) => `publish ${file} next`));
    expect(kinds(run.events).filter((kind) => kind === "read")).toHaveLength(12);
  });

  it("takes a version npm refuses as already published for published, where the registry did not serve it yet, and still reads it back", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest, publish: () => CONFLICT, served: (name, reading, packed) => (reading === 1 ? null : packedAs(packed, name)) });
    await publishEnginePackages(manifest, "next", run);
    expect(kinds(run.events)).toEqual([...Array(6).fill("exists"), ...Array(6).fill("read"), ...Array(6).fill("publish"), ...Array(6).fill("read")]);
  });

  // A run whose jobs were all run again has rebuilt its tarballs. Were the versions npm lacks published before this
  // is noticed, they would stay on npm beside the earlier attempt's, and no core could pin all six.
  it("publishes nothing when npm has a version as other bytes than this run packed, as after a rebuild", async () => {
    const manifest = manifestOf();
    const other = `sha512-${Buffer.alloc(64, "rebuilt").toString("base64")}`;
    const run = steps({ manifest, onNpm: { [macIntel.name]: other } });
    const failure = await publishEnginePackages(manifest, "next", run).catch((error) => error.message);
    expect(failure).toContain(`npm has ${macIntel.name}@1.2.0 as ${other}, and this run packed ${manifest.packages[1].integrity}`);
    // Whether core of the version is on npm is not asked, so the refusal covers both: an attempt that finished, and one that did not.
    expect(failure).toContain("Where core 1.2.0 is already on npm, it is released with the bytes npm has, and this run is not needed. Where it is not, release the next version");
    expect(kinds(run.events)).not.toContain("publish");
    expect(run.events).not.toContain("wait");
  });

  it("reads again a version the registry does not serve yet, and only that one", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest, served: (name, reading, packed) => (reading === 1 || (name === macArm.name && reading < 4) ? null : packedAs(packed, name)) });
    await publishEnginePackages(manifest, "next", run);
    const reads = run.events.filter((event) => event.startsWith("read") || event === "wait");
    expect(reads.slice(12)).toEqual(["wait", `read ${macArm.name}@1.2.0`, "wait", `read ${macArm.name}@1.2.0`]);
  });

  it("fails when the registry never serves a version: unread is not the same as published as packed", async () => {
    const manifest = manifestOf();
    const run = steps({ manifest, served: (name, reading, packed) => (reading === 1 || name === macArm.name ? null : packedAs(packed, name)) });
    await expect(publishEnginePackages(manifest, "next", run)).rejects.toThrow(`npm does not serve ${macArm.name}@1.2.0 yet`);
    // Once before anything is published, then the read-back's twenty.
    expect(run.events.filter((event) => event === `read ${macArm.name}@1.2.0`)).toHaveLength(21);
  });
});

describe("engine/release.mjs publish", () => {
  const publish = (folder, env) => spawnSync(process.execPath, [SCRIPT, "publish", folder], { encoding: "utf8", env: { ...process.env, RELEASE_VERSION: "", RELEASE_CHANNEL: "", ...env } });

  // Each of these is refused from the folder alone, before the registry is asked anything.
  it("refuses, before it asks npm anything, a folder of another version than the run releases", () => {
    const { folder } = packedFolder();
    const run = publish(folder, { RELEASE_VERSION: "1.2.1", RELEASE_CHANNEL: "next" });
    expect(run.status).toBe(1);
    expect(run.stdout).toContain("::error title=Engine packages::");
    expect(run.stdout).toContain("holds the engine packages of 1.2.0, and the run releases 1.2.1. This run publishes no core.");
  });

  // Node gives a module its real path. Compared with the path as typed, a script started through a link would run no
  // command and exit 0, and release.yml publishes core on that exit code alone.
  it("runs its command, and refuses with exit code 1, when started through a link to the checkout", () => {
    const linked = join(temporary(), "checkout");
    linkFolder(process.cwd(), linked);
    const script = join(linked, SCRIPT);
    const specs = spawnSync(process.execPath, [script, "specs", "1.2.0", "next"], { encoding: "utf8" });
    expect(specs.stdout.trim().split(" ")).toEqual(ENGINE_PACKAGES.map(({ name }) => `${name}@1.2.0:next`));
    const run = spawnSync(process.execPath, [script, "publish", join(linked, "no-such-folder")], { encoding: "utf8", env: { ...process.env, RELEASE_VERSION: "1.2.0", RELEASE_CHANNEL: "next" } });
    expect(run.status).toBe(1);
    expect(run.stdout).toContain(`has no ${ENGINE_PACKAGES_FILE}`);
  });

  // package.yml and the install checks import this file from a script given with `-e`, whose first argument is theirs.
  it("runs no command, and does not fail, when a script that imports it has a first argument that names no file", () => {
    const run = spawnSync(process.execPath, ["--input-type=module", "-e", `const { engineSpecs } = await import(${JSON.stringify(pathToFileURL(SCRIPT).href)}); console.log(engineSpecs("1.2.0").length);`, "1.2.0"], { encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.stdout).toBe(`${ENGINE_PACKAGES.length}\n`);
  });

  it("refuses a tarball that is not the one the manifest lists, a folder with fewer than six, and a run that names no channel", () => {
    const { folder, manifest } = packedFolder();
    writeFileSync(join(folder, manifest.packages[0].file), "another build's engine");
    expect(publish(folder, { RELEASE_VERSION: "1.2.0", RELEASE_CHANNEL: "next" }).stdout).toContain(`is not the tarball ${ENGINE_PACKAGES_FILE} lists for ${macArm.name}`);
    const few = packedFolder(ENGINE_PACKAGES.slice(0, 5));
    const run = publish(few.folder, { RELEASE_VERSION: "1.2.0", RELEASE_CHANNEL: "next" });
    expect(run.status).toBe(1);
    expect(run.stdout).toContain(`the manifest lacks ${ENGINE_PACKAGES[5].name}`);
    expect(publish(packedFolder().folder, { RELEASE_VERSION: "1.2.0" }).stdout).toContain("RELEASE_VERSION and RELEASE_CHANNEL name what the run releases");
  });
});
