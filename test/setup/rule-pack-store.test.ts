import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, utimesSync, writeFileSync } from "node:fs";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import semver from "semver";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadRuleCatalog } from "../../src/engine/index.js";
import { CommandTimeoutError, KILL_GRACE_MS, type NpmRunner, type Progress } from "../../src/runtime/index.js";
import {
  addPackSpecs,
  discoverPacks,
  installedPackagesOf,
  installedPackVersion,
  installPackVersions,
  newerPackVersions,
  npmPackStore,
  packsLeftOutByCore,
  parsePackVersions,
  updatePacks,
  type PackStore,
  type PackVersion,
} from "../../src/setup/index.js";
import { PACKS_LOCK_WAIT_MS } from "../../src/setup/packs-lock.js";
import { directoryPackStore } from "../support/directory-pack-store.js";
import { installPackDirectory, linkingPackStore, thirdPartyPack, uninstallPack } from "../support/installed-packs.js";
import { fixturesPath, repositoryPath } from "../support/oracles.js";
import { holdRemovals } from "../support/platform/held-removal.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";

// Spied on so a test can make Windows hold a run's staging folder for a moment.
vi.mock("node:fs/promises", { spy: true });
const { rm: actualRm } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");

const homes: string[] = [];
afterEach(() => {
  vi.mocked(rm).mockReset();
  for (const home of homes.splice(0)) removeFolderSync(home);
});

function temporaryHome(): string {
  const home = mkdtempSync(join(tmpdir(), "rerail-pack-store-"));
  homes.push(home);
  return home;
}

/** A fake npm that records each call's arguments and fails a call whose last spec `fail` has an error for. */
function fakeNpm(fail: (spec: string) => string | undefined = () => undefined): { npm: NpmRunner; calls: string[][] } {
  const calls: string[][] = [];
  const npm: NpmRunner = async args => {
    calls.push([...args]);
    const reason = fail(args.at(-1)!);
    if (reason !== undefined) throw new Error(reason);
    return { stdout: "", stderr: "" };
  };
  return { npm, calls };
}

/** Rerail's own typescript pack at `version`. */
const typescriptAt = (version: string): PackVersion => ({ packageName: "@neralabs/rerail-pack-typescript", id: "typescript", version });

/** The value of `--prefix` in one npm call. */
const prefixOf = (args: readonly string[]): string => args[args.indexOf("--prefix") + 1]!;
/** What follows `--`: the specs npm reads as packages, never as options. */
const specsOf = (args: readonly string[]): string[] => args.slice(args.indexOf("--") + 1);
/** Installs here are checked as the catalog checks them, compiling every rule of the TypeScript pack: slow under load. */
const COMPILING_TIMEOUT_MS = 30_000;

describe("npmPackStore", () => {
  it("fetches each pack under the tag into a staging folder of the run's own, without running install scripts, and removes it on release", async () => {
    const home = temporaryHome();
    const { npm, calls } = fakeNpm();

    const fetch = await npmPackStore(home, npm).fetch(["typescript", "complexity"], "latest");

    expect(fetch.problems).toEqual([]);
    const run = dirname(prefixOf(calls[0]!));
    expect(dirname(run)).toBe(join(home, "packs", "staging"));
    expect(fetch.fetched).toEqual([
      { id: "typescript", directory: join(run, "typescript", "node_modules", "@neralabs", "rerail-pack-typescript") },
      { id: "complexity", directory: join(run, "complexity", "node_modules", "@neralabs", "rerail-pack-complexity") },
    ]);
    // The installs run at once, and each reaches npm after its own setup, so they may call it in either order.
    expect(calls.map(prefixOf).sort()).toEqual([join(run, "complexity"), join(run, "typescript")]);
    for (const args of calls) {
      const id = basename(prefixOf(args));
      expect(args[0]).toBe("install");
      expect(args).toContain("--ignore-scripts");
      expect(specsOf(args)).toEqual([`@neralabs/rerail-pack-${id}@latest`]);
    }
    expect(existsSync(run)).toBe(true);
    await fetch.release();
    expect(existsSync(run)).toBe(false);
  });

  it("removes the run's staging folder on release while Windows holds a file in it for a moment", async () => {
    const { npm, calls } = fakeNpm();
    const fetch = await npmPackStore(temporaryHome(), npm).fetch(["typescript"], "latest");
    const run = dirname(prefixOf(calls[0]!));
    const refused = holdRemovals(vi.mocked(rm), actualRm, 2, path => path === run);

    await fetch.release();

    expect(refused).toEqual([run, run]);
    expect(existsSync(run)).toBe(false);
  });

  it("gives two fetches at once separate staging folders", async () => {
    const home = temporaryHome();
    const { npm, calls } = fakeNpm();
    const store = npmPackStore(home, npm);

    await Promise.all([store.fetch(["typescript"], "latest"), store.fetch(["typescript"], "latest")]);

    expect(new Set(calls.map(prefixOf)).size).toBe(2);
  });

  it("fetches a pack again under next when the tag has no version of it", async () => {
    const home = temporaryHome();
    const { npm, calls } = fakeNpm(spec => spec === "@neralabs/rerail-pack-typescript@latest"
      ? "npm error code ETARGET\nnpm error notarget No matching version found for @neralabs/rerail-pack-typescript@latest."
      : undefined);

    const { fetched, problems } = await npmPackStore(home, npm).fetch(["typescript"], "latest");

    expect(problems).toEqual([]);
    expect(fetched.map(pack => pack.id)).toEqual(["typescript"]);
    expect(calls.map(args => args.at(-1))).toEqual(["@neralabs/rerail-pack-typescript@latest", "@neralabs/rerail-pack-typescript@next"]);
    expect(prefixOf(calls[1]!)).toBe(prefixOf(calls[0]!));
  });

  it("reports a pack npm cannot find as a problem of that pack and still fetches the others", async () => {
    const notFound = "npm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/@neralabs%2frerail-pack-gone";
    const { npm, calls } = fakeNpm(spec => spec.startsWith("@neralabs/rerail-pack-gone@") ? notFound : undefined);

    const { fetched, problems } = await npmPackStore(temporaryHome(), npm).fetch(["typescript", "gone", "complexity"], "latest");

    expect(fetched.map(pack => pack.id)).toEqual(["typescript", "complexity"]);
    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-gone", reason: "npm cannot read @neralabs/rerail-pack-gone: it is not published, or the registry or token in ~/.npmrc refuses it" }]);
    // A 404 is not a missing version under the tag, so it is not fetched again under next.
    expect(calls.filter(args => args.at(-1)!.startsWith("@neralabs/rerail-pack-gone@"))).toHaveLength(1);
  });

  it("says in one line when npm cannot reach the registry", async () => {
    const offline = "npm error code ECONNREFUSED\nnpm error syscall connect\nnpm error FetchError: request to https://registry.npmjs.org/ failed";
    const { npm } = fakeNpm(() => offline);

    const { problems } = await npmPackStore(temporaryHome(), npm).fetch(["typescript"], "latest");

    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-typescript", reason: "could not reach the npm registry (https://registry.npmjs.org, ECONNREFUSED): check your network, proxy, or registry in ~/.npmrc" }]);
  });

  it("removes a staging folder a run left long ago, and keeps one another run may be using", async () => {
    const home = temporaryHome();
    const { npm, calls } = fakeNpm();
    const store = npmPackStore(home, npm);
    await store.fetch(["typescript"], "latest");
    await store.fetch(["typescript"], "latest");
    // Each run's folder holds one prefix per pack; neither was released.
    const [abandoned, current] = calls.map(args => dirname(prefixOf(args)));
    const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60 * 1000);
    utimesSync(abandoned!, twoHoursAgo, twoHoursAgo);

    await store.fetch(["typescript"], "latest");

    expect(existsSync(abandoned!)).toBe(false);
    expect(existsSync(current!)).toBe(true);
  });

  it("installs exact versions into the Rerail home's packs from npm's cache, after `--`", async () => {
    const home = temporaryHome();
    const { npm, calls } = fakeNpm();

    await npmPackStore(home, npm).install(["@neralabs/rerail-pack-typescript@0.1.1", "@neralabs/rerail-pack-complexity@0.1.2-next.0"]);

    const [args] = calls;
    expect(prefixOf(args!)).toBe(join(home, "packs"));
    expect(args).toEqual(expect.arrayContaining(["--save-exact", "--ignore-scripts"]));
    // The versions discovery fetched, or `rerail packs update`'s npm view read, are in npm's cache, so the install
    // does not ask the registry for them again.
    expect(args).toContain("--prefer-offline");
    expect(specsOf(args!)).toEqual(["@neralabs/rerail-pack-typescript@0.1.1", "@neralabs/rerail-pack-complexity@0.1.2-next.0"]);
  });

  it("asks the registry when a spec names a tag or range rather than a version", async () => {
    const { npm, calls } = fakeNpm();

    await npmPackStore(temporaryHome(), npm).install(["@neralabs/rerail-pack-typescript@latest"]);

    expect(calls[0]).not.toContain("--prefer-offline");
  });

  it("stops an install, and the install and removal that put it back, within the packs lock's wait", async () => {
    const timeouts: (number | undefined)[] = [];
    const npm: NpmRunner = async (_args, options) => {
      timeouts.push(options?.timeoutMs);
      return { stdout: "", stderr: "" };
    };
    const store = npmPackStore(temporaryHome(), npm);

    await store.install(["@neralabs/rerail-pack-typescript@0.1.1"]);
    await store.remove(["@neralabs/rerail-pack-typescript"]);

    expect(timeouts).toHaveLength(2);
    const [install, remove] = timeouts.map(timeout => {
      expect(timeout).toBeDefined();
      return timeout! + 2 * KILL_GRACE_MS;
    });
    // The lock is held through a stopped install and its put-back: an install, then a removal.
    expect(install! * 2 + remove!).toBeLessThan(PACKS_LOCK_WAIT_MS);
  });
});

