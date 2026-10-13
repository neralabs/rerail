import { existsSync, mkdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { readBundledCatalog } from "../../../src/engine/catalog/bundled-rules.js";
import { LintConfigFormatError, parseJsonc, readTomlStringLists } from "../../../src/engine/catalog/lint-config-formats.js";
import type { BundledCatalog } from "../../../src/engine/catalog/repository-model.js";
import {
  proposeLintConfigSettings,
  readLintConfigFiles,
  readRepositoryLintConfig,
  ruffSelects,
} from "../../../src/engine/catalog/repository-lint-config.js";

const TYPE_DEFINITIONS = "typescript-eslint-consistent-type-definitions";
const TSLINT_COMMENT = "typescript-eslint-ban-tslint-comment";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-lint-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, name)), { recursive: true });
    writeFileSync(join(root, name), text);
  }
  return root;
}

const packDirectory = (id: string): string => decodeURIComponent(new URL(`../../../packs/${id}`, import.meta.url).pathname);

const HOOKS_RULE = "react-hooks-rules-of-hooks";

/**
 * A pack with one port of eslint-plugin-react-hooks' rules-of-hooks, which oxlint also implements: the javascript
 * pack's no-console document with its suppress replaced. Only its upstream names matter here.
 */
async function hooksPack(): Promise<string> {
  const directory = join(realpathSync(await mkdtemp(join(tmpdir(), "rerail-hooks-pack-"))), "hooks");
  mkdirSync(join(directory, "rules"), { recursive: true });
  const document = JSON.parse(readFileSync(join(packDirectory("javascript"), "rules", "eslint-no-console.json"), "utf8")) as {
    id: string;
    clauses: { violation?: { suppress?: unknown } }[];
  };
  document.id = HOOKS_RULE;
  for (const clause of document.clauses) {
    if (clause.violation?.suppress !== undefined) {
      clause.violation.suppress = { eslint: { rules: ["react-hooks/rules-of-hooks"] }, oxlint: { rules: ["rules-of-hooks"] } };
    }
  }
  writeFileSync(join(directory, "rules", `${HOOKS_RULE}.json`), JSON.stringify(document));
  writeFileSync(join(directory, "pack.json"), JSON.stringify({
    format: "rerail.rule-pack/v1",
    id: "hooks",
    name: "Hooks",
    category: "framework",
    covers: "React hooks.",
    reason: "React.",
    rules: [{ id: HOOKS_RULE, document: `rules/${HOOKS_RULE}.json`, title: "Rules of hooks", group: "React" }],
  }));
  return directory;
}

let bundle: BundledCatalog;
let hooksDirectory: string;
beforeAll(async () => {
  hooksDirectory = await hooksPack();
  bundle = readBundledCatalog([...["typescript", "javascript", "python"].map(packDirectory), hooksDirectory]);
  return () => rm(join(hooksDirectory, ".."), { recursive: true, force: true });
});

describe("parseJsonc", () => {
  it("reads comments and trailing commas, leaving strings alone", () => {
    const text = [
      "\uFEFF{",
      "  // a line comment, with a trailing comma after it",
      "  \"rules\": { \"no-console\": \"off\", /* a block, comment */ },",
      "  \"url\": \"https://example.com/a,]\", // the string keeps its // and ,]",
      "  \"list\": [1, 2, /* three */],",
      "}",
    ].join("\n");
    expect(parseJsonc(text)).toEqual({ rules: { "no-console": "off" }, url: "https://example.com/a,]", list: [1, 2] });
  });

  it("refuses YAML and an unclosed comment", () => {
    expect(() => parseJsonc("rules:\n  no-console: off\n")).toThrow(LintConfigFormatError);
    expect(() => parseJsonc("{ /* open ")).toThrow(LintConfigFormatError);
    expect(() => parseJsonc("{ \"a\": \"open }")).toThrow(LintConfigFormatError);
  });
});

