import { existsSync, mkdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function repository(files: Readonly<Record<string, string>>): Promise<string> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-lint-config-")));
  temporaryDirectories.push(root);
  for (const [name, text] of Object.entries(files)) writeFileSync(join(root, name), text);
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
  it("reports flat config and the eslintrc ESLint then ignores as not read, and never runs the config", async () => {
    const root = await repository({
      "eslint.config.ts": "import { writeFileSync } from \"node:fs\";\nwriteFileSync(new URL(\"./ran\", import.meta.url), \"\");\nexport default [];\n",
      ".eslintrc.json": "{ \"rules\": { \"no-console\": \"off\" } }",
    });
    const files = readLintConfigFiles(root);
    expect(files.eslint).toBeUndefined();
    expect(files.notes.map(note => note.text)).toEqual([
      "eslint.config.ts is not read: Rerail never runs your config.",
      ".eslintrc.json is not read: ESLint uses eslint.config.ts instead.",
    ]);
    expect(existsSync(join(root, "ran"))).toBe(false);
  });

  it("reads .eslintrc.json as JSONC, noting what besides its top-level rules it does not read", async () => {
    const root = await repository({ ".eslintrc.json": "{\n  // ours\n  \"extends\": [\"airbnb\"],\n  \"rules\": { \"no-console\": \"off\", },\n}\n" });
    const files = readLintConfigFiles(root);
    expect(files.eslint).toEqual({ file: ".eslintrc.json", rules: { "no-console": "off" } });
    expect(files.notes.map(note => note.text)).toEqual([".eslintrc.json: only its own top-level rules are read, not extends."]);
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
    expect(readLintConfigFiles(root).eslint).toEqual({ file: "package.json eslintConfig", rules: { "no-console": 0 } });
  });

  it("does not follow a symlinked config", async () => {
    const root = await repository({ "elsewhere.json": "{ \"rules\": {} }" });
    symlinkSync(join(root, "elsewhere.json"), join(root, ".oxlintrc.json"));
    expect(readLintConfigFiles(root).oxlint).toBeUndefined();
    expect(readLintConfigFiles(root).notes.map(note => note.text)).toEqual([".oxlintrc.json is not read: it is not a regular file of at most 1 MiB in the repository."]);
  });

  it("reads Ruff's ignore lists from the config file Ruff uses, skipping a pyproject.toml without [tool.ruff]", async () => {
    const plain = await repository({ "pyproject.toml": "[project]\nname = \"acme\"\n" });
    expect(readLintConfigFiles(plain)).toMatchObject({ ruff: [], notes: [] });

    const both = await repository({
      "pyproject.toml": "[tool.ruff.lint]\nignore = [\"F\"]\n",
      "ruff.toml": "extend = \"../base.toml\"\nignore = [\"E7\", \"e7\"]\n[lint]\nextend-ignore = [\"W\"]\n",
    });
    const files = readLintConfigFiles(both);
    expect(files.ruff).toEqual([
      { file: "ruff.toml", key: "ignore", selector: "E7" },
      { file: "ruff.toml", key: "lint.extend-ignore", selector: "W" },
    ]);
    expect(files.notes.map(note => note.text)).toEqual([
      "ruff.toml: only ignore and extend-ignore are read, not extend.",
      "ruff.toml: \"e7\" in ignore is not a Ruff rule code or prefix.",
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
        ruleId: "typescript-eslint-consistent-type-definitions",
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
      { ruleId: "eslint-no-console", on: false, config: null, because: ".eslintrc.json: \"no-console\": \"off\"" },
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
      { ruleId: HOOKS_RULE, on: false, config: null, because: ".oxlintrc.json: \"react-hooks/rules-of-hooks\": \"off\"" },
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
      settings: [{ ruleId: HOOKS_RULE, on: false, config: null, because: ".oxlintrc.json: \"rules-of-hooks\": \"allow\"" }],
      notes: [`${HOOKS_RULE}: .eslintrc.json: "react-hooks/rules-of-hooks": "error" keeps it on, but .oxlintrc.json: "rules-of-hooks": "allow" turns it off; it starts off.`],
    });
  });

  it("turns off the python pack's Ruff ports [tool.ruff.lint] ignore = [\"E7\"] selects, E711 among them", async () => {
    const root = await repository({ "pyproject.toml": "[tool.ruff.lint]\nignore = [\"E7\"]\n", "eslint.config.ts": "export default [];\n" });
    expect(readRepositoryLintConfig(root, bundle, ["python"])).toEqual({
      settings: [
        { ruleId: "ruff-e711-none-comparison", on: false, config: null, because: "pyproject.toml: \"E7\" in tool.ruff.lint.ignore" },
        { ruleId: "ruff-e722-bare-except", on: false, config: null, because: "pyproject.toml: \"E7\" in tool.ruff.lint.ignore" },
      ],
      // No pack being set up ports an ESLint rule, so the flat config is not worth a note.
      notes: [],
    });
    const files = readLintConfigFiles(root);
    expect(proposeLintConfigSettings(files, bundle, ["python", "javascript"]).notes).toEqual(["eslint.config.ts is not read: Rerail never runs your config."]);
  });
});
