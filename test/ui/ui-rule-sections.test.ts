import { afterEach, describe, expect, it } from "vitest";
import { data, keptPacks, ruleSections, ui } from "../../ui/state.js";
import { attachedPacksOf } from "../../ui/poll.js";

type SourceKind = "pack" | "rerail_rule" | "library_rule";

interface StoredRule {
  kind: string;
  origin: "rerail" | "user";
  source: { kind: SourceKind; id: string; packId: string | null };
  folders: string[];
  enabled: boolean;
  rule: { id: string; mode: string; clauses: unknown[] };
}

/** A rule as `poll.js` stores it: present through a pack, added on its own, or created by you. */
const fileRule = (id: string, kind: SourceKind, sourceId: string = id): StoredRule => ({
  kind: "file",
  origin: kind === "library_rule" ? "user" : "rerail",
  source: { kind, id: sourceId, packId: kind === "pack" ? sourceId : null },
  folders: [""],
  enabled: true,
  rule: { id, mode: "steer", clauses: [{ id: "c", violation: { message: id } }] },
});

const pack = (id: string, name: string, rules: Array<[string, string]>) => ({
  id,
  name,
  category: "repository",
  covers: "",
  reason: "",
  rules: rules.map(([ruleId, group]) => ({ id: ruleId, kind: "file", title: ruleId, group, mode: "steer", on: true })),
});

function load(rules: StoredRule[]): void {
  data.rules = new Map(rules.map(r => [r.rule.id, r]));
}

const ids = (rules: Array<{ rule: { id: string } }>) => rules.map(r => r.rule.id);

afterEach(() => {
  ui.filter = "all";
  ui.query = "";
  data.rules = new Map();
  data.packs = [];
  data.attachedPacks = new Map();
  data.repository = null;
  data.recommendations = null;
});

describe("Rules sections", () => {
  it("puts Created rules first, then each added pack in Rerail's order with its own groups, then rules added on their own", () => {
    data.packs = [
      pack("tslint", "TSLint", [["tslint:triple-equals", "TSLint"], ["tslint:no-console", "TSLint"]]),
      pack("acme", "acme", [["acme-api:backend-conditional", "acme-api"], ["acme-sql:hand-rolled-sql", "acme-sql"]]),
    ];
    load([
      fileRule("acme-sql:hand-rolled-sql", "pack", "acme"),
      fileRule("acme-api:backend-conditional", "pack", "acme"),
      fileRule("tslint:no-console", "pack", "tslint"),
      fileRule("acme-spirit:raw-fetch", "rerail_rule"),
      fileRule("mine:todo", "library_rule"),
    ]);

    const sections = ruleSections();

    expect(sections.map(s => s.key)).toEqual(["mine", "pack:tslint", "pack:acme", "legacy"]);
    expect(ids(sections[0]!.groups[0]!.all)).toEqual(["mine:todo"]);
    // A pack group with none of its rules present is left out, not shown empty.
    expect(sections[1]!.groups.map(g => [g.name, ids(g.all)])).toEqual([["TSLint", ["tslint:no-console"]]]);
    // Groups follow the pack's order, not the catalog's.
    expect(sections[2]!.groups.map(g => [g.name, ids(g.all)])).toEqual([
      ["acme-api", ["acme-api:backend-conditional"]],
      ["acme-sql", ["acme-sql:hand-rolled-sql"]],
    ]);
    expect(sections[3]!.groups.map(g => g.name)).toEqual(["acme-spirit"]);
  });

  it("keeps an empty Created rules section, and filters rows without dropping the groups they belong to", () => {
    data.packs = [pack("tslint", "TSLint", [["tslint:triple-equals", "TSLint"]])];
    load([fileRule("tslint:triple-equals", "pack", "tslint")]);

    ui.filter = "mine";
    const [mine, typescript] = ruleSections();
    expect(mine!.groups[0]!.all).toEqual([]);
    expect(typescript!.groups[0]!.all).toHaveLength(1);
    expect(typescript!.groups[0]!.shown).toEqual([]);

    ui.filter = "all";
    ui.query = "tslint";
    expect(ids(ruleSections()[1]!.groups[0]!.shown)).toEqual(["tslint:triple-equals"]);
  });

  it("files a pack's rule added on its own under Other rules by Rerail, named after its pack, when the pack is not added", () => {
    data.packs = [pack("tslint", "TSLint", [["tslint:triple-equals", "TSLint"]]), pack("acme", "acme", [["acme-sql:hand-rolled-sql", "acme-sql"]])];
    load([fileRule("tslint:triple-equals", "pack", "tslint"), fileRule("acme-sql:hand-rolled-sql", "rerail_rule")]);

    const sections = ruleSections();

    expect(sections.map(s => s.key)).toEqual(["mine", "pack:tslint", "legacy"]);
    expect(sections[2]!.groups.map(g => [g.name, ids(g.all)])).toEqual([["acme", ["acme-sql:hand-rolled-sql"]]]);
  });
});

describe("Packs added here", () => {
  it("are the packs rules are present through, with every folder each is attached at, plus unavailable pack attachments", () => {
    const entry = (id: string, kind: SourceKind, sourceId: string, folder: string) => ({ rule: { id }, folder, source: { kind, id: sourceId } });
    const attached = attachedPacksOf(
      [
        entry("ts:a", "pack", "typescript", "web"),
        entry("ts:b", "pack", "typescript", "web"),
        entry("ts:a", "pack", "typescript", "api"),
        entry("acme:x", "rerail_rule", "acme:x", ""),
        entry("mine:todo", "library_rule", "mine:todo", ""),
      ],
      [
        { sourceKind: "pack", sourceId: "retired", folder: "", reason: "missing" },
        { sourceKind: "rerail_rule", sourceId: "gone:rule", folder: "", reason: "missing" },
      ],
    );

    expect([...attached]).toEqual([["typescript", ["web", "api"]], ["retired", [""]]]);
  });
});

describe("Review before starting", () => {
  it("keeps the recommended packs on until the repository is set up, then exactly the packs attached to it", () => {
    data.repository = { setUp: false };
    data.recommendations = [{ packId: "tslint" }, { packId: "acme" }];
    expect(keptPacks()).toEqual(["tslint", "acme"]);

    data.repository = { setUp: true };
    data.attachedPacks = new Map([["acme", [""]]]);
    expect(keptPacks()).toEqual(["acme"]);

    // A repository set up with every pack switched off keeps none, not the recommendation.
    data.attachedPacks = new Map();
    expect(keptPacks()).toEqual([]);
  });
});
