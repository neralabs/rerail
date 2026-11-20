// scripts/release/packs.mjs: what a pack release run publishes and under which dist-tag, what a promotion moves to
// `latest` (and `next`), the lockfile sync after `changeset version`, the publish gate's reading of the pack
// validation, the patch-only bumps of packs below 1.0.0, the changeset a pull request owes each pack whose
// published files it changes, the changelog entry a changeset becomes (changesetNotes), and the rule summary
// a released pack's changelog section opens with (pack-changelog.mjs).

import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PACK_CHANGELOG_INTRO, packRuleChanges, packRuleNotes, packRules, releasedPackChangelog } from "../../scripts/release/pack-changelog.mjs";
import {
  changesetBumpFailures,
  changesetNotes,
  changesetRuleRefusal,
  changesetSummary,
  changesetTypeRefusal,
  gateFailures,
  isPrivatePack,
  listPacks,
  manifestChanged,
  missingChangesets,
  onRegistry,
  packIds,
  planPacks,
  planPromotions,
  promotePacks,
  privateChangesetFailures,
  promotionRequest,
  releasedRequest,
  publishedPacks,
  readChangesets,
  releasePacks,
  syncPackLockfile,
  tagPacks,
} from "../../scripts/release/packs.mjs";

const pack = (id, version) => ({ id, name: `@neralabs/rerail-pack-${id}`, version });
const privatePack = (id, version) => ({ ...pack(id, version), private: true });

describe("listPacks and publishedPacks", () => {
  const packsDirectory = mkdtempSync(join(tmpdir(), "release-packs-"));
  for (const [id, extra] of [["open", {}], ["closed", { private: true }]]) {
    mkdirSync(join(packsDirectory, id));
    writeFileSync(join(packsDirectory, id, "package.json"), JSON.stringify({ name: `@neralabs/rerail-pack-${id}`, version: "0.1.0", ...extra }));
  }

  it("lists every pack with whether it is private, and publishes only the others", () => {
    expect(listPacks(packsDirectory)).toEqual([
      { ...privatePack("closed", "0.1.0") },
      { ...pack("open", "0.1.0"), private: false },
    ]);
    expect(publishedPacks(packsDirectory).map((entry) => entry.id)).toEqual(["open"]);
    expect(planPacks(publishedPacks(packsDirectory), [], () => true)).toEqual({ publish: [], first: ["open"], unpublished: [], unreadable: [] });
  });
});

describe("isPrivatePack", () => {
  it("reads true as private, false or absent as published, and refuses anything else", () => {
    expect(isPrivatePack({ private: true })).toBe(true);
    expect(isPrivatePack({ private: false })).toBe(false);
    expect(isPrivatePack({ name: "x" })).toBe(false);
    expect(() => isPrivatePack({ private: "true" }, "packs/odd/package.json")).toThrow('packs/odd/package.json: "private" must be true or false, not "true"');
  });
});

describe(".changeset/config.json", () => {
  it("pins that changesets never version or tag a private pack", () => {
    const config = JSON.parse(readFileSync(join(import.meta.dirname, "../../.changeset/config.json"), "utf8"));
    expect(config.privatePackages).toEqual({ version: false, tag: false });
  });
});

describe("privateChangesetFailures", () => {
  const changeset = (file, ...ids) => ({ file, text: `---\n${ids.map((id) => `"@neralabs/rerail-pack-${id}": patch`).join("\n")}\n---\n\nSummary.\n` });
  const packs = [pack("open", "0.1.0"), privatePack("closed", "0.1.0")];

  it("refuses a changeset naming a private pack, alone or beside a published one", () => {
    expect(privateChangesetFailures([changeset("a.md", "closed"), changeset("b.md", "open", "closed"), changeset("c.md", "open")], packs)).toEqual([
      "a.md: @neralabs/rerail-pack-closed is private and never released, so no changeset names it",
      "b.md: @neralabs/rerail-pack-closed is private and never released, so no changeset names it",
    ]);
  });

  it("passes the repository's pending changesets", () => {
    expect(privateChangesetFailures(readChangesets(), listPacks())).toEqual([]);
  });
});