/**
 * A store that installs `@neralabs/rerail-pack-typescript@<version>` by linking a copy of this repository's
 * typescript pack at that version, whose core range is `ranges[version]`.
 */
function copiesStore(home: string, ranges: Record<string, string>): PackStore & { installs: string[][]; removes: string[][] } {
  const installs: string[][] = [];
  const removes: string[][] = [];
  return {
    home,
    installs,
    removes,
    fetch: async () => ({ fetched: [], problems: [], release: async () => {} }),
    async install(specs) {
      installs.push([...specs]);
      for (const spec of specs) {
        const version = spec.slice(spec.lastIndexOf("@") + 1);
        // A restore names the folder npm recorded (`…@file:<copy>`); any other spec is a version to copy the pack at.
        const copy = version.startsWith("file:") ? version.slice("file:".length) : join(home, "copies", version);
        if (!existsSync(copy)) {
          cpSync(repositoryPath("packs", "typescript"), copy, { recursive: true, filter: source => !source.includes("upstream") });
          const manifest = JSON.parse(readFileSync(join(copy, "package.json"), "utf8")) as Record<string, unknown>;
          writeFileSync(join(copy, "package.json"), JSON.stringify({ ...manifest, version, peerDependencies: { "@neralabs/rerail": ranges[version] } }));
        }
        installPackDirectory(copy, "typescript", home);
      }
    },
    async remove(packageNames) {
      removes.push([...packageNames]);
      for (const packageName of packageNames) uninstallPack(packageName, home);
    },
  };
}

/** The core range Rerail's own packs declare: it fits this core, whatever its version. */
const FITS = ">=0.1.0-0";
const VALID = fixturesPath("packs", "valid");

