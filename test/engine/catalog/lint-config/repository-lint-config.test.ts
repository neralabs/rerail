import { existsSync, mkdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { readRuleCatalog } from "../../../../src/engine/catalog/rule-catalog.js";
import { readEslintRules } from "../../../../src/engine/catalog/lint-config/eslint-rules.js";
import { portsOf, type LinterNote, type RepositoryLintConfig } from "../../../../src/engine/catalog/lint-config/linter-config.js";
import type { RuleCatalog } from "../../../../src/engine/catalog/repository-model.js";
import { readRepositoryLintConfig } from "../../../../src/engine/catalog/lint-config/repository-lint-config.js";
import { readRuffSelection } from "../../../../src/engine/catalog/lint-config/ruff-config.js";

/** The fixture packs' ports start on, so that a config turning one off shows as a setting. */
const on = (ruleId: string): string => `on-${ruleId}`;
const CONSOLE = on("eslint-no-console");
const TYPE_DEFINITIONS = on("typescript-eslint-consistent-type-definitions");
const TSLINT_COMMENT = on("typescript-eslint-ban-tslint-comment");
/** Packs of ports that start on: the javascript pack's no-console; some of the typescript and python packs' ports. */
const JS = ["js-on"];
const TS = ["ts-on"];
const PY = ["py-on"];
/** The py-on pack's ports of E711, E721, E722, E742 and E743, in pack order. */
const E7 = ["ruff-e711-none-comparison", "ruff-e721-type-comparison", "ruff-e722-bare-except", "ruff-e742-ambiguous-class-name", "ruff-e743-ambiguous-function-name"].map(on);

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

const packDirectory = (id: string): string => decodeURIComponent(new URL(`../../../../packs/${id}`, import.meta.url).pathname);

const HOOKS_RULE = "react-hooks-rules-of-hooks";

interface FixtureRule {
  readonly id: string;
  /** The pack and rule whose document it copies. */
  readonly from: readonly [string, string];
  /** The document's `suppress` in place of its own, where it has one. */
  readonly suppress?: unknown;
  readonly on: boolean;
}

/** A pack `id` of copies of shipped rule documents under other ids, with their own start and, where given, `suppress`. */
async function fixturePack(id: string, rules: readonly FixtureRule[]): Promise<string> {
  const directory = join(realpathSync(await mkdtemp(join(tmpdir(), "rerail-fixture-pack-"))), id);
  mkdirSync(join(directory, "rules"), { recursive: true });
  for (const rule of rules) {
    const document = JSON.parse(readFileSync(join(packDirectory(rule.from[0]), "rules", `${rule.from[1]}.json`), "utf8")) as {
      id: string;
      clauses: { violation?: { suppress?: unknown } }[];
    };
    document.id = rule.id;
    for (const clause of document.clauses) {
      if (clause.violation?.suppress !== undefined && rule.suppress !== undefined) clause.violation.suppress = rule.suppress;
    }
    writeFileSync(join(directory, "rules", `${rule.id}.json`), JSON.stringify(document));
  }
  writeFileSync(join(directory, "pack.json"), JSON.stringify({
    format: "rerail.rule-pack/v1",
    id,
    name: id,
    category: "framework",
    covers: "Fixture rules.",
    reason: "Tests.",
    match: { any: [{ dependencies: { ecosystem: "npm", names: ["react"] } }] },
    rules: rules.map(rule => ({ id: rule.id, document: `rules/${rule.id}.json`, title: rule.id, group: "Fixture", on: rule.on })),
  }));
  return directory;
}

/** Copies of `ruleIds` of shipped pack `pack`, each starting on. */
const startingOn = (pack: string, ruleIds: readonly string[]): FixtureRule[] => ruleIds.map(ruleId => ({ id: on(ruleId), from: [pack, ruleId], on: true }));

/** A port of Ruff code `code`, starting off: the python pack's E711 document with its suppress replaced. */
const ruffPort = (code: string): FixtureRule => ({ id: `ruff-${code.toLowerCase()}`, from: ["python", "ruff-e711-none-comparison"], suppress: { noqa: { codes: [code] } }, on: false });

let ruleCatalog: RuleCatalog;
const fixtureDirectories: string[] = [];
beforeAll(async () => {
  fixtureDirectories.push(
    // One port of eslint-plugin-react-hooks' rules-of-hooks, which oxlint also implements: only its upstream names matter.
    await fixturePack("hooks", [{
      id: HOOKS_RULE,
      from: ["javascript", "eslint-no-console"],
      suppress: { eslint: { rules: ["react-hooks/rules-of-hooks"] }, oxlint: { rules: ["rules-of-hooks"] } },
      on: true,
    }]),
    await fixturePack("js-on", startingOn("javascript", ["eslint-no-console"])),
    await fixturePack("js-more", startingOn("javascript", ["eslint-no-debugger", "eslint-eqeqeq"])),
    // A port of a rule typescript-eslint's eslint-recommended config turns off for TypeScript files.
    await fixturePack("js-dupe", startingOn("javascript", ["eslint-no-dupe-class-members"])),
    await fixturePack("ts-on", startingOn("typescript", [
      "typescript-eslint-consistent-type-definitions", "typescript-eslint-ban-tslint-comment", "typescript-eslint-default-param-last",
      "typescript-eslint-no-namespace", "typescript-eslint-no-unused-expressions",
    ])),
    await fixturePack("py-on", startingOn("python", [
      "ruff-e711-none-comparison", "ruff-e721-type-comparison", "ruff-e722-bare-except", "ruff-e742-ambiguous-class-name", "ruff-e743-ambiguous-function-name",
    ])),
    // AIR003 is a preview rule in preview's default selection, AIR321 a preview rule outside it, AIR001 a stable rule
    // only preview's default selection enables (Ruff 0.16.9).
    await fixturePack("ruff-preview", ["AIR003", "AIR321", "AIR001"].map(ruffPort)),
    // Ports that start off of rules @eslint/js's and typescript-eslint's recommended configs turn on.
    await fixturePack("js-off", [{ id: "off-eslint-no-debugger", from: ["javascript", "eslint-no-debugger"], on: false }]),
    await fixturePack("ts-off", [{ id: "off-typescript-eslint-no-namespace", from: ["typescript", "typescript-eslint-no-namespace"], on: false }]),
  );
  ruleCatalog = readRuleCatalog([...["typescript", "javascript", "python"].map(packDirectory), ...fixtureDirectories]);
  return async () => {
    await Promise.all(fixtureDirectories.map(directory => rm(join(directory, ".."), { recursive: true, force: true })));
  };
});

/** The settings of `ruleId` in `proposed`. */
const settingsOf = (proposed: RepositoryLintConfig, ruleId: string) => proposed.settings.filter(setting => setting.ruleId === ruleId);

/** What `readEslintRules` reads at `root`, with its notes. */
function eslintAt(root: string) {
  const notes: LinterNote[] = [];
  const eslint = readEslintRules(root, notes);
  return { eslint, notes: notes.map(note => note.text) };
}

/** What `readRuffSelection` reads at `root`, with its notes. */
function ruffAt(root: string) {
  const notes: LinterNote[] = [];
  const ruff = readRuffSelection(root, notes);
  return { ruff, notes: notes.map(note => note.text) };
}

/** The ESLint rules read at `root`, every field but those a test names at their empty values. */
const eslintRules = (fields: Readonly<Record<string, unknown>>) => ({
  rules: {}, presets: new Map(), scoped: [], unresolved: new Map(), shadowed: new Map(), optionsUnknown: new Map(), ...fields,
});

describe("reading the config files", () => {
  it("never runs a flat config, reads it only when it is data, and reports the eslintrc ESLint then ignores as not read", async () => {
    const root = await repository({
      "eslint.config.ts": "import { writeFileSync } from \"node:fs\";\nwriteFileSync(new URL(\"./ran\", import.meta.url), \"\");\nexport default [{ rules: { \"no-console\": \"off\" } }];\n",
      ".eslintrc.json": "{ \"rules\": { \"no-debugger\": \"off\" } }",
    });
    const read = eslintAt(root);
    expect(read.eslint).toMatchObject({ file: "eslint.config.ts", rules: {}, unreadable: expect.stringMatching(/^it runs code/u) });
    expect(read.notes).toEqual([
      expect.stringMatching(/^eslint\.config\.ts is not statically readable: it runs code Rerail does not read as data \(line 2: writeFileSync\(.+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u),
      ".eslintrc.json is not read: ESLint uses eslint.config.ts instead.",
    ]);
    expect(existsSync(join(root, "ran"))).toBe(false);

    const data = await repository({ "eslint.config.ts": "import type { Linter } from \"eslint\";\ntype Rules = Linter.RulesRecord;\nconst rules: Rules = { \"no-console\": \"off\" };\nexport default [{ rules }];\n" });
    expect(eslintAt(data).eslint).toEqual(eslintRules({ file: "eslint.config.ts", rules: { "no-console": "off" }, severities: "eslint" }));
  });

  it("reports a flat config it cannot parse or that exports nothing it can find as not read", async () => {
    const broken = await repository({ "eslint.config.js": "export default [\n" });
    expect(eslintAt(broken).notes).toEqual([
      expect.stringMatching(/^eslint\.config\.js is not statically readable: it does not parse: .+\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u),
    ]);
    const thrown = await repository({ "eslint.config.mjs": "throw new Error(\"run\");\n", "eslint.config.ts": "export default [];\n" });
    expect(eslintAt(thrown).notes).toEqual([
      "eslint.config.mjs is not statically readable: it exports no config Rerail can find (export default or module.exports). Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.",
      "eslint.config.ts is not read: ESLint uses eslint.config.mjs instead.",
    ]);
    // A flat config Rerail cannot load (a link, or one over 1 MiB) is not statically readable either.
    const linked = await repository({ "real.mjs": "export default [];\n" });
    symlinkSync(join(linked, "real.mjs"), join(linked, "eslint.config.mjs"));
    expect(eslintAt(linked)).toMatchObject({
      eslint: { file: "eslint.config.mjs", unreadable: "it is not a regular file of at most 1 MiB in the repository" },
      notes: ["eslint.config.mjs is not statically readable: it is not a regular file of at most 1 MiB in the repository. Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off."],
    });
  });

  it("reads .eslintrc.json as JSONC, noting an extends it does not recognise, which may set any rule", async () => {
    const root = await repository({ ".eslintrc.json": "{\n  // ours\n  \"extends\": [\"airbnb\"],\n  \"rules\": { \"no-console\": \"off\", },\n}\n" });
    const read = eslintAt(root);
    expect(read.eslint).toEqual(eslintRules({ file: ".eslintrc.json", rules: { "no-console": "off" }, severities: "eslintrc", unread: ".eslintrc.json extends \"airbnb\"" }));
    expect(read.notes).toEqual([".eslintrc.json: only its own rules are read, not extends."]);
  });

  it("reads an eslintrc extends it recognises as the preset's entries, before its own rules", async () => {
    const root = await repository({ ".eslintrc.json": JSON.stringify({ extends: ["eslint:recommended"], rules: { "no-debugger": "off" } }) });
    const read = eslintAt(root);
    expect(read.notes).toEqual([]);
    expect(read.eslint).toMatchObject({ file: ".eslintrc.json", rules: { "no-debugger": "off", "no-dupe-keys": "error" } });
    expect(read.eslint?.unread).toBeUndefined();
    // The file's own entry replaces the preset's; the preset's own stand as the preset's.
    expect(read.eslint?.presets.get("no-debugger")).toBeUndefined();
    expect(read.eslint?.presets.get("no-dupe-keys")).toBe("eslint:recommended");
  });

  it("reports an .eslintrc in YAML, and an .eslintrc.js before it, as not read", async () => {
    const yaml = await repository({ ".eslintrc": "rules:\n  no-console: off\n" });
    expect(eslintAt(yaml)).toEqual({ eslint: undefined, notes: [".eslintrc is not read: it is not a JSON object (Rerail reads JSON lint configs only)."] });

    const code = await repository({ ".eslintrc.js": "module.exports = {};\n", ".eslintrc.json": "{}" });
    expect(eslintAt(code).notes).toEqual([
      ".eslintrc.json is not read: ESLint uses .eslintrc.js instead.",
      ".eslintrc.js is not read: Rerail never runs your config.",
    ]);
  });

  it("reads package.json eslintConfig where there is no config file", async () => {
    const root = await repository({ "package.json": JSON.stringify({ name: "acme", eslintConfig: { rules: { "no-console": 0 } } }) });
    expect(eslintAt(root).eslint).toEqual(eslintRules({ file: "package.json eslintConfig", rules: { "no-console": 0 }, severities: "eslintrc" }));
  });

  it.each([
    [".eslintrc.yaml", "rules:\n  no-console: off\n", ".eslintrc.yaml is not read: Rerail reads JSON lint configs only."],
    [".eslintrc.yml", "rules:\n  no-console: off\n", ".eslintrc.yml is not read: Rerail reads JSON lint configs only."],
    [".eslintrc.json", "[{ \"rules\": {} }]", ".eslintrc.json is not read: it is not a JSON object (Rerail reads JSON lint configs only)."],
    [".eslintrc.json", "{ \"rules\": [] }", ".eslintrc.json is not read: its rules are not an object."],
  ])("reports %s holding %j as not read", async (name, text, note) => {
    expect(eslintAt(await repository({ [name]: text }))).toEqual({ eslint: undefined, notes: [note] });
  });

  it.each([
    ["an extends that is not a name", { extends: 5 }, ".eslintrc.json extends", [".eslintrc.json: only its own rules are read, not extends."]],
    ["an extends list holding a non-name", { extends: ["eslint:recommended", 1] }, ".eslintrc.json extends", [".eslintrc.json: only its own rules are read, not extends."]],
    ["plugins next to an unrecognised extends", { extends: "airbnb", plugins: ["import"] }, ".eslintrc.json extends \"airbnb\"", [".eslintrc.json: only its own rules are read, not extends or plugins."]],
    ["overrides that are not a list", { overrides: {} }, ".eslintrc.json overrides", [".eslintrc.json: its overrides are not read: they are not a list."]],
    ["an override without files", { overrides: [{ rules: { eqeqeq: "off" } }] }, ".eslintrc.json overrides[0]", [".eslintrc.json: overrides[0] is not read: it has no files or its rules are not an object."]],
  ])("names %s in an eslintrc as not read, after the rules it may replace", async (_, fields, unread, notes) => {
    const root = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "off" }, ...fields }) });
    const read = eslintAt(root);
    expect(read.notes).toEqual(notes);
    expect(read.eslint).toMatchObject({ unread, rules: { "no-console": "off" } });
    // An extends applies before the eslintrc's own rules, its overrides after them.
    expect(read.eslint?.shadowed.get("no-console")).toBe("extends" in fields ? undefined : unread);
  });

  it("reads an override's files as a folder only when it excludes none", async () => {
    const overrides = [
      { files: "scripts/**", rules: { "no-console": "off" } },
      { files: ["src/**"], excludedFiles: ["src/generated/**"], rules: { "no-console": "off" } },
      { files: [], rules: { eqeqeq: "off" } },
    ];
    const root = await repository({ ".eslintrc.json": JSON.stringify({ overrides }) });
    expect(eslintAt(root).eslint?.scoped).toEqual([
      { key: "no-console", value: "off", folders: ["scripts"], files: "\"scripts/**\"" },
      { key: "no-console", value: "off", folders: undefined, files: "[\"src/**\"]" },
      { key: "eqeqeq", value: "off", folders: undefined, files: "[]" },
    ]);
  });

  it.each([
    ["a folder", undefined, []],
    ["not JSON", "{ name: acme }", []],
    ["JSON without an eslintConfig", JSON.stringify({ name: "acme" }), []],
    ["JSON with an eslintConfig that is not an object", JSON.stringify({ name: "acme", eslintConfig: "airbnb" }), ["package.json eslintConfig is not read: it is not an object."]],
  ])("reads no ESLint config from a package.json that is %s", async (_, manifest, notes) => {
    const root = await repository(manifest === undefined ? { "package.json/index.json": "{}" } : { "package.json": manifest });
    expect(eslintAt(root)).toEqual({ eslint: undefined, notes });
  });

  it("does not follow a symlinked config", async () => {
    const root = await repository({ "elsewhere.json": "{ \"rules\": {} }" });
    symlinkSync(join(root, "elsewhere.json"), join(root, ".oxlintrc.json"));
    // No oxlint config is read, so the port keeps its pack's value.
    expect(readRepositoryLintConfig(root, ruleCatalog, JS)).toEqual({
      settings: [],
      notes: [".oxlintrc.json is not read: it is not a regular file of at most 1 MiB in the repository."],
    });
  });

  it("reads Ruff's rule selection from the config file Ruff uses, the lint section over the top level, skipping a pyproject.toml without [tool.ruff]", async () => {
    const plain = await repository({ "pyproject.toml": "[project]\nname = \"acme\"\n" });
    expect(ruffAt(plain)).toEqual({ ruff: undefined, notes: [] });

    const both = await repository({
      "pyproject.toml": "[tool.ruff.lint]\nignore = [\"F\"]\n",
      "ruff.toml": [
        "extend = \"../base.toml\"",
        "ignore = [\"E7\", \"e7\"]",
        "select = [\"F\"]",
        "preview = true",
        "[lint]",
        "extend-ignore = [\"W\"]",
        "select = [\"E\"]",
        "[lint.per-file-ignores]",
        "\"tests/**\" = [\"E711\"]",
        "\"**/test_*.py\" = [\"E721\"]",
      ].join("\n"),
    });
    const read = ruffAt(both);
    expect(read.ruff).toEqual({
      file: "ruff.toml",
      layers: [{
        file: "ruff.toml",
        select: [{ file: "ruff.toml", key: "lint.select", selector: "E" }],
        selectKey: "lint.select",
        extendSelect: [],
        ignore: [{ file: "ruff.toml", key: "ignore", selector: "E7" }, { file: "ruff.toml", key: "lint.extend-ignore", selector: "W" }],
      }],
      perFileIgnores: [
        { pattern: "tests/**", folder: "tests", ignore: { file: "ruff.toml", key: "lint.per-file-ignores.\"tests/**\"", selector: "E711" } },
        { pattern: "**/test_*.py", folder: undefined, ignore: { file: "ruff.toml", key: "lint.per-file-ignores.\"**/test_*.py\"", selector: "E721" } },
      ],
      preview: true,
      // The extended config, not read, may set it.
      explicitPreviewRules: undefined,
      unread: "ruff.toml: the config it extends, \"../base.toml\",",
    });
    expect(read.notes).toEqual([
      "ruff.toml: \"e7\" in ignore is not a Ruff rule code or prefix.",
      "ruff.toml: the config it extends, \"../base.toml\", is not read (it is outside the repository), so the rules only that config ignores stay on.",
    ]);

    const unreadable = await repository({ "pyproject.toml": "[tool.ruff.lint]\nignore = \"E7\"\n" });
    expect(ruffAt(unreadable).notes).toEqual(["pyproject.toml is not read: line 2: tool.ruff.lint.ignore is not a list of strings."]);
  });

  it("reports a Ruff config nested deeper than it reads as not read", async () => {
    const root = await repository({ "ruff.toml": `[lint]\nignore = ["E7"]\nx = ${"[".repeat(100_000)}\n` });
    expect(ruffAt(root)).toEqual({ ruff: undefined, notes: ["ruff.toml is not read: line 3: values nest more than 64 deep."] });
    expect(readRepositoryLintConfig(root, ruleCatalog, ["python"])).toEqual({ settings: [], notes: ["ruff.toml is not read: line 3: values nest more than 64 deep."] });
  });
});

