/**
 * The contract between set-up's reading of a repository's lint config and each linter's reader: ports from a catalog,
 * how one linter's decisions for a port's several names combine, how verdicts across linters settle into settings, and
 * how a config file at the root is read. Ports, decisions and verdicts are built by hand.
 */
import { chmodSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  MAX_CONFIG_BYTES,
  portsLinter,
  portsOf,
  readRootFile,
  settleVerdicts,
  type FolderDecision,
  type LintConfigSetting,
  type MappedOptions,
  type Port,
  type PortDecision,
  type PortVerdict,
  type SuppressKind,
} from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { combineDecisions } from "../../../../src/engine/catalog/lint-config/linter-decisions.js";
import { readRepositoryLintConfig } from "../../../../src/engine/catalog/lint-config/repository-lint-config.js";
import type { RuleCatalog, RuleConfig } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { POSIX_MODES } from "../../../support/platform/file-mode.js";
import { linkFolder, symlinkOrSkip } from "../../../support/platform/links.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const NO_NAMES: Port["suppress"] = {
  eslint: [], noqa: [], nosec: [], oxlint: [], biome: [], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [],
};

/** Port `r`, starting on or off as `startsOn`. */
const port = (startsOn: boolean): Port => ({ ruleId: "r", suppress: NO_NAMES, declared: {}, startsOn });

const offX = (because = "x off"): PortDecision => ({ on: false, explicit: true, because });
const onX = (because = "x on", config?: MappedOptions): PortDecision => ({ on: true, explicit: true, because, ...(config === undefined ? {} : { config }) });
const onI = (because = "i on", config?: MappedOptions): PortDecision => ({ on: true, explicit: false, because, ...(config === undefined ? {} : { config }) });
const offI = (because = "i off"): PortDecision => ({ on: false, explicit: false, because });
const unknown = (because = "unread"): PortDecision => ({ on: "unknown", because });
const mapped = (config: RuleConfig): MappedOptions => ({ ok: true, config });

/** The setting for every file of port `r`. */
const whole = (on: boolean | null, explicit: boolean, because: string, config: RuleConfig | null = null): LintConfigSetting =>
  ({ ruleId: "r", folder: "", on, config, explicit, because });

/** What `verdicts`, each a linter's, settle to for port `r` starting as `startsOn`, with the notes. */
function settle(startsOn: boolean, verdicts: readonly PortVerdict[]): { settings: LintConfigSetting[]; notes: string[] } {
  const notes: string[] = [];
  return { settings: settleVerdicts(port(startsOn), verdicts, notes), notes };
}
/** One verdict per decision, each from its own linter. */
const decided = (...decisions: PortDecision[]): PortVerdict[] => decisions.map(decision => ({ decision }));

