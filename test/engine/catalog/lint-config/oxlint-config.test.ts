import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { LinterNote, Port, PortVerdict } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import { oxlintReader } from "../../../../src/engine/catalog/lint-config/oxlint-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** A repository holding `files` (path → text). */
async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-oxlint-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
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

/** What reading a repository gives: the verdict on a port, the notes reading it gave and whether it read a config. */
interface Read {
  readonly verdict: PortVerdict | undefined;
  readonly notes: string[];
  readonly read: boolean;
}

/** The verdict on `ported` of the repository holding `files`, with the notes reading it gave and whether it read a config. */
async function read(files: Readonly<Record<string, string>>, ported: Port): Promise<Read> {
  const notes: LinterNote[] = [];
  const config = oxlintReader.read(await repository(files), notes);
  return { verdict: config?.verdict(ported, ruleCatalog), notes: notes.map(note => note.text), read: config !== undefined };
}

/** The same, for a root `.oxlintrc.json` holding `config`. */
const oxlint = (config: unknown, ported: Port): Promise<Read> => read({ ".oxlintrc.json": JSON.stringify(config) }, ported);

// oxlint 1.88.0: no-debugger is ESLint's, correctness; eqeqeq is ESLint's, pedantic; nextjs's google-font-display is
// correctness; node's no-process-env is restriction; typescript's await-thenable is correctness and type-aware.
const DEBUGGER = port(["no-debugger"], ["no-debugger"]);
const EQEQEQ = port(["eqeqeq"], ["eqeqeq"]);
const FONT_DISPLAY = port(["@next/next/google-font-display"], ["google-font-display"]);
const PROCESS_ENV = port(["n/no-process-env"], ["no-process-env"]);
const AWAIT_THENABLE = port(["@typescript-eslint/await-thenable"], ["await-thenable"]);
const BY_DEFAULT = { on: true, explicit: false, because: ".oxlintrc.json: oxlint 1.88.0's default (the correctness category)" };
const NOT_ON = { on: false, explicit: false, because: ".oxlintrc.json does not turn it on" };
const refused = (why: string, file = ".oxlintrc.json"): string => `${file} is not read: oxlint 1.88.0 refuses it (${why}), so it runs no rule.`;

