// scripts/release/packs.mjs: what a pack release run publishes, the lockfile sync after `changeset version`, and the
// publish gate's reading of the pack validation.

import { describe, expect, it } from "vitest";
import { gateFailures, packIds, planPacks, releasePacks, syncPackLockfile } from "../../scripts/release/packs.mjs";

const pack = (id, version) => ({ id, name: `@neralabs/rerail-pack-${id}`, version });

describe("planPacks", () => {
  it("publishes a version without its tag, skips one with it, and sends a never-published pack to its first publish", () => {
    const packs = [pack("bumped", "0.2.0"), pack("released", "0.1.0"), pack("new", "0.1.0")];
    const tags = ["@neralabs/rerail-pack-bumped@0.1.0", "@neralabs/rerail-pack-released@0.1.0", "@neralabs/rerail-pack-newer@0.1.0"];
    expect(planPacks(packs, tags)).toEqual({ publish: ["bumped"], first: ["new"] });
  });
});

describe("syncPackLockfile", () => {
  const lock = {
    lockfileVersion: 3,
    packages: {
      "": { name: "@neralabs/rerail", version: "0.1.0-next.1" },
      "packs/a": { name: "@neralabs/rerail-pack-a", version: "0.1.0", license: "Apache-2.0" },
      "packs/b": { name: "@neralabs/rerail-pack-b", version: "0.1.0" },
      "node_modules/@neralabs/rerail-pack-a": { resolved: "packs/a", link: true },
    },
  };

  it("sets each pack's recorded version and changes nothing else", () => {
    const synced = syncPackLockfile(lock, [pack("a", "0.2.0"), pack("b", "0.1.0")]);
    expect(synced).toEqual({ ...lock, packages: { ...lock.packages, "packs/a": { name: "@neralabs/rerail-pack-a", version: "0.2.0", license: "Apache-2.0" } } });
    expect(lock.packages["packs/a"].version).toBe("0.1.0");
  });

  it("refuses a pack the lockfile does not know, which needs npm install", () => {
    expect(() => syncPackLockfile(lock, [pack("c", "0.1.0")])).toThrow("package-lock.json has no packs/c; run npm install");
  });
});

describe("gateFailures", () => {
  const result = (pack, title, status) => ({ ancestorTitles: [`pack ${pack}`], title, status });
  const report = (...results) => ({ testResults: [{ assertionResults: results }] });

  it("passes a pack whose checks all passed, a rule not supported yet skipped", () => {
    expect(gateFailures(report(result("a", "check 1: rules", "passed"), result("a", "check 2: proof", "passed"), result("a", "rule x", "skipped")), ["a"], 2)).toEqual([]);
  });

  it("fails a pack with a failed test", () => {
    expect(gateFailures(report(result("a", "check 1: rules", "passed"), result("a", "check 2: proof", "passed"), result("a", "rule x", "failed")), ["a"], 2)).toEqual(["a: rule x failed"]);
  });

  it("fails a pack whose checks did not all run, as when the filter matched nothing", () => {
    expect(gateFailures(report(result("a", "check 1: rules", "passed")), ["a", "b"], 2)).toEqual(["a: 1 of 2 pack checks passed", "b: 0 of 2 pack checks passed"]);
  });
});

describe("packIds", () => {
  const packs = [pack("a", "0.1.0"), pack("b", "0.1.0")];

  it("names every pack for --all", () => {
    expect(packIds(["--all"], packs)).toEqual(["a", "b"]);
  });

  it("refuses an id that is not a pack", () => {
    expect(() => packIds(["a", "c"], packs)).toThrow("not a pack under packs/: c");
  });
});

describe("releasePacks", () => {
  const entry = (id, version) => ({ name: `@neralabs/rerail-pack-${id}`, version, file: `${id}-${version}.tgz` });
  const conflict = "npm error 403 403 Forbidden - PUT https://registry.npmjs.org/@neralabs%2frerail-pack-a - You cannot publish over the previously published versions: 0.2.0.";

  /** Steps that record what they do; `published` maps a tarball to npm publish's result. */
  function recording(published = {}) {
    const calls = [];
    return {
      calls,
      steps: {
        publish: (file) => (calls.push(`publish ${file}`), published[file] ?? { ok: true, output: "" }),
        assertRestricted: async (name) => void calls.push(`restricted ${name}`),
        tag: (tag) => void calls.push(`tag ${tag}`),
        log: () => {},
      },
    };
  }

  it("skips a version already tagged, so a re-run with the first run's tarballs reaches the rest", async () => {
    const { calls, steps } = recording();
    await releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], ["@neralabs/rerail-pack-a@0.2.0"], steps);
    expect(calls).toEqual(["publish b-0.2.0.tgz", "restricted @neralabs/rerail-pack-b", "tag @neralabs/rerail-pack-b@0.2.0"]);
  });

  it("checks and tags a version the registry already has without its tag, without publishing it again", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: conflict } });
    await releasePacks([entry("a", "0.2.0")], [], steps);
    expect(calls).toEqual(["publish a-0.2.0.tgz", "restricted @neralabs/rerail-pack-a", "tag @neralabs/rerail-pack-a@0.2.0"]);
  });

  it("stops at any other publish failure, tagging nothing", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: "npm error code ENEEDAUTH" } });
    await expect(releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], [], steps)).rejects.toThrow("npm publish @neralabs/rerail-pack-a@0.2.0 failed");
    expect(calls).toEqual(["publish a-0.2.0.tgz"]);
  });
});