describe("settleVerdicts", () => {
  const CONFLICT = "r: x on keeps it on, but x off turns it off; it starts off.";
  // Each row: the decisions, the setting for a port starting on, for one starting off, and the notes for both.
  it.each<[string, PortDecision[], LintConfigSetting | undefined, LintConfigSetting | undefined, string[]]>([
    ["no decision", [], undefined, undefined, []],
    ["an explicit off", [offX()], whole(false, true, "x off"), undefined, []],
    ["an explicit on", [onX()], undefined, whole(true, true, "x on"), []],
    ["an implicit on", [onI()], undefined, whole(true, false, "i on"), []],
    ["an unknown", [unknown()], undefined, undefined, []],
    ["an implicit off", [offI()], whole(false, false, "i off"), undefined, []],
    ["an explicit off over an explicit on", [onX(), offX()], whole(false, true, "x off"), undefined, [CONFLICT]],
    ["an explicit off over an unknown", [unknown(), offX()], whole(false, true, "x off"), undefined, []],
    ["an explicit on over an implicit on, naming only the explicit", [onI(), onX()], undefined, whole(true, true, "x on"), []],
    ["an explicit on over an implicit off", [offI(), onX()], undefined, whole(true, true, "x on"), []],
    ["an implicit on over an unknown", [unknown(), onI()], undefined, whole(true, false, "i on"), []],
    ["an unknown over an implicit off: the pack's value", [offI(), unknown()], undefined, undefined, []],
    ["two implicit offs, both named", [offI("a"), offI("b")], whole(false, false, "a; b"), undefined, []],
  ])("settles %s", (_, decisions, startingOn, startingOff, notes) => {
    for (const [startsOn, expected] of [[true, startingOn], [false, startingOff]] as const) {
      expect(settle(startsOn, decided(...decisions))).toEqual({ settings: expected === undefined ? [] : [expected], notes });
    }
  });

  it.each<[string, boolean, PortDecision[], LintConfigSetting | undefined, string[]]>([
    ["options for a rule left on: options only", true, [onX("x", mapped({ a: 1 }))], whole(null, true, "x", { a: 1 }), []],
    ["options for a rule turned on: once named", false, [onX("x", mapped({ a: 1 }))], whole(true, true, "x", { a: 1 }), []],
    ["options from an implicit on, with the explicit on", false, [onX("x"), onI("i", mapped({ a: 1 }))], whole(true, true, "x; i", { a: 1 }), []],
    ["the same options from two linters", true, [onX("x", mapped({ a: 1 })), onX("y", mapped({ a: 1 }))], whole(null, true, "x; y", { a: 1 }), []],
    ["no options at all", true, [onX("x", mapped({}))], undefined, []],
    [
      "different options from two linters: none", true, [onX("x", mapped({ a: 1 })), onX("y", mapped({ a: 2 }))], undefined,
      ["r: x and y set different options; none is used."],
    ],
    [
      "options not mapped: the defaults", true, [onX("x", { ok: false, reason: "no such option" }), onX("y", mapped({ a: 1 }))], undefined,
      ["r: option not mapped (no such option); it keeps its default options. From x."],
    ],
    ["options of a rule that ends off: unused, unremarked", true, [offX(), onI("i", { ok: false, reason: "no such option" })], whole(false, true, "x off"), []],
    ["options of a rule an unknown leaves off", false, [unknown(), onI("i", mapped({ a: 1 })), offI()], whole(true, false, "i", { a: 1 }), []],
  ])("settles %s", (_, startsOn, decisions, expected, notes) => {
    expect(settle(startsOn, decided(...decisions))).toEqual({ settings: expected === undefined ? [] : [expected], notes });
  });

  const folder = (name: string, on: FolderDecision["on"], because: string, explicit = true): FolderDecision =>
    on === "unknown" ? { folder: name, on, because } : { folder: name, on, explicit, because };
  const inFolder = (name: string, on: boolean, because: string, explicit = true): LintConfigSetting => ({ ruleId: "r", folder: name, on, config: null, explicit, because });

  it.each<[string, boolean, PortVerdict[], LintConfigSetting[]]>([
    ["a folder that turns off a rule on everywhere", true, [{ folders: [folder("a", false, "fa"), folder("b", true, "fb")] }], [inFolder("a", false, "fa")]],
    ["a folder that turns on a rule off everywhere", false, [{ folders: [folder("a", false, "fa"), folder("b", true, "fb")] }], [inFolder("b", true, "fb")]],
    ["an off from one linter over an on from another, in one folder", true, [{ folders: [folder("a", true, "x")] }, { folders: [folder("a", false, "y")] }], [inFolder("a", false, "y")]],
    ["two ons in one folder, both named", false, [{ folders: [folder("a", true, "x")] }, { folders: [folder("a", true, "y")] }], [inFolder("a", true, "x; y")]],
    [
      "an unknown folder in a rule on everywhere: the pack's off, by the unknown", false,
      [{ decision: onX(), folders: [folder("a", "unknown", "u"), folder("b", "unknown", "v")] }, { folders: [folder("b", "unknown", "w")] }],
      [whole(true, true, "x on"), inFolder("a", false, "u", false), inFolder("b", false, "v; w", false)],
    ],
    ["an unknown folder in a rule off everywhere: the pack's on", true, [{ decision: offX(), folders: [folder("a", "unknown", "u")] }], [whole(false, true, "x off"), inFolder("a", true, "u", false)]],
    ["an off over an unknown in one folder", true, [{ folders: [folder("a", "unknown", "u")] }, { folders: [folder("a", false, "y")] }], [inFolder("a", false, "y")]],
    ["an on over an unknown in one folder", true, [{ decision: offX(), folders: [folder("a", "unknown", "u")] }, { folders: [folder("a", true, "y")] }], [whole(false, true, "x off"), inFolder("a", true, "y")]],
    ["an unknown folder the same as every file: nothing", true, [{ folders: [folder("a", "unknown", "u")] }], []],
    [
      "a nested folder against the nearest folder holding it, not every file", true,
      [{ folders: [folder("a", false, "fa"), folder("a/b", true, "fab"), folder("a/b/c", true, "fabc"), folder("ab", true, "fab2")] }],
      [inFolder("a", false, "fa"), inFolder("a/b", true, "fab")],
    ],
    [
      "a folder against the settled whole, not the pack's start", true, [{ decision: offX(), folders: [folder("a", true, "fa"), folder("b", false, "fb")] }],
      [whole(false, true, "x off"), inFolder("a", true, "fa")],
    ],
    // A nested config's default (oxlint's nearest config, Biome's root: false) decides only as a default does.
    ["a folder's implicit off under another linter's explicit on everywhere: still on", false, [{ decision: onX() }, { decision: offI(), folders: [folder("a", false, "d", false)] }], [whole(true, true, "x on")]],
    ["a folder's explicit off over another linter's explicit on everywhere", false, [{ decision: onX() }, { folders: [folder("a", false, "y")] }], [whole(true, true, "x on"), inFolder("a", false, "y")]],
    ["a folder's implicit on over another linter's implicit off everywhere", true, [{ decision: offI() }, { decision: offI("j off"), folders: [folder("a", true, "d", false)] }], [whole(false, false, "i off; j off"), inFolder("a", true, "d", false)]],
    [
      "a folder one linter does not decide in, by its decision for the nearest folder holding it", true,
      [{ folders: [folder("a", false, "fa")] }, { decision: onI(), folders: [folder("a/b", true, "fab", false)] }],
      [inFolder("a", false, "fa")],
    ],
  ])("settles %s", (_, startsOn, verdicts, expected) => {
    expect(settle(startsOn, verdicts)).toEqual({ settings: expected, notes: [] });
  });

  it("passes on each verdict's notes, in order, before the notes settling adds", () => {
    expect(settle(true, [{ notes: ["first"] }, { decision: onX() }, { decision: offX(), notes: ["second", "third"] }])).toEqual({
      settings: [whole(false, true, "x off")],
      notes: ["first", "second", "third", "r: x on keeps it on, but x off turns it off; it starts off."],
    });
  });

  describe("for a rule its pack holds off", () => {
    const heldOff: Port = { ...port(false), heldOff: "its steer did harm" };
    const setAside = (because: string) => `r: ${because} would turn it on, but its pack holds it off (its steer did harm); it stays off.`;

    it.each<[string, PortVerdict[], LintConfigSetting[], string[]]>([
      ["never turns it on by the linter's defaults or a preset", decided(onI("default on", mapped({ a: 1 }))), [], [setAside("default on")]],
      ["never turns it on by name, prefix or group", decided(onX("select = [\"B009\"]"), onX("select = [\"B\"]")), [], [setAside("select = [\"B009\"]; select = [\"B\"]")]],
      ["never turns it on in a folder", [{ folders: [folder("a", true, "fa"), folder("b", "unknown", "fb")] }], [], [setAside("fa")]],
      ["keeps an off, setting nothing new and noting no conflict", decided(offX(), onX()), [], [setAside("x on")]],
      ["keeps an off alone, unremarked", decided(offX(), offI(), unknown()), [], []],
    ])("%s", (_, verdicts, expected, notes) => {
      const settled: string[] = [];
      expect(settleVerdicts(heldOff, verdicts, settled)).toEqual(expected);
      expect(settled).toEqual(notes);
    });

    it("keeps Python's B009 and B010 off under a Ruff config that sets only the line length", async () => {
      const root = await mkdtemp(join(tmpdir(), "rerail-held-off-"));
      try {
        writeFileSync(join(root, "ruff.toml"), "line-length = 100\n");
        const python = readRuleCatalog([fileURLToPath(new URL("../../../../packs/python", import.meta.url))]);
        const held = ["ruff-b009-get-attr-with-constant", "ruff-b010-set-attr-with-constant"];
        expect(portsOf(python, ["python"]).filter(found => held.includes(found.ruleId)).map(found => [found.startsOn, found.heldOff !== undefined])).toEqual([[false, true], [false, true]]);
        const config = readRepositoryLintConfig(root, python, ["python"]);
        // Ruff's default selection turns both on: settling notes that and sets it aside.
        expect(config.settings.filter(setting => held.includes(setting.ruleId))).toEqual([]);
        expect(held.map(ruleId => config.notes.some(note => note.startsWith(`${ruleId}: ruff.toml`) && note.includes("its pack holds it off")))).toEqual([true, true]);
      } finally {
        await removeFolder(root);
      }
    });
  });
});