describe("installPackVersions", { timeout: COMPILING_TIMEOUT_MS }, () => {
  it("puts the installed version back as npm recorded it when the one named installs but its core range leaves it out, and says why", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "0.1.1": FITS, "9.0.0": ">=9.0.0" });
    expect(await installPackVersions([typescriptAt("0.1.1")], store)).toBeUndefined();

    const problem = await installPackVersions([typescriptAt("9.0.0")], store);

    expect(problem).toMatch(/^@neralabs\/rerail-pack-typescript@9\.0\.0 cannot be used: .*>=9\.0\.0.*\. Nothing was changed\.$/u);
    expect(store.installs.at(-1)).toEqual([`@neralabs/rerail-pack-typescript@file:${join(home, "copies", "0.1.1")}`]);
    expect(loadRuleCatalog(home).packs.get("typescript")?.version).toBe("0.1.1");
  });

  it("removes a pack it installed that is left out, when none was installed before", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "9.0.0": ">=9.0.0" });

    expect(await installPackVersions([typescriptAt("9.0.0")], store)).toMatch(/cannot be used/u);

    expect(store.removes).toEqual([["@neralabs/rerail-pack-typescript"]]);
    expect(loadRuleCatalog(home).packs.has("typescript")).toBe(false);
  });

  it("installs nothing when every pack named is installed at that version", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "0.1.1": FITS });
    await installPackVersions([typescriptAt("0.1.1")], store);

    expect(await installPackVersions([typescriptAt("0.1.1")], store)).toBeUndefined();
    expect(store.installs).toHaveLength(1);
  });

  it("puts everything back when the pack named loads but pushes an installed pack out", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": FITS });
    await installPackVersions([typescriptAt("0.1.1")], copies);
    // A pack listing typescript's rules under an id that sorts first: the catalog loads it, and leaves typescript out.
    const rival = join(home, "rival");
    cpSync(repositoryPath("packs", "typescript"), rival, { recursive: true, filter: source => !source.includes("upstream") });
    const pack = JSON.parse(readFileSync(join(rival, "pack.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(rival, "pack.json"), JSON.stringify({ ...pack, id: "aaa-rival" }));
    const manifest = JSON.parse(readFileSync(join(rival, "package.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(rival, "package.json"), JSON.stringify({ ...manifest, name: "@neralabs/rerail-pack-aaa-rival", version: "1.0.0", repository: undefined }));
    const store: PackStore = {
      ...copies,
      async install(specs) {
        for (const spec of specs) {
          if (spec.startsWith("@neralabs/rerail-pack-aaa-rival@")) installPackDirectory(rival, "aaa-rival", home);
          else await copies.install([spec]);
        }
      },
    };

    const problem = await installPackVersions([{ packageName: "@neralabs/rerail-pack-aaa-rival", id: "aaa-rival", version: "1.0.0" }], store);

    expect(problem).toMatch(/@neralabs\/rerail-pack-typescript would stop loading: .*\. Nothing was changed\.$/u);
    expect(copies.removes).toEqual([["@neralabs/rerail-pack-aaa-rival"]]);
    expect([...loadRuleCatalog(home).packs.keys()]).toEqual(["typescript"]);
  });

  it("says what failed and where to look when putting a pack back fails, rather than throwing", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": FITS, "9.0.0": ">=9.0.0" });
    await installPackVersions([typescriptAt("0.1.1")], copies);
    const store: PackStore = {
      ...copies,
      async install(specs) {
        if (specs.some(spec => spec.includes("@file:"))) throw new Error("npm error code ENOSPC\nnpm error nospc no space left on device");
        await copies.install(specs);
      },
    };

    const problem = await installPackVersions([typescriptAt("9.0.0")], store);

    expect(problem).toMatch(/cannot be used: .*\. Then putting back @neralabs\/rerail-pack-typescript@file:.* failed: nospc no space left on device\. `rerail packs list` shows what is installed now\.$/u);
  });

  it("says nothing was changed when npm fails and leaves the packs as they were, putting nothing back", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": FITS });
    await installPackVersions([typescriptAt("0.1.1")], copies);
    const store: PackStore = { ...copies, install: async () => { throw new Error("npm error code ENOTFOUND\nnpm error network request failed"); } };

    expect(await installPackVersions([typescriptAt("0.1.2")], store)).toBe("could not reach the npm registry (ENOTFOUND): check your network, proxy, or registry in ~/.npmrc. Nothing was changed.");
    expect(copies.installs).toHaveLength(1);
    expect(loadRuleCatalog(home).packs.get("typescript")?.version).toBe("0.1.1");
  });

  it("puts the packs back when npm fails partway through, having changed one", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": FITS, "0.1.2": FITS });
    await installPackVersions([typescriptAt("0.1.1")], copies);
    // npm wrote the new version, then ran out of disk before it finished.
    const store: PackStore = {
      ...copies,
      async install(specs) {
        await copies.install(specs);
        if (specs.some(spec => spec.endsWith("@0.1.2"))) throw new Error("npm error code ENOSPC\nnpm error nospc no space left on device");
      },
    };

    expect(await installPackVersions([typescriptAt("0.1.2")], store)).toBe("nospc no space left on device. Nothing was changed.");
    expect(loadRuleCatalog(home).packs.get("typescript")?.version).toBe("0.1.1");
  });
});

