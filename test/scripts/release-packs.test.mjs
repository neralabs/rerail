// scripts/release/packs.mjs: what a pack release run publishes and under which dist-tag, what a promotion moves to
// `latest`, the lockfile sync after `changeset version`, the publish gate's reading of the pack validation, and the
// patch-only bumps of packs below 1.0.0.

import { describe, expect, it } from "vitest";
import {
  changesetBumpFailures,
  gateFailures,
  heldPacks,
  listPacks,
  packIds,
  planPacks,
  planPromotions,
  promotePacks,
  promotionRequest,
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

describe("promotionRequest", () => {
  const packs = [pack("python", "0.1.4"), pack("react", "0.1.3")];

  it("asks for every pack with no id, and for one pack, at its own version unless given one, with an id", () => {
    expect(promotionRequest(packs, undefined, undefined)).toEqual({ packs, named: false });
    expect(promotionRequest(packs, "react", undefined)).toEqual({ packs: [{ ...pack("react", "0.1.3"), hold: false }], named: true });
  });

  it("holds a pack named at an older version than its own, which puts it back on purpose", () => {
    expect(promotionRequest(packs, "python", "0.1.3").packs).toEqual([{ ...pack("python", "0.1.3"), hold: true }]);
  });

  it("refuses an id that is not a pack, and a version without the pack it is for", () => {
    expect(() => promotionRequest(packs, "pyhton", undefined)).toThrow("pyhton is not a pack under packs/");
    expect(() => promotionRequest(packs, undefined, "0.1.3")).toThrow("version 0.1.3 needs the pack it is for");
  });
});

describe("planPromotions", () => {
  const name = (id) => `@neralabs/rerail-pack-${id}`;
  const tags = ["behind@0.1.3", "current@0.1.2", "untagged@0.1.1", "fresh@0.1.0", "unread@0.1.2", "held@0.1.4", "stale@0.1.4"].map((tag) => name(tag));
  const distTags = {
    [name("behind")]: { latest: "0.1.2", next: "0.1.3" },
    [name("current")]: { latest: "0.1.2" },
    [name("fresh")]: {},
    [name("unread")]: null,
    [name("held")]: { latest: "0.1.3", held: "0.1.3", next: "0.1.4" },
    [name("stale")]: { latest: "0.1.3", held: "0.1.2", next: "0.1.4" },
  };
  const promoted = (plan) => plan.promote.map(({ id, version, latest, hold, release }) => ({ id, version, latest, hold, release }));

  it("sweeps every released version not latest yet, leaving out one held where it was put back, and one npm could not read", () => {
    const asked = [];
    const plan = planPromotions(
      [pack("behind", "0.1.3"), pack("current", "0.1.2"), pack("untagged", "0.1.2"), pack("fresh", "0.1.0"), pack("unread", "0.1.2"), pack("held", "0.1.4"), pack("stale", "0.1.4")],
      tags,
      (packageName) => (asked.push(packageName), distTags[packageName]),
    );

    expect(promoted(plan)).toEqual([
      { id: "behind", version: "0.1.3", latest: "0.1.2", hold: false, release: false },
      { id: "fresh", version: "0.1.0", latest: null, hold: false, release: false },
      // Held somewhere `latest` no longer is: the hold is out of date, and this promotion clears it.
      { id: "stale", version: "0.1.4", latest: "0.1.3", hold: false, release: true },
    ]);
    expect(plan.held.map(({ id, latest }) => [id, latest])).toEqual([["held", "0.1.3"]]);
    expect(plan.unreleased.map(({ id, version }) => [id, version])).toEqual([["untagged", "0.1.2"]]);
    expect(plan.unreadable.map(({ id }) => id)).toEqual(["unread"]);
    // npm is not asked about a version that cannot be promoted.
    expect(asked).not.toContain(name("untagged"));
  });

  it("promotes a held pack when it is named, releasing the hold", () => {
    const plan = planPromotions([{ ...pack("held", "0.1.4"), hold: false }], tags, (packageName) => distTags[packageName], { named: true });
    expect(promoted(plan)).toEqual([{ id: "held", version: "0.1.4", latest: "0.1.3", hold: false, release: true }]);
  });

  it("puts a named older version back and holds it, and does nothing once it is held there", () => {
    const rollback = { ...pack("behind", "0.1.2"), hold: true };
    const before = planPromotions([rollback], [...tags, name("behind@0.1.2")], () => ({ latest: "0.1.3" }), { named: true });
    expect(promoted(before)).toEqual([{ id: "behind", version: "0.1.2", latest: "0.1.3", hold: true, release: false }]);

    const after = planPromotions([rollback], [...tags, name("behind@0.1.2")], () => ({ latest: "0.1.2", held: "0.1.2" }), { named: true });
    expect(after.promote).toEqual([]);
  });
});

describe("promotePacks", () => {
  const promotion = (id, version, flags = {}) => ({ name: `@neralabs/rerail-pack-${id}`, version, ...flags });

  it("promotes the rest when one fails, holds and releases as told, and returns each failure, a version never released included", () => {
    const calls = [];
    const failures = promotePacks(
      [promotion("a", "0.1.3"), promotion("b", "0.1.3"), promotion("c", "0.1.2", { hold: true }), promotion("e", "0.1.4", { release: true }), promotion("d", "9.9.9")],
      ["a@0.1.3", "b@0.1.3", "c@0.1.2", "e@0.1.4"].map((tag) => `@neralabs/rerail-pack-${tag}`),
      {
        promote: (name, version) => {
          if (name.endsWith("-b")) throw new Error("npm error code E403");
          calls.push(`latest ${name}@${version}`);
        },
        hold: (name, version) => void calls.push(`held ${name}@${version}`),
        release: (name) => void calls.push(`release ${name}`),
        log: () => {},
      },
    );

    expect(calls).toEqual([
      "latest @neralabs/rerail-pack-a@0.1.3",
      "latest @neralabs/rerail-pack-c@0.1.2",
      "held @neralabs/rerail-pack-c@0.1.2",
      "latest @neralabs/rerail-pack-e@0.1.4",
      "release @neralabs/rerail-pack-e",
    ]);
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
        publish: (file, distTag) => (calls.push(`publish ${file} ${distTag}`), published[file] ?? { ok: true, output: "" }),
        assertRestricted: async (name) => void calls.push(`restricted ${name}`),
        tag: (tag) => void calls.push(`tag ${tag}`),
        log: () => {},
      },
    };
  }

  it("skips a version already tagged, so a re-run with the first run's tarballs reaches the rest", async () => {
    const { calls, steps } = recording();
    const released = await releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], ["@neralabs/rerail-pack-a@0.2.0"], steps);
    expect(calls).toEqual(["publish b-0.2.0.tgz latest", "restricted @neralabs/rerail-pack-b", "tag @neralabs/rerail-pack-b@0.2.0"]);
    expect(released).toEqual([{ name: "@neralabs/rerail-pack-b", version: "0.2.0", distTag: "latest" }]);
  });

  it("publishes a held pack under next, leaving its latest where it was put back, and every other under latest", async () => {
    const { calls, steps } = recording();
    const released = await releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], [], steps, ["@neralabs/rerail-pack-a"]);
    expect(calls.filter((call) => call.startsWith("publish"))).toEqual(["publish a-0.2.0.tgz next", "publish b-0.2.0.tgz latest"]);
    expect(released.map(({ distTag }) => distTag)).toEqual(["next", "latest"]);
  });

  it("checks and tags a version the registry already has without its tag, without publishing it again", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: conflict } });
    await releasePacks([entry("a", "0.2.0")], [], steps);
    expect(calls).toEqual(["publish a-0.2.0.tgz latest", "restricted @neralabs/rerail-pack-a", "tag @neralabs/rerail-pack-a@0.2.0"]);
  });

  it("stops at any other publish failure, tagging nothing", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: "npm error code ENEEDAUTH" } });
    await expect(releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], [], steps)).rejects.toThrow("npm publish @neralabs/rerail-pack-a@0.2.0 failed");
    expect(calls).toEqual(["publish a-0.2.0.tgz latest"]);
  });
});

describe("heldPacks", () => {
  it("holds a pack whose held dist-tag is its latest, not one whose hold was overtaken, and reports one npm could not read", () => {
    const distTags = { held: { latest: "0.1.3", held: "0.1.3", next: "0.1.4" }, stale: { latest: "0.1.4", held: "0.1.2" }, plain: { latest: "0.1.4" }, unread: null };
    expect(heldPacks(Object.keys(distTags), (name) => distTags[name])).toEqual({ held: ["held"], unreadable: ["unread"] });
  });
});