describe("combineDecisions", () => {
  it.each<[string, PortDecision[], PortDecision | undefined]>([
    ["nothing from no decisions", [], undefined],
    ["an explicit off over everything", [onX("a"), onI("b"), unknown("c"), offI("d"), offX("e")], offX("e")],
    ["an explicit on over an implicit on, an unknown and an implicit off", [offI("d"), unknown("c"), onI("b"), onX("a")], onX("a")],
    ["an implicit on over an unknown", [unknown("c"), onI("b")], onI("b")],
    ["an unknown over an implicit off", [offI("d"), unknown("c"), unknown("e")], unknown("c; e")],
    ["an implicit off alone", [offI("d")], offI("d")],
    ["the reasons of one tier joined, each once", [offX("a"), offX("b"), offX("a")], offX("a; b")],
    ["the first options of the tier", [onX("a"), onX("b", mapped({ n: 1 })), onX("c", mapped({ n: 2 }))], onX("a; b; c", mapped({ n: 1 }))],
  ])("comes to %s", (_, decisions, expected) => {
    expect(combineDecisions(decisions)).toEqual(expected);
  });
});

describe("portsOf", () => {
  /** A code rule document: its clauses' `suppress` and `define` options as given. */
  const code = (clauses: readonly unknown[]) => ({ kind: "code", rule: { clauses } });
  const rule = (id: string, on = true, heldOff?: string) => ({ id, on, ...(heldOff === undefined ? {} : { heldOff }) });
  const catalog = {
    packs: new Map([
      ["one", { rules: [rule("all-kinds"), rule("conversation"), rule("no-document"), rule("plain", false, "its steer did harm")] }],
      ["two", { rules: [rule("plain", false, "its steer did harm"), rule("define-only")] }],
    ]),
    documents: new Map<string, unknown>([
      ["all-kinds", code([
        {
          violation: {
            suppress: {
              eslint: { rules: ["e1", "e2"] }, noqa: { codes: ["N1"] }, nosec: { codes: ["B1"] }, oxlint: { rules: ["o1"] },
              biome: { rules: ["lint/a/b"] }, phpstan: { identifiers: ["p.i"] }, psalm: { issues: ["Issue"] },
              phpcs: { sniffs: ["S.s"] }, clippy: { lints: ["clippy::c"] }, sqlfluff: { rules: ["LT01"] },
            },
          },
          define: { options: { a: { type: "boolean" } } },
        },
        { violation: { suppress: { eslint: { rules: ["e2", "e3"] } } }, define: { options: { b: { type: "number" } } } },
        { violation: {}, define: {} },
        {},
      ])],
      ["conversation", { kind: "conversation", rule: {} }],
      ["plain", { kind: "code", rule: { clauses: [{ violation: { suppress: {} } }], scope: { include: ["**/*.py"], exclude: ["tests/**"] } } }],
      ["define-only", code([{ define: { options: { c: { type: "string" } } } }])],
    ]),
  } as unknown as RuleCatalog;

  it("ports each code rule of the packs once, with every suppress name over its clauses and every declared option", () => {
    expect(portsOf(catalog, ["one", "missing", "two"])).toEqual([
      {
        ruleId: "all-kinds",
        suppress: {
          eslint: ["e1", "e2", "e3"], noqa: ["N1"], nosec: ["B1"], oxlint: ["o1"], biome: ["lint/a/b"], phpstan: ["p.i"],
          psalm: ["Issue"], phpcs: ["S.s"], clippy: ["clippy::c"], sqlfluff: ["LT01"],
        },
        declared: { a: { type: "boolean" }, b: { type: "number" } },
        startsOn: true,
      },
      { ruleId: "plain", suppress: NO_NAMES, declared: {}, startsOn: false, heldOff: "its steer did harm", scope: { include: ["**/*.py"], exclude: ["tests/**"] } },
      { ruleId: "define-only", suppress: NO_NAMES, declared: { c: { type: "string" } }, startsOn: true },
    ]);
  });

  it("ports nothing for packs not in the catalog", () => {
    expect(portsOf(catalog, ["missing"])).toEqual([]);
  });

  it("makes a port one of a linter's when it has a name of one of the linter's kinds", () => {
    const [allKinds, plain] = portsOf(catalog, ["one"]);
    const kindsOf: SuppressKind[][] = [["noqa"], ["nosec", "noqa"], []];
    expect(kindsOf.map(kinds => portsLinter(allKinds!, kinds))).toEqual([true, true, false]);
    expect(portsLinter(plain!, ["eslint", "noqa", "nosec", "oxlint", "biome", "phpstan", "psalm", "phpcs", "clippy", "sqlfluff"])).toBe(false);
  });
});