describe("readRepositoryLintConfig", () => {
  it("configures typescript-eslint's consistent-type-definitions from [\"error\", \"type\"]", async () => {
    const root = await repository({ ".eslintrc.json": "{ \"rules\": { \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"] } }" });
    const proposed = readRepositoryLintConfig(root, ruleCatalog, TS);
    expect(settingsOf(proposed, TYPE_DEFINITIONS)).toEqual([{
      ruleId: TYPE_DEFINITIONS,
      folder: "",
      on: null,
      config: { option: "type" },
      explicit: true,
      because: ".eslintrc.json: \"@typescript-eslint/consistent-type-definitions\": [\"error\",\"type\"]",
    }]);
    expect(proposed.notes).toEqual([]);
  });

  it("configures typescript-eslint's no-unused-expressions from allowShortCircuit and allowTernary", async () => {
    const root = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "@typescript-eslint/no-unused-expressions": ["error", { allowShortCircuit: true, allowTernary: true }] } }) });
    const proposed = readRepositoryLintConfig(root, ruleCatalog, TS);
    expect(settingsOf(proposed, on("typescript-eslint-no-unused-expressions"))).toEqual([{
      ruleId: on("typescript-eslint-no-unused-expressions"),
      folder: "",
      on: null,
      config: { allowShortCircuit: true, allowTernary: true },
      explicit: true,
      because: ".eslintrc.json: \"@typescript-eslint/no-unused-expressions\": [\"error\",{\"allowShortCircuit\":true,\"allowTernary\":true}]",
    }]);
    expect(proposed.notes).toEqual([]);
  });

  it("turns off a port for \"no-console\": \"off\", and only for packs being set up", async () => {
    const root = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": \"off\" } }" });
    expect(readRepositoryLintConfig(root, ruleCatalog, JS).settings).toEqual([
      { ruleId: CONSOLE, folder: "", on: false, config: null, explicit: true, because: ".eslintrc.json: \"no-console\": \"off\"" },
    ]);
    expect(settingsOf(readRepositoryLintConfig(root, ruleCatalog, TS), CONSOLE)).toEqual([]);
  });

  // PR #50 review: ESLint's eslintrc loader lower-cases a severity string. ESLint 9.39.1 with
  // ESLINT_USE_FLAT_CONFIG=false reports no console call for "no-console": "OFF" and warns for "no-debugger": "Warn",
  // from an .eslintrc.json and from package.json eslintConfig alike.
  it("takes an .eslintrc or package.json eslintConfig severity in any case, as ESLint's eslintrc loader does", async () => {
    const off = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": \"OFF\" } }" });
    expect(readRepositoryLintConfig(off, ruleCatalog, JS)).toEqual({
      settings: [{ ruleId: CONSOLE, folder: "", on: false, config: null, explicit: true, because: ".eslintrc.json: \"no-console\": \"OFF\"" }],
      notes: [],
    });
    const warn = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"Warn\", { \"allow\": [\"log\"] }] } }" });
    expect(readRepositoryLintConfig(warn, ruleCatalog, JS)).toEqual({
      settings: [{ ruleId: CONSOLE, folder: "", on: null, config: { allow: ["log"] }, explicit: true, because: ".eslintrc.json: \"no-console\": [\"Warn\",{\"allow\":[\"log\"]}]" }],
      notes: [],
    });
    // An override's "WARN" turns the rule back on where it applies.
    const back = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "Off" }, overrides: [{ files: ["src/**/*.ts"], rules: { "no-console": "WARN" } }] }) });
    expect(readRepositoryLintConfig(back, ruleCatalog, JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: .eslintrc.json: \"no-console\": \"Off\" turns it off, but .eslintrc.json: \"no-console\": \"WARN\" turns it back on for [\"src/**/*.ts\"]; it stays on."],
    });
    const manifest = await repository({ "package.json": JSON.stringify({ name: "acme", eslintConfig: { rules: { "no-console": "OFF" } } }) });
    expect(readRepositoryLintConfig(manifest, ruleCatalog, JS).settings).toEqual([
      { ruleId: CONSOLE, folder: "", on: false, config: null, explicit: true, because: "package.json eslintConfig: \"no-console\": \"OFF\"" },
    ]);
  });

  // ESLint 9.39.1 refuses this flat config: 'Configuration for rule "no-console" is invalid. Expected severity of
  // "off", 0, "warn", 1, "error", or 2. You passed '"OFF"'.' (and the same for "Warn").
  it("takes a flat config severity in lower case only, as ESLint 9 does", async () => {
    for (const level of ["OFF", "Warn"]) {
      const root = await repository({ "eslint.config.js": `export default [{ rules: { "no-console": "${level}" } }];` });
      expect(readRepositoryLintConfig(root, ruleCatalog, JS)).toEqual({
        settings: [],
        notes: [`${CONSOLE}: eslint.config.js: "no-console": "${level}" is not a severity ESLint takes; not read.`],
      });
    }
  });

  it("maps an option object onto the options the port declares, and skips one it cannot map with a note", async () => {
    const mapped = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"warn\", { \"allow\": [\"warn\", \"error\"] }] } }" });
    expect(readRepositoryLintConfig(mapped, ruleCatalog, JS).settings).toMatchObject([{ ruleId: CONSOLE, on: null, config: { allow: ["warn", "error"] } }]);

    const wrongValue = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"error\", { \"allow\": \"warn\" }] } }" });
    expect(readRepositoryLintConfig(wrongValue, ruleCatalog, JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: option not mapped (config.allow: must be an array); it keeps its default options. From .eslintrc.json: \"no-console\": [\"error\",{\"allow\":\"warn\"}]."],
    });

    const unknownOption = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": [\"error\", { \"bogus\": true }] } }" });
    expect(readRepositoryLintConfig(unknownOption, ruleCatalog, JS).notes).toEqual([
      "on-eslint-no-console: option not mapped (bogus is not an option the port declares); it keeps its default options. From .eslintrc.json: \"no-console\": [\"error\",{\"bogus\":true}].",
    ]);
  });

  it.each([
    ["a second positional option the port does not declare", ["error", "type", "interface"], "option2 is not an option the port declares"],
    ["a nested option the port does not declare", ["error", { style: { prefer: "type" } }], "style.prefer is not an option the port declares"],
    ["a list of objects", ["error", [{}]], "option: [{}] is not a value Rerail maps"],
    ["an option set positionally and by name", ["error", "type", { option: "interface" }], "option is set twice"],
  ])("maps no options for %s", async (_, value, reason) => {
    const rules = { "@typescript-eslint/consistent-type-definitions": value };
    const root = await repository({ ".eslintrc.json": JSON.stringify({ rules }) });
    const proposed = readRepositoryLintConfig(root, ruleCatalog, TS);
    expect(settingsOf(proposed, TYPE_DEFINITIONS)).toEqual([]);
    expect(proposed.notes).toEqual([
      `${TYPE_DEFINITIONS}: option not mapped (${reason}); it keeps its default options. From .eslintrc.json: "@typescript-eslint/consistent-type-definitions": ${JSON.stringify(value)}.`,
    ]);
  });

  it("turns off a port oxlint implements for a plugin-prefixed .oxlintrc.json key, without mapping oxlint's options", async () => {
    const off = await repository({ ".oxlintrc.json": "{ \"rules\": { \"react-hooks/rules-of-hooks\": \"off\" } }" });
    expect(readRepositoryLintConfig(off, ruleCatalog, ["hooks"]).settings).toEqual([
      { ruleId: HOOKS_RULE, folder: "", on: false, config: null, explicit: true, because: ".oxlintrc.json: \"react-hooks/rules-of-hooks\": \"off\"" },
    ]);

    const options = await repository({ ".oxlintrc.json": "{ \"plugins\": [\"react\"], \"rules\": { \"rules-of-hooks\": [\"deny\", { \"x\": 1 }] } }" });
    expect(readRepositoryLintConfig(options, ruleCatalog, ["hooks"])).toEqual({
      settings: [],
      notes: [`${HOOKS_RULE}: option not mapped: Rerail maps ESLint options, not oxlint's (.oxlintrc.json: "rules-of-hooks": ["deny",{"x":1}]).`],
    });
  });

  // oxlint 1.88.0 reports no rules-of-hooks for `{"rules": {"react-hooks/rules-of-hooks": "error"}}`: its default
  // plugins are eslint, unicorn, typescript and oxc, and a rule of a plugin it does not run is not run.
  it("leaves a rule of a plugin oxlint does not run off, whatever its entry says", async () => {
    const root = await repository({ ".oxlintrc.json": "{ \"rules\": { \"react-hooks/rules-of-hooks\": \"error\" } }" });
    expect(readRepositoryLintConfig(root, ruleCatalog, ["hooks"]).settings).toEqual([{
      ruleId: HOOKS_RULE,
      folder: "",
      on: false,
      config: null,
      explicit: false,
      because: ".oxlintrc.json: \"react-hooks/rules-of-hooks\": \"error\", but .oxlintrc.json does not run the react plugin",
    }]);
  });

  /** The rules of `packs` an `.oxlintrc.json` with `rules` turns off by an entry naming them. */
  async function oxlintOff(rules: Readonly<Record<string, unknown>>, packs: readonly string[]): Promise<string[]> {
    const root = await repository({ ".oxlintrc.json": JSON.stringify({ rules }) });
    return readRepositoryLintConfig(root, ruleCatalog, packs).settings.filter(setting => setting.on === false && setting.explicit).map(setting => setting.ruleId);
  }

  it("resolves an .oxlintrc.json key to a built-in rule as oxlint does: another plugin's rule of the same name is not it", async () => {
    expect(await oxlintOff({ "custom/no-console": "off" }, JS)).toEqual([]);
    expect(await oxlintOff({ "custom/no-console": "off", "eslint/no-console": "error" }, JS)).toEqual([]);
    expect(await oxlintOff({ "react/no-console": "off" }, JS)).toEqual([]);
    expect(await oxlintOff({ "eslint/no-console": "off" }, JS)).toEqual([CONSOLE]);
    expect(await oxlintOff({ "no-console": "off" }, JS)).toEqual([CONSOLE]);
  });

  it("takes typescript-eslint's prefixes and its extension rules as oxlint does", async () => {
    for (const key of ["typescript/ban-tslint-comment", "@typescript-eslint/ban-tslint-comment", "ban-tslint-comment"]) {
      expect(await oxlintOff({ [key]: "off" }, TS)).toEqual([TSLINT_COMMENT]);
    }
    // oxlint runs its ESLint default-param-last for typescript-eslint's; every key names that one rule.
    for (const key of ["default-param-last", "typescript/default-param-last", "@typescript-eslint/default-param-last"]) {
      expect(await oxlintOff({ [key]: "off" }, TS)).toEqual([on("typescript-eslint-default-param-last")]);
    }
    // A bare no-namespace is the import plugin's, the first of three plugins with a rule of that name.
    expect(await oxlintOff({ "no-namespace": "off" }, TS)).toEqual([]);
    expect(await oxlintOff({ "typescript/no-namespace": "off" }, TS)).toEqual([on("typescript-eslint-no-namespace")]);
  });

  it("takes react-hooks as oxlint's react plugin, and an unknown plugin as no built-in one", async () => {
    for (const key of ["react-hooks/rules-of-hooks", "react/rules-of-hooks", "eslint-plugin-react-hooks/rules-of-hooks", "rules-of-hooks"]) {
      expect(await oxlintOff({ [key]: "off" }, ["hooks"])).toEqual([HOOKS_RULE]);
    }
    expect(await oxlintOff({ "hooks/rules-of-hooks": "off" }, ["hooks"])).toEqual([]);
  });

  it("applies a rule's last .oxlintrc.json key, as oxlint does", async () => {
    expect(await oxlintOff({ "no-console": "off", "eslint/no-console": "error" }, JS)).toEqual([]);
    expect(await oxlintOff({ "eslint/no-console": "error", "no-console": "off" }, JS)).toEqual([CONSOLE]);
  });

  it("starts a rule off when one config turns it off and another keeps it on", async () => {
    const root = await repository({
      ".eslintrc.json": "{ \"rules\": { \"react-hooks/rules-of-hooks\": \"error\" } }",
      ".oxlintrc.json": "{ \"rules\": { \"rules-of-hooks\": \"allow\" } }",
    });
    expect(readRepositoryLintConfig(root, ruleCatalog, ["hooks"])).toEqual({
      settings: [{ ruleId: HOOKS_RULE, folder: "", on: false, config: null, explicit: true, because: ".oxlintrc.json: \"rules-of-hooks\": \"allow\"" }],
      notes: [`${HOOKS_RULE}: .eslintrc.json: "react-hooks/rules-of-hooks": "error" keeps it on, but .oxlintrc.json: "rules-of-hooks": "allow" turns it off; it starts off.`],
    });
  });

  const ruffOff = (ruleId: string, because: string, folder = "") => ({ ruleId, folder, on: false, config: null, explicit: true, because });

  it("turns off every Ruff port [tool.ruff.lint] ignore = [\"E7\"] selects, noting the flat config only where a pack ports ESLint rules", async () => {
    const root = await repository({ "pyproject.toml": "[tool.ruff.lint]\nignore = [\"E7\"]\n", "eslint.config.ts": "import makeConfig from \"shared-config\";\nexport default makeConfig();\n" });
    expect(readRepositoryLintConfig(root, ruleCatalog, PY)).toEqual({
      settings: E7.map(ruleId => ruffOff(ruleId, "pyproject.toml: \"E7\" in tool.ruff.lint.ignore")),
      // No pack being set up ports an ESLint rule, so the flat config is not worth a note.
      notes: [],
    });
    expect(readRepositoryLintConfig(root, ruleCatalog, [...PY, ...JS]).notes).toEqual([
      "eslint.config.ts: the rules of makeConfig() are not read: Rerail does not run code to find them.",
    ]);
  });

  /** The ported settings and notes for a pyproject.toml whose [tool.ruff.lint] section holds `lines`. */
  async function ruffLint(...lines: string[]) {
    const root = await repository({ "pyproject.toml": ["[tool.ruff.lint]", ...lines].join("\n") });
    return readRepositoryLintConfig(root, ruleCatalog, PY);
  }

  it("lets a more specific select win over an ignore, as Ruff applies selectors from least to most specific", async () => {
    expect(await ruffLint("select = [\"E711\"]", "ignore = [\"E7\"]")).toEqual({
      settings: E7.slice(1).map(ruleId => ruffOff(ruleId, "pyproject.toml: \"E7\" in tool.ruff.lint.ignore")),
      notes: [`${E7[0]}: pyproject.toml: "E7" in tool.ruff.lint.ignore is overridden by pyproject.toml: "E711" in tool.ruff.lint.select, which Ruff applies after it; it stays on.`],
    });
    // extend-select re-enables the same way; ALL is the least specific selector of all.
    expect((await ruffLint("extend-select = [\"E71\"]", "ignore = [\"E7\"]")).settings.map(setting => setting.ruleId)).not.toContain(E7[0]);
    expect((await ruffLint("ignore = [\"ALL\"]", "select = [\"E711\"]")).settings.map(setting => setting.ruleId)).not.toContain(E7[0]);
  });

  it("lets an ignore win over a select of the same specificity, and over a less specific one", async () => {
    expect((await ruffLint("extend-select = [\"E7\"]", "ignore = [\"E7\"]")).settings.map(setting => setting.ruleId)).toContain(E7[0]);
    expect(settingsOf(await ruffLint("select = [\"E\"]", "ignore = [\"E711\"]"), E7[0]!)).toEqual([ruffOff(E7[0]!, "pyproject.toml: \"E711\" in tool.ruff.lint.ignore")]);
  });

  it("turns a rule off in a folder a per-file ignore covers, and leaves it on for a narrower pattern", async () => {
    const root = await repository({
      "pyproject.toml": "[tool.ruff.lint.per-file-ignores]\n\"tests/**\" = [\"E722\"]\n\"**/test_*.py\" = [\"E721\"]\n",
    });
    const proposed = readRepositoryLintConfig(root, ruleCatalog, PY);
    // E722 is in Ruff's default selection, so it is on but in tests/.
    expect(settingsOf(proposed, on("ruff-e722-bare-except"))).toEqual([ruffOff(on("ruff-e722-bare-except"), "pyproject.toml: \"E722\" in tool.ruff.lint.per-file-ignores.\"tests/**\"", "tests")]);
    expect(proposed.notes).toEqual([`${on("ruff-e721-type-comparison")}: pyproject.toml: "E721" in tool.ruff.lint.per-file-ignores."**/test_*.py" ignores it only in the files "**/test_*.py" matches; it stays on.`]);
  });

  // Each case checked against `ruff check` 0.15.0 on the same files: a runs E711 only; b runs E711, E721 and E722;
  // c runs E721 and E722.
  it("follows extend as Ruff layers configs: a later config re-enables, a select replaces, select = [] carries its ignores", async () => {
    const offs = async (pyproject: string, base: string): Promise<string[]> => {
      const root = await repository({ "pyproject.toml": `[tool.ruff]\nextend = "configs/base.toml"\n${pyproject}`, "configs/base.toml": base });
      return readRepositoryLintConfig(root, ruleCatalog, PY).settings.filter(setting => setting.on === false).map(setting => setting.ruleId).filter(id => /e71|e72/u.test(id));
    };
    expect(await offs("[tool.ruff.lint]\nextend-select = [\"E711\"]\n", "[lint]\nignore = [\"E7\"]\n")).toEqual([E7[1], E7[2]]);
    expect(await offs("[tool.ruff.lint]\nselect = [\"E\"]\n", "[lint]\nignore = [\"E7\"]\n")).toEqual([]);
    expect(await offs("[tool.ruff.lint]\nselect = [\"E7\"]\n", "[lint]\nselect = []\nignore = [\"E711\"]\n")).toEqual([E7[0]]);
  });

  // Self-review: an explicitly empty per-file-ignores keeps the extended config's out, as Ruff 0.15.0's combine
  // (`self.per_file_ignores.or(config.per_file_ignores)`) does; `ruff check` reports E722 in tests/ for both.
  it("lets an empty per-file-ignores, inline or as a table header, replace the extended config's", async () => {
    const base = "[lint.per-file-ignores]\n\"tests/**\" = [\"E722\"]\n";
    for (const own of ["[lint]\nper-file-ignores = {}\n", "[lint.per-file-ignores]\n"]) {
      const root = await repository({ "ruff.toml": `extend = "base.toml"\n${own}`, "base.toml": base });
      expect(settingsOf(readRepositoryLintConfig(root, ruleCatalog, PY), E7[2]!)).toEqual([]);
    }
    const inherited = await repository({ "ruff.toml": "extend = \"base.toml\"\n", "base.toml": base });
    expect(settingsOf(readRepositoryLintConfig(inherited, ruleCatalog, PY), E7[2]!)).toEqual([
      ruffOff(E7[2]!, "base.toml: \"E722\" in lint.per-file-ignores.\"tests/**\"", "tests"),
    ]);
  });
});

