import { realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { LinterNote, Port, PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { oxlintReader } from "../../../../src/engine/catalog/lint-config/oxlint-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** A repository whose `.oxlintrc.json` holds `config`; without one where `config` is undefined. */
async function repository(config: unknown): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-oxlint-config-")));
  temporaryDirectories.push(root);
  if (config !== undefined) writeFileSync(join(root, ".oxlintrc.json"), JSON.stringify(config));
  return root;
}

// The oxlint reader does not look into the catalog; an empty one stands in.
let ruleCatalog: RuleCatalog;
beforeAll(() => {
  ruleCatalog = readRuleCatalog([]);
});

const PORT = "oxlint-port";

/** A port of ESLint rules `eslint`, honouring oxlint's bare names `oxlint`. */
const port = (eslint: readonly string[], oxlint: readonly string[]): Port => ({
  ruleId: PORT,
  suppress: { eslint, noqa: [], nosec: [], oxlint, biome: [], phpstan: [], psalm: [], phpcs: [], clippy: [], sqlfluff: [] },
  declared: {},
  startsOn: true,
});

/** The verdict of the `.oxlintrc.json` holding `config` on `ported` (undefined without a config), with the notes reading it gave. */
async function oxlint(config: unknown, ported: Port): Promise<{ verdict: PortVerdict | undefined; notes: string[]; read: boolean }> {
  const notes: LinterNote[] = [];
  const read = oxlintReader.read(await repository(config), notes);
  return { verdict: read?.verdict(ported, ruleCatalog), notes: notes.map(note => note.text), read: read !== undefined };
}

// oxlint 1.88.0: no-debugger is ESLint's, correctness; nextjs's google-font-display is correctness; node's
// no-process-env is restriction; typescript's await-thenable is correctness and type-aware.
const DEBUGGER = port(["no-debugger"], ["no-debugger"]);
const FONT_DISPLAY = port(["@next/next/google-font-display"], ["google-font-display"]);
const PROCESS_ENV = port(["n/no-process-env"], ["no-process-env"]);
const AWAIT_THENABLE = port(["@typescript-eslint/await-thenable"], ["await-thenable"]);
const BY_DEFAULT = { on: true, explicit: false, because: ".oxlintrc.json: oxlint 1.88.0's default (the correctness category)" };
const NOT_ON = { on: false, explicit: false, because: ".oxlintrc.json does not turn it on" };

describe("oxlintReader: plugins", () => {
  it.each([
    [["nextjs"], BY_DEFAULT],
    [["@next/next"], BY_DEFAULT],
    [["@next/eslint-plugin-next"], BY_DEFAULT],
    [["@next/eslint-plugin"], BY_DEFAULT],
    [["oxlint-plugin-nextjs"], BY_DEFAULT],
    // eslint-plugin-next is no package oxlint knows: an external plugin, which runs none of oxlint's own rules.
    [["eslint-plugin-next"], NOT_ON],
    [[], NOT_ON],
  ])("runs a plugin's rules by any name oxlint takes for it: plugins %j", async (plugins, decision) => {
    expect((await oxlint({ plugins }, FONT_DISPLAY)).verdict).toEqual({ decision, notes: [] });
  });

  it("models eslint-plugin-n's rules as oxlint's node plugin, which takes no n/ key", async () => {
    expect((await oxlint({ plugins: ["node"], rules: { "node/no-process-env": "error" } }, PROCESS_ENV)).verdict)
      .toEqual({ decision: { on: true, explicit: true, because: ".oxlintrc.json: \"node/no-process-env\": \"error\"" }, notes: [] });
    expect((await oxlint({ plugins: ["node"], rules: { "n/no-process-env": "error" } }, PROCESS_ENV)).verdict).toEqual({ decision: NOT_ON, notes: [] });
  });

  it("decides nothing for a rule of an external plugin no entry names, and has no verdict for a port without an oxlint rule", async () => {
    expect((await oxlint({}, port(["custom/no-foo"], ["no-foo"]))).verdict).toEqual({ notes: [] });
    expect((await oxlint({}, port(["no-debugger"], []))).verdict).toBeUndefined();
    expect((await oxlint({}, port(["no-debugger"], ["no-console"]))).verdict).toBeUndefined();
  });

  it("takes a typescript-eslint extension rule's key as ESLint's rule, which oxlint runs for it", async () => {
    const defaultParamLast = port(["@typescript-eslint/default-param-last"], ["default-param-last"]);
    expect((await oxlint({ rules: { "typescript/default-param-last": "off" } }, defaultParamLast)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"typescript/default-param-last\": \"off\"" }, notes: [] });
  });

  it("leaves off a rule an entry turns on in a plugin oxlint does not run", async () => {
    expect((await oxlint({ rules: { "nextjs/google-font-display": "error" } }, FONT_DISPLAY)).verdict).toEqual({
      decision: { on: false, explicit: false, because: ".oxlintrc.json: \"nextjs/google-font-display\": \"error\", but .oxlintrc.json does not run the nextjs plugin" },
      notes: [],
    });
  });
});

