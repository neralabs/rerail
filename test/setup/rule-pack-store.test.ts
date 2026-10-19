import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadRuleCatalog } from "../../src/engine/index.js";
import type { NpmRunner } from "../../src/runtime/index.js";
import { discoverPacks, installPackVersions, npmPackStore, parsePackVersions, type PackStore } from "../../src/setup/index.js";
import { installPackDirectory, uninstallPack } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";

const homes: string[] = [];
afterEach(() => {
  for (const home of homes.splice(0)) rmSync(home, { recursive: true, force: true });
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

/** The value of `--prefix` in one npm call. */
const prefixOf = (args: readonly string[]): string => args[args.indexOf("--prefix") + 1]!;
/** What follows `--`: the specs npm reads as packages, never as options. */
const specsOf = (args: readonly string[]): string[] => args.slice(args.indexOf("--") + 1);

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
    for (const [args, id] of calls.map((args, index) => [args, ["typescript", "complexity"][index]!] as const)) {
      expect(args[0]).toBe("install");
      expect(prefixOf(args)).toBe(join(run, id));
      expect(args).toContain("--ignore-scripts");
      expect(specsOf(args)).toEqual([`@neralabs/rerail-pack-${id}@latest`]);
    }
    expect(existsSync(run)).toBe(true);
    await fetch.release();
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
    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-gone", reason: "no read access to @neralabs/rerail-pack-gone, or it is not published; ask Nera for access" }]);
    // A 404 is not a missing version under the tag, so it is not fetched again under next.
    expect(calls.filter(args => args.at(-1)!.startsWith("@neralabs/rerail-pack-gone@"))).toHaveLength(1);
  });

  it("says in one line when npm cannot reach the registry", async () => {
    const offline = "npm error code ECONNREFUSED\nnpm error syscall connect\nnpm error FetchError: request to https://registry.npmjs.org/ failed";
    const { npm } = fakeNpm(() => offline);

    const { problems } = await npmPackStore(temporaryHome(), npm).fetch(["typescript"], "latest");

    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-typescript", reason: "npm could not reach the registry (ECONNREFUSED)" }]);
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
    // The versions discovery fetched are in npm's cache, so the install does not ask the registry for them again.
    expect(args).toContain("--prefer-offline");
    expect(specsOf(args!)).toEqual(["@neralabs/rerail-pack-typescript@0.1.1", "@neralabs/rerail-pack-complexity@0.1.2-next.0"]);
  });

  it("asks the registry when a spec names a tag or range rather than a version", async () => {
    const { npm, calls } = fakeNpm();

    await npmPackStore(temporaryHome(), npm).install(["@neralabs/rerail-pack-typescript@latest"]);

    expect(calls[0]).not.toContain("--prefer-offline");
  });
});