describe("changesetSummary and changesetNotes", () => {
  const BLADE = "@neralabs/rerail-pack-blade";
  const text = `---\n"${BLADE}": patch\n---\n\nBlade pack: reports at the directive.\n\nNot after it.\n`;

  it("reads the summary after the front matter", () => {
    expect(changesetSummary(text)).toBe("Blade pack: reports at the directive.\n\nNot after it.");
    expect(changesetSummary("---\n---\n")).toBe("");
  });

  it("writes each summary naming the pack as one entry of the type its file name opens with, its later paragraphs under the bullet", () => {
    const changesets = [
      { file: "changed-blade-directive.md", text },
      { file: "fixed-other-pack.md", text: text.replace(BLADE, "@neralabs/rerail-pack-vue") },
    ];
    expect(changesetNotes(changesets, BLADE)).toEqual([{ type: "Changed", entry: "- Blade pack: reports at the directive.\n\n  Not after it." }]);
  });

  it("writes no entry for an empty summary, which bumps the pack with nothing to say", () => {
    expect(changesetNotes([{ file: "changed-readme.md", text: `---\n"${BLADE}": patch\n---\n` }], BLADE)).toEqual([]);
  });

  it("gives a changeset whose name opens with no change type none, and says how to name it", () => {
    expect(changesetNotes([{ file: "blade-directive.md", text }], BLADE)[0].type).toBeNull();
    expect(changesetTypeRefusal("blade-directive.md")).toContain("names no change type: start its file name with added-");
    expect(changesetTypeRefusal("blade-directive.md")).toContain(".changeset/README.md");
    expect(changesetTypeRefusal("fixed-blade-directive.md")).toBeNull();
  });

  describe("changesetRuleRefusal", () => {
    const ruleIds = (name) => new Set(name === BLADE ? ["blade-a"] : []);
    const changeset = (file, summary, names = [BLADE]) => ({ file, text: `---\n${names.map((name) => `"${name}": patch`).join("\n")}\n---\n\n${summary}\n` });

    it("takes a summary opening with a rule the pack ships under Added and Fixed, and one it no longer ships under Removed", () => {
      expect(changesetRuleRefusal(changeset("fixed-a.md", "`blade-a` reads comments."), ruleIds)).toBeNull();
      expect(changesetRuleRefusal(changeset("added-a.md", "`blade-a` gains an option."), ruleIds)).toBeNull();
      expect(changesetRuleRefusal(changeset("removed-b.md", "`blade-b`: it reported every template."), ruleIds)).toBeNull();
    });

    it("refuses a summary under Added, Removed or Fixed opening with no rule", () => {
      for (const file of ["added-a.md", "removed-a.md", "fixed-a.md"]) expect(changesetRuleRefusal(changeset(file, "Reads comments."), ruleIds), file).toContain("opens with no rule id");
    });

    it("refuses a rule the pack does not ship, or under Removed one it still does", () => {
      expect(changesetRuleRefusal(changeset("fixed-b.md", "`blade-b` reads comments."), ruleIds)).toBe(`opens with \`blade-b\`, which ${BLADE} does not ship`);
      expect(changesetRuleRefusal(changeset("fixed-a.md", "`blade-a` reads comments.", [BLADE, "@neralabs/rerail-pack-vue"]), ruleIds)).toBe("opens with `blade-a`, which @neralabs/rerail-pack-vue does not ship");
      expect(changesetRuleRefusal(changeset("removed-a.md", "`blade-a`: gone."), ruleIds)).toBe(`says \`blade-a\` was removed, but ${BLADE} still ships it`);
    });

    it("leaves an empty summary, and the types that need no rule, alone", () => {
      expect(changesetRuleRefusal({ file: "added-a.md", text: `---\n"${BLADE}": patch\n---\n` }, ruleIds)).toBeNull();
      for (const file of ["changed-readme.md", "security-a.md", "deprecated-a.md", "untyped.md"]) expect(changesetRuleRefusal(changeset(file, "Reads comments."), ruleIds), file).toBeNull();
    });
  });

  it("every pending changeset names its change type", () => {
    expect(readChangesets().filter(({ file }) => changesetTypeRefusal(file) !== null)).toEqual([]);
  });
});