describe("readRootFile", () => {
  let root: string;
  beforeAll(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-root-file-")));
    mkdirSync(join(root, "nested", "deeper"), { recursive: true });
    writeFileSync(join(root, "config.json"), "{}");
    writeFileSync(join(root, "nested", "deeper", "config.toml"), "a = 1");
    writeFileSync(join(root, "largest.json"), "x".repeat(MAX_CONFIG_BYTES));
    writeFileSync(join(root, "too-large.json"), "x".repeat(MAX_CONFIG_BYTES + 1));
    writeFileSync(join(root, "unreadable.json"), "{}");
    if (POSIX_MODES) chmodSync(join(root, "unreadable.json"), 0o000);
    linkFolder(join(root, "nested"), join(root, "linked-folder"));
  });
  afterAll(async () => {
    await removeFolder(root);
  });

  it.each<[string, string, unknown]>([
    ["a file at the root", "config.json", { present: true, text: "{}" }],
    ["a file in folders", "nested/deeper/config.toml", { present: true, text: "a = 1" }],
    ["a file of the largest size read", "largest.json", { present: true, text: "x".repeat(MAX_CONFIG_BYTES) }],
    ["nothing at the path", "absent.json", { present: false }],
    ["nothing in a folder that is not there", "absent/config.json", { present: false }],
    ["a folder", "nested", { present: true, text: undefined }],
    ["a file over the largest size", "too-large.json", { present: true, text: undefined }],
    ["a file through a linked folder", "linked-folder/deeper/config.toml", { present: true, text: undefined }],
  ])("reads %s", (_, name, expected) => {
    expect(readRootFile(root, name)).toEqual(expected);
  });

  it("reads a symlink to a file as present, without text", context => {
    symlinkOrSkip(context, join(root, "config.json"), join(root, "linked.json"));
    expect(readRootFile(root, "linked.json")).toEqual({ present: true, text: undefined });
  });

  // A file mode denies root nothing, and Windows keeps no mode bits: there no file of the repository is unreadable this way.
  it.skipIf(!POSIX_MODES || process.getuid?.() === 0)("reads a file it may not read as present, without text", () => {
    expect(readRootFile(root, "unreadable.json")).toEqual({ present: true, text: undefined });
  });
});