describe("readTomlStringLists", () => {
  const PYPROJECT_KEYS = ["tool.ruff.ignore", "tool.ruff.extend-ignore", "tool.ruff.lint.ignore", "tool.ruff.lint.extend-ignore"];

  it("reads the wanted lists across lines, with comments, both quotes and a trailing comma", () => {
    const text = [
      "[project]",
      "name = \"acme\" # a comment",
      "description = \"\"\"",
      "[tool.ruff.lint]",
      "ignore = [\"X1\"]",
      "\"\"\"",
      "version = 1.2",
      "released = 1979-05-27 07:32:00Z",
      "",
      "[tool.ruff]",
      "line-length = 100",
      "lint.extend-ignore = ['D1']",
      "",
      "[tool.ruff.lint]",
      "ignore = [",
      "  \"E7\",   # comparisons",
      "  'F401',",
      "]",
      "select = [\"E\", \"F\"]",
      "",
      "[[tool.other]]",
      "ignore = [1]",
    ].join("\n");
    const found = readTomlStringLists(text, PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.lint.extend-ignore": ["D1"], "tool.ruff.lint.ignore": ["E7", "F401"] });
    expect(found.paths).toContain("tool.ruff");
  });

  it("reads inline tables, and keeps a quoted dotted key apart", () => {
    const found = readTomlStringLists("[tool]\nruff = { lint = { ignore = [\"E1\"] }, \"lint.extend-ignore\" = [\"E2\"] }\n", PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.lint.ignore": ["E1"] });
  });

  it("refuses what it cannot read rather than guess", () => {
    expect(() => readTomlStringLists("[tool.ruff.lint]\nignore = \"E7\"\n", PYPROJECT_KEYS)).toThrow("line 2: tool.ruff.lint.ignore is not a list of strings");
    expect(() => readTomlStringLists("[tool.ruff.lint]\nignore = [\"E7\"\n", PYPROJECT_KEYS)).toThrow(LintConfigFormatError);
    expect(() => readTomlStringLists("[tool.ruff]\nlint.ignore = []\n[tool.ruff.lint]\nignore = []\n", PYPROJECT_KEYS)).toThrow("is set twice");
    expect(() => readTomlStringLists("[tool.ruff.lint]\nignore = [\"E7\"] extra\n", PYPROJECT_KEYS)).toThrow("expected the end of the line");
    expect(() => readTomlStringLists("key = \"\\q\"\n", PYPROJECT_KEYS)).toThrow("unknown escape");
  });
});

describe("ruffSelects", () => {
  it("matches a code by prefix within its linter, as Ruff selectors do", () => {
    expect(ruffSelects("E7", "E711")).toBe(true);
    expect(ruffSelects("E711", "E711")).toBe(true);
    expect(ruffSelects("ALL", "B011")).toBe(true);
    expect(ruffSelects("PL", "PLR0913")).toBe(true);
    expect(ruffSelects("E", "ERA001")).toBe(false);
    expect(ruffSelects("F", "FURB101")).toBe(false);
    expect(ruffSelects("E71", "E721")).toBe(false);
  });
});

describe("readLintConfigFiles", () => {
  it("parses a flat config without running it, and reports the eslintrc ESLint then ignores as not read", async () => {
    const root = await repository({
      "eslint.config.ts": "import { writeFileSync } from \"node:fs\";\nwriteFileSync(new URL(\"./ran\", import.meta.url), \"\");\nexport default [{ rules: { \"no-console\": \"off\" } }];\n",
      ".eslintrc.json": "{ \"rules\": { \"no-debugger\": \"off\" } }",
    });
    const files = readLintConfigFiles(root);
    expect(files.eslint).toEqual({ file: "eslint.config.ts", rules: { "no-console": "off" }, scoped: [], unresolved: new Map(), shadowed: new Map() });
    expect(files.notes.map(note => note.text)).toEqual([".eslintrc.json is not read: ESLint uses eslint.config.ts instead."]);
    expect(existsSync(join(root, "ran"))).toBe(false);
  });

  it("reports a flat config it cannot parse or that exports nothing it can find as not read", async () => {
    const broken = await repository({ "eslint.config.js": "export default [\n" });
    expect(readLintConfigFiles(broken).notes.map(note => note.text)).toEqual([
      expect.stringMatching(/^eslint\.config\.js is not read: it does not parse: .+\. Rerail never runs your config\.$/u),
    ]);
    const thrown = await repository({ "eslint.config.mjs": "throw new Error(\"run\");\n", "eslint.config.ts": "export default [];\n" });
    expect(readLintConfigFiles(thrown).notes.map(note => note.text)).toEqual([
      "eslint.config.ts is not read: ESLint uses eslint.config.mjs instead.",
      "eslint.config.mjs is not read: it exports no config Rerail can find (export default or module.exports). Rerail never runs your config.",
    ]);
  });

  it("reads .eslintrc.json as JSONC, noting what besides its top-level rules it does not read", async () => {
    const root = await repository({ ".eslintrc.json": "{\n  // ours\n  \"extends\": [\"airbnb\"],\n  \"rules\": { \"no-console\": \"off\", },\n}\n" });
    const files = readLintConfigFiles(root);
    expect(files.eslint).toEqual({ file: ".eslintrc.json", rules: { "no-console": "off" }, scoped: [], unresolved: new Map(), shadowed: new Map() });
    expect(files.notes.map(note => note.text)).toEqual([".eslintrc.json: only its own rules are read, not extends."]);
  });

  it("reports an .eslintrc in YAML, and an .eslintrc.js before it, as not read", async () => {
    const yaml = await repository({ ".eslintrc": "rules:\n  no-console: off\n" });
    expect(readLintConfigFiles(yaml).eslint).toBeUndefined();
    expect(readLintConfigFiles(yaml).notes.map(note => note.text)).toEqual([".eslintrc is not read: it is not a JSON object (Rerail reads JSON lint configs only)."]);

    const code = await repository({ ".eslintrc.js": "module.exports = {};\n", ".eslintrc.json": "{}" });
    expect(readLintConfigFiles(code).notes.map(note => note.text)).toEqual([
      ".eslintrc.json is not read: ESLint uses .eslintrc.js instead.",
      ".eslintrc.js is not read: Rerail never runs your config.",
    ]);
  });

  it("reads package.json eslintConfig where there is no config file", async () => {
    const root = await repository({ "package.json": JSON.stringify({ name: "acme", eslintConfig: { rules: { "no-console": 0 } } }) });
    expect(readLintConfigFiles(root).eslint).toEqual({ file: "package.json eslintConfig", rules: { "no-console": 0 }, scoped: [], unresolved: new Map(), shadowed: new Map() });
  });

  it("does not follow a symlinked config", async () => {
    const root = await repository({ "elsewhere.json": "{ \"rules\": {} }" });
    symlinkSync(join(root, "elsewhere.json"), join(root, ".oxlintrc.json"));
    expect(readLintConfigFiles(root).oxlint).toBeUndefined();
    expect(readLintConfigFiles(root).notes.map(note => note.text)).toEqual([".oxlintrc.json is not read: it is not a regular file of at most 1 MiB in the repository."]);
  });

  it("reads Ruff's rule selection from the config file Ruff uses, the lint section over the top level, skipping a pyproject.toml without [tool.ruff]", async () => {
    const plain = await repository({ "pyproject.toml": "[project]\nname = \"acme\"\n" });
    expect(readLintConfigFiles(plain)).toMatchObject({ ruff: undefined, notes: [] });

    const both = await repository({
      "pyproject.toml": "[tool.ruff.lint]\nignore = [\"F\"]\n",
      "ruff.toml": [
        "extend = \"../base.toml\"",
        "ignore = [\"E7\", \"e7\"]",
        "select = [\"F\"]",
        "[lint]",
        "extend-ignore = [\"W\"]",
        "select = [\"E\"]",
        "[lint.per-file-ignores]",
        "\"tests/**\" = [\"E711\"]",
        "\"**/test_*.py\" = [\"E721\"]",
      ].join("\n"),
    });
    const files = readLintConfigFiles(both);
    expect(files.ruff).toEqual({
      layers: [{
        select: [{ file: "ruff.toml", key: "lint.select", selector: "E" }],
        extendSelect: [],
        ignore: [{ file: "ruff.toml", key: "ignore", selector: "E7" }, { file: "ruff.toml", key: "lint.extend-ignore", selector: "W" }],
      }],
      perFileIgnores: [
        { pattern: "tests/**", folder: "tests", ignore: { file: "ruff.toml", key: "lint.per-file-ignores.\"tests/**\"", selector: "E711" } },
        { pattern: "**/test_*.py", folder: undefined, ignore: { file: "ruff.toml", key: "lint.per-file-ignores.\"**/test_*.py\"", selector: "E721" } },
      ],
    });
    expect(files.notes.map(note => note.text)).toEqual([
      "ruff.toml: \"e7\" in ignore is not a Ruff rule code or prefix.",
      "ruff.toml: the config it extends, \"../base.toml\", is not read (it is outside the repository), so the rules only that config ignores stay on.",
    ]);

    const unreadable = await repository({ "pyproject.toml": "[tool.ruff.lint]\nignore = \"E7\"\n" });
    expect(readLintConfigFiles(unreadable).notes.map(note => note.text)).toEqual(["pyproject.toml is not read: line 2: tool.ruff.lint.ignore is not a list of strings."]);
  });
});

describe("proposeLintConfigSettings", () => {
  it("configures typescript-eslint's consistent-type-definitions from [\"error\", \"type\"]", async () => {
    const root = await repository({ ".eslintrc.json": "{ \"rules\": { \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"] } }" });
    expect(readRepositoryLintConfig(root, bundle, ["typescript"])).toEqual({
      settings: [{
        ruleId: TYPE_DEFINITIONS,
        folder: "",
        on: null,
        config: { option: "type" },
        because: ".eslintrc.json: \"@typescript-eslint/consistent-type-definitions\": [\"error\",\"type\"]",
      }],
      notes: [],
    });
  });

  it("turns off the javascript pack's no-console for \"no-console\": \"off\", and only for packs being set up", async () => {
    const root = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": \"off\" } }" });
    expect(readRepositoryLintConfig(root, bundle, ["javascript"]).settings).toEqual([
      { ruleId: "eslint-no-console", folder: "", on: false, config: null, because: ".eslintrc.json: \"no-console\": \"off\"" },
    ]);
    expect(readRepositoryLintConfig(root, bundle, ["typescript"]).settings).toEqual([]);
  });

  it("maps an option object onto the options the port declares, and skips one it cannot map with a note", async () => {
    const mapped = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"warn\", { \"allow\": [\"warn\", \"error\"] }] } }" });
    expect(readRepositoryLintConfig(mapped, bundle, ["javascript"]).settings).toMatchObject([{ ruleId: "eslint-no-console", on: null, config: { allow: ["warn", "error"] } }]);

    const wrongValue = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"error\", { \"allow\": \"warn\" }] } }" });
    expect(readRepositoryLintConfig(wrongValue, bundle, ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: option not mapped (config.allow: must be an array); it keeps its default options. From .eslintrc.json: \"no-console\": [\"error\",{\"allow\":\"warn\"}]."],
    });

    const unknownOption = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"error\", { \"bogus\": true }] } }" });
    expect(readRepositoryLintConfig(unknownOption, bundle, ["javascript"]).notes).toEqual([
      "eslint-no-console: option not mapped (bogus is not an option the port declares); it keeps its default options. From .eslintrc.json: \"no-console\": [\"error\",{\"bogus\":true}].",
    ]);
  });

  it("turns off a port oxlint implements for a plugin-prefixed .oxlintrc.json key, without mapping oxlint's options", async () => {
    const off = await repository({ ".oxlintrc.json": "{ \"rules\": { \"react-hooks/rules-of-hooks\": \"off\" } }" });
    expect(readRepositoryLintConfig(off, bundle, ["hooks"]).settings).toEqual([
      { ruleId: HOOKS_RULE, folder: "", on: false, config: null, because: ".oxlintrc.json: \"react-hooks/rules-of-hooks\": \"off\"" },
    ]);

    const options = await repository({ ".oxlintrc.json": "{ \"rules\": { \"rules-of-hooks\": [\"deny\", { \"x\": 1 }] } }" });
    expect(readRepositoryLintConfig(options, bundle, ["hooks"])).toEqual({
      settings: [],
      notes: [`${HOOKS_RULE}: option not mapped: Rerail maps ESLint options, not oxlint's (.oxlintrc.json: "rules-of-hooks": ["deny",{"x":1}]).`],
    });
  });

  /** The rules of `packs` an `.oxlintrc.json` with `rules` turns off. */
  async function oxlintOff(rules: Readonly<Record<string, unknown>>, packs: readonly string[]): Promise<string[]> {
    const root = await repository({ ".oxlintrc.json": JSON.stringify({ rules }) });
    return readRepositoryLintConfig(root, bundle, packs).settings.filter(setting => setting.on === false).map(setting => setting.ruleId);
  }

  it("resolves an .oxlintrc.json key to a built-in rule as oxlint does: another plugin's rule of the same name is not it", async () => {
    expect(await oxlintOff({ "custom/no-console": "off" }, ["javascript"])).toEqual([]);
    expect(await oxlintOff({ "custom/no-console": "off", "eslint/no-console": "error" }, ["javascript"])).toEqual([]);
    expect(await oxlintOff({ "react/no-console": "off" }, ["javascript"])).toEqual([]);
    expect(await oxlintOff({ "eslint/no-console": "off" }, ["javascript"])).toEqual(["eslint-no-console"]);
    expect(await oxlintOff({ "no-console": "off" }, ["javascript"])).toEqual(["eslint-no-console"]);
  });

  it("takes typescript-eslint's prefixes and its extension rules as oxlint does", async () => {
    for (const key of ["typescript/ban-tslint-comment", "@typescript-eslint/ban-tslint-comment", "ban-tslint-comment"]) {
      expect(await oxlintOff({ [key]: "off" }, ["typescript"])).toEqual(["typescript-eslint-ban-tslint-comment"]);
    }
    // oxlint runs its ESLint default-param-last for typescript-eslint's; every key names that one rule.
    for (const key of ["default-param-last", "typescript/default-param-last", "@typescript-eslint/default-param-last"]) {
      expect(await oxlintOff({ [key]: "off" }, ["typescript"])).toEqual(["typescript-eslint-default-param-last"]);
    }
    // A bare no-namespace is the import plugin's, the first of three plugins with a rule of that name.
    expect(await oxlintOff({ "no-namespace": "off" }, ["typescript"])).toEqual([]);
    expect(await oxlintOff({ "typescript/no-namespace": "off" }, ["typescript"])).toEqual(["typescript-eslint-no-namespace"]);
  });

  it("takes react-hooks as oxlint's react plugin, and an unknown plugin as no built-in one", async () => {
    for (const key of ["react-hooks/rules-of-hooks", "react/rules-of-hooks", "eslint-plugin-react-hooks/rules-of-hooks", "rules-of-hooks"]) {
      expect(await oxlintOff({ [key]: "off" }, ["hooks"])).toEqual([HOOKS_RULE]);
    }
    expect(await oxlintOff({ "hooks/rules-of-hooks": "off" }, ["hooks"])).toEqual([]);
  });

  it("applies a rule's last .oxlintrc.json key, as oxlint does", async () => {
    expect(await oxlintOff({ "no-console": "off", "eslint/no-console": "error" }, ["javascript"])).toEqual([]);
    expect(await oxlintOff({ "eslint/no-console": "error", "no-console": "off" }, ["javascript"])).toEqual(["eslint-no-console"]);
  });

  it("starts a rule off when one config turns it off and another keeps it on", async () => {
    const root = await repository({
      ".eslintrc.json": "{ \"rules\": { \"react-hooks/rules-of-hooks\": \"error\" } }",
      ".oxlintrc.json": "{ \"rules\": { \"rules-of-hooks\": \"allow\" } }",
    });
    expect(readRepositoryLintConfig(root, bundle, ["hooks"])).toEqual({
      settings: [{ ruleId: HOOKS_RULE, folder: "", on: false, config: null, because: ".oxlintrc.json: \"rules-of-hooks\": \"allow\"" }],
      notes: [`${HOOKS_RULE}: .eslintrc.json: "react-hooks/rules-of-hooks": "error" keeps it on, but .oxlintrc.json: "rules-of-hooks": "allow" turns it off; it starts off.`],
    });
  });

  it("turns off every shipped Ruff port [tool.ruff.lint] ignore = [\"E7\"] selects, and no candidate", async () => {
    const root = await repository({ "pyproject.toml": "[tool.ruff.lint]\nignore = [\"E7\"]\n", "eslint.config.ts": "export default makeConfig();\n" });
    expect(readRepositoryLintConfig(root, bundle, ["python"])).toEqual({
      // The python pack's rules/ ports of E7xx; its candidates (E712, E713, E714, E731, E741) do not ship.
      settings: ["ruff-e711-none-comparison", "ruff-e721-type-comparison", "ruff-e722-bare-except", "ruff-e742-ambiguous-class-name", "ruff-e743-ambiguous-function-name"]
        .map(ruleId => ruffOff(ruleId, "pyproject.toml: \"E7\" in tool.ruff.lint.ignore")),
      // No pack being set up ports an ESLint rule, so the flat config is not worth a note.
      notes: [],
    });
    const files = readLintConfigFiles(root);
    expect(proposeLintConfigSettings(files, bundle, ["python", "javascript"]).notes).toEqual([
      "eslint.config.ts: the rules of makeConfig() are not read: Rerail does not run code to find them.",
    ]);
  });

  const ruffOff = (ruleId: string, because: string, folder = "") => ({ ruleId, folder, on: false, config: null, because });
  /** The python pack's settings and notes for a pyproject.toml whose [tool.ruff.lint] section holds `lines`. */
  async function ruffLint(...lines: string[]) {
    const root = await repository({ "pyproject.toml": ["[tool.ruff.lint]", ...lines].join("\n") });
    return readRepositoryLintConfig(root, bundle, ["python"]);
  }

  it("lets a more specific select win over an ignore, as Ruff applies selectors from least to most specific", async () => {
    expect(await ruffLint("select = [\"E711\"]", "ignore = [\"E7\"]")).toEqual({
      settings: ["ruff-e721-type-comparison", "ruff-e722-bare-except", "ruff-e742-ambiguous-class-name", "ruff-e743-ambiguous-function-name"]
        .map(ruleId => ruffOff(ruleId, "pyproject.toml: \"E7\" in tool.ruff.lint.ignore")),
      notes: ["ruff-e711-none-comparison: pyproject.toml: \"E7\" in tool.ruff.lint.ignore is overridden by pyproject.toml: \"E711\" in tool.ruff.lint.select, which Ruff applies after it; it stays on."],
    });
    // extend-select re-enables the same way; ALL is the least specific selector of all.
    expect((await ruffLint("extend-select = [\"E71\"]", "ignore = [\"E7\"]")).settings.map(setting => setting.ruleId)).not.toContain("ruff-e711-none-comparison");
    expect((await ruffLint("ignore = [\"ALL\"]", "select = [\"E711\"]")).settings.map(setting => setting.ruleId)).not.toContain("ruff-e711-none-comparison");
  });

  it("lets an ignore win over a select of the same specificity, and over a less specific one", async () => {
    expect((await ruffLint("extend-select = [\"E7\"]", "ignore = [\"E7\"]")).settings.map(setting => setting.ruleId)).toContain("ruff-e711-none-comparison");
    expect((await ruffLint("select = [\"E\"]", "ignore = [\"E711\"]")).settings).toEqual([ruffOff("ruff-e711-none-comparison", "pyproject.toml: \"E711\" in tool.ruff.lint.ignore")]);
  });

  it("turns a rule off in a folder a per-file ignore covers, and leaves it on for a narrower pattern", async () => {
    const root = await repository({
      "pyproject.toml": "[tool.ruff.lint.per-file-ignores]\n\"tests/**\" = [\"E711\"]\n\"**/test_*.py\" = [\"E721\"]\n",
    });
    expect(readRepositoryLintConfig(root, bundle, ["python"])).toEqual({
      settings: [ruffOff("ruff-e711-none-comparison", "pyproject.toml: \"E711\" in tool.ruff.lint.per-file-ignores.\"tests/**\"", "tests")],
      notes: ["ruff-e721-type-comparison: pyproject.toml: \"E721\" in tool.ruff.lint.per-file-ignores.\"**/test_*.py\" ignores it only in the files \"**/test_*.py\" matches; it stays on."],
    });
  });

  // Each case checked against `ruff check` 0.15.0 on the same files: a runs E711 only; b runs E711, E721 and E722;
  // c runs E721 and E722.
  it("follows extend as Ruff layers configs: a later config re-enables, a select replaces, select = [] carries its ignores", async () => {
    const offs = async (pyproject: string, base: string): Promise<string[]> => {
      const root = await repository({ "pyproject.toml": `[tool.ruff]\nextend = "configs/base.toml"\n${pyproject}`, "configs/base.toml": base });
      return readRepositoryLintConfig(root, bundle, ["python"]).settings.map(setting => setting.ruleId).filter(id => /e71|e72/u.test(id));
    };
    expect(await offs("[tool.ruff.lint]\nextend-select = [\"E711\"]\n", "[lint]\nignore = [\"E7\"]\n")).toEqual(["ruff-e721-type-comparison", "ruff-e722-bare-except"]);
    expect(await offs("[tool.ruff.lint]\nselect = [\"E\"]\n", "[lint]\nignore = [\"E7\"]\n")).toEqual([]);
    expect(await offs("[tool.ruff.lint]\nselect = [\"E7\"]\n", "[lint]\nselect = []\nignore = [\"E711\"]\n")).toEqual(["ruff-e711-none-comparison"]);
  });

  // Self-review: an explicitly empty per-file-ignores keeps the extended config's out, as Ruff 0.15.0's combine
  // (`self.per_file_ignores.or(config.per_file_ignores)`) does; `ruff check` reports E711 in tests/ for both.
  it("lets an empty per-file-ignores, inline or as a table header, replace the extended config's", async () => {
    const base = "[lint.per-file-ignores]\n\"tests/**\" = [\"E711\"]\n";
    for (const own of ["[lint]\nper-file-ignores = {}\n", "[lint.per-file-ignores]\n"]) {
      const root = await repository({ "ruff.toml": `extend = "base.toml"\n${own}`, "base.toml": base });
      expect(readRepositoryLintConfig(root, bundle, ["python"]).settings).toEqual([]);
    }
    const inherited = await repository({ "ruff.toml": "extend = \"base.toml\"\n", "base.toml": base });
    expect(readRepositoryLintConfig(inherited, bundle, ["python"]).settings).toEqual([
      ruffOff("ruff-e711-none-comparison", "base.toml: \"E711\" in lint.per-file-ignores.\"tests/**\"", "tests"),
    ]);
  });
});