describe("the rule summary of a released pack version", () => {
  const rule = (id, start, description = `${id} wants this.`) => ({ id, start, description });

  it("reads how each rule starts, off, else pack.json's mode, else its document's, and its description from its document", () => {
    const pack = { rules: [{ id: "a", on: false, mode: "log" }, { id: "b", mode: "log" }, { id: "c" }] };
    expect(packRules(pack, (entry) => ({ mode: "steer", description: `${entry.id} wants this.` }))).toEqual([rule("a", "off"), rule("b", "log"), rule("c", "steer")]);
    expect(packRules({ rules: [{ id: "a", on: false }] }, () => undefined)).toEqual([{ id: "a", start: "off", description: undefined }]);
  });

  it("gives each new, dropped and restarted rule an entry of its own against the last release, whatever changesets said on the way", () => {
    // Shipped, dropped and shipped again between releases: only the net change shows.
    const before = [rule("kept", "steer"), rule("dropped", "steer"), rule("quieted", "steer"), rule("woken", "off")];
    const after = [rule("kept", "steer"), rule("quieted", "log"), rule("woken", "steer"), rule("new-on", "steer"), rule("new-log", "log"), rule("new-off", "off")];
    expect(packRuleNotes(packRuleChanges(before, after))).toEqual({
      lead: "6 rules, 5 of them on by default.",
      entries: [
        { type: "Added", entry: "- `new-on` (starts in Steer): new-on wants this." },
        { type: "Added", entry: "- `new-log` (starts in Log): new-log wants this." },
        { type: "Added", entry: "- `new-off` (starts off): new-off wants this." },
        { type: "Removed", entry: "- `dropped`: no longer shipped." },
        { type: "Changed", entry: "- `woken` now starts in Steer (was off)." },
        { type: "Changed", entry: "- `quieted` now starts in Log (was in Steer)." },
      ],
    });
  });

  it("lists new rules that start on before those that start off, by id within each, however many there are", () => {
    const added = Array.from({ length: 30 }, (_, index) => rule(`r${String(index).padStart(2, "0")}`, index % 2 === 0 ? "off" : "steer"));
    const entries = packRuleNotes(packRuleChanges([], added)).entries;
    expect(entries).toHaveLength(30);
    expect(entries[0].entry).toBe("- `r01` (starts in Steer): r01 wants this.");
    expect(entries[15].entry).toBe("- `r00` (starts off): r00 wants this.");
  });

  it("refuses a new rule with no description, which pack validation requires", () => {
    expect(() => packRuleNotes(packRuleChanges([], [{ id: "a", start: "off" }]))).toThrow("a's document gives it no description");
  });

  it("leads with the rule count, all on, all off or none", () => {
    expect(packRuleNotes(packRuleChanges([], [])).lead).toBe("Ships no rules yet.");
    expect(packRuleNotes(packRuleChanges([rule("a", "steer")], [rule("a", "steer")]))).toEqual({ lead: "1 rule, on by default.", entries: [] });
    expect(packRuleNotes(packRuleChanges([], [rule("a", "off"), rule("b", "off")])).lead).toBe("2 rules, all off by default.");
    expect(packRuleNotes(packRuleChanges([], [rule("a", "steer"), rule("b", "log")])).lead).toBe("2 rules, all on by default.");
    expect(packRuleNotes(packRuleChanges([], [rule("a", "off")])).lead).toBe("1 rule, off by default.");
    expect(packRuleNotes(packRuleChanges([], [rule("a", "off"), rule("b", "log")])).lead).toBe("2 rules, 1 of them on by default.");
  });

  const NAME = "@neralabs/rerail-pack-blade";
  const history = ["## 0.1.5 (2026-10-27)", "", "- Old.", ""];
  const lead = "10 rules, 8 of them on by default.";
  const header = [`# ${NAME}`, "", PACK_CHANGELOG_INTRO, ""];
  const released = (changelog, notes, written) => releasedPackChangelog(changelog, NAME, "0.1.6", "2026-11-04", notes, written);

  it("writes a dated section: the lead, then each change type's entries in the spec's order, the rule entries first within a type", () => {
    const notes = { lead, entries: [{ type: "Added", entry: "- `x` (starts in Steer): Do x." }, { type: "Changed", entry: "- `y` now starts in Log (was in Steer)." }] };
    const written = [
      { type: "Fixed", entry: "- Reports at the directive.\n\n  Not after it." },
      { type: "Added", entry: "- `z` reads Blade components." },
      { type: "Security", entry: "- Closed a hole." },
    ];
    expect(released([`# ${NAME}`, "", ...history].join("\n"), notes, written)).toBe(
      [
        ...header,
        "## 0.1.6 (2026-11-04)",
        "",
        lead,
        "",
        "### Added",
        "",
        "- `x` (starts in Steer): Do x.",
        "- `z` reads Blade components.",
        "",
        "### Changed",
        "",
        "- `y` now starts in Log (was in Steer).",
        "",
        "### Fixed",
        "",
        "- Reports at the directive.",
        "",
        "  Not after it.",
        "",
        "### Security",
        "",
        "- Closed a hole.",
        "",
        ...history,
      ].join("\n"),
    );
  });

  it("says the rules did not change when neither the rules nor a changeset has anything to say", () => {
    expect(released([`# ${NAME}`, "", ...history].join("\n"), { lead, entries: [] }, [])).toBe([...header, "## 0.1.6 (2026-11-04)", "", `${lead} No change to its rules.`, "", ...history].join("\n"));
  });

  it("keeps one intro above the new section, release after release, and starts the file of a pack that has none", () => {
    const once = released("", { lead, entries: [] }, []);
    expect(once).toBe([...header, "## 0.1.6 (2026-11-04)", "", `${lead} No change to its rules.`, ""].join("\n"));
    const twice = releasedPackChangelog(once, NAME, "0.1.7", "2026-11-05", { lead, entries: [] }, []);
    expect(twice.split(PACK_CHANGELOG_INTRO)).toHaveLength(2);
    expect(twice.indexOf("## 0.1.7 (")).toBeLessThan(twice.indexOf("## 0.1.6 ("));
  });

  it("takes a removed rule's reason from the changeset that opens with it, in place of the generated entry", () => {
    const notes = { lead, entries: [{ type: "Removed", entry: "- `gone`: no longer shipped." }, { type: "Removed", entry: "- `left`: no longer shipped." }] };
    const text = released("", notes, [{ type: "Removed", entry: "- `gone`: it reported every template." }, { type: "Fixed", entry: "- `left` reads comments." }]);
    expect(text).not.toContain("- `gone`: no longer shipped.");
    expect(text).toContain("### Removed\n\n- `left`: no longer shipped.\n- `gone`: it reported every template.\n\n### Fixed");
  });

  it("refuses a changeset entry with no type, which no heading could hold", () => {
    expect(() => released("", { lead, entries: [] }, [{ type: null, entry: "- Lost." }])).toThrow("a changeset has no change type: - Lost.");
  });
});