describe("oxlintReader: plugins", () => {
  it.each([
    [["nextjs"], BY_DEFAULT],
    [["oxlint-plugin-nextjs"], BY_DEFAULT],
    [["eslint-plugin-react", "nextjs"], BY_DEFAULT],
    [[], NOT_ON],
  ])("runs a plugin's rules by any name plugins takes for it: plugins %j", async (plugins, decision) => {
    expect((await oxlint({ plugins }, FONT_DISPLAY)).verdict).toEqual({ decision, notes: [] });
  });

  it.each([
    [["@next/next"], "its plugins name \"@next/next\", which is no plugin built into oxlint"],
    [["@next/eslint-plugin-next"], "its plugins name \"@next/eslint-plugin-next\", which is no plugin built into oxlint"],
    [["eslint-plugin-next"], "its plugins name \"eslint-plugin-next\", which is no plugin built into oxlint"],
    ["nextjs", "its plugins are not a list of plugin names"],
  ])("reads no config whose plugins oxlint refuses: plugins %j", async (plugins, why) => {
    expect(await oxlint({ plugins }, FONT_DISPLAY)).toEqual({ verdict: undefined, notes: [refused(why)], read: false });
  });

  it("models eslint-plugin-n's rules as oxlint's node plugin, whose n/ key oxlint refuses", async () => {
    expect((await oxlint({ plugins: ["node"], rules: { "node/no-process-env": "error" } }, PROCESS_ENV)).verdict)
      .toEqual({ decision: { on: true, explicit: true, because: ".oxlintrc.json: \"node/no-process-env\": \"error\"" }, notes: [] });
    expect(await oxlint({ plugins: ["node"], rules: { "n/no-process-env": "error" } }, PROCESS_ENV))
      .toEqual({ verdict: undefined, notes: [refused("\"n/no-process-env\" names a plugin it does not load")], read: false });
  });

  it("has no verdict for a port without a built-in oxlint rule", async () => {
    expect((await oxlint({}, port(["custom/no-foo"], ["no-foo"]))).verdict).toBeUndefined();
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

  it("takes a key of a plugin jsPlugins loads, whose rules Rerail does not know", async () => {
    expect((await oxlint({ jsPlugins: ["./custom.mjs"], rules: { "custom/no-foo": "error", "no-debugger": "off" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" }, notes: [] });
  });
});

describe("oxlintReader: categories, severities and type-aware rules", () => {
  it("leaves a type-aware rule unknown where it would be on, as only --type-aware runs it, unless options.typeAware does", async () => {
    expect((await oxlint({}, AWAIT_THENABLE)).verdict).toEqual({
      decision: { on: "unknown", because: ".oxlintrc.json: oxlint 1.88.0's default (the correctness category), for a rule oxlint runs only with --type-aware" },
      notes: [],
    });
    expect((await oxlint({ rules: { "typescript/await-thenable": "error" } }, AWAIT_THENABLE)).verdict).toEqual({
      decision: { on: "unknown", because: ".oxlintrc.json: \"typescript/await-thenable\": \"error\", for a rule oxlint runs only with --type-aware" },
      notes: [],
    });
    expect((await oxlint({ categories: { correctness: "off" } }, AWAIT_THENABLE)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: categories.correctness: \"off\"" }, notes: [] });
    expect((await oxlint({ options: { typeAware: true } }, AWAIT_THENABLE)).verdict).toEqual({ decision: BY_DEFAULT, notes: [] });
  });

  it("reads no config where the repository has no oxlint config", async () => {
    expect(await read({}, DEBUGGER)).toEqual({ verdict: undefined, notes: [], read: false });
  });

  it("takes an entry's severity, noting oxlint's options it does not map, and passes over another rule's bare key", async () => {
    expect((await oxlint({ rules: { "no-debugger": "off", "no-console": "error" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" }, notes: [] });
    expect((await oxlint({ rules: { "no-console": "off" } }, DEBUGGER)).verdict).toEqual({ decision: BY_DEFAULT, notes: [] });
    expect((await oxlint({ rules: { eqeqeq: ["error", "smart"] } }, EQEQEQ)).verdict).toEqual({
      decision: { on: true, explicit: true, because: ".oxlintrc.json: \"eqeqeq\": [\"error\",\"smart\"]" },
      notes: [`${PORT}: option not mapped: Rerail maps ESLint options, not oxlint's (.oxlintrc.json: "eqeqeq": ["error","smart"]).`],
    });
  });

  it("turns a category on or off by its level, and leaves one it does not set and is not the default off", async () => {
    expect((await oxlint({ categories: { pedantic: 2 } }, EQEQEQ)).verdict)
      .toEqual({ decision: { on: true, explicit: true, because: ".oxlintrc.json: categories.pedantic: 2" }, notes: [] });
    expect((await oxlint({ categories: { correctness: "allow" } }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: categories.correctness: \"allow\"" }, notes: [] });
    expect((await oxlint({}, EQEQEQ)).verdict).toEqual({ decision: NOT_ON, notes: [] });
  });

  it.each([
    [{ rules: { "no-debugger": "loud" } }, "its rules: \"no-debugger\": \"loud\" is not a severity oxlint takes"],
    [{ rules: { "no-such-rule": "off" } }, "\"no-such-rule\" names no rule oxlint's eslint plugin has"],
    [{ rules: { "react/no-console": "off" } }, "\"react/no-console\" names no rule oxlint's react plugin has"],
    [{ rules: { "custom/no-console": "off" } }, "\"custom/no-console\" names a plugin it does not load"],
    [{ categories: { correctness: "loud" } }, "its categories: \"correctness\": \"loud\" is not a category and level oxlint takes"],
    [{ categories: { correctness: ["error"] } }, "its categories: \"correctness\": [\"error\"] is not a category and level oxlint takes"],
    [{ categories: { bogus: "off" } }, "its categories: \"bogus\": \"off\" is not a category and level oxlint takes"],
    [{ categories: ["correctness"] }, "its rules, categories or options are not objects of what oxlint takes"],
    [{ rules: ["no-debugger"] }, "its rules, categories or options are not objects of what oxlint takes"],
    [{ options: { typeAware: "yes" } }, "its rules, categories or options are not objects of what oxlint takes"],
    [{ bogus: 1 }, "it has a key \"bogus\" oxlint does not take"],
  ])("reads no config oxlint refuses: %j", async (config, why) => {
    expect(await oxlint(config, DEBUGGER)).toEqual({ verdict: undefined, notes: [refused(why)], read: false });
  });
});

describe("oxlintReader: what it does not read", () => {
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

  it("notes ignorePatterns, which it does not read", async () => {
    expect((await oxlint({ ignorePatterns: ["dist/**"] }, DEBUGGER)).notes)
      .toEqual([".oxlintrc.json: its ignorePatterns are not read; the rules Rerail sets apply to the files they ignore too."]);
  });

  it("leaves the rules unknown where the config is an oxlint.config.ts, or not JSON", async () => {
    expect(await read({ "oxlint.config.ts": "export default {};\n" }, DEBUGGER)).toEqual({
      verdict: { decision: { on: "unknown", because: "oxlint.config.ts is not read" }, notes: [] },
      notes: ["oxlint.config.ts is not read: Rerail never runs a config; the oxlint rules in the repository keep the pack's values."],
      read: true,
    });
    expect(await read({ ".oxlintrc.json": "[]" }, DEBUGGER)).toEqual({
      verdict: { decision: { on: "unknown", because: ".oxlintrc.json is not read" }, notes: [] },
      notes: [".oxlintrc.json is not read: it is not a JSON object (Rerail reads JSON lint configs only)."],
      read: true,
    });
  });
});

describe("oxlintReader: config files per folder", () => {
  it("reads an .oxlintrc.jsonc, and JSONC in an .oxlintrc.json", async () => {
    expect((await read({ ".oxlintrc.jsonc": "{ // ours\n \"rules\": { \"no-debugger\": \"off\", }, }" }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.jsonc: \"no-debugger\": \"off\"" }, notes: [] });
    expect((await read({ ".oxlintrc.json": "{ /* ours */ \"rules\": { \"no-debugger\": \"off\" } }" }, DEBUGGER)).verdict)
      .toEqual({ decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" }, notes: [] });
  });

  it("decides a folder holding its own config by that config alone, where it differs from the folder holding it", async () => {
    const files = {
      ".oxlintrc.json": JSON.stringify({ rules: { "no-debugger": "off" } }),
      "sub/.oxlintrc.json": JSON.stringify({ categories: { pedantic: "error" } }),
      "sub/deeper/.oxlintrc.jsonc": "{ \"rules\": { \"no-debugger\": \"allow\" } }",
      "sub/same/.oxlintrc.json": "{ \"rules\": { \"no-debugger\": \"warn\" } }",
      "other/.oxlintrc.json": JSON.stringify({ categories: { correctness: "off" } }),
    };
    // other/ has it as the folder holding it does; sub/same/ names the rule where sub/ has it by default, which settles otherwise.
    expect((await read(files, DEBUGGER)).verdict).toEqual({
      decision: { on: false, explicit: true, because: ".oxlintrc.json: \"no-debugger\": \"off\"" },
      folders: [
        { folder: "sub", on: true, explicit: false, because: "sub/.oxlintrc.json: oxlint 1.88.0's default (the correctness category)" },
        { folder: "sub/deeper", on: false, explicit: true, because: "sub/deeper/.oxlintrc.jsonc: \"no-debugger\": \"allow\"" },
        { folder: "sub/same", on: true, explicit: true, because: "sub/same/.oxlintrc.json: \"no-debugger\": \"warn\"" },
      ],
      notes: [],
    });
  });

  it("decides only the folders holding a config where the root holds none, and leaves one with an oxlint.config.ts unknown", async () => {
    const files = { "sub/.oxlintrc.json": JSON.stringify({ rules: { "no-debugger": "off" } }), "other/oxlint.config.ts": "export default {};\n" };
    expect(await read(files, DEBUGGER)).toEqual({
      verdict: {
        folders: [
          { folder: "other", on: "unknown", because: "other/oxlint.config.ts is not read" },
          { folder: "sub", on: false, explicit: true, because: "sub/.oxlintrc.json: \"no-debugger\": \"off\"" },
        ],
        notes: [],
      },
      notes: ["other/oxlint.config.ts is not read: Rerail never runs a config; the oxlint rules in other/ keep the pack's values."],
      read: true,
    });
  });

  it("reads no config where a folder holds two config files or a nested config oxlint refuses, as oxlint then runs nothing", async () => {
    expect(await read({ ".oxlintrc.json": "{}", ".oxlintrc.jsonc": "{}" }, DEBUGGER)).toEqual({
      verdict: undefined,
      notes: [".oxlintrc.json and .oxlintrc.jsonc are not read: oxlint 1.88.0 takes one config file per folder and refuses to run with more, so it runs no rule."],
      read: false,
    });
    expect(await read({ ".oxlintrc.json": "{}", "sub/.oxlintrc.json": JSON.stringify({ rules: { "bogus-rule": "off" } }) }, DEBUGGER)).toEqual({
      verdict: undefined,
      notes: [refused("\"bogus-rule\" names no rule oxlint's eslint plugin has", "sub/.oxlintrc.json")],
      read: false,
    });
  });
});
