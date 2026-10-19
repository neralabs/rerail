import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { NpmRunner } from "../../src/runtime/index.js";
import { npmPackStore, parsePackVersions } from "../../src/setup/index.js";

const HOME = "/home/rerail";

/** A fake npm that records each call's arguments and fails a call whose spec `fail` has an error for. */
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
const prefixOf = (args: readonly string[]): string | undefined => args[args.indexOf("--prefix") + 1];

describe("npmPackStore", () => {
  it("fetches each pack under the tag into its own staging prefix, without running install scripts", async () => {
    const { npm, calls } = fakeNpm();

    const { fetched, problems } = await npmPackStore(HOME, npm).fetch(["typescript", "complexity"], "latest");

    expect(problems).toEqual([]);
    expect(fetched).toEqual([
      { id: "typescript", directory: join(HOME, "packs", "staging", "typescript", "node_modules", "@neralabs", "rerail-pack-typescript") },
      { id: "complexity", directory: join(HOME, "packs", "staging", "complexity", "node_modules", "@neralabs", "rerail-pack-complexity") },
    ]);
    expect(calls).toHaveLength(2);
    for (const [args, id] of calls.map((args, index) => [args, ["typescript", "complexity"][index]!] as const)) {
      expect(args[0]).toBe("install");
      expect(prefixOf(args)).toBe(join(HOME, "packs", "staging", id));
      expect(args).toContain("--ignore-scripts");
      expect(args.at(-1)).toBe(`@neralabs/rerail-pack-${id}@latest`);
    }
  });

  it("fetches a pack again under next when the tag has no version of it", async () => {
    const { npm, calls } = fakeNpm(spec => spec === "@neralabs/rerail-pack-typescript@latest"
      ? "npm error code ETARGET\nnpm error notarget No matching version found for @neralabs/rerail-pack-typescript@latest."
      : undefined);

    const { fetched, problems } = await npmPackStore(HOME, npm).fetch(["typescript"], "latest");

    expect(problems).toEqual([]);
    expect(fetched.map(pack => pack.id)).toEqual(["typescript"]);
    expect(calls.map(args => args.at(-1))).toEqual(["@neralabs/rerail-pack-typescript@latest", "@neralabs/rerail-pack-typescript@next"]);
    expect(calls.map(prefixOf)).toEqual([join(HOME, "packs", "staging", "typescript"), join(HOME, "packs", "staging", "typescript")]);
  });

  it("reports a pack npm cannot find as a problem of that pack and still fetches the others", async () => {
    const notFound = "npm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/@neralabs%2frerail-pack-gone";
    const { npm, calls } = fakeNpm(spec => spec.startsWith("@neralabs/rerail-pack-gone@") ? notFound : undefined);

    const { fetched, problems } = await npmPackStore(HOME, npm).fetch(["typescript", "gone", "complexity"], "latest");

    expect(fetched.map(pack => pack.id)).toEqual(["typescript", "complexity"]);
    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-gone", reason: "no read access to @neralabs/rerail-pack-gone, or it is not published; ask Nera for access" }]);
    // A 404 is not a missing version under the tag, so it is not fetched again under next.
    expect(calls.filter(args => args.at(-1)!.startsWith("@neralabs/rerail-pack-gone@"))).toHaveLength(1);
  });

  it("installs the exact specs into the Rerail home's packs prefix", async () => {
    const { npm, calls } = fakeNpm();

    await npmPackStore(HOME, npm).install(["@neralabs/rerail-pack-typescript@0.1.1", "@neralabs/rerail-pack-complexity@0.1.2-next.0"]);

    expect(calls).toHaveLength(1);
    const [args] = calls;
    expect(args![0]).toBe("install");
    expect(prefixOf(args!)).toBe(join(HOME, "packs"));
    expect(args).toEqual(expect.arrayContaining(["--save-exact", "--ignore-scripts"]));
    // The versions discovery fetched are in npm's cache, so the install does not ask the registry for them again.
    expect(args).toContain("--prefer-offline");
    expect(args!.slice(-2)).toEqual(["@neralabs/rerail-pack-typescript@0.1.1", "@neralabs/rerail-pack-complexity@0.1.2-next.0"]);
  });

  it("asks the registry when a spec names a tag or range rather than a version", async () => {
    const { npm, calls } = fakeNpm();

    await npmPackStore(HOME, npm).install(["@neralabs/rerail-pack-typescript@latest"]);

    expect(calls[0]).not.toContain("--prefer-offline");
  });

  it("says in one line when npm cannot reach the registry", async () => {
    const offline = "npm error code ECONNREFUSED\nnpm error syscall connect\nnpm error FetchError: request to https://registry.npmjs.org/ failed";
    const { npm } = fakeNpm(() => offline);

    const { problems } = await npmPackStore(HOME, npm).fetch(["typescript"], "latest");

    expect(problems).toEqual([{ packageName: "@neralabs/rerail-pack-typescript", reason: "npm could not reach the registry (ECONNREFUSED)" }]);
  });
});

describe("parsePackVersions", () => {
  it("reads each <id>@<version> entry, prerelease versions included", () => {
    expect(parsePackVersions("typescript@0.1.1,complexity@0.1.2-next.0")).toEqual([
      { id: "typescript", version: "0.1.1" },
      { id: "complexity", version: "0.1.2-next.0" },
    ]);
  });

  it("refuses an entry without a version, naming it", () => {
    expect(() => parsePackVersions("complexity@0.1.1,typescript")).toThrow(/<id>@<version>.*not typescript$/u);
  });
});