describe("planPacks", () => {
  const has = (onNpm) => (name, version) => onNpm[`${name}@${version}`] ?? true;

  it("publishes a version without its tag, skips one tagged and on npm, and sends a never-published pack to its first publish", () => {
    const packs = [pack("bumped", "0.2.0"), pack("released", "0.1.0"), pack("new", "0.1.0")];
    const tags = ["@neralabs/rerail-pack-bumped@0.1.0", "@neralabs/rerail-pack-released@0.1.0", "@neralabs/rerail-pack-newer@0.1.0"];
    expect(planPacks(packs, tags, has({}))).toEqual({ publish: ["bumped"], first: ["new"], unpublished: [], unreadable: [] });
  });

  it("publishes again a version tagged but not on npm, so re-running all jobs or the next push finishes its release", () => {
    const packs = [pack("failed", "0.1.7"), pack("released", "0.1.0")];
    const tags = ["@neralabs/rerail-pack-failed@0.1.6", "@neralabs/rerail-pack-failed@0.1.7", "@neralabs/rerail-pack-released@0.1.0"];
    const plan = planPacks(packs, tags, has({ "@neralabs/rerail-pack-failed@0.1.7": false }));
    expect(plan).toEqual({ publish: ["failed"], first: [], unpublished: ["failed"], unreadable: [] });
  });

  it("reports a tagged version npm could not be read for, never taking it as released", () => {
    const plan = planPacks([pack("a", "0.1.7")], ["@neralabs/rerail-pack-a@0.1.7"], () => null);
    expect(plan).toEqual({ publish: [], first: [], unpublished: [], unreadable: ["a"] });
  });
});