describe("addPackSpecs", { timeout: COMPILING_TIMEOUT_MS }, () => {
  it("puts back a loaded pack that the install replaced with one its core range leaves out, and says why", async () => {
    const home = temporaryHome();
    // What `rerail packs add typescript` installs once `latest` moves past this core.
    const copies = copiesStore(home, { "0.1.1": FITS, "9.0.0": ">=9.0.0" });
    expect(await addPackSpecs(["@neralabs/rerail-pack-typescript@0.1.1"], copies)).toEqual({ installed: [typescriptAt("0.1.1")] });

    const added = await addPackSpecs(["@neralabs/rerail-pack-typescript@9.0.0"], copies);

    expect(added).toEqual({ problem: expect.stringMatching(/^@neralabs\/rerail-pack-typescript@9\.0\.0 cannot be used: .*>=9\.0\.0.*\. Nothing was changed\.$/u) });
    expect(loadRuleCatalog(home).packs.get("typescript")?.version).toBe("0.1.1");
  });

  it("checks a third party's pack as it does Rerail's own, and says what it installed under its package name", async () => {
    const home = temporaryHome();
    const store = linkingPackStore(home, { "@acme/rerail-pack-acme-valid": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-acme-valid", "acme-valid") });

    expect(await addPackSpecs(["@acme/rerail-pack-acme-valid@1.0.0"], store)).toEqual({ installed: [{ packageName: "@acme/rerail-pack-acme-valid", id: "acme-valid", version: "1.0.0" }] });
    expect(loadRuleCatalog(home).packs.get("acme-valid")).toMatchObject({ packageName: "@acme/rerail-pack-acme-valid", publisher: "@acme" });
    expect(installedPackVersion("@acme/rerail-pack-acme-valid", home)).toBe("1.0.0");
    expect(installedPackVersion("@neralabs/rerail-pack-acme-valid", home)).toBeNull();
  });

  it("puts back a third party's pack whose pack id Rerail's own pack holds, naming that pack", async () => {
    const home = temporaryHome();
    installPackDirectory(VALID, "valid", home);
    const store = linkingPackStore(home, { "@acme/rerail-pack-valid": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-valid", "valid") });

    const added = await addPackSpecs(["@acme/rerail-pack-valid@1.0.0"], store);

    expect(added).toEqual({ problem: "@acme/rerail-pack-valid@1.0.0 cannot be used: pack id valid is taken by @neralabs/rerail-pack-valid. Nothing was changed." });
    expect(store.removes).toEqual([["@acme/rerail-pack-valid"]]);
    expect(installedPackagesOf("valid", home)).toEqual(["@neralabs/rerail-pack-valid"]);
  });

  it("puts a third party's pack back as npm recorded it when the version installed in its place is left out", async () => {
    const home = temporaryHome();
    const first = thirdPartyPack(join(home, "copies"), "rerail-pack-acme-valid", "acme-valid");
    const store = linkingPackStore(home, { "rerail-pack-acme-valid": first });
    await addPackSpecs(["rerail-pack-acme-valid@1.0.0"], store);
    // npm recorded the registry install as its version; anything else installs the newer one in its place.
    const newer: PackStore = {
      ...store,
      async install(specs) {
        if (specs.every(spec => spec.endsWith("@1.0.0"))) return await store.install(specs);
        installPackDirectory(thirdPartyPack(join(home, "copies"), "rerail-pack-acme-valid", "acme-valid", "2.0.0", ">=9.0.0"), "rerail-pack-acme-valid", home, "2.0.0");
      },
    };

    const added = await addPackSpecs(["rerail-pack-acme-valid@2.0.0"], newer);

    expect(added).toEqual({ problem: expect.stringMatching(/^rerail-pack-acme-valid@2\.0\.0 cannot be used: .*>=9\.0\.0.*\. Nothing was changed\.$/u) });
    expect(store.installs.at(-1)).toEqual(["rerail-pack-acme-valid@1.0.0"]);
    expect(loadRuleCatalog(home).packs.get("acme-valid")).toMatchObject({ packageName: "rerail-pack-acme-valid", version: "1.0.0", publisher: "rerail-pack-acme-valid" });
  });
});

describe("packsLeftOutByCore", () => {
  // A real pack install and three catalog reads: over 4 s on a loaded CI shard, near vitest's 5 s default.
  it("names an installed pack the new core leaves out that the previous one loaded, with why, and nothing else", async () => {
    const home = temporaryHome();
    await installPackVersions([typescriptAt("0.1.1")], copiesStore(home, { "0.1.1": ">=0.1.0-0 <9.0.0" }));

    expect(packsLeftOutByCore(home, "9.0.5", "8.9.9")).toEqual([expect.stringMatching(/^@neralabs\/rerail-pack-typescript@0\.1\.1: .*9\.0\.5/u)]);
    // Already left out before the change, or still fitting after it: not this change's to name.
    expect(packsLeftOutByCore(home, "9.0.6", "9.0.5")).toEqual([]);
    expect(packsLeftOutByCore(home, "8.9.9", "8.9.8")).toEqual([]);
  }, 30_000);
});

/** A package as a fake registry holds it: its dist-tags, and each version's core range (null for none named). */
interface RegistryPackage {
  readonly tags: Readonly<Record<string, string>>;
  readonly versions: Readonly<Record<string, string | null>>;
}

type NpmCall = { args: string[]; cwd: string | undefined; timeoutMs: number | undefined };

/**
 * An npm whose `view` answers from `registry`, as npm does: `view <package> dist-tags versions` with the listing, and
 * `view <package>@<a || b> <fields>` with the fields asked for of each version the range names, oldest first: one answer
 * alone unwrapped, a version that names no core range without `peerDependencies`, and, as npm 11's `view.js` does,
 * every answer reduced to its one field when the first has only one. A package it lacks is a 404; an Error is thrown
 * as npm's failure.
 */
function registryNpm(registry: Readonly<Record<string, RegistryPackage | Error>>): { npm: NpmRunner; calls: NpmCall[] } {
  const calls: NpmCall[] = [];
  const npm: NpmRunner = async (args, options = {}) => {
    calls.push({ args: [...args], cwd: options.cwd, timeoutMs: options.timeoutMs });
    const spec = args[1]!;
    const listing = args[2] === "dist-tags";
    const packageName = listing ? spec : spec.slice(0, spec.lastIndexOf("@"));
    const found = registry[packageName];
    if (found instanceof Error) throw found;
    if (found === undefined) throw new Error(`npm failed (1): npm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/${packageName} - Not found`);
    if (listing) return { stdout: JSON.stringify({ "dist-tags": found.tags, versions: Object.keys(found.versions) }), stderr: "" };
    const fields = args.slice(2, args.indexOf("--json"));
    const named = spec.slice(spec.lastIndexOf("@") + 1).split(" || ").filter(version => version in found.versions).sort(compareVersions);
    const answers: Record<string, unknown>[] = named.map(version => {
      const range = found.versions[version];
      const all: Record<string, unknown> = { name: packageName, version, ...range === null ? {} : { peerDependencies: { "@neralabs/rerail": range } } };
      return Object.fromEntries(fields.filter(field => field in all).map(field => [field, all[field]]));
    });
    const first = Object.keys(answers[0] ?? {});
    const printed = first.length === 1 ? answers.map(answer => answer[first[0]!]) : answers;
    return { stdout: printed.length === 0 ? "" : JSON.stringify(printed.length === 1 ? printed[0] : printed), stderr: "" };
  };
  return { npm, calls };
}

const compareVersions = (left: string, right: string): number => semver.compare(left, right);

/** The packages the views in `calls` asked about, each as `<package> <fields>`, in order. */
const viewsOf = (calls: readonly NpmCall[]): string[] => calls.map(call => `${call.args[1]} ${call.args[2]}`);

describe("newerPackVersions", () => {
  /** Installs a bare pack package (a `package.json` only) into `home` as npm records it: `spec`, by default its version. */
  function installBare(home: string, packageName: string, version: string, spec = version): void {
    const directory = join(home, "copies", packageName, version);
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, "package.json"), JSON.stringify({ name: packageName, version }));
    installPackDirectory(directory, packageName, home, spec);
  }

  it("names each installed pack with a newer version under the channel that fits the active core, asking npm from no project", async () => {
    const home = temporaryHome();
    installBare(home, "@neralabs/rerail-pack-typescript", "0.1.5");
    installBare(home, "@neralabs/rerail-pack-python", "0.1.6");
    installBare(home, "@acme/rerail-pack-sql", "1.0.0");
    const { npm, calls } = registryNpm({
      "@neralabs/rerail-pack-typescript": { tags: { latest: "0.1.6" }, versions: { "0.1.5": FITS, "0.1.6": ">=0.1.0-0 <0.2.0" } },
      // No peer dependencies: it fits any core.
      "@acme/rerail-pack-sql": { tags: { latest: "1.1.0" }, versions: { "1.0.0": null, "1.1.0": null } },
      "@neralabs/rerail-pack-python": { tags: { latest: "0.1.6" }, versions: { "0.1.6": FITS } },
    });

    await expect(newerPackVersions(home, "latest", "0.1.5", npm)).resolves.toEqual({
      updates: [
        { packageName: "@acme/rerail-pack-sql", installed: "1.0.0", available: "1.1.0", tag: "latest" },
        { packageName: "@neralabs/rerail-pack-typescript", installed: "0.1.5", available: "0.1.6", tag: "latest" },
      ],
      problems: [],
    });
    // The listing of each pack, and the core ranges only of a pack with a newer version.
    expect(viewsOf(calls).sort()).toEqual([
      "@acme/rerail-pack-sql dist-tags",
      "@acme/rerail-pack-sql@1.1.0 name",
      "@neralabs/rerail-pack-python dist-tags",
      "@neralabs/rerail-pack-typescript dist-tags",
      "@neralabs/rerail-pack-typescript@0.1.6 name",
    ]);
    for (const call of calls) {
      expect(call.args.slice(-2)).toEqual(["--json", "--fetch-timeout=30000"]);
      expect(call.cwd).toBeDefined();
      expect(call.cwd!.startsWith(home)).toBe(false);
      expect(existsSync(join(call.cwd!, "package.json"))).toBe(false);
      expect(call.timeoutMs).toBeGreaterThan(0);
    }
  });

  it("takes the newest version the active core loads when the channel's own is for a later core, asking about every candidate at once", async () => {
    const home = temporaryHome();
    installBare(home, "@neralabs/rerail-pack-typescript", "0.1.5");
    installBare(home, "@neralabs/rerail-pack-python", "0.1.6");
    const { npm, calls } = registryNpm({
      "@neralabs/rerail-pack-typescript": {
        tags: { latest: "0.1.9" },
        versions: { "0.1.5": FITS, "0.1.6": FITS, "0.1.7": FITS, "0.1.8": ">=0.3.0-0", "0.1.9": ">=0.3.0-0" },
      },
      // Every newer version is for a later core: nothing to move to.
      "@neralabs/rerail-pack-python": { tags: { latest: "0.2.0" }, versions: { "0.1.6": FITS, "0.2.0": ">=0.3.0-0" } },
    });

    await expect(newerPackVersions(home, "latest", "0.2.1", npm)).resolves.toEqual({
      updates: [{ packageName: "@neralabs/rerail-pack-typescript", installed: "0.1.5", available: "0.1.7", tag: "latest" }],
      problems: [],
    });
    expect(viewsOf(calls)).toContain("@neralabs/rerail-pack-typescript@0.1.9 || 0.1.8 || 0.1.7 || 0.1.6 name");
  });

  it("keeps every candidate's core range when the oldest names none, which npm would otherwise print as bare versions", async () => {
    const home = temporaryHome();
    installBare(home, "@acme/rerail-pack-sql", "1.0.0");
    const { npm } = registryNpm({
      "@acme/rerail-pack-sql": { tags: { latest: "1.2.0" }, versions: { "1.0.0": null, "1.1.0": null, "1.2.0": ">=0.3.0-0" } },
    });

    await expect(newerPackVersions(home, "latest", "0.2.1", npm)).resolves.toEqual({
      updates: [{ packageName: "@acme/rerail-pack-sql", installed: "1.0.0", available: "1.1.0", tag: "latest" }],
      problems: [],
    });
  });

  it("offers no older version, none past the tag's, no prerelease on a channel of releases, nothing for a pack from a folder, and nothing npm has no version of under the tag or next", async () => {
    const home = temporaryHome();
    // Installed from next, newer than latest.
    installBare(home, "@neralabs/rerail-pack-typescript", "0.1.7-next.1");
    installBare(home, "@neralabs/rerail-pack-python", "0.1.6");
    installBare(home, "@neralabs/rerail-pack-go", "0.1.0");
    installBare(home, "@neralabs/rerail-pack-php", "0.1.0");
    const folder = join(home, "copies", "folder-pack");
    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, "package.json"), JSON.stringify({ name: "rerail-pack-mine", version: "0.0.1" }));
    installPackDirectory(folder, "rerail-pack-mine", home);
    const { npm, calls } = registryNpm({
      "@neralabs/rerail-pack-typescript": { tags: { latest: "0.1.6", next: "0.1.7-next.1" }, versions: { "0.1.6": null, "0.1.7-next.1": null } },
      // Only a prerelease and the version past latest are newer.
      "@neralabs/rerail-pack-python": { tags: { latest: "0.1.6", next: "0.1.7" }, versions: { "0.1.6": FITS, "0.1.6-next.0": FITS, "0.1.7": FITS } },
      "@neralabs/rerail-pack-go": { tags: { beta: "0.2.0" }, versions: { "0.1.0": FITS, "0.2.0": FITS } },
      "@neralabs/rerail-pack-php": { tags: { latest: "0.1.1" }, versions: { "0.1.0": FITS, "0.1.1-next.0": FITS, "0.1.1": ">=0.3.0-0" } },
    });

    await expect(newerPackVersions(home, "latest", "0.1.5", npm)).resolves.toEqual({ updates: [], problems: [] });
    // Only php has a candidate newer than its install, and the prerelease before it is not one.
    expect(viewsOf(calls).filter(view => !view.endsWith(" dist-tags"))).toEqual(["@neralabs/rerail-pack-php@0.1.1 name"]);
    expect(viewsOf(calls).some(view => view.startsWith("rerail-pack-mine"))).toBe(false);
  });

  it("looks a pack up under next when the channel has no version of it, where set-up installs it from, prereleases included", async () => {
    const home = temporaryHome();
    installBare(home, "@neralabs/rerail-pack-java", "0.1.0-next.1");
    const { npm, calls } = registryNpm({
      "@neralabs/rerail-pack-java": { tags: { next: "0.1.0-next.3" }, versions: { "0.1.0-next.1": FITS, "0.1.0-next.2": FITS, "0.1.0-next.3": ">=0.3.0-0" } },
    });

    await expect(newerPackVersions(home, "latest", "0.1.5", npm)).resolves.toEqual({
      // Under next, not the channel, so the update says so; next.3 is for a later core.
      updates: [{ packageName: "@neralabs/rerail-pack-java", installed: "0.1.0-next.1", available: "0.1.0-next.2", tag: "next" }],
      problems: [],
    });
    expect(viewsOf(calls)).toEqual(["@neralabs/rerail-pack-java dist-tags", "@neralabs/rerail-pack-java@0.1.0-next.3 || 0.1.0-next.2 name"]);
  });

  it("names a pack npm could not be asked about, with why, and still checks the others", async () => {
    const home = temporaryHome();
    installBare(home, "@neralabs/rerail-pack-typescript", "0.1.5");
    installBare(home, "@neralabs/rerail-pack-python", "0.1.6");
    installBare(home, "@acme/rerail-pack-gone", "1.0.0");
    const { npm } = registryNpm({
      "@neralabs/rerail-pack-typescript": new Error("npm failed (1): npm error code ECONNRESET\nnpm error network request to https://registry.npmjs.org/x failed"),
      "@neralabs/rerail-pack-python": { tags: { latest: "0.1.7" }, versions: { "0.1.6": null, "0.1.7": null } },
    });

    const check = await newerPackVersions(home, "latest", "0.1.5", npm);
    expect(check.updates).toEqual([{ packageName: "@neralabs/rerail-pack-python", installed: "0.1.6", available: "0.1.7", tag: "latest" }]);
    expect([...check.problems].sort()).toEqual([
      expect.stringMatching(/^@acme\/rerail-pack-gone: npm cannot read @acme\/rerail-pack-gone: it is not published/u),
      expect.stringMatching(/^@neralabs\/rerail-pack-typescript: could not reach the npm registry \(https:\/\/registry\.npmjs\.org, ECONNRESET\)/u),
    ]);
  });

  it("says a pack check npm did not finish was stopped, on one line, with the last line npm printed", async () => {
    const home = temporaryHome();
    installBare(home, "@neralabs/rerail-pack-typescript", "0.1.5");
    installBare(home, "@neralabs/rerail-pack-python", "0.1.6");
    const { npm } = registryNpm({
      "@neralabs/rerail-pack-typescript": new CommandTimeoutError("npm view", 60_000, "npm http fetch GET 200 https://registry.npmjs.org/a\nnpm http fetch GET attempt 3 failed with ETIMEDOUT"),
      // Nothing printed, so there is no last line to name.
      "@neralabs/rerail-pack-python": new CommandTimeoutError("npm view", 60_000),
    });

    const check = await newerPackVersions(home, "latest", "0.1.5", npm);
    expect([...check.problems].sort()).toEqual([
      "@neralabs/rerail-pack-python: npm view did not finish within 1 min and was stopped",
      "@neralabs/rerail-pack-typescript: npm view did not finish within 1 min and was stopped (its last output: npm http fetch GET attempt 3 failed with ETIMEDOUT)",
    ]);
  });

  it("asks npm nothing when no pack is installed", async () => {
    const { npm, calls } = registryNpm({});
    await expect(newerPackVersions(temporaryHome(), "latest", "0.1.5", npm)).resolves.toEqual({ updates: [], problems: [] });
    expect(calls).toEqual([]);
  });
});