describe("proposeLintConfigSettings for .eslintrc overrides", () => {
  it("turns a rule off in a folder an override covers, and keeps one on that an override turns back on", async () => {
    const scripts = await repository({ ".eslintrc.json": JSON.stringify({ overrides: [{ files: "scripts/**", rules: { "no-console": "off" } }] }) });
    expect(readRepositoryLintConfig(scripts, bundle, ["javascript"]).settings).toEqual([
      { ruleId: "eslint-no-console", folder: "scripts", on: false, config: null, because: ".eslintrc.json: \"no-console\": \"off\" for \"scripts/**\"" },
    ]);
    const back = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "off" }, overrides: [{ files: ["src/**/*.ts"], rules: { "no-console": "warn" } }] }) });
    expect(readRepositoryLintConfig(back, bundle, ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: .eslintrc.json: \"no-console\": \"off\" turns it off, but .eslintrc.json: \"no-console\": \"warn\" turns it back on for [\"src/**/*.ts\"]; it stays on."],
    });
  });

  // Self-review: ESLint 9.39.1 --print-config scripts/a.js gives severity 2 when a later override turns it back on.
  it("keeps a rule on in a folder when a later override turns it back on there", async () => {
    const root = await repository({ ".eslintrc.json": JSON.stringify({ overrides: [
      { files: "scripts/**", rules: { "no-console": "off" } },
      { files: "scripts/**", rules: { "no-console": "error" } },
    ] }) });
    expect(readRepositoryLintConfig(root, bundle, ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: .eslintrc.json: \"no-console\": \"off\" for \"scripts/**\" turns it off, but .eslintrc.json: \"no-console\": \"error\" turns it back on for \"scripts/**\"; it stays on."],
    });
  });
});