describe("onRegistry", () => {
  it("takes a dist-tag at the version as published at once, though the version list lags it", () => {
    expect(onRegistry("0.1.7", { latest: "0.1.7", next: "0.1.7" }, ["0.1.6"])).toBe(true);
    expect(onRegistry("0.1.7", { latest: "0.1.7" }, null)).toBe(true);
  });

  it("reads the version list for a version no dist-tag points at, and says nothing when either can't be read", () => {
    expect(onRegistry("0.1.6", { latest: "0.1.7" }, ["0.1.6", "0.1.7"])).toBe(true);
    expect(onRegistry("0.1.8", { latest: "0.1.7" }, ["0.1.6", "0.1.7"])).toBe(false);
    expect(onRegistry("0.1.8", { latest: "0.1.7" }, null)).toBeNull();
    expect(onRegistry("0.1.8", null, ["0.1.8"])).toBeNull();
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

  it("promotes the rest when one fails, holds and releases as told, raises a trailing next, and returns what moved and each failure", async () => {
    const calls = [];
    const nexts = { "@neralabs/rerail-pack-a": "0.1.2", "@neralabs/rerail-pack-c": "0.1.4", "@neralabs/rerail-pack-e": null };
    const { promoted, failures } = await promotePacks(
      [promotion("a", "0.1.3"), promotion("b", "0.1.3"), promotion("c", "0.1.2", { hold: true }), promotion("e", "0.1.4", { release: true }), promotion("d", "9.9.9")],
      ["a@0.1.3", "b@0.1.3", "c@0.1.2", "e@0.1.4"].map((tag) => `@neralabs/rerail-pack-${tag}`),
      {
        promote: (name, version) => {
          if (name.endsWith("-b")) throw new Error("npm error code E403");
          calls.push(`latest ${name}@${version}`);
        },
        hold: (name, version) => void calls.push(`held ${name}@${version}`),
        release: (name) => void calls.push(`release ${name}`),
        readNext: async (name) => nexts[name],
        readLatest: async () => null,
        readCoreRange: async () => ">=0.1.0-0",
        readCoreLatest: async () => "0.1.5",
        distTag: (name, version, tag) => void calls.push(`${tag} ${name}@${version}`),
        log: () => {},
      },
    );

    expect(calls).toEqual([
      "latest @neralabs/rerail-pack-a@0.1.3",
      "next @neralabs/rerail-pack-a@0.1.3",
      "latest @neralabs/rerail-pack-c@0.1.2",
      "held @neralabs/rerail-pack-c@0.1.2",
      "latest @neralabs/rerail-pack-e@0.1.4",
      "release @neralabs/rerail-pack-e",
      "next @neralabs/rerail-pack-e@0.1.4",
    ]);
    expect(promoted.map(({ name }) => name)).toEqual(["@neralabs/rerail-pack-a", "@neralabs/rerail-pack-c", "@neralabs/rerail-pack-e"]);
    expect(failures).toEqual([
      "@neralabs/rerail-pack-b@0.1.3: npm error code E403",
      "@neralabs/rerail-pack-d@9.9.9: it was never released from this repository (no such git tag)",
    ]);
  });
});

describe("promotePacks, when the approval comes", () => {
  const promotion = (id, version, flags = {}) => ({ name: `@neralabs/rerail-pack-${id}`, version, ...flags });
  const tags = ["a@0.2.0", "b@0.2.0", "c@0.1.0", "d@0.2.0"].map((tag) => `@neralabs/rerail-pack-${tag}`);

  /** Steps where `latest` and `ranges` map a pack id to its `latest` and to its version's core range. */
  function steps(latest = {}, ranges = {}, coreLatest = "0.1.5") {
    const calls = [];
    const id = (name) => name.replace("@neralabs/rerail-pack-", "");
    return {
      calls,
      steps: {
        promote: (name, version) => void calls.push(`latest ${name}@${version}`),
        hold: (name, version) => void calls.push(`held ${name}@${version}`),
        release: () => {},
        readNext: async () => null,
        readLatest: async (name) => latest[id(name)] ?? null,
        readCoreRange: async (name) => (Object.hasOwn(ranges, id(name)) ? ranges[id(name)] : ">=0.1.0-0"),
        readCoreLatest: async () => coreLatest,
        distTag: () => {},
        log: () => {},
      },
    };
  }

  it("leaves out a pack the core on latest can't load, naming why, and promotes the rest", async () => {
    const { calls, steps: run } = steps({}, { a: ">=0.2.0-0", b: undefined });
    const { promoted, failures } = await promotePacks([promotion("a", "0.2.0"), promotion("b", "0.2.0"), promotion("d", "0.2.0")], tags, run);
    expect(promoted.map(({ name }) => name)).toEqual(["@neralabs/rerail-pack-d"]);
    expect(failures).toEqual([
      "@neralabs/rerail-pack-a@0.2.0: it needs core >=0.2.0-0, and latest is 0.1.5: promote core first, then run this again",
      "@neralabs/rerail-pack-b@0.2.0: it names no core range, so whether the core on latest loads it is unknown",
    ]);
    expect(calls).toEqual(["latest @neralabs/rerail-pack-d@0.2.0"]);
  });

  it("refuses a range other than one lower bound, which it can't read without semver", async () => {
    const { steps: run } = steps({}, { a: "^0.1.0" });
    const { failures } = await promotePacks([promotion("a", "0.2.0")], tags, run);
    expect(failures).toEqual(["@neralabs/rerail-pack-a@0.2.0: its core range ^0.1.0 is not one lower bound (>=<version>), so the promote can't check it"]);
  });

  it("never moves latest back past a newer release, as approving an older release's promote last would", async () => {
    const { calls, steps: run } = steps({ a: "0.2.1" });
    const { promoted, failures } = await promotePacks([promotion("a", "0.2.0")], tags, run);
    expect(promoted).toEqual([]);
    expect(failures).toEqual(["@neralabs/rerail-pack-a@0.2.0: latest is 0.2.1, newer, so this release is superseded"]);
    expect(calls).toEqual([]);
  });

  it("puts a pack back whatever latest and core hold, since that is what a hold is for", async () => {
    const { calls, steps: run } = steps({ c: "0.2.0" }, { c: ">=9.0.0-0" });
    const { promoted, failures } = await promotePacks([promotion("c", "0.1.0", { hold: true })], tags, run);
    expect(failures).toEqual([]);
    expect(promoted.map(({ version }) => version)).toEqual(["0.1.0"]);
    expect(calls).toEqual(["latest @neralabs/rerail-pack-c@0.1.0", "held @neralabs/rerail-pack-c@0.1.0"]);
  });
});

describe("releasedRequest", () => {
  const packs = [pack("python", "0.1.6"), pack("react", "0.1.3")];

  it("asks for exactly the versions a release run published, whatever main holds now, and none of them a hold", () => {
    expect(releasedRequest(packs, "@neralabs/rerail-pack-python@0.1.5:next  @neralabs/rerail-pack-react@0.1.3:next")).toEqual({
      packs: [
        { ...pack("python", "0.1.5"), hold: false },
        { ...pack("react", "0.1.3"), hold: false },
      ],
      named: false,
    });
  });

  it("refuses a package that is no pack here", () => {
    expect(() => releasedRequest(packs, "@neralabs/rerail-pack-nope@0.1.0:next")).toThrow(/@neralabs\/rerail-pack-nope is not a published pack under packs\//u);
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

describe("missingChangesets", () => {
  const files = ["pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"];
  const packs = [
    { id: "php", name: "@neralabs/rerail-pack-php", files },
    { id: "vue", name: "@neralabs/rerail-pack-vue", files },
  ];
  const changeset = (...ids) => ({ file: ".changeset/x.md", text: `---\n${ids.map((id) => `"@neralabs/rerail-pack-${id}": patch`).join("\n")}\n---\n\nSummary.\n` });

  it("fails each pack whose published files changed with no changeset naming it, naming up to three of its paths", () => {
    const changed = [
      "packs/php/rules/a.json",
      "packs/php/rules/b.json",
      "packs/php/README.md",
      "packs/php/package.json",
      "packs/vue/rules/c.json",
    ];
    expect(missingChangesets(changed, [changeset("vue")], packs)).toEqual([
      "@neralabs/rerail-pack-php: packs/php/rules/a.json, packs/php/rules/b.json, packs/php/README.md and 1 more changed, and no changeset names it",
    ]);
  });

  it("passes changes to what a pack does not publish (its test data and their locks too), a file that only starts like a published one, and other paths", () => {
    const changed = [
      "packs/php/upstream/phpcs/gates.yaml",
      "packs/php/upstream/phpcs/test-data.lock",
      "packs/php/candidates/rules/x.json",
      "packs/php/CHANGELOG.md",
      "packs/php/rules.md",
      "packs/README.md",
      "src/a.ts",
    ];
    expect(missingChangesets(changed, [], packs)).toEqual([]);
  });

  it("passes a published change once any changeset of the pull request names the pack", () => {
    expect(missingChangesets(["packs/php/pack.json", "packs/vue/LICENSE"], [changeset("php", "vue")], packs)).toEqual([]);
  });

  it("passes a private pack's changes with no changeset, since it is never released", () => {
    const closed = [{ id: "own", name: "@neralabs/rerail-pack-own", files, private: true }];
    expect(missingChangesets(["packs/own/rules/a.json", "packs/own/package.json"], [], closed)).toEqual([]);
  });
});

describe("manifestChanged", () => {
  const manifest = (fields) => JSON.stringify({ name: "@neralabs/rerail-pack-php", version: "0.1.4", files: ["pack.json"], ...fields });

  it("needs no release for a change to publishConfig alone, which only steers npm publish", () => {
    expect(manifestChanged(manifest({ publishConfig: { access: "restricted" } }), manifest({ publishConfig: { access: "public" } }))).toBe(false);
  });

  it("counts any other change, beside publishConfig or not, and a package.json the pack did not have", () => {
    expect(manifestChanged(manifest({ publishConfig: { access: "restricted" } }), manifest({ publishConfig: { access: "public" }, files: ["pack.json", "rules"] }))).toBe(true);
    expect(manifestChanged(manifest({}), manifest({ peerDependencies: { "@neralabs/rerail": ">=0.2.0" } }))).toBe(true);
    expect(manifestChanged(undefined, manifest({}))).toBe(true);
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
  const packs = [pack("a", "0.1.0"), pack("b", "0.1.0"), privatePack("p", "0.1.0")];

  it("names every published pack for --all", () => {
    expect(packIds(["--all"], packs)).toEqual(["a", "b"]);
  });

  it("refuses an id that is not a pack, and a private pack, which is never packed for npm", () => {
    expect(() => packIds(["a", "c"], packs)).toThrow("not a pack under packs/: c");
    expect(() => packIds(["p"], packs)).toThrow("private, never packed for npm: p");
  });
});

describe("tagPacks", () => {
  const entry = (id, version) => ({ name: `@neralabs/rerail-pack-${id}`, version, file: `${id}-${version}.tgz` });

  it("tags only what is not tagged yet, so a re-run tags the rest, and names every version for its GitHub Release", () => {
    const calls = [];
    const steps = { tag: (tag) => void calls.push(tag), log: () => {} };
    const released = tagPacks([entry("a", "0.2.0"), entry("b", "0.2.0")], ["@neralabs/rerail-pack-a@0.2.0"], steps);
    expect(calls).toEqual(["@neralabs/rerail-pack-b@0.2.0"]);
    expect(released).toEqual([
      { name: "@neralabs/rerail-pack-a", version: "0.2.0", distTag: "next" },
      { name: "@neralabs/rerail-pack-b", version: "0.2.0", distTag: "next" },
    ]);
  });
});

describe("releasePacks", () => {
  const entry = (id, version) => ({ name: `@neralabs/rerail-pack-${id}`, version, file: `${id}-${version}.tgz` });
  const conflict = "npm error 403 403 Forbidden - PUT https://registry.npmjs.org/@neralabs%2frerail-pack-a - You cannot publish over the previously published versions: 0.2.0.";

  /** Steps that record what they do; `published` maps a tarball to npm publish's result, `latest` and `next` a name to that dist-tag. */
  function recording(published = {}, latest = {}, next = {}) {
    const calls = [];
    const lines = [];
    return {
      calls,
      lines,
      steps: {
        publish: (file, distTag) => (calls.push(`publish ${file} ${distTag}`), published[file] ?? { ok: true, output: "" }),
        readDistTags: async (name) => ({ ...(latest[name] === undefined ? {} : { latest: latest[name] }), ...(next[name] === undefined ? {} : { next: next[name] }) }),
        log: (line) => void lines.push(line),
      },
    };
  }

  it("publishes every pack under next, where it waits to be promoted, and moves no other dist-tag", async () => {
    const { calls, steps } = recording();
    const { released } = await releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], steps);
    expect(calls).toEqual(["publish a-0.2.0.tgz next", "publish b-0.2.0.tgz next"]);
    expect(released).toEqual([
      { name: "@neralabs/rerail-pack-a", version: "0.2.0", distTag: "next" },
      { name: "@neralabs/rerail-pack-b", version: "0.2.0", distTag: "next" },
    ]);
  });

  it("leaves out a version older than latest, which would put next behind latest, and publishes the rest", async () => {
    const latest = { "@neralabs/rerail-pack-a": "0.1.8", "@neralabs/rerail-pack-b": "0.1.7" };
    const { calls, steps } = recording({}, latest);
    const { released, superseded } = await releasePacks([entry("a", "0.1.7"), entry("b", "0.1.7"), entry("c", "0.1.0")], steps);
    expect(calls).toEqual(["publish b-0.1.7.tgz next", "publish c-0.1.0.tgz next"]);
    expect(superseded).toEqual([{ name: "@neralabs/rerail-pack-a", version: "0.1.7", why: "0.1.7 is older than latest 0.1.8: a release on next must not trail latest" }]);
    expect(released.map(({ name }) => name)).toEqual(["@neralabs/rerail-pack-b", "@neralabs/rerail-pack-c"]);
  });

  it("leaves out a version older than next, so re-running an old run never moves next back", async () => {
    const { calls, steps } = recording({}, { "@neralabs/rerail-pack-a": "0.1.6" }, { "@neralabs/rerail-pack-a": "0.1.8" });
    const { released, superseded } = await releasePacks([entry("a", "0.1.7"), entry("b", "0.1.7")], steps);
    expect(calls).toEqual(["publish b-0.1.7.tgz next"]);
    expect(superseded).toEqual([{ name: "@neralabs/rerail-pack-a", version: "0.1.7", why: "0.1.7 is older than next 0.1.8: publishing it under next would move next back" }]);
    expect(released.map(({ name }) => name)).toEqual(["@neralabs/rerail-pack-b"]);
  });

  it("takes a version a re-run finds on the registry already as released, without publishing it again, and goes on", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: conflict } });
    const { released } = await releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], steps);
    expect(calls).toEqual(["publish a-0.2.0.tgz next", "publish b-0.2.0.tgz next"]);
    expect(released.map(({ name }) => name)).toEqual(["@neralabs/rerail-pack-a", "@neralabs/rerail-pack-b"]);
  });

  it("stops at any other publish failure", async () => {
    const { calls, steps } = recording({ "a-0.2.0.tgz": { ok: false, output: "npm error code ENEEDAUTH" } });
    await expect(releasePacks([entry("a", "0.2.0"), entry("b", "0.2.0")], steps)).rejects.toThrow("npm publish @neralabs/rerail-pack-a@0.2.0 failed");
    expect(calls).toEqual(["publish a-0.2.0.tgz next"]);
  });
});