describe("oxlintReader: categories and severities", () => {
  it("leaves a type-aware rule its category turns on unknown, as only --type-aware runs it", async () => {
    expect((await oxlint({}, AWAIT_THENABLE)).verdict).toEqual({
      decision: { on: "unknown", because: ".oxlintrc.json: oxlint 1.88.0's default (the correctness category), for a rule oxlint runs only with --type-aware" },
      notes: [],
    });
    expect((await oxlint({ categories: { correctness: "off" } }, AWAIT_THENABLE)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: categories.correctness: \"off\"" }, notes: [] });
  });

  it("reads no config where the repository has no .oxlintrc.json", async () => {
    expect(await oxlint(undefined, DEBUGGER)).toEqual({ verdict: undefined, notes: [], read: false });
  });

  it("takes an entry's severity, noting oxlint's options it does not map, and passes over another rule's bare key", async () => {
    expect((await oxlint({ rules: { "no-debugger": "off", "no-console": "error" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" }, notes: [] });
    expect((await oxlint({ rules: { "no-console": "off" } }, DEBUGGER)).verdict).toEqual({ decision: BY_DEFAULT, notes: [] });
    expect((await oxlint({ rules: { "no-debugger": ["error", { x: 1 }] } }, DEBUGGER)).verdict).toEqual({
      decision: { on: true, explicit: true, because: ".oxlintrc.json: \"no-debugger\": [\"error\",{\"x\":1}]" },
      notes: [`${PORT}: option not mapped: Rerail maps ESLint options, not oxlint's (.oxlintrc.json: "no-debugger": ["error",{"x":1}]).`],
    });
  });

  it("leaves a rule unknown for an entry or a category level oxlint does not take", async () => {
    expect((await oxlint({ rules: { "no-debugger": "loud" } }, DEBUGGER)).verdict).toEqual({
      decision: { on: "unknown", because: ".oxlintrc.json: \"no-debugger\": \"loud\"" },
      notes: [`${PORT}: .oxlintrc.json: "no-debugger": "loud" is not a severity oxlint takes; not read.`],
    });
    expect((await oxlint({ categories: { correctness: "loud" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: "unknown", because: ".oxlintrc.json: categories.correctness: \"loud\" is not a level oxlint takes" }, notes: [] });
  });
});

describe("oxlintReader: what it does not read", () => {
  it("reads no config whose rules are not an object, saying so", async () => {
    expect(await oxlint({ rules: ["no-debugger"] }, DEBUGGER))
      .toEqual({ verdict: undefined, notes: [".oxlintrc.json is not read: its rules are not an object."], read: false });
  });

  it.each([
    ["plugins", { plugins: "nextjs" }, ".oxlintrc.json: its plugins are not read: they are not a list of plugin names."],
    ["categories", { categories: ["correctness"] }, ".oxlintrc.json: its categories are not read: they are not an object."],
  ])("leaves a rule no entry sets unknown where its %s cannot be read, and takes an entry as written", async (_, config, note) => {
    expect(await oxlint(config, DEBUGGER))
      .toEqual({ verdict: { decision: { on: "unknown", because: ".oxlintrc.json is not read in full" }, notes: [] }, notes: [note], read: true });
    expect((await oxlint({ ...config, rules: { "no-debugger": "warn" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: true, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"warn\"" }, notes: [] });
  });

  it("leaves a rule no entry sets unknown where its extends and overrides, not read, may set it", async () => {
    expect(await oxlint({ extends: ["./base.json"], overrides: [] }, DEBUGGER)).toEqual({
      verdict: { decision: { on: "unknown", because: ".oxlintrc.json extends and .oxlintrc.json overrides, which Rerail does not read, may set it" }, notes: [] },
      notes: [".oxlintrc.json: only its own rules are read, not extends or overrides."],
      read: true,
    });
  });

  it("does not use an entry the unread overrides may replace for some files, and uses one only extends sits beside", async () => {
    expect((await oxlint({ rules: { "no-debugger": "off" }, overrides: [] }, DEBUGGER)).verdict).toEqual({
      decision: { on: "unknown", because: ".oxlintrc.json: \"no-debugger\": \"off\"" },
      notes: [`${PORT}: .oxlintrc.json: "no-debugger": "off" is not used: .oxlintrc.json overrides, which Rerail does not read, may set it for some files.`],
    });
    expect((await oxlint({ rules: { "no-debugger": "off" }, extends: [] }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" }, notes: [] });
  });
});