describe("updatePacks", { timeout: COMPILING_TIMEOUT_MS }, () => {
  /** Installs `packageName` (pack `id`) at `version` into `home`, as npm records a registry install. */
  function installThirdParty(home: string, packageName: string, id: string, version: string): void {
    installPackDirectory(thirdPartyPack(join(home, "copies"), packageName, id, version), packageName, home, version);
  }

  /**
   * A store on `home` that installs `<package>@<version>` by linking `folders` of that spec, or, for a version already
   * under `home/copies` (one installed before, put back), that copy; recorded as that version, as npm records it.
   */
  function versionedStore(home: string, folders: Readonly<Record<string, string>>): PackStore & { installs: string[][] } {
    const installs: string[][] = [];
    return {
      home,
      installs,
      fetch: async () => ({ fetched: [], problems: [], release: async () => {} }),
      async install(specs) {
        installs.push([...specs]);
        for (const spec of specs) {
          const at = spec.lastIndexOf("@");
          const [packageName, version] = [spec.slice(0, at), spec.slice(at + 1)];
          const folder = folders[spec] ?? join(home, "copies", packageName, version);
          if (!existsSync(folder)) throw new Error(`npm error code E404\nnpm error 404 Not Found - ${spec}`);
          installPackDirectory(folder, packageName, home, version);
        }
      },
      async remove(packageNames) {
        for (const packageName of packageNames) uninstallPack(packageName, home);
      },
    };
  }

  it("moves each pack to the newest version the active core loads, and puts back alone one that does not load", async () => {
    const home = temporaryHome();
    installThirdParty(home, "@acme/rerail-pack-sql", "sql", "1.0.0");
    installThirdParty(home, "@acme/rerail-pack-lint", "lint", "1.0.0");
    const store = versionedStore(home, {
      "@acme/rerail-pack-sql@1.1.0": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-sql", "sql", "1.1.0"),
      // npm says it fits; what installs names a core range this core is outside of.
      "@acme/rerail-pack-lint@1.1.0": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-lint", "lint", "1.1.0", ">=99.0.0"),
    });
    const { npm } = registryNpm({
      "@acme/rerail-pack-sql": { tags: { latest: "1.2.0" }, versions: { "1.0.0": null, "1.1.0": null, "1.2.0": ">=99.0.0" } },
      "@acme/rerail-pack-lint": { tags: { latest: "1.1.0" }, versions: { "1.0.0": null, "1.1.0": null } },
    });
    const steps: string[] = [];
    const progress: Progress = { step: async (label, run) => { steps.push(label); return await run(); } };

    const run = await updatePacks(store, progress, npm);

    expect(run.updated).toEqual([{ packageName: "@acme/rerail-pack-sql", installed: "1.0.0", available: "1.1.0", tag: "latest" }]);
    expect(run.failed).toEqual([{
      update: { packageName: "@acme/rerail-pack-lint", installed: "1.0.0", available: "1.1.0", tag: "latest" },
      problem: expect.stringMatching(/^@acme\/rerail-pack-lint@1\.1\.0 cannot be used: .*>=99\.0\.0.*\. Nothing was changed\.$/u),
    }]);
    expect(run.problems).toEqual([]);
    expect(installedPackVersion("@acme/rerail-pack-sql", home)).toBe("1.1.0");
    expect(installedPackVersion("@acme/rerail-pack-lint", home)).toBe("1.0.0");
    expect(loadRuleCatalog(home).packs.get("sql")?.version).toBe("1.1.0");
    // One install each, so the one that failed put back only itself, as npm recorded it.
    expect(store.installs).toEqual([["@acme/rerail-pack-lint@1.1.0"], ["@acme/rerail-pack-lint@1.0.0"], ["@acme/rerail-pack-sql@1.1.0"]]);
    expect(steps).toEqual(["Checking your rule packs for newer versions", "Installing @acme/rerail-pack-lint@1.1.0", "Installing @acme/rerail-pack-sql@1.1.0"]);
  });

  it("keeps the packs already moved when a later pack's step throws, and tries none after it", async () => {
    const home = temporaryHome();
    installThirdParty(home, "@acme/rerail-pack-lint", "lint", "1.0.0");
    installThirdParty(home, "@acme/rerail-pack-sql", "sql", "1.0.0");
    installThirdParty(home, "@acme/rerail-pack-tidy", "tidy", "1.0.0");
    const store = versionedStore(home, {
      "@acme/rerail-pack-lint@1.1.0": thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-lint", "lint", "1.1.0"),
    });
    const { npm } = registryNpm(Object.fromEntries(["lint", "sql", "tidy"].map(id =>
      [`@acme/rerail-pack-${id}`, { tags: { latest: "1.1.0" }, versions: { "1.0.0": null, "1.1.0": null } }])));
    const stalls: (string | undefined)[] = [];
    // What installChecked throws, rather than returns, when another process holds the packs past the lock's wait.
    const progress: Progress = {
      step: async (label, run, stall) => {
        if (label === "Installing @acme/rerail-pack-sql@1.1.0") {
          throw new Error("Another Rerail process (pid 4242) is changing the rule packs in /home/packs; try again once it is done.");
        }
        if (label.startsWith("Installing")) stalls.push(stall?.hint);
        return await run();
      },
    };

    const run = await updatePacks(store, progress, npm);

    expect(run.updated).toEqual([{ packageName: "@acme/rerail-pack-lint", installed: "1.0.0", available: "1.1.0", tag: "latest" }]);
    expect(run.failed).toEqual([
      {
        update: { packageName: "@acme/rerail-pack-sql", installed: "1.0.0", available: "1.1.0", tag: "latest" },
        problem: "Another Rerail process (pid 4242) is changing the rule packs in /home/packs; try again once it is done. `rerail packs list` shows what is installed now.",
      },
      {
        update: { packageName: "@acme/rerail-pack-tidy", installed: "1.0.0", available: "1.1.0", tag: "latest" },
        problem: "not tried, since updating @acme/rerail-pack-sql stopped",
      },
    ]);
    expect(store.installs).toEqual([["@acme/rerail-pack-lint@1.1.0"]]);
    // A step that waits on npm or the lock says so, as the check step does.
    expect(stalls).toEqual([expect.stringMatching(/npm is stopped at 1 min 20 s/u)]);
  });

  it("installs nothing and names the problem when npm cannot be asked about a pack", async () => {
    const home = temporaryHome();
    installThirdParty(home, "@acme/rerail-pack-sql", "sql", "1.0.0");
    const store = versionedStore(home, {});
    const { npm } = registryNpm({ "@acme/rerail-pack-sql": new Error("npm failed (1): npm error code ECONNRESET\nnpm error network request to https://registry.npmjs.org/x failed") });

    const run = await updatePacks(store, undefined, npm);

    expect(run).toEqual({ updated: [], failed: [], problems: [expect.stringMatching(/^@acme\/rerail-pack-sql: could not reach the npm registry/u)] });
    expect(store.installs).toEqual([]);
  });
});