describe("installPackVersions", () => {
  /**
   * A store that installs `@neralabs/rerail-pack-typescript@<version>` by linking a copy of this repository's
   * typescript pack at that version, whose core range is `range`.
   */
  function copiesStore(home: string, ranges: Record<string, string>): PackStore & { installs: string[][]; removes: string[][] } {
    const installs: string[][] = [];
    const removes: string[][] = [];
    return {
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
      async remove(ids) {
        removes.push([...ids]);
        for (const id of ids) uninstallPack(id, home);
      },
    };
  }

  it("puts the installed version back as npm recorded it when the one named installs but its core range leaves it out, and says why", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "0.1.1": ">=0.1.0-0 <0.2.0", "9.0.0": ">=9.0.0" });
    expect(await installPackVersions([{ id: "typescript", version: "0.1.1" }], store, home)).toBeUndefined();

    const problem = await installPackVersions([{ id: "typescript", version: "9.0.0" }], store, home);

    expect(problem).toMatch(/^@neralabs\/rerail-pack-typescript@9\.0\.0 cannot be used: .*>=9\.0\.0.*\. Nothing was changed\.$/u);
    expect(store.installs.at(-1)).toEqual([`@neralabs/rerail-pack-typescript@file:${join(home, "copies", "0.1.1")}`]);
    expect(loadRuleCatalog(home).packs.get("typescript")?.version).toBe("0.1.1");
  });

  it("removes a pack it installed that is left out, when none was installed before", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "9.0.0": ">=9.0.0" });

    expect(await installPackVersions([{ id: "typescript", version: "9.0.0" }], store, home)).toMatch(/cannot be used/u);

    expect(store.removes).toEqual([["typescript"]]);
    expect(loadRuleCatalog(home).packs.has("typescript")).toBe(false);
  });

  it("installs nothing when every pack named is installed at that version", async () => {
    const home = temporaryHome();
    const store = copiesStore(home, { "0.1.1": ">=0.1.0-0 <0.2.0" });
    await installPackVersions([{ id: "typescript", version: "0.1.1" }], store, home);

    expect(await installPackVersions([{ id: "typescript", version: "0.1.1" }], store, home)).toBeUndefined();
    expect(store.installs).toHaveLength(1);
  });

  it("puts everything back when the pack named loads but pushes an installed pack out", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": ">=0.1.0-0 <0.2.0" });
    await installPackVersions([{ id: "typescript", version: "0.1.1" }], copies, home);
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

    const problem = await installPackVersions([{ id: "aaa-rival", version: "1.0.0" }], store, home);

    expect(problem).toMatch(/@neralabs\/rerail-pack-typescript would stop loading: .*\. Nothing was changed\.$/u);
    expect(copies.removes).toEqual([["aaa-rival"]]);
    expect([...loadRuleCatalog(home).packs.keys()]).toEqual(["typescript"]);
  });

  it("says what failed and where to look when putting a pack back fails, rather than throwing", async () => {
    const home = temporaryHome();
    const copies = copiesStore(home, { "0.1.1": ">=0.1.0-0 <0.2.0", "9.0.0": ">=9.0.0" });
    await installPackVersions([{ id: "typescript", version: "0.1.1" }], copies, home);
    const store: PackStore = {
      ...copies,
      async install(specs) {
        if (specs.some(spec => spec.includes("@file:"))) throw new Error("npm error code ENOSPC\nnpm error nospc no space left on device");
        await copies.install(specs);
      },
    };

    const problem = await installPackVersions([{ id: "typescript", version: "9.0.0" }], store, home);

    expect(problem).toMatch(/cannot be used: .*\. Then putting back @neralabs\/rerail-pack-typescript@file:.* failed: nospc no space left on device\. `rerail packs list` shows what is installed now\.$/u);
  });
});

describe("discoverPacks", () => {
  it("reads the installed packs of the home it is given, as stand-ins for packs npm cannot give", async () => {
    const home = temporaryHome();
    const installed = installPackDirectory(repositoryPath("packs", "typescript"), "typescript", home);
    const version = (JSON.parse(readFileSync(join(installed, "package.json"), "utf8")) as { version: string }).version;
    const offline: PackStore = {
      fetch: async ids => ({ fetched: [], problems: ids.map(id => ({ packageName: `@neralabs/rerail-pack-${id}`, reason: "npm could not reach the registry (ENOTFOUND)" })), release: async () => {} }),
      install: async () => {},
      remove: async () => {},
    };

    const discovery = await discoverPacks(undefined, offline, home);

    expect(discovery.available.map(found => [found.pack.id, found.version, found.installed])).toEqual([["typescript", version, version]]);
  });
});

describe("parsePackVersions", () => {
  it("reads each <id>@<version> entry, prerelease versions included, and a pack named twice at one version once", () => {
    expect(parsePackVersions("typescript@0.1.1,complexity@0.1.2-next.0,typescript@0.1.1")).toEqual([
      { id: "typescript", version: "0.1.1" },
      { id: "complexity", version: "0.1.2-next.0" },
    ]);
  });

  it("refuses an entry without a version, naming it", () => {
    expect(() => parsePackVersions("complexity@0.1.1,typescript")).toThrow(/<id>@<version>.*not typescript$/u);
  });

  it("takes any well-formed pack id, a project's own included, and refuses one named at two versions", () => {
    expect(parsePackVersions("acme-conventions@1.0.0")).toEqual([{ id: "acme-conventions", version: "1.0.0" }]);
    expect(() => parsePackVersions("typescript@0.1.1,typescript@0.1.2")).toThrow("typescript is named twice, at 0.1.1 and 0.1.2");
  });
});