describe("proposeLintConfigSettings for ESLint's flat config", () => {
  /** The settings and notes for packs `packs` from a repository whose only lint config is `name` holding `text`. */
  async function flat(text: string, packs: readonly string[] = ["typescript"], name = "eslint.config.ts") {
    const root = await repository({ [name]: text });
    return readRepositoryLintConfig(root, bundle, packs);
  }

  // NER-1789's repro, dolanmiu/docx at PR 3716: the option is set in a config object for **/src/**/*.ts, after shared
  // configs, in a typed const the file exports.
  it("configures consistent-type-definitions from a docx-shaped eslint.config.ts, naming the shared configs it does not read", async () => {
    const text = [
      "import eslint from \"@eslint/js\";",
      "import type { Linter } from \"eslint\";",
      "import tsEslint from \"typescript-eslint\";",
      "const config: Linter.Config<Linter.RulesRecord>[] = [",
      "  { ignores: [\"**/dist/**\"] },",
      "  eslint.configs.recommended,",
      "  ...tsEslint.configs.stylistic,",
      "  {",
      "    files: [\"**/src/**/*.ts\"],",
      "    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },",
      "    rules: {",
      "      \"no-undef\": \"off\",",
      "      \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"],",
      "    },",
      "  },",
      "];",
      "export default config;",
    ].join("\n");
    expect(await flat(text)).toEqual({
      settings: [{
        ruleId: TYPE_DEFINITIONS,
        folder: "",
        on: null,
        config: { option: "type" },
        because: "eslint.config.ts: \"@typescript-eslint/consistent-type-definitions\": [\"error\",\"type\"] for [\"**/src/**/*.ts\"]",
      }],
      notes: ["eslint.config.ts: the rules of eslint.configs.recommended, tsEslint.configs.stylistic are not read: Rerail does not run code to find them."],
    });
  });

  it("reads defineConfig, typescript-eslint's config(), extends, module.exports and const bindings", async () => {
    const off = (ruleId: string, because: string) => ({ ruleId, folder: "", on: false, config: null, because });
    expect((await flat([
      "import { defineConfig } from \"eslint/config\";",
      "const level = \"off\";",
      "const base = { rules: { \"no-console\": level } };",
      "export default defineConfig([{ extends: [base] }]);",
    ].join("\n"), ["javascript"], "eslint.config.mjs")).settings).toEqual([off("eslint-no-console", "eslint.config.mjs: \"no-console\": \"off\"")]);
    expect((await flat("module.exports = [{ rules: { \"no-console\": 0 } }];\n", ["javascript"], "eslint.config.cjs")).settings)
      .toEqual([off("eslint-no-console", "eslint.config.cjs: \"no-console\": 0")]);
    expect((await flat([
      "import tseslint from \"typescript-eslint\";",
      "export default tseslint.config({ rules: { \"@typescript-eslint/ban-tslint-comment\": \"off\" } });",
    ].join("\n"))).settings).toEqual([off(TSLINT_COMMENT, "eslint.config.ts: \"@typescript-eslint/ban-tslint-comment\": \"off\"")]);
  });

  it("applies config objects in order: a later one replaces a rule, keeping its options when it sets only a severity", async () => {
    expect((await flat("export default [{ rules: { \"no-console\": \"off\" } }, { rules: { \"no-console\": \"error\" } }];", ["javascript"])).settings).toEqual([]);
    expect((await flat("export default [{ rules: { \"no-console\": \"error\" } }, { rules: { \"no-console\": \"off\" } }];", ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", on: false }]);
    expect((await flat("export default [{ rules: { \"no-console\": [\"error\", { allow: [\"warn\"] }] } }, { rules: { \"no-console\": \"warn\" } }];", ["javascript"])).settings)
      .toEqual([{ ruleId: "eslint-no-console", folder: "", on: null, config: { allow: ["warn"] }, because: "eslint.config.ts: \"no-console\": [\"warn\",{\"allow\":[\"warn\"]}]" }]);
  });

  it("keeps a rule on that a later object turns back on for some files, and turns it off in a folder a files glob covers", async () => {
    expect(await flat("export default [{ rules: { \"no-console\": \"off\" } }, { files: [\"src/**/*.ts\"], rules: { \"no-console\": \"error\" } }];", ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: eslint.config.ts: \"no-console\": \"off\" turns it off, but eslint.config.ts: \"no-console\": \"error\" turns it back on for [\"src/**/*.ts\"]; it stays on."],
    });
    expect(await flat("export default [{ files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { files: [\"**/*.test.ts\"], rules: { \"no-console\": 0 } }];", ["javascript"])).toEqual({
      settings: [{ ruleId: "eslint-no-console", folder: "scripts", on: false, config: null, because: "eslint.config.ts: \"no-console\": \"off\" for [\"scripts/**\"]" }],
      notes: ["eslint-no-console: eslint.config.ts: \"no-console\": 0 for [\"**/*.test.ts\"] turns it off only for some files; it stays on."],
    });
  });

  it("turns a port off where its entry's value cannot be read without running the config, unless a later entry replaces it", async () => {
    const imported = "import shared from \"./shared.js\";\n";
    expect(await flat(`${imported}export default [{ rules: { "no-console": shared.level } }];`, ["javascript"])).toEqual({
      settings: [{ ruleId: "eslint-no-console", folder: "", on: false, config: null, because: "eslint.config.ts: \"no-console\" is not read (it is shared.level, which Rerail does not run)" }],
      notes: [],
    });
    expect((await flat(`${imported}export default [{ files: ["**/*.ts"], rules: { "no-console": ["error", shared.options] } }];`, ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", folder: "", on: false }]);
    // `["error"]` is a severity alone: ESLint keeps the earlier options, which Rerail cannot read.
    expect((await flat(`${imported}export default [{ rules: { "no-console": shared.level } }, { rules: { "no-console": ["error"] } }];`, ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", on: false }]);
    expect((await flat(`${imported}export default [{ rules: { "no-console": shared.level } }, { rules: { "no-console": ["error", { allow: ["warn"] }] } }];`, ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", on: null, config: { allow: ["warn"] } }]);
  });

  // Self-review findings 4, 6, 7 and 8, each checked against ESLint 9.39.1 on the same config.
  it("keeps a rule on in a folder when a later object turns it back on there", async () => {
    expect(await flat("export default [{ files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { files: [\"scripts/**\"], rules: { \"no-console\": \"error\" } }];", ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: eslint.config.ts: \"no-console\": \"off\" for [\"scripts/**\"] turns it off, but eslint.config.ts: \"no-console\": \"error\" turns it back on for [\"scripts/**\"]; it stays on."],
    });
  });

  it("does not widen options set for some files to the whole repository when the rule is set for every file", async () => {
    expect(await flat("export default [{ rules: { \"no-console\": \"error\" } }, { files: [\"tests/**\"], rules: { \"no-console\": [\"error\", { allow: [\"log\"] }] } }];", ["javascript"])).toEqual({
      settings: [],
      notes: ["eslint-no-console: the options of eslint.config.ts: \"no-console\": [\"error\",{\"allow\":[\"log\"]}] for [\"tests/**\"] are not used: eslint.config.ts: \"no-console\": \"error\" sets the rule for every file, and Rerail sets options for the whole repository only."],
    });
  });

  it("reads one rules object as JavaScript builds it: a later key replaces a spread or an earlier key whole", async () => {
    const common = "const common = { \"no-console\": [\"error\", { allow: [\"log\"] }] };\n";
    expect((await flat(`${common}export default [{ rules: { ...common, "no-console": "error" } }];`, ["javascript"])).settings).toEqual([]);
    expect((await flat("export default [{ rules: { \"no-console\": [\"error\", { allow: [\"log\"] }], \"no-console\": \"error\" } }];", ["javascript"], "eslint.config.js")).settings).toEqual([]);
    expect((await flat(`${common}export default [{ rules: { "no-console": "error", ...common } }];`, ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", config: { allow: ["log"] } }]);
  });

  it("merges a one-element severity list as a severity alone, keeping the earlier options", async () => {
    expect((await flat("export default [{ rules: { \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"] } }, { rules: { \"@typescript-eslint/consistent-type-definitions\": [\"warn\"] } }];")).settings)
      .toEqual([{ ruleId: TYPE_DEFINITIONS, folder: "", on: null, config: { option: "type" }, because: "eslint.config.ts: \"@typescript-eslint/consistent-type-definitions\": [\"warn\",\"type\"]" }]);
  });

  it("uses no entry that a later config element it cannot read may replace", async () => {
    expect(await flat("import prettier from \"eslint-config-prettier\";\nexport default [{ rules: { \"no-console\": \"off\" } }, prettier];", ["javascript"])).toEqual({
      settings: [],
      notes: [
        "eslint.config.ts: the rules of prettier are not read: Rerail does not run code to find them.",
        "eslint-no-console: eslint.config.ts: \"no-console\": \"off\" is not used: prettier, after it, is not read and may replace it.",
      ],
    });
    const shared = "import shared from \"./shared.js\";\n";
    expect((await flat(`${shared}export default [{ rules: { "no-console": "off", ...shared.rules } }];`, ["javascript"])).settings).toEqual([]);
    expect((await flat(`${shared}export default [{ rules: { ...shared.rules, "no-console": "off" } }];`, ["javascript"])).settings)
      .toMatchObject([{ ruleId: "eslint-no-console", on: false }]);
  });
});
