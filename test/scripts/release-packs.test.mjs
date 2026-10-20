// scripts/release/packs.mjs: what a pack release run publishes, what a promotion moves to `latest`, the lockfile sync
// after `changeset version`, the publish gate's reading of the pack validation, and the patch-only bumps of packs below
// 1.0.0.

import { describe, expect, it } from "vitest";
import {
  changesetBumpFailures,
  gateFailures,
  listPacks,
  packIds,
  planPacks,
  planPromotions,
  promotePacks,
  readChangesets,
  releasePacks,
  syncPackLockfile,
} from "../../scripts/release/packs.mjs";

const pack = (id, version) => ({ id, name: `@neralabs/rerail-pack-${id}`, version });

describe("planPacks", () => {
  it("publishes a version without its tag, skips one with it, and sends a never-published pack to its first publish", () => {
    const packs = [pack("bumped", "0.2.0"), pack("released", "0.1.0"), pack("new", "0.1.0")];
    const tags = ["@neralabs/rerail-pack-bumped@0.1.0", "@neralabs/rerail-pack-released@0.1.0", "@neralabs/rerail-pack-newer@0.1.0"];
    expect(planPacks(packs, tags)).toEqual({ publish: ["bumped"], first: ["new"] });
  });
});

describe("planPromotions", () => {
  const tags = ["@neralabs/rerail-pack-behind@0.1.3", "@neralabs/rerail-pack-current@0.1.2", "@neralabs/rerail-pack-untagged@0.1.1", "@neralabs/rerail-pack-fresh@0.1.0"];
  const latest = { "@neralabs/rerail-pack-behind": "0.1.2", "@neralabs/rerail-pack-current": "0.1.2", "@neralabs/rerail-pack-untagged": "0.1.1", "@neralabs/rerail-pack-fresh": null };

  it("promotes each released version that is not latest, skips one that is, and leaves an untagged version out as unreleased", () => {
    const asked = [];
    const plan = planPromotions(
      [pack("behind", "0.1.3"), pack("current", "0.1.2"), pack("untagged", "0.1.2"), pack("fresh", "0.1.0")],
      tags,
      (name) => (asked.push(name), latest[name]),
    );

    expect(plan.promote.map(({ id, version, latest: was }) => [id, version, was])).toEqual([["behind", "0.1.3", "0.1.2"], ["fresh", "0.1.0", null]]);
    expect(plan.unreleased.map(({ id, version }) => [id, version])).toEqual([["untagged", "0.1.2"]]);
    // npm is not asked about a version that cannot be promoted.
    expect(asked).not.toContain("@neralabs/rerail-pack-untagged");
  });
});

describe("promotePacks", () => {
  const promotion = (id, version) => ({ name: `@neralabs/rerail-pack-${id}`, version });

  it("promotes the rest when one fails, and returns each failure, a version never released included", () => {
    const promoted = [];
    const failures = promotePacks(
      [promotion("a", "0.1.3"), promotion("b", "0.1.3"), promotion("c", "0.1.3"), promotion("d", "9.9.9")],
      ["@neralabs/rerail-pack-a@0.1.3", "@neralabs/rerail-pack-b@0.1.3", "@neralabs/rerail-pack-c@0.1.3"],
      {
        promote: (name, version) => {
          if (name.endsWith("-b")) throw new Error("npm error code E403");
          promoted.push(`${name}@${version}`);
        },
        log: () => {},
      },
    );

    expect(promoted).toEqual(["@neralabs/rerail-pack-a@0.1.3", "@neralabs/rerail-pack-c@0.1.3"]);
    expect(failures).toEqual([
      "@neralabs/rerail-pack-b@0.1.3: npm error code E403",
      "@neralabs/rerail-pack-d@9.9.9: it was never released from this repository (no such git tag)",
    ]);
  });
});

describe("changesetBumpFailures", () => {
  const changeset = (file, ...bumps) => ({ file, text: `---\n${bumps.map(([id, bump]) => `"@neralabs/rerail-pack-${id}": ${bump}`).join("\n")}\n---\n\nSummary.\n` });
  const packs = [pack("young", "0.1.1"), pack("stable", "1.2.0")];

  it("refuses a minor or major bump of a pack below 1.0.0, which would take 0.1.1 to 0.2.0 or 1.0.0", () => {
    const failures = changesetBumpFailures([changeset("a.md", ["young", "minor"]), changeset("b.md", ["stable", "patch"], ["young", "major"])], packs);
    expect(failures).toEqual([
      "a.md: @neralabs/rerail-pack-young is 0.1.1, below 1.0.0, so its bump is patch, not minor",
      "b.md: @neralabs/rerail-pack-young is 0.1.1, below 1.0.0, so its bump is patch, not major",
    ]);
  });

  it("passes a patch below 1.0.0 and any bump from 1.0.0", () => {
    expect(changesetBumpFailures([changeset("a.md", ["young", "patch"], ["stable", "major"])], packs)).toEqual([]);
  });

  it("passes the repository's pending changesets", () => {
    expect(changesetBumpFailures(readChangesets(), listPacks())).toEqual([]);
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