describe("discoverPacks", () => {
  it("offers an installed third party's pack under its package name, and names one whose pack id Rerail's own holds", async () => {
    const home = temporaryHome();
    installPackDirectory(thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-acme-valid", "acme-valid"), "@acme/rerail-pack-acme-valid", home);
    // Its scope sorts before @neralabs: Rerail's own pack still takes the id.
    installPackDirectory(thirdPartyPack(join(home, "copies"), "@aaa/rerail-pack-valid", "valid"), "@aaa/rerail-pack-valid", home);
    const store: PackStore = {
      home,
      fetch: async () => ({ fetched: [{ id: "valid", directory: VALID }], problems: [], release: async () => {} }),
      install: async () => {},
      remove: async () => {},
    };

    const discovery = await discoverPacks(undefined, store);

    expect(discovery.available.map(found => [found.packageName, found.pack.publisher, found.version, found.installed]).sort()).toEqual([
      ["@acme/rerail-pack-acme-valid", "@acme", "1.0.0", "1.0.0"],
      ["@neralabs/rerail-pack-valid", "rerail", "0.1.0", null],
    ]);
    expect(discovery.problems).toEqual(["@aaa/rerail-pack-valid@1.0.0: pack id valid is taken by @neralabs/rerail-pack-valid"]);
  });

  it("reads the installed packs of its store's home, as stand-ins for packs npm cannot give", async () => {
    const home = temporaryHome();
    const installed = installPackDirectory(repositoryPath("packs", "typescript"), "typescript", home);
    const version = (JSON.parse(readFileSync(join(installed, "package.json"), "utf8")) as { version: string }).version;
    const offline: PackStore = {
      home,
      fetch: async ids => ({ fetched: [], problems: ids.map(id => ({ packageName: `@neralabs/rerail-pack-${id}`, reason: "npm could not reach the registry (ENOTFOUND)" })), release: async () => {} }),
      install: async () => {},
      remove: async () => {},
    };

    const discovery = await discoverPacks(undefined, offline);

    expect(discovery.available.map(found => [found.pack.id, found.version, found.installed])).toEqual([["typescript", version, version]]);
  });

  it("keeps offering the installed version when the one npm gives is for a newer Rerail, and names that one with why", { timeout: COMPILING_TIMEOUT_MS }, async () => {
    const home = temporaryHome();
    const installing = copiesStore(home, { "0.1.1": FITS });
    expect(await installPackVersions([typescriptAt("0.1.1")], installing)).toBeUndefined();
    // What npm has under the channel: a version whose core range leaves this core out.
    const newer = join(home, "copies", "9.0.0");
    cpSync(repositoryPath("packs", "typescript"), newer, { recursive: true, filter: source => !source.includes("upstream") });
    const manifest = JSON.parse(readFileSync(join(newer, "package.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(newer, "package.json"), JSON.stringify({ ...manifest, version: "9.0.0", peerDependencies: { "@neralabs/rerail": ">=9.0.0" } }));
    const store: PackStore = { ...installing, fetch: async () => ({ fetched: [{ id: "typescript", directory: newer }], problems: [], release: async () => {} }) };

    const discovery = await discoverPacks(undefined, store);

    expect(discovery.available.map(found => [found.pack.id, found.version, found.installed])).toEqual([["typescript", "0.1.1", "0.1.1"]]);
    expect(discovery.problems).toEqual([expect.stringMatching(/^@neralabs\/rerail-pack-typescript@9\.0\.0: it needs a newer Rerail \(>=9\.0\.0\) than this one \(.*\); `rerail update` installs the newest$/u) as string]);
  });

  it("names a version once when the one installed is the one npm gives and it is for a newer Rerail", { timeout: COMPILING_TIMEOUT_MS }, async () => {
    const home = temporaryHome();
    const installing = copiesStore(home, { "9.0.0": ">=9.0.0" });
    // Installed straight through the store: set-up's own install refuses a version this core leaves out.
    await installing.install(["@neralabs/rerail-pack-typescript@9.0.0"]);
    const store: PackStore = { ...installing, fetch: async () => ({ fetched: [{ id: "typescript", directory: join(home, "copies", "9.0.0") }], problems: [], release: async () => {} }) };

    const discovery = await discoverPacks(undefined, store);

    expect(discovery.available).toEqual([]);
    expect(discovery.problems).toEqual([expect.stringMatching(/^@neralabs\/rerail-pack-typescript@9\.0\.0: it needs a newer Rerail /u) as string]);
  });

  it("names a fetched version that does not load and offers nothing in its place when none is installed", { timeout: COMPILING_TIMEOUT_MS }, async () => {
    const home = temporaryHome();
    const newer = join(home, "copies", "9.0.0");
    cpSync(repositoryPath("packs", "typescript"), newer, { recursive: true, filter: source => !source.includes("upstream") });
    const manifest = JSON.parse(readFileSync(join(newer, "package.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(join(newer, "package.json"), JSON.stringify({ ...manifest, version: "9.0.0", peerDependencies: { "@neralabs/rerail": ">=9.0.0" } }));
    const store: PackStore = { home, fetch: async () => ({ fetched: [{ id: "typescript", directory: newer }], problems: [], release: async () => {} }), install: async () => {}, remove: async () => {} };

    const discovery = await discoverPacks(undefined, store);

    expect(discovery.available).toEqual([]);
    expect(discovery.problems).toEqual([expect.stringMatching(/^@neralabs\/rerail-pack-typescript@9\.0\.0: it needs a newer Rerail /u) as string]);
  });

  describe("when no pack fits", { timeout: COMPILING_TIMEOUT_MS }, () => {
    // Rerail's own Python and SQL packs: Python skips tests/ and scripts/, and SQL looks for *.sql only.
    const store = (home: string): PackStore => directoryPackStore(repositoryPath("packs"), ["python", "sql"], home);

    function project(files: Record<string, string>): string {
      const root = temporaryHome();
      for (const [path, content] of Object.entries(files)) {
        mkdirSync(dirname(join(root, path)), { recursive: true });
        writeFileSync(join(root, path), content);
      }
      return root;
    }

    it("says what the scan found, the pack whose excludes left out every file it looks for, and what each pack looks for", async () => {
      const root = project({ "tests/test_a.py": "a = 1\n", "tests/test_b.py": "b = 1\n", "scripts/release.py": "r = 1\n", "README.md": "hi\n" });

      const discovery = await discoverPacks(root, store(temporaryHome()));

      expect(discovery.recommended).toEqual([]);
      expect(discovery.nothingFits).toEqual({
        fileCount: 4,
        endings: [{ ending: ".py", fileCount: 3 }, { ending: ".md", fileCount: 1 }],
        notScanned: [],
        git: false,
        stoppedEarly: false,
        nearMisses: [{ name: "Python", fileCount: 3, excludedBy: [{ glob: "tests/", fileCount: 2 }, { glob: "scripts/", fileCount: 1 }] }],
        packs: [{ id: "python", name: "Python", files: ["*.py", "*.pyi"], dependencies: [] }, { id: "sql", name: "SQL", files: ["*.sql"], dependencies: [] }],
      });
    });

    it("names the folder it did not scan when that held the only source file", async () => {
      const root = project({ "fixtures/app.py": "app = 1\n" });

      const discovery = await discoverPacks(root, store(temporaryHome()));

      expect(discovery.nothingFits).toMatchObject({ endings: [], notScanned: [{ folder: "fixtures" }], nearMisses: [] });
    });

    it("explains nothing when a pack fits, or when there is no repository to scan", async () => {
      const root = project({ "src/app.py": "app = 1\n", "tests/test_app.py": "t = 1\n" });

      const fits = await discoverPacks(root, store(temporaryHome()));
      const unscanned = await discoverPacks(undefined, store(temporaryHome()));

      expect(fits.recommended.map(found => found.pack.id)).toEqual(["python"]);
      expect(fits.nothingFits).toBeUndefined();
      expect(unscanned.nothingFits).toBeUndefined();
    });

    it("explains nothing when the only pack that fits has no rules yet: that reason is the whole answer", async () => {
      // Rerail's Rust pack, and its SQL pack as it would be published before any of its rules is ready.
      const packsRoot = temporaryHome();
      for (const id of ["rust", "sql"]) cpSync(repositoryPath("packs", id), join(packsRoot, id), { recursive: true, filter: source => !source.includes("upstream") });
      const packJson = join(packsRoot, "sql", "pack.json");
      writeFileSync(packJson, JSON.stringify({ ...JSON.parse(readFileSync(packJson, "utf8")) as Record<string, unknown>, rules: [] }));
      const root = project({ "db/schema.sql": "select 1;\n" });

      const discovery = await discoverPacks(root, directoryPackStore(packsRoot, ["rust", "sql"], temporaryHome()));

      expect(discovery.withoutRules.map(found => [found.pack.id, found.match !== undefined])).toEqual([["sql", true]]);
      expect(discovery.recommended).toEqual([]);
      expect(discovery.nothingFits).toBeUndefined();
    });

    it("explains nothing when no pack could be fetched", async () => {
      const root = project({ "tests/test_a.py": "a = 1\n" });
      const offline: PackStore = {
        ...store(temporaryHome()),
        fetch: async ids => ({ fetched: [], problems: ids.map(id => ({ packageName: `@neralabs/rerail-pack-${id}`, reason: "npm could not reach the registry (ENOTFOUND)" })), release: async () => {} }),
      };

      const discovery = await discoverPacks(root, offline);

      expect(discovery.available).toEqual([]);
      expect(discovery.nothingFits).toBeUndefined();
    });
  });
});

describe("installedPackagesOf", () => {
  it("names every installed package holding a pack id, whoever publishes it, and only those", () => {
    const home = temporaryHome();
    installPackDirectory(VALID, "valid", home);
    installPackDirectory(thirdPartyPack(join(home, "copies"), "rerail-pack-valid", "valid"), "rerail-pack-valid", home);
    installPackDirectory(thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-valid", "valid"), "@acme/rerail-pack-valid", home);
    installPackDirectory(thirdPartyPack(join(home, "copies"), "@acme/rerail-pack-other", "other"), "@acme/rerail-pack-other", home);

    expect(installedPackagesOf("valid", home)).toEqual(["@acme/rerail-pack-valid", "@neralabs/rerail-pack-valid", "rerail-pack-valid"]);
    expect(installedPackagesOf("missing", home)).toEqual([]);
  });
});

describe("parsePackVersions", () => {
  it("reads each <id>@<version> entry, prerelease versions included, and a pack named twice at one version once", () => {
    expect(parsePackVersions("typescript@0.1.1,complexity@0.1.2-next.0,typescript@0.1.1")).toEqual([
      typescriptAt("0.1.1"),
      { packageName: "@neralabs/rerail-pack-complexity", id: "complexity", version: "0.1.2-next.0" },
    ]);
  });

  it("reads a pack's package name, a third party's included, as that package", () => {
    expect(parsePackVersions("@acme/rerail-pack-sql@1.0.0,rerail-pack-lint@0.2.0,@neralabs/rerail-pack-typescript@0.1.1")).toEqual([
      { packageName: "@acme/rerail-pack-sql", id: "sql", version: "1.0.0" },
      { packageName: "rerail-pack-lint", id: "lint", version: "0.2.0" },
      typescriptAt("0.1.1"),
    ]);
  });

  it("refuses an entry without a version, naming it", () => {
    expect(() => parsePackVersions("complexity@0.1.1,typescript")).toThrow(/<id>@<version>.*not typescript$/u);
  });

  it.each(["typescript@01.2.3", "typescript@1.0.0-a..b", "@acme/sql@1.0.0"])("refuses %s, not a pack at a version semver reads", entry => {
    expect(() => parsePackVersions(entry)).toThrow(`<id>@<version> or <package>@<version>, for example typescript@0.1.1, not ${entry}`);
  });

  it("takes any well-formed pack id, a project's own included, and refuses one named at two versions", () => {
    expect(parsePackVersions("acme-conventions@1.0.0")).toEqual([{ packageName: "@neralabs/rerail-pack-acme-conventions", id: "acme-conventions", version: "1.0.0" }]);
    expect(() => parsePackVersions("typescript@0.1.1,typescript@0.1.2")).toThrow("typescript is named twice, at 0.1.1 and 0.1.2");
  });
});