describe("readRepositoryLintConfig for .eslintrc overrides", () => {
  it("turns a rule off in a folder an override covers, and keeps one on that an override turns back on", async () => {
    const scripts = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "error" }, overrides: [{ files: "scripts/**", rules: { "no-console": "off" } }] }) });
    expect(readRepositoryLintConfig(scripts, ruleCatalog, JS).settings).toEqual([
      { ruleId: CONSOLE, folder: "scripts", on: false, config: null, explicit: true, because: ".eslintrc.json: \"no-console\": \"off\" for \"scripts/**\"" },
    ]);
    const back = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "off" }, overrides: [{ files: ["src/**/*.ts"], rules: { "no-console": "warn" } }] }) });
    expect(readRepositoryLintConfig(back, ruleCatalog, JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: .eslintrc.json: \"no-console\": \"off\" turns it off, but .eslintrc.json: \"no-console\": \"warn\" turns it back on for [\"src/**/*.ts\"]; it stays on."],
    });
  });

  // Self-review: ESLint 9.39.1 --print-config scripts/a.js gives severity 2 when a later override turns it back on.
  // Nothing else sets the rule, so ESLint runs it in scripts/ only.
  it("runs a rule only in a folder a later override turns it back on in", async () => {
    const root = await repository({ ".eslintrc.json": JSON.stringify({ overrides: [
      { files: "scripts/**", rules: { "no-console": "off" } },
      { files: "scripts/**", rules: { "no-console": "error" } },
    ] }) });
    expect(readRepositoryLintConfig(root, ruleCatalog, JS)).toEqual({
      settings: [
        { ruleId: CONSOLE, folder: "", on: false, config: null, explicit: false, because: ".eslintrc.json does not turn it on" },
        { ruleId: CONSOLE, folder: "scripts", on: true, config: null, explicit: true, because: ".eslintrc.json: \"no-console\": \"error\" for \"scripts/**\"" },
      ],
      notes: ["on-eslint-no-console: .eslintrc.json: \"no-console\": \"off\" for \"scripts/**\" turns it off, but .eslintrc.json: \"no-console\": \"error\" turns it back on for \"scripts/**\"; it stays on."],
    });
  });

  // Self-review round 4: an override's extends and nested overrides are not read and may turn the rule back on
  // (ESLINT_USE_FLAT_CONFIG=false eslint 9.39.1 --print-config tests/a.js gives "error" for both).
  it("uses no off an override's extends or nested overrides may replace", async () => {
    const extended = await repository({ ".eslintrc.json": JSON.stringify({ rules: { "no-console": "off" }, overrides: [{ files: ["tests/**"], extends: ["./strict.json"] }] }) });
    expect(readRepositoryLintConfig(extended, ruleCatalog, JS).settings).toEqual([]);
    const nested = await repository({ ".eslintrc.json": JSON.stringify({ overrides: [{ files: ["tests/**"], rules: { "no-console": "off" }, overrides: [{ files: ["tests/**/*.js"], rules: { "no-console": "error" } }] }] }) });
    expect(readRepositoryLintConfig(nested, ruleCatalog, JS).settings).toEqual([]);
  });

  // Self-review round 4: oxlint 1.88.0 reports no-console in tests/ for this config.
  it("uses no .oxlintrc.json off its overrides, which Rerail does not read, may replace", async () => {
    const root = await repository({ ".oxlintrc.json": JSON.stringify({ rules: { "no-console": "off" }, overrides: [{ files: ["tests/**"], rules: { "no-console": "error" } }] }) });
    expect(readRepositoryLintConfig(root, ruleCatalog, JS)).toEqual({
      settings: [],
      notes: [
        ".oxlintrc.json: only its own rules are read, not overrides.",
        "on-eslint-no-console: .oxlintrc.json: \"no-console\": \"off\" is not used: .oxlintrc.json overrides, which Rerail does not read, may set it for some files.",
      ],
    });
  });
});

/**
 * What set-up proposes for packs `packs` from a flat config it cannot read statically: every port of ESLint rules that
 * starts on, off with no options, in port order.
 */
const eslintPortsOff = (packs: readonly string[]) => portsOf(ruleCatalog, packs)
  .filter(port => port.startsOn && port.suppress.eslint.length > 0)
  .map(port => ({ ruleId: port.ruleId, folder: "", on: false, config: null, explicit: true, because: expect.stringMatching(/^eslint\.config\.[cm]?[jt]s is not statically readable$/u) }));

describe("readRepositoryLintConfig for ESLint's flat config", () => {
  /** The settings and notes for packs `packs` from a repository whose only lint config is `name` holding `text`. */
  async function flat(text: string, packs: readonly string[] = TS, name = "eslint.config.ts") {
    const root = await repository({ [name]: text });
    return readRepositoryLintConfig(root, ruleCatalog, packs);
  }

  // The repro from dolanmiu/docx at PR 3716: the option is set in a config object for **/src/**/*.ts, after shared
  // configs, in a typed const the file exports. Both shared configs are recognised: typescript-eslint 8.70.1's
  // stylistic turns ban-tslint-comment and consistent-type-definitions on, and neither it nor @eslint/js's recommended
  // turns on default-param-last, no-namespace or no-unused-expressions, so with the whole config read those start off.
  it("configures consistent-type-definitions from a docx-shaped eslint.config.ts, reading the shared configs it takes", async () => {
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
    const unset = (ruleId: string) => ({ ruleId: on(ruleId), folder: "", on: false, config: null, explicit: false, because: "eslint.config.ts does not turn it on" });
    expect(await flat(text)).toEqual({
      settings: [
        {
          ruleId: TYPE_DEFINITIONS,
          folder: "",
          on: null,
          config: { option: "type" },
          explicit: true,
          because: "eslint.config.ts: \"@typescript-eslint/consistent-type-definitions\": [\"error\",\"type\"] for [\"**/src/**/*.ts\"]",
        },
        ...["typescript-eslint-default-param-last", "typescript-eslint-no-namespace", "typescript-eslint-no-unused-expressions"].map(unset),
      ],
      notes: [],
    });
  });

  it("reads defineConfig, typescript-eslint's config(), extends, module.exports and const bindings", async () => {
    const off = (ruleId: string, because: string) => ({ ruleId, folder: "", on: false, config: null, explicit: true, because });
    expect((await flat([
      "import { defineConfig } from \"eslint/config\";",
      "const level = \"off\";",
      "const base = { rules: { \"no-console\": level } };",
      "export default defineConfig([{ extends: [base] }]);",
    ].join("\n"), JS, "eslint.config.mjs")).settings).toEqual([off(CONSOLE, "eslint.config.mjs: \"no-console\": \"off\"")]);
    expect((await flat("module.exports = [{ rules: { \"no-console\": 0 } }];\n", JS, "eslint.config.cjs")).settings)
      .toEqual([off(CONSOLE, "eslint.config.cjs: \"no-console\": 0")]);
    expect(settingsOf(await flat([
      "import tseslint from \"typescript-eslint\";",
      "export default tseslint.config({ rules: { \"@typescript-eslint/ban-tslint-comment\": \"off\" } });",
    ].join("\n")), TSLINT_COMMENT)).toEqual([off(TSLINT_COMMENT, "eslint.config.ts: \"@typescript-eslint/ban-tslint-comment\": \"off\"")]);
  });

  it("applies config objects in order: a later one replaces a rule, keeping its options when it sets only a severity", async () => {
    expect((await flat("export default [{ rules: { \"no-console\": \"off\" } }, { rules: { \"no-console\": \"error\" } }];", JS)).settings).toEqual([]);
    expect((await flat("export default [{ rules: { \"no-console\": \"error\" } }, { rules: { \"no-console\": \"off\" } }];", JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
    expect((await flat("export default [{ rules: { \"no-console\": [\"error\", { allow: [\"warn\"] }] } }, { rules: { \"no-console\": \"warn\" } }];", JS)).settings)
      .toEqual([{ ruleId: CONSOLE, folder: "", on: null, config: { allow: ["warn"] }, explicit: true, because: "eslint.config.ts: \"no-console\": [\"warn\",{\"allow\":[\"warn\"]}]" }]);
  });

  it("keeps a rule on that a later object turns back on for some files, and turns it off in a folder a files glob covers", async () => {
    expect(await flat("export default [{ rules: { \"no-console\": \"off\" } }, { files: [\"src/**/*.ts\"], rules: { \"no-console\": \"error\" } }];", JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: eslint.config.ts: \"no-console\": \"off\" turns it off, but eslint.config.ts: \"no-console\": \"error\" turns it back on for [\"src/**/*.ts\"]; it stays on."],
    });
    expect(await flat("export default [{ rules: { \"no-console\": \"error\" } }, { files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { files: [\"**/*.test.ts\"], rules: { \"no-console\": 0 } }];", JS)).toEqual({
      settings: [{ ruleId: CONSOLE, folder: "scripts", on: false, config: null, explicit: true, because: "eslint.config.ts: \"no-console\": \"off\" for [\"scripts/**\"]" }],
      notes: ["on-eslint-no-console: eslint.config.ts: \"no-console\": 0 for [\"**/*.test.ts\"] turns it off only for some files; it stays on."],
    });
  });

  it("turns a port off where its entry's value cannot be read without running the config, unless a later entry replaces it", async () => {
    const imported = "import shared from \"./shared.js\";\n";
    expect(await flat(`${imported}export default [{ rules: { "no-console": shared.level } }];`, JS)).toEqual({
      settings: [{ ruleId: CONSOLE, folder: "", on: false, config: null, explicit: true, because: "eslint.config.ts: \"no-console\" is not read (it is shared.level, which Rerail does not run)" }],
      notes: [],
    });
    expect((await flat(`${imported}export default [{ files: ["**/*.ts"], rules: { "no-console": ["error", shared.options] } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, folder: "", on: false }]);
    // `["error"]` is a severity alone: ESLint keeps the earlier options, which Rerail cannot read.
    expect((await flat(`${imported}export default [{ rules: { "no-console": shared.level } }, { rules: { "no-console": ["error"] } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
    expect((await flat(`${imported}export default [{ rules: { "no-console": shared.level } }, { rules: { "no-console": ["error", { allow: ["warn"] }] } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: null, config: { allow: ["warn"] } }]);
  });

  // Self-review findings 4, 6, 7 and 8, each checked against ESLint 9.39.1 on the same config: ESLint runs it in
  // scripts/ only.
  it("runs a rule only in a folder a later object turns it back on in", async () => {
    expect(await flat("export default [{ files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { files: [\"scripts/**\"], rules: { \"no-console\": \"error\" } }];", JS)).toEqual({
      settings: [
        { ruleId: CONSOLE, folder: "", on: false, config: null, explicit: false, because: "eslint.config.ts does not turn it on" },
        { ruleId: CONSOLE, folder: "scripts", on: true, config: null, explicit: true, because: "eslint.config.ts: \"no-console\": \"error\" for [\"scripts/**\"]" },
      ],
      notes: ["on-eslint-no-console: eslint.config.ts: \"no-console\": \"off\" for [\"scripts/**\"] turns it off, but eslint.config.ts: \"no-console\": \"error\" turns it back on for [\"scripts/**\"]; it stays on."],
    });
  });

  it("does not widen options set for some files to the whole repository when the rule is set for every file", async () => {
    expect(await flat("export default [{ rules: { \"no-console\": \"error\" } }, { files: [\"tests/**\"], rules: { \"no-console\": [\"error\", { allow: [\"log\"] }] } }];", JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: the options of eslint.config.ts: \"no-console\": [\"error\",{\"allow\":[\"log\"]}] for [\"tests/**\"] are not used: eslint.config.ts: \"no-console\": \"error\" sets the rule for every file, and Rerail sets options for the whole repository only."],
    });
  });

  it("reads one rules object as JavaScript builds it: a later key replaces a spread or an earlier key whole", async () => {
    const common = "const common = { \"no-console\": [\"error\", { allow: [\"log\"] }] };\n";
    expect((await flat(`${common}export default [{ rules: { ...common, "no-console": "error" } }];`, JS)).settings).toEqual([]);
    expect((await flat("export default [{ rules: { \"no-console\": [\"error\", { allow: [\"log\"] }], \"no-console\": \"error\" } }];", JS, "eslint.config.js")).settings).toEqual([]);
    expect((await flat(`${common}export default [{ rules: { "no-console": "error", ...common } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, config: { allow: ["log"] } }]);
  });

  it("merges a one-element severity list as a severity alone, keeping the earlier options", async () => {
    expect(settingsOf(await flat("export default [{ rules: { \"@typescript-eslint/consistent-type-definitions\": [\"error\", \"type\"] } }, { rules: { \"@typescript-eslint/consistent-type-definitions\": [\"warn\"] } }];"), TYPE_DEFINITIONS))
      .toEqual([{ ruleId: TYPE_DEFINITIONS, folder: "", on: null, config: { option: "type" }, explicit: true, because: "eslint.config.ts: \"@typescript-eslint/consistent-type-definitions\": [\"warn\",\"type\"]" }]);
  });

  it("uses no entry that a later config element it cannot read may replace", async () => {
    expect(await flat("import prettier from \"eslint-config-prettier\";\nexport default [{ rules: { \"no-console\": \"off\" } }, prettier];", JS)).toEqual({
      settings: [],
      notes: [
        "eslint.config.ts: the rules of prettier are not read: Rerail does not run code to find them.",
        "on-eslint-no-console: eslint.config.ts: \"no-console\": \"off\" is not used: prettier, after it, is not read and may replace it.",
      ],
    });
    const shared = "import shared from \"./shared.js\";\n";
    expect((await flat(`${shared}export default [{ rules: { "no-console": "off", ...shared.rules } }];`, JS)).settings).toEqual([]);
    expect((await flat(`${shared}export default [{ rules: { ...shared.rules, "no-console": "off" } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  // Self-review round 2, each checked against ESLint 9.39.1 --print-config on the same config.
  it("keeps an entry a later unread element may replace unused through a severity alone after it", async () => {
    const shared = "import shared from \"./shared.js\";\n";
    expect((await flat(`${shared}export default [{ rules: { "no-console": ["error", { allow: ["log"] }] } }, shared, { rules: { "no-console": ["warn"] } }];`, JS)).settings).toEqual([]);
    // A full replacement after it is read again.
    expect((await flat(`${shared}export default [{ rules: { "no-console": "off" } }, shared, { rules: { "no-console": ["warn", { allow: ["info"] }] } }];`, JS)).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: null, config: { allow: ["info"] } }]);
  });

  it("does not turn a port off for an unread value that a later unread element may replace", async () => {
    expect(await flat("import shared from \"./shared.js\";\nexport default [{ rules: { \"no-console\": shared.rules[\"no-console\"] } }, shared];", JS)).toEqual({
      settings: [],
      notes: [
        "eslint.config.ts: the rules of shared are not read: Rerail does not run code to find them.",
        "on-eslint-no-console: eslint.config.ts: \"no-console\" is not read (it is shared.rules[\"no-console\"], which Rerail does not run) and is not used: shared, after it, is not read and may replace it.",
      ],
    });
  });

  it("reads a config object with a getter as one it cannot read, never calling the getter", async () => {
    const text = "export default [{ rules: { \"no-console\": \"off\" } }, { get rules() { throw new Error(\"ran\"); } }];";
    expect((await flat(text, JS, "eslint.config.js")).settings).toEqual(eslintPortsOff(JS));
  });

  it("does not read a const the file changes after setting it, however it does", async () => {
    for (const text of [
      "const rules = { \"no-console\": \"off\" };\nrules[\"no-console\"] = \"error\";\nexport default [{ rules }];",
      "const config = [{ rules: { \"no-console\": \"off\" } }];\nconfig.push({ rules: { \"no-console\": \"error\" } });\nexport default config;",
      "const rules = { \"no-console\": \"off\" };\nconst base = { rules };\nfunction tweak(r) { r.rules[\"no-console\"] = \"error\"; }\ntweak(base);\nexport default [{ rules }];",
    ]) {
      expect((await flat(text, JS, "eslint.config.js")).settings).toEqual(eslintPortsOff(JS));
    }
    // A const nothing changes is read.
    expect((await flat("const rules = { \"no-console\": \"off\" };\nexport default [{ rules }];", JS, "eslint.config.js")).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  // Self-review round 3: a const is read only where every reference to it is data the export is built from.
  it("does not read a const that code can reach through an alias, a loop, a destructuring target, a return or an export", async () => {
    for (const text of [
      "const config = { rules: { \"no-console\": \"off\" } };\nconst { rules } = config;\nrules[\"no-console\"] = \"error\";\nexport default [config];",
      "const configs = [{ rules: { \"no-console\": \"off\" } }];\nfor (const config of configs) config.rules[\"no-console\"] = \"error\";\nexport default configs;",
      "const rules = { \"no-console\": \"off\" };\n({ level: rules[\"no-console\"] } = { level: \"error\" });\nexport default [{ rules }];",
      "const rules = { \"no-console\": \"off\" };\nfunction getRules() { return rules; }\nObject.assign(getRules(), { \"no-console\": \"error\" });\nexport default [{ rules }];",
      // An exported const is not read: a module that imports it may change it, so the file is not statically readable.
      "export const rules = { \"no-console\": \"off\" };\nexport default [{ rules }];",
      "const rules = { \"no-console\": \"off\" };\nexport { rules };\nexport default [{ rules }];",
    ]) {
      expect((await flat(text, JS, "eslint.config.mjs")).settings).toEqual(eslintPortsOff(JS));
    }
  });

  // Self-review round 4: a flat config is read only when it is data throughout; each of these runs code, so none gives a
  // setting (each ESLint 9.39.1 config turns no-console on).
  it("reads nothing from a flat config that runs code, however small", async () => {
    for (const text of [
      "const rules = { \"no-console\": \"off\" };\nconst { alias = rules } = {};\nalias[\"no-console\"] = \"error\";\nexport default [{ rules }];",
      "const rules = { \"no-console\": \"off\" };\nfunction change(alias = rules) { alias[\"no-console\"] = \"error\"; }\nchange();\nexport default [{ rules }];",
      "const rules = { \"no-console\": \"off\" };\nnamespace Runtime { rules[\"no-console\"] = \"error\"; }\nexport default [{ rules }];",
      "module.exports = [{ rules: { \"no-console\": \"off\" } }];\nmodule.exports[0].rules[\"no-console\"] = \"error\";",
      "import tseslint from \"typescript-eslint\";\n({ config: tseslint.config } = { config: () => [{ rules: { \"no-console\": \"error\" } }] });\nexport default tseslint.config({ rules: { \"no-console\": \"off\" } });",
      "import tseslint from \"typescript-eslint\";\nfor (tseslint!.config of [() => []]);\nexport default tseslint.config({ rules: { \"no-console\": \"off\" } });",
      "const severity = \"off\";\nfunction describe(severity: string) { return severity; }\nexport default [{ rules: { \"no-console\": severity } }];",
      "const allow = [\"log\"];\nconst metadata = { allow: 0 };\nObject.freeze(metadata);\nexport default [{ rules: { \"no-console\": [\"error\", { allow }] } }];",
    ]) {
      const proposed = await flat(text, JS);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.ts is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
  });

  // Self-review round 5: each of these changes the config at load time (ESLint 9.39.1 turns no-console on for every
  // one) through eval, module.exports, an exported const handed to a call, or a module binding of the file's own.
  it("reads nothing from a flat config that reaches its own data through eval, module.exports or an exported const", async () => {
    const cases: [string, string][] = [
      ["eslint.config.mjs", "const rules = { \"no-console\": \"off\" };\nconst x = eval('rules[\"no-console\"] = \"error\"');\nexport default [{ rules }];"],
      ["eslint.config.cjs", "module.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst x = Object.assign(module.exports[0].rules, { \"no-console\": \"error\" });"],
      ["eslint.config.mjs", "const rules = { \"no-console\": \"off\" };\nexport const alias = { rules };\nconst x = Object.assign(alias.rules, { \"no-console\": \"error\" });\nexport default [{ rules }];"],
      ["eslint.config.mjs", "const rules = { \"no-console\": \"off\" };\nconst alias = { rules };\nexport { alias };\nconst x = Object.assign(alias.rules, { \"no-console\": \"error\" });\nexport default [{ rules }];"],
    ];
    for (const [name, text] of cases) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.[cm]js is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
    const shadowed = await flat("const module = {};\nexport default [{ rules: { \"no-console\": \"error\" } }];\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];", JS, "eslint.config.mjs");
    expect(shadowed.settings).toEqual(eslintPortsOff(JS));
    // A CommonJS config that requires its modules is still read.
    const required = "const js = require(\"@eslint/js\");\nconst merge = require(\"merge\");\nmodule.exports = [merge(js.configs.recommended), { rules: { \"no-console\": \"off\" } }];";
    expect((await flat(required, JS, "eslint.config.cjs")).settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  it("reads a flat config that calls imported functions with nothing of its own", async () => {
    const text = [
      "import { fileURLToPath } from \"node:url\";",
      "import { createResolver } from \"resolver\";",
      "const root = fileURLToPath(new URL(\".\", import.meta.url));",
      "export default [{ settings: { resolver: createResolver(), root }, rules: { \"no-console\": \"off\" } }];",
    ].join("\n");
    expect((await flat(text, JS)).settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
    // Handing a call one of its own consts is code: the call could change it.
    expect((await flat("import merge from \"merge\";\nconst rules = { \"no-console\": \"off\" };\nexport default [{ rules: merge(rules) }];", JS)).settings).toEqual(eslintPortsOff(JS));
  });

  // Self-review round 6: each of these changes the exported config at load time through a global (ESLint 9.39.1 turns
  // no-console on for every one); a flat config may call only into its imports, import.meta, require, URL and
  // process.cwd.
  it("reads nothing from a flat config that calls a global, or reaches its module through arguments", async () => {
    const payload = "'process.getBuiltinModule(\"module\")'";
    const cases: [string, string][] = [
      ["eslint.config.cjs", "module.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst x = Object.assign(arguments[2].exports[0].rules, { \"no-console\": \"error\" });"],
      ["eslint.config.mjs", `const x = globalThis.eval(${payload});\nexport default [{ rules: { "no-console": "off" } }];`],
      ["eslint.config.mjs", `const x = new Function(${payload}).call(null);\nexport default [{ rules: { "no-console": "off" } }];`],
      ["eslint.config.mjs", `const x = Reflect.apply(globalThis.eval, null, [${payload}]);\nexport default [{ rules: { "no-console": "off" } }];`],
      ["eslint.config.mjs", `export default [{ [globalThis.eval(${payload})]: 1, rules: { "no-console": "off" } }];`],
      ["eslint.config.mjs", "import tseslint from \"typescript-eslint\";\nconst changed = Object.defineProperty(tseslint, \"config\", { value: () => [] });\nexport default tseslint.config({ rules: { \"no-console\": \"off\" } });"],
    ];
    for (const [name, text] of cases) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.[cm]js is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
    // process.cwd() and import.meta's own functions are still read.
    const allowed = "const root = process.cwd();\nconst here = import.meta.resolve(\"./x.js\");\nexport default [{ settings: { root, here }, rules: { \"no-console\": \"off\" } }];";
    expect((await flat(allowed, JS, "eslint.config.mjs")).settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
    // Only the call with no argument: handed one, process.cwd is a global's call like any other.
    const withArgument = await flat("const root = process.cwd(\"x\");\nexport default [{ settings: { root }, rules: { \"no-console\": \"off\" } }];", JS, "eslint.config.mjs");
    expect(withArgument.settings).toEqual(eslintPortsOff(JS));
    expect(withArgument.notes).toEqual(["eslint.config.mjs is not statically readable: it runs code Rerail does not read as data (line 1: process.cwd(\"x\")). Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off."]);
  });

  // Self-review round 7: node:vm runs code by design, and a config may require itself; calls into either change the
  // exported config at load time (ESLint 9.39.1 turns no-console on for both).
  it("reads nothing from a flat config that calls into a code-running Node module or a module it names by path", async () => {
    const payload = "'process.getBuiltinModule(\"module\")'";
    const cases: [string, string][] = [
      ["eslint.config.cjs", `const vm = require("node:vm");\nmodule.exports = [{ rules: { "no-console": "off" } }];\nconst x = vm.runInThisContext(${payload});`],
      ["eslint.config.mjs", `import { runInThisContext } from "vm";\nconst x = runInThisContext(${payload});\nexport default [{ rules: { "no-console": "off" } }];`],
      ["eslint.config.mjs", "import process from \"node:process\";\nconst m = process.getBuiltinModule(\"module\");\nexport default [{ rules: { \"no-console\": \"off\" } }];"],
      ["eslint.config.cjs", "module.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst self = require(\"./eslint.config.cjs\");\nconst changed = self.push({ rules: { \"no-console\": \"error\" } });"],
    ];
    for (const [name, text] of cases) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.[cm]js is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
    // A local module used as data, not called, is still read.
    expect((await flat("import shared from \"./shared.mjs\";\nexport default [...shared, { rules: { \"no-console\": \"off\" } }];", JS, "eslint.config.mjs")).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  // Self-review round 8: `.constructor` of an imported function is `Function`, and JavaScript calls `toString` itself
  // inside a template, so this changes the exported config at load time (ESLint 9.39.1 reports no-console).
  it("reads nothing from a flat config that reaches Function through a member, or hands JavaScript a function to call", async () => {
    const payload = "\"process.getBuiltinModule('module')\"";
    const cases = [
      `const path = require("node:path");\nmodule.exports = [{ rules: { "no-console": "off" } }];\nconst changed = \`\${{ toString: path.join.constructor(${payload}) }}\`;`,
      `const path = require("node:path");\nmodule.exports = [{ rules: { "no-console": "off" } }];\nconst changed = path.join["constructor"](${payload})();`,
      `const path = require("node:path");\nconst key = "con" + "structor";\nmodule.exports = [{ rules: { "no-console": "off" } }];\nconst changed = path.join[key](${payload})();`,
      "const path = require(\"node:path\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = `${{ valueOf: path.join }}`;",
      "const vm = require(\"node:vm\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = `${{ [Symbol.toPrimitive]: vm.runInThisContext }}`;",
    ];
    for (const text of cases) {
      const proposed = await flat(text, JS, "eslint.config.cjs");
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.cjs is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
  });

  // Self-review round 9: ESLint awaits the config and loads it with import(), and JavaScript calls a `then` it finds on
  // either; `then: [].forEach` hands the loader element 0 (ESLint 9.39.1 reports no-console for the first case).
  it("reads nothing from a flat config that JavaScript would call as it awaits or imports it", async () => {
    const cases = [
      "export default { 0: [{ rules: { \"no-console\": \"error\" } }], length: 1, then: [].forEach, rules: { \"no-console\": \"off\" } };",
      "export const then = [].forEach;\nexport default [{ rules: { \"no-console\": \"off\" } }];",
      "const forEach = [].forEach;\nexport { forEach as then };\nexport default [{ rules: { \"no-console\": \"off\" } }];",
      "export * from \"./more.mjs\";\nexport default [{ rules: { \"no-console\": \"off\" } }];",
    ];
    for (const text of cases) {
      const proposed = await flat(text, JS, "eslint.config.mjs");
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.mjs is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
  });

  // Self-review round 10: require.cache holds the config's own module, so a package helper handed its rules changes
  // the export (ESLint 9.39.1 reports no-console for the first case); process, globalThis and the like reach it too.
  it("reads nothing from a flat config that names a global other than its trusted calls", async () => {
    const cases = [
      "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = util._extend(require.cache[__filename].exports[0].rules, { \"no-console\": \"error\" });",
      "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = util._extend(require.main, {});",
      "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = util._extend(process.mainModule, {});",
      "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = util._extend(globalThis, {});",
    ];
    for (const text of cases) {
      const proposed = await flat(text, JS, "eslint.config.cjs");
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.cjs is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
    // Reading the environment, the config's own folder, and its trusted calls are still data.
    const allowed = "const path = require(\"node:path\");\nconst root = path.join(__dirname, process.cwd());\nmodule.exports = [{ settings: { root, ci: process.env.CI }, rules: { \"no-console\": \"off\" } }];";
    expect((await flat(allowed, JS, "eslint.config.cjs")).settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  // Self-review round 11: ESLint 9.39.1 loads a TypeScript config through Jiti and takes `config.default ?? config`, so
  // for the first case it reports no-console, which the outer object turns off.
  it("reads nothing from an exported object that may hold a default config", async () => {
    const cases: [string, string][] = [
      ["eslint.config.cts", "module.exports = { default: [{ rules: { \"no-console\": \"error\" } }], rules: { \"no-console\": \"off\" } };"],
      ["eslint.config.cts", "export = { default: [{ rules: { \"no-console\": \"error\" } }], rules: { \"no-console\": \"off\" } };"],
      ["eslint.config.mjs", "import shared from \"shared\";\nexport default { ...shared, rules: { \"no-console\": \"off\" } };"],
    ];
    for (const [name, text] of cases) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([`${name} is not statically readable: it exports an object that may hold a \`default\` config, which ESLint's loader can take instead. Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.`]);
    }
    // An exported object without one is read as the one config it is.
    expect((await flat("export default { rules: { \"no-console\": \"off\" } };", JS, "eslint.config.mjs")).settings)
      .toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  // Self-review rounds 11 and 14: a later severity alone for every file keeps the options an entry for some files set
  // there (ESLint 9.39.1 allows only warn in tests/ for this config), and options for every file are the port's only
  // when no entry for some files sets others: allow: ["log"] for the whole repository would let console.log through in
  // tests/, where ESLint reports it.
  it("proposes no options for every file where an entry for some files sets others, through a later severity alone too", async () => {
    const text = "export default [{ rules: { \"no-console\": [\"error\", { allow: [\"log\"] }] } }, { files: [\"tests/**\"], rules: { \"no-console\": [\"error\", { allow: [\"warn\"] }] } }, { rules: { \"no-console\": \"error\" } }];";
    expect(await flat(text, JS)).toEqual({
      settings: [],
      notes: ["on-eslint-no-console: the options of eslint.config.ts: \"no-console\": [\"error\",{\"allow\":[\"log\"]}] are not used: eslint.config.ts: \"no-console\": [\"error\",{\"allow\":[\"warn\"]}] sets others for [\"tests/**\"], and Rerail sets options for the whole repository only."],
    });
  });

  // Self-review round 12: a module named by path may be the config itself, and a type-only import is erased, so its
  // names are the globals Object and module; each case changes the export at load time (ESLint 9.39.1 through Jiti
  // reports no-console).
  it("reads nothing from a flat config that hands a call a module named by path, or calls a global a type import names", async () => {
    const cases: [string, string][] = [
      ["eslint.config.cjs", "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst self = require(\"./eslint.config.cjs\");\nconst changed = util._extend(self[0].rules, { \"no-console\": \"error\" });"],
      ["eslint.config.cjs", "const util = require(\"node:util\");\nmodule.exports = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = util._extend(require(\"./eslint.config.cjs\")[0].rules, { \"no-console\": \"error\" });"],
      ["eslint.config.cts", "import type { Object, module } from \"not-a-runtime-package\";\nexport = [{ rules: { \"no-console\": \"off\" } }];\nconst changed = Object.assign(module.exports[0].rules, { \"no-console\": \"error\" });"],
      ["eslint.config.ts", "import { type Object } from \"not-a-runtime-package\";\nconst rules = { \"no-console\": \"off\" };\nconst changed = Object.keys({});\nexport default [{ rules }];"],
    ];
    for (const [name, text] of cases) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.c?[jt]s is not statically readable: it runs code Rerail does not read as data \(line \d+: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
  });

  // Self-review round 13: a `declare const` is erased, so its name is the global (here the config's own path), not
  // the initializer; ESLint 9.39.1 through Jiti reports console.log, which allow: ["log"] would let through.
  it("reads nothing from a flat config with an ambient const", async () => {
    for (const [name, text] of [
      ["eslint.config.ts", "declare const __filename = \"log\";\nexport default [{ rules: { \"no-console\": [\"error\", { allow: [__filename] }] } }];"],
      ["eslint.config.mts", "export declare const allowed = \"log\";\nexport default [{ rules: { \"no-console\": [\"error\", { allow: [allowed] }] } }];"],
    ] as const) {
      const proposed = await flat(text, JS, name);
      expect(proposed.settings).toEqual(eslintPortsOff(JS));
      expect(proposed.notes, text).toEqual([expect.stringMatching(/^eslint\.config\.m?ts is not statically readable: it runs code Rerail does not read as data \(line 1: .+\)\. Rerail never runs your config\. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off\.$/u)]);
    }
  });

  // Final review: something of the file's own the reader cannot follow makes the whole file not statically readable,
  // never a partial read; only a value from an imported module is named as not read and read around.
  it("starts the ESLint ports off for a config element of the file's own it cannot follow, and reads a literal computed key", async () => {
    expect((await flat("export default [{ [\"rules\"]: { \"no-console\": \"off\" } }, { rules: { eqeqeq: [\"error\", \"always\"] } }];", JS)).settings)
      .toContainEqual(expect.objectContaining({ ruleId: CONSOLE, on: false, folder: "" }));
    const local = await flat("const strict = process.env.CI === \"true\";\nexport default [strict ? { rules: { \"no-console\": \"off\" } } : {}, { rules: { eqeqeq: [\"error\", \"always\"] } }];", JS);
    expect(local.settings).toEqual(eslintPortsOff(JS));
    expect(local.notes).toEqual([expect.stringMatching(/^eslint\.config\.ts is not statically readable: Rerail cannot read strict \? .+ as data\. /u)]);
    // A choice between imported configs, or one and nothing, is named as not read; entries after it are read.
    const imported = await flat("import tseslint from \"typescript-eslint\";\nconst check = process.env.CI === \"true\";\nexport default [...(check ? tseslint.configs.strict : []), { rules: { \"no-console\": \"off\" } }];", JS);
    expect(imported.settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  it("lets an entry for the same files that sets the rule in full replace an unread one", async () => {
    const text = "import shared from \"./shared.js\";\nexport default [{ files: [\"tests/**\"], rules: { \"no-console\": shared.level } }, { files: [\"tests/**\"], rules: { \"no-console\": [\"error\", {}] } }];";
    expect((await flat(text, JS)).settings).toEqual([]);
  });

  it("does not trust a config helper the file changes", async () => {
    const text = "import tseslint from \"typescript-eslint\";\ntseslint.config = () => [{ rules: { \"no-console\": \"error\" } }];\nexport default tseslint.config({ rules: { \"no-console\": \"off\" } });";
    expect((await flat(text, JS)).settings).toEqual(eslintPortsOff(JS));
  });

  it("keeps options a later unread element may have set unknown through a severity alone, off included", async () => {
    const shared = "import shared from \"./shared.js\";\n";
    const start = `${shared}export default [{ rules: { "no-console": ["error", { allow: ["log"] }] } }, shared, { rules: { "no-console": "off" } }`;
    expect((await flat(`${start}, { rules: { "no-console": "error" } }];`, JS)).settings).toEqual([]);
    // An off after it is still an off.
    expect((await flat(`${start}];`, JS)).settings).toMatchObject([{ ruleId: CONSOLE, on: false }]);
  });

  it("lets an entry for every file after one for some files drop it, or replace its options, as ESLint merges them", async () => {
    expect(await flat("export default [{ files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { rules: { \"no-console\": \"warn\" } }];", JS))
      .toEqual({ settings: [], notes: [] });
    expect(await flat("export default [{ files: [\"scripts/**\"], rules: { \"no-console\": \"off\" } }, { rules: { \"no-console\": [\"error\", { allow: [\"log\"] }] } }];", JS)).toEqual({
      settings: [{ ruleId: CONSOLE, folder: "", on: null, config: { allow: ["log"] }, explicit: true, because: "eslint.config.ts: \"no-console\": [\"error\",{\"allow\":[\"log\"]}]" }],
      notes: [],
    });
  });

  it("turns a rule off for every file and on in the folder a later entry turns it back on in", async () => {
    expect(await flat("export default [{ rules: { \"no-console\": \"off\" } }, { files: [\"scripts/**\"], rules: { \"no-console\": \"error\" } }];", JS)).toEqual({
      settings: [
        { ruleId: CONSOLE, folder: "", on: false, config: null, explicit: true, because: "eslint.config.ts: \"no-console\": \"off\"" },
        { ruleId: CONSOLE, folder: "scripts", on: true, config: null, explicit: true, because: "eslint.config.ts: \"no-console\": \"error\" for [\"scripts/**\"]" },
      ],
      notes: [],
    });
  });

  it("does not weigh options for some files that a later unread element may replace against those for every file", async () => {
    const text = [
      "import shared from \"shared-config\";",
      "export default [",
      "  { rules: { \"no-console\": [\"error\", { allow: [\"log\"] }] } },",
      "  { files: [\"tests/**\"], rules: { \"no-console\": [\"error\", { allow: [\"warn\"] }] } },",
      "  shared,",
      "  { rules: { \"no-console\": \"error\" } },",
      "];",
    ].join("\n");
    // The entry for tests/ is not used, so it is no "entry for some files that sets other options".
    expect(await flat(text, JS)).toEqual({
      settings: [],
      notes: [
        "eslint.config.ts: the rules of shared are not read: Rerail does not run code to find them.",
        `${CONSOLE}: the options of eslint.config.ts: "no-console": ["error",{"allow":["log"]}] are not used: shared, before it, is not read and may have set them.`,
        `${CONSOLE}: eslint.config.ts: "no-console": ["error",{"allow":["warn"]}] for ["tests/**"] is not used: shared, after it, is not read and may replace it.`,
      ],
    });
  });

  it("notes no entry of a shared config it does not use, nor a shared config's off for some files", async () => {
    const shadowed = "import js from \"@eslint/js\";\nimport shared from \"shared-config\";\nexport default [js.configs.recommended, shared];";
    expect(await flat(shadowed, ["js-more"])).toEqual({ settings: [], notes: ["eslint.config.ts: the rules of shared are not read: Rerail does not run code to find them."] });
    const forTypeScript = "import tseslint from \"typescript-eslint\";\nexport default [...tseslint.configs.recommended];";
    expect(await flat(forTypeScript, ["js-dupe"])).toEqual({
      settings: [{ ruleId: on("eslint-no-dupe-class-members"), folder: "", on: false, config: null, explicit: false, because: "eslint.config.ts does not turn it on" }],
      notes: [],
    });
  });

  it("cuts a long entry short where a note names it", async () => {
    const text = "export default [{ files: [\"**/*.test.ts\"], rules: { \"no-console\": [\"off\", { allow: [\"log\", \"warn\", \"error\", \"info\", \"debug\", \"trace\"] }] } }];";
    expect((await flat(text, JS)).notes).toEqual([
      `${CONSOLE}: eslint.config.ts: "no-console": ["off",{"allow":["log","warn","error","info","debug","trace… for ["**/*.test.ts"] turns it off only for some files; it stays on.`,
    ]);
  });

  // PR #50 review: a const named twice, a choice with the same const on both sides, or an array spread twice doubles
  // what the reader walks at each level, so 40 levels would take it 2^40 visits, and a spread of over about 100,000
  // items overflowed the stack.
  describe("a config that doubles itself through consts", () => {
    /** `const <name>0 = <first>;`, then `levels` consts, each made of the one before by `step`. */
    const doubling = (name: string, first: string, step: (previous: string) => string, levels: number): string => [
      `const ${name}0 = ${first};`,
      ...Array.from({ length: levels }, (_, index) => `const ${name}${index + 1} = ${step(`${name}${index}`)};`),
    ].join("\n");

    /** What set-up proposes for `text` as eslint.config.js, and what it proposes for a file it stops reading for `why`. */
    async function readTooLarge(text: string, why: string) {
      return {
        proposed: await flat(text, JS, "eslint.config.js"),
        refused: {
          settings: eslintPortsOff(JS),
          notes: [`eslint.config.js is not statically readable: ${why}. Rerail never runs your config. The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.`],
        },
      };
    }

    it("stops reading an array that names the one before it twice", async () => {
      const text = `${doubling("a", "{ rules: { \"no-console\": \"off\" } }", a => `[${a}, ${a}]`, 40)}\nexport default a40;`;
      const { proposed, refused } = await readTooLarge(text, "it takes more than 100000 steps to read");
      expect(proposed).toEqual(refused);
    });

    it("stops reading a choice between an imported config and itself", async () => {
      const text = `import shared from "shared-config";\n${doubling("c", "shared", c => `undefined ? ${c} : ${c}`, 40)}\nexport default [c40, { rules: { "no-console": "off" } }];`;
      const { proposed, refused } = await readTooLarge(text, "it takes more than 100000 steps to read");
      expect(proposed).toEqual(refused);
    });

    it("stops reading an array that spreads the one before it twice", async () => {
      const text = `${doubling("s", "[\"src/**\"]", s => `[...${s}, ...${s}]`, 20)}\nexport default [{ files: s20, rules: { "no-console": "off" } }];`;
      const { proposed, refused } = await readTooLarge(text, "it builds more than 100000 values");
      expect(proposed).toEqual(refused);
    });
  });
});

/** A setting the config's own entry, or its whole read, gives for every file. */
const whole = (ruleId: string, state: boolean, explicit: boolean, because: string) => ({ ruleId, folder: "", on: state, config: null, explicit, because });

describe("the same rules on, the same rules off: ESLint", () => {
  it("turns on a port that starts off for an entry that turns its rule on", async () => {
    const root = await repository({ ".eslintrc.json": "{ \"rules\": { \"no-console\": \"error\" } }" });
    expect(settingsOf(readRepositoryLintConfig(root, ruleCatalog, ["javascript"]), "eslint-no-console"))
      .toEqual([whole("eslint-no-console", true, true, ".eslintrc.json: \"no-console\": \"error\"")]);
  });

  it("turns off a port its config never turns on, when the whole config is read", async () => {
    const root = await repository({ "eslint.config.js": "export default [{ rules: { \"no-debugger\": \"warn\" } }];\n" });
    expect(readRepositoryLintConfig(root, ruleCatalog, ["js-more"])).toEqual({
      settings: [whole(on("eslint-eqeqeq"), false, false, "eslint.config.js does not turn it on")],
      notes: [],
    });
  });

  it("leaves the pack's values where a shared config it does not recognise may set the rule", async () => {
    const flatConfig = await repository({ "eslint.config.js": "import airbnb from \"eslint-config-airbnb\";\nexport default [airbnb, { rules: { eqeqeq: \"off\" } }];\n" });
    // An entry after the shared config still decides.
    expect(readRepositoryLintConfig(flatConfig, ruleCatalog, ["js-more"])).toEqual({
      settings: [whole(on("eslint-eqeqeq"), false, true, "eslint.config.js: \"eqeqeq\": \"off\"")],
      notes: ["eslint.config.js: the rules of airbnb are not read: Rerail does not run code to find them."],
    });
    const eslintrc = await repository({ ".eslintrc.json": "{ \"extends\": [\"airbnb\"] }" });
    expect(readRepositoryLintConfig(eslintrc, ruleCatalog, ["js-more"])).toEqual({ settings: [], notes: [".eslintrc.json: only its own rules are read, not extends."] });
  });

  // @eslint/js at ESLint 10.11.0 turns no-debugger on and leaves eqeqeq off.
  it("reads @eslint/js's recommended config: what it turns on is on, the rest off", async () => {
    for (const [name, text] of [
      ["eslint.config.js", "import js from \"@eslint/js\";\nexport default [js.configs.recommended];\n"],
      ["eslint.config.cjs", "const js = require(\"@eslint/js\");\nmodule.exports = [{ rules: { ...js.configs.recommended.rules } }];\n"],
      [".eslintrc.json", "{ \"extends\": \"eslint:recommended\" }"],
    ] as const) {
      const root = await repository({ [name]: text });
      const preset = name === ".eslintrc.json" ? "eslint:recommended" : text.includes(".rules") ? "js.configs.recommended.rules" : "js.configs.recommended";
      expect(readRepositoryLintConfig(root, ruleCatalog, ["js-more", "js-off"]), name).toEqual({
        settings: [
          whole(on("eslint-eqeqeq"), false, false, `${name} does not turn it on`),
          whole("off-eslint-no-debugger", true, false, `${name}: ${preset}`),
        ],
        notes: [],
      });
    }
  });

  // typescript-eslint 8.70.1's recommended config turns no-namespace on and leaves default-param-last off.
  it("reads typescript-eslint's recommended config, flat or eslintrc", async () => {
    for (const [name, text, preset] of [
      ["eslint.config.mjs", "import tseslint from \"typescript-eslint\";\nexport default tseslint.config(...tseslint.configs.recommended);\n", "tseslint.configs.recommended"],
      ["eslint.config.mjs", "import { configs } from \"typescript-eslint\";\nexport default [...configs.recommended];\n", "configs.recommended"],
      [".eslintrc.json", "{ \"extends\": [\"plugin:@typescript-eslint/recommended\"] }", "plugin:@typescript-eslint/recommended"],
    ] as const) {
      const root = await repository({ [name]: text });
      const proposed = readRepositoryLintConfig(root, ruleCatalog, [...TS, "ts-off"]);
      expect(settingsOf(proposed, "off-typescript-eslint-no-namespace"), text).toEqual([whole("off-typescript-eslint-no-namespace", true, false, `${name}: ${preset}`)]);
      expect(settingsOf(proposed, on("typescript-eslint-default-param-last")), text)
        .toEqual([whole(on("typescript-eslint-default-param-last"), false, false, `${name} does not turn it on`)]);
      expect(proposed.notes, text).toEqual([]);
    }
  });

  it("lets an entry after a shared config turn its rule off, and runs a shared config spread into a folder's object there", async () => {
    const off = await repository({ "eslint.config.js": "import js from \"@eslint/js\";\nexport default [js.configs.recommended, { rules: { \"no-debugger\": \"off\" } }];\n" });
    expect(settingsOf(readRepositoryLintConfig(off, ruleCatalog, ["js-more"]), on("eslint-no-debugger")))
      .toEqual([whole(on("eslint-no-debugger"), false, true, "eslint.config.js: \"no-debugger\": \"off\"")]);
    const folder = await repository({ "eslint.config.js": "import reactHooks from \"eslint-plugin-react-hooks\";\nexport default [{ files: [\"src/**\"], ...reactHooks.configs.flat.recommended }];\n" });
    expect(readRepositoryLintConfig(folder, ruleCatalog, ["hooks"]).settings).toEqual([
      whole(HOOKS_RULE, false, false, "eslint.config.js does not turn it on"),
      { ruleId: HOOKS_RULE, folder: "src", on: true, config: null, explicit: true, because: "eslint.config.js: reactHooks.configs.flat.recommended for [\"src/**\"]" },
    ]);
  });

  it("turns a port on in a folder an entry for that folder's files turns it on in", async () => {
    const root = await repository({ "eslint.config.js": "export default [{ files: [\"tests/**\"], rules: { \"no-console\": \"error\" } }];\n" });
    expect(readRepositoryLintConfig(root, ruleCatalog, ["javascript"]).settings.filter(setting => setting.ruleId === "eslint-no-console")).toEqual([
      { ruleId: "eslint-no-console", folder: "tests", on: true, config: null, explicit: true, because: "eslint.config.js: \"no-console\": \"error\" for [\"tests/**\"]" },
    ]);
  });
});

describe("the same rules on, the same rules off: oxlint", () => {
  /** The js-more pack's settings for an `.oxlintrc.json` holding `config`. */
  async function oxlint(config: Readonly<Record<string, unknown>>) {
    const root = await repository({ ".oxlintrc.json": JSON.stringify(config) });
    return readRepositoryLintConfig(root, ruleCatalog, ["js-more"]);
  }

  // oxlint 1.88.0 with an empty .oxlintrc.json runs no-debugger (correctness), not eqeqeq (pedantic).
  it("runs the correctness category by default, and what categories turn on", async () => {
    expect((await oxlint({})).settings).toEqual([whole(on("eslint-eqeqeq"), false, false, ".oxlintrc.json does not turn it on")]);
    expect((await oxlint({ categories: { pedantic: "warn", correctness: "off" } })).settings).toEqual([
      whole(on("eslint-no-debugger"), false, true, ".oxlintrc.json: categories.correctness: \"off\""),
    ]);
  });

  it("leaves the pack's values where its extends or overrides, which it does not read, may set the rule", async () => {
    expect((await oxlint({ extends: ["./base.json"] })).settings).toEqual([]);
    expect((await oxlint({ overrides: [{ files: ["tests/**"], rules: { eqeqeq: "error" } }] })).settings).toEqual([]);
  });
});

describe("the same rules on, the same rules off: Ruff", () => {
  // Ruff 0.16.9's default selection runs E722 and none of E711, E721, E742, E743.
  it("replays Ruff's default selection under a config that sets no selector", async () => {
    const root = await repository({ "pyproject.toml": "[tool.ruff]\nline-length = 100\n\n[tool.ruff.lint]\nignore = [\"E501\"]\n" });
    const notDefault = "pyproject.toml does not select it, nor does Ruff 0.16.9's default rule selection";
    expect(readRepositoryLintConfig(root, ruleCatalog, PY)).toEqual({
      settings: [E7[0], E7[1], E7[3], E7[4]].map(ruleId => whole(ruleId!, false, false, notDefault)),
      notes: [],
    });
  });

  it("turns off what a select replaces Ruff's default with and leaves out", async () => {
    const root = await repository({ "ruff.toml": "[lint]\nselect = [\"F\"]\n" });
    expect(settingsOf(readRepositoryLintConfig(root, ruleCatalog, PY), E7[2]!)).toEqual([whole(E7[2]!, false, false, "ruff.toml: lint.select does not select it")]);
  });

  it("leaves the pack's values where a config it extends and does not read may select the rule", async () => {
    const root = await repository({ "ruff.toml": "extend = \"/etc/ruff.toml\"\n[lint]\nextend-select = [\"E711\"]\n" });
    expect(readRepositoryLintConfig(root, ruleCatalog, PY).settings).toEqual([]);
  });

  /** The ruff-preview pack's settings and notes for a ruff.toml holding `lines`. */
  async function preview(...lines: string[]) {
    const root = await repository({ "ruff.toml": lines.join("\n") });
    return readRepositoryLintConfig(root, ruleCatalog, ["ruff-preview"]);
  }

  it("runs preview rules, and preview's default selection, only with preview on", async () => {
    expect(await preview("[lint]", "ignore = [\"E501\"]")).toEqual({ settings: [], notes: [] });
    expect(await preview("preview = true")).toEqual({
      settings: ["ruff-air003", "ruff-air001"].map(ruleId => whole(ruleId, true, false, "ruff.toml: Ruff 0.16.9's default rule selection with preview")),
      notes: [],
    });
    // The linter's own preview wins over the top level's.
    expect((await preview("preview = true", "[lint]", "preview = false")).settings).toEqual([]);
  });

  it("runs a preview rule a config selects by its code only with preview on, and by a prefix only without explicit-preview-rules", async () => {
    expect(await preview("[lint]", "extend-select = [\"AIR321\"]")).toEqual({
      settings: [],
      notes: ["ruff-air321: ruff.toml: \"AIR321\" in lint.extend-select selects a preview rule, which Ruff runs only with preview = true; it is not on."],
    });
    expect(settingsOf(await preview("[lint]", "preview = true", "extend-select = [\"AIR3\"]"), "ruff-air321"))
      .toEqual([whole("ruff-air321", true, true, "ruff.toml: \"AIR3\" in lint.extend-select")]);
    expect(settingsOf(await preview("[lint]", "preview = true", "explicit-preview-rules = true", "extend-select = [\"AIR3\"]"), "ruff-air321")).toEqual([]);
    expect(settingsOf(await preview("[lint]", "preview = true", "explicit-preview-rules = true", "extend-select = [\"AIR321\"]"), "ruff-air321"))
      .toEqual([whole("ruff-air321", true, true, "ruff.toml: \"AIR321\" in lint.extend-select")]);
  });

  it("leaves a rule preview decides unknown where a config Rerail does not read may turn preview on", async () => {
    const root = await repository({ "ruff.toml": "extend = \"/etc/ruff.toml\"\n[lint]\nselect = [\"AIR\"]\n" });
    const proposed = readRepositoryLintConfig(root, ruleCatalog, ["ruff-preview"]);
    // AIR001 is stable, selected either way; AIR003 is a preview rule.
    expect(proposed.settings).toEqual([whole("ruff-air001", true, true, "ruff.toml: \"AIR\" in lint.select")]);
  });
});
