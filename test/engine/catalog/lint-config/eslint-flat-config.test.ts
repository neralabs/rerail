import { describe, expect, it } from "vitest";
import { presetScope, readEslintFlatConfig, type FilePatterns, type FlatConfigEntry, type FlatConfigScope } from "../../../../src/engine/catalog/lint-config/eslint-flat-config.js";
import { readFormat } from "../../../../src/engine/catalog/lint-config/formats/lint-config-formats.js";

const ALL: FlatConfigScope = { kind: "all" };
const TS_GLOBS = ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"];
const TS_FILES = JSON.stringify(TS_GLOBS);
/** The files `files` less `ignores` match, as one group of globs. */
const globs = (files: string[], ignores: string[] = []): FilePatterns => ({ files: [files], ignores });

/** What the reader makes of `text` as file `name`: what it reads, or why it refuses the file. */
const outcome = (text: string, name = "eslint.config.js") => readFormat(() => readEslintFlatConfig(text, name));

/** Why the reader refuses `text` as file `name`. */
function refusal(text: string, name = "eslint.config.js"): string | undefined {
  const read = outcome(text, name);
  return read.ok ? undefined : read.why;
}

/** The entries the reader reads from `text` as file `name`; undefined where it refuses the file. */
function entriesOf(text: string, name = "eslint.config.js"): readonly FlatConfigEntry[] | undefined {
  const read = outcome(text, name);
  return read.ok ? read.value.entries : undefined;
}

const read = (key: string, value: unknown, scope: FlatConfigScope = ALL): FlatConfigEntry => ({ key, scope, value: { ok: true, value } });
const runsCode = (line: number, source: string): string => `it runs code Rerail does not read as data (line ${line}: ${source})`;
const NO_CONSOLE_OFF = "export default [{ rules: { \"no-console\": \"off\" } }];";

describe("readEslintFlatConfig: the exported config", () => {
  it.each([
    ["an export default of a function", "export default function config() {}", "eslint.config.mjs"],
    ["an export default of a class", "export default class Config {}", "eslint.config.mjs"],
    ["no export", "const rules = {};", "eslint.config.js"],
  ])("refuses a file with %s as exporting no config", (_, text, name) => {
    expect(refusal(text, name)).toBe("it exports no config Rerail can find (export default or module.exports)");
  });

  it("refuses a file that exports its config both as a module and through module.exports", () => {
    expect(refusal("export default [];\nmodule.exports = [];", "eslint.config.mjs")).toBe("it exports its config both as a module and through module.exports");
  });

  it("refuses a file that does not parse, with the parser's reason", () => {
    expect(refusal("export default [", "eslint.config.mjs")).toMatch(/^it does not parse: Unexpected token \(1:16\)$/u);
  });

  it("takes the last export, refusing a function an earlier export default declares", () => {
    expect(refusal(`export default function config() {}\nexport = ${NO_CONSOLE_OFF.slice("export default ".length)}`, "eslint.config.ts"))
      .toBe(runsCode(1, "function config() {}"));
  });

  it("reads re-exports from packages, type declarations and ambient namespaces as data", () => {
    const text = [
      "export * from \"shared-config\";",
      "export * as shared from \"shared-config\";",
      "export type Level = \"off\";",
      "export interface Options { readonly allow: string[] }",
      "declare namespace NodeJS { interface ProcessEnv { CI?: string } }",
      NO_CONSOLE_OFF,
    ].join("\n");
    expect(entriesOf(text, "eslint.config.ts")).toEqual([read("no-console", "off")]);
  });
});

describe("readEslintFlatConfig: code is not read", () => {
  it.each([
    ["a let", "let level = \"off\";\nexport default [];", "eslint.config.mjs", runsCode(1, "let level = \"off\";")],
    ["a function value", "const make = () => [];\nexport default [];", "eslint.config.mjs", runsCode(1, "() => []")],
    ["a delete", "const gone = delete globalThis.x;\nexport default [];", "eslint.config.mjs", runsCode(1, "delete globalThis.x")],
    ["an exported function", "export function helper() {}\nexport default [];", "eslint.config.mjs", runsCode(1, "function helper() {}")],
    ["a destructured export", "export const { allow } = { allow: [] };\nexport default [];", "eslint.config.mjs", runsCode(1, "{ allow } = { allow: [] }")],
    ["a `then` exported under a string name", "const forEach = [].forEach;\nexport { forEach as \"then\" };\nexport default [];", "eslint.config.mjs", runsCode(2, "export { forEach as \"then\" };")],
    ["a code-running module's binding, uncalled", "import { runInThisContext } from \"node:vm\";\nexport default [{ settings: { run: runInThisContext } }];", "eslint.config.mjs", runsCode(2, "runInThisContext")],
    ["a write to a member of module.exports", "export default [];\nmodule.exports.extra = [];", "eslint.config.mjs", runsCode(2, "module.exports.extra = [];")],
    ["a compound assignment to module.exports", "module.exports = [];\nmodule.exports ||= [];", "eslint.config.cjs", runsCode(2, "module.exports ||= [];")],
    ["a write to module through a computed key", "module.exports = [];\nmodule[\"exports\"] = [];", "eslint.config.cjs", runsCode(2, "module[\"exports\"] = [];")],
  ])("refuses %s", (_, text, name, why) => {
    expect(refusal(text, name)).toBe(why);
  });

  it("cuts a long piece of code short where it names it", () => {
    expect(refusal("const make = () => ({ rules: { \"no-console\": \"off\", eqeqeq: \"error\" } });\nexport default [];", "eslint.config.mjs"))
      .toBe(runsCode(1, "() => ({ rules: { \"no-console\": \"off\", eqeqeq: …"));
  });

  it("reads holes, signs, typeof and TypeScript's type wrappers as data", () => {
    const text = [
      "import merge from \"merge\";",
      "const holes = [, \"a\"];",
      "const negative = -1;",
      "const kind = typeof negative;",
      "const instantiated = merge<string>;",
      "const asserted = merge!;",
      "export default [, { settings: { holes, kind, instantiated, asserted } }, { rules: { \"no-console\": \"off\" } }];",
    ].join("\n");
    expect(entriesOf(text, "eslint.config.ts")).toEqual([read("no-console", "off")]);
  });
});

describe("readEslintFlatConfig: rule values", () => {
  const shared = "import shared from \"./shared.js\";";
  it.each([
    ["null", "null", { ok: true, value: null }],
    ["a template without substitutions", "`off`", { ok: true, value: "off" }],
    ["a template with a substitution", "`${\"off\"}`", { ok: false, why: "it is `${\"off\"}`, which Rerail does not run" }],
    ["a negative number", "-1", { ok: true, value: -1 }],
    ["a positive number", "+2", { ok: true, value: 2 }],
    ["a negation", "!0", { ok: false, why: "it is !0, which Rerail does not run" }],
    ["a negated string", "-\"1\"", { ok: false, why: "it is -\"1\", which Rerail does not run" }],
    ["undefined", "undefined", { ok: false, why: "it is undefined, which Rerail does not run" }],
    ["an array with a hole", "[\"error\", , \"x\"]", { ok: false, why: "it is [\"error\", , \"x\"], which Rerail does not run" }],
    ["a spread const array", "[\"error\", ...list]", { ok: true, value: ["error", "log"] }],
    ["a spread imported array", "[\"error\", ...shared.list]", { ok: false, why: "it is shared.list, which Rerail does not run" }],
    ["a spread string", "[\"error\", ...\"ab\"]", { ok: false, why: "it is [\"error\", ...\"ab\"], which Rerail does not run" }],
    ["a spread const object, a later key replacing its own", "[\"error\", { ...options, allow: [\"log\"] }]", { ok: true, value: ["error", { allow: ["log"], hoist: "all" }] }],
    ["a spread imported object", "[\"error\", { ...shared.options }]", { ok: false, why: "it is shared.options, which Rerail does not run" }],
    ["a spread null", "[\"error\", { ...null }]", { ok: false, why: "it is { ...null }, which Rerail does not run" }],
    ["a spread array in an object", "[\"error\", { ...list }]", { ok: false, why: "it is { ...list }, which Rerail does not run" }],
    ["a spread string in an object", "[\"error\", { ...\"ab\" }]", { ok: false, why: "it is { ...\"ab\" }, which Rerail does not run" }],
    ["an imported option", "[\"error\", { allow: shared.allow }]", { ok: false, why: "it is shared.allow, which Rerail does not run" }],
    ["a value nested deeper than Rerail reads", `["error", ${"[".repeat(70)}${"]".repeat(70)}]`, { ok: false, why: "it nests too deep" }],
  ])("reads %s", (_, value, expected) => {
    const text = `${shared}\nconst list = ["log"];\nconst options = { allow: ["warn"], hoist: "all" };\nexport default [{ rules: { "no-console": ${value} } }];`;
    expect(entriesOf(text)).toEqual([{ key: "no-console", scope: ALL, value: expected }]);
  });

  it("reads through TypeScript's type wrappers and const bindings", () => {
    const text = [
      "const level = \"off\" as const;",
      "const options = { allow: [\"log\"] } satisfies object;",
      "export default [{ rules: { \"no-console\": level, eqeqeq: <const>\"error\", \"no-alert\": [\"warn\", options!] } }];",
    ].join("\n");
    expect(entriesOf(text, "eslint.config.ts")).toEqual([read("no-console", "off"), read("eqeqeq", "error"), read("no-alert", ["warn", { allow: ["log"] }])]);
  });

  it("reads a literal computed key as JavaScript names it, a number as its string", () => {
    expect(entriesOf("export default [{ rules: { [\"no-console\"]: \"off\", [1]: \"warn\", 2: \"error\" } }];"))
      .toEqual([read("no-console", "off"), read("1", "warn"), read("2", "error")]);
  });
});

describe("readEslintFlatConfig: config elements", () => {
  it("skips the holes of the exported array", () => {
    expect(entriesOf(`export default [, ${NO_CONSOLE_OFF.slice("export default [".length)}`)).toEqual([read("no-console", "off")]);
  });

  it("refuses arrays nested deeper than it reads", () => {
    expect(refusal(`export default ${"[".repeat(66)}{ rules: {} }${"]".repeat(66)};`)).toBe("it nests deeper than Rerail reads");
  });

  it("refuses rules spread into each other deeper than it reads", () => {
    expect(refusal(`export default [{ rules: ${"{ ...".repeat(70)}{}${" }".repeat(70)} }];`)).toBe("it nests deeper than Rerail reads");
  });

  it("refuses a chain of consts longer than it follows", () => {
    const chain = Array.from({ length: 66 }, (_, index) => `const c${index + 1} = c${index};`);
    expect(refusal(["const c0 = [{ rules: { \"no-console\": \"off\" } }];", ...chain, "export default c66;"].join("\n"))).toBe("Rerail cannot read c66 as data");
  });

  it("reads an object spread into a config object, and refuses a value of its own it cannot spread there", () => {
    expect(entriesOf("const base = { files: [\"src/**\"] };\nexport default [{ ...base, ...{ name: \"strict\" }, rules: { \"no-console\": \"off\" } }];"))
      .toEqual([read("no-console", "off", { kind: "folders", folders: ["src"], files: "[\"src/**\"]", patterns: globs(["src/**"]) })]);
    expect(refusal("const list = [\"a\"];\nexport default [{ ...list, rules: {} }];")).toBe("Rerail cannot read list as data");
  });

  it("names a config object an unrecognised imported config is spread into as not read, and what it may replace", () => {
    const text = "import shared from \"shared-config\";\nexport default [{ rules: { \"no-console\": \"off\" } }, { ...shared, rules: { eqeqeq: \"error\" } }];";
    expect(outcome(text)).toEqual({
      ok: true,
      value: {
        entries: [{ ...read("no-console", "off"), shadowedBy: "{ ...shared, rules: { eqeqeq: \"error\" } }" }],
        unread: ["{ ...shared, rules: { eqeqeq: \"error\" } }"],
        notes: ["the rules of { ...shared, rules: { eqeqeq: \"error\" } } are not read: Rerail does not run code to find them."],
        namespaces: new Map(),
      },
    });
  });

  it("sets no rule for an object of only a name and ignores", () => {
    expect(entriesOf("export default [{ name: \"ignored\", ignores: [\"dist/**\"] }];")).toEqual([]);
  });
});

describe("readEslintFlatConfig: where an entry applies", () => {
  const some = (files: string, patterns?: FilePatterns): FlatConfigScope => ({ kind: "some", files, ...(patterns === undefined ? {} : { patterns }) });
  it.each([
    ["a folder glob with ./", "files: [\"./src/**\"]", { kind: "folders", folders: ["src"], files: "[\"./src/**\"]", patterns: globs(["./src/**"]) }],
    ["a folder's files glob", "files: [\"src/**/*\", \"lib/**\"]", { kind: "folders", folders: ["src", "lib"], files: "[\"src/**/*\", \"lib/**\"]", patterns: globs(["src/**/*", "lib/**"]) }],
    ["a folder outside the repository", "files: [\"../x/**\"]", some("[\"../x/**\"]", globs(["../x/**"]))],
    ["a folder through a dot segment", "files: [\"src/./x/**\"]", some("[\"src/./x/**\"]", globs(["src/./x/**"]))],
    ["a pattern that is not a folder's", "files: [\"src/*.ts\"]", some("[\"src/*.ts\"]", globs(["src/*.ts"]))],
    ["a nested pattern list", "files: [\"src/**\", [\"lib/**\"]]", some("[\"src/**\", [\"lib/**\"]]")],
    ["a negated pattern", "files: [\"src/**\", \"!src/gen/**\"]", some("[\"src/**\", \"!src/gen/**\"]")],
    ["no patterns", "files: []", some("[]", globs([]))],
    ["a string", "files: \"src/**\"", some("\"src/**\"")],
    ["patterns it cannot read", "files: shared.files", some("shared.files")],
    ["ignores and no files", "ignores: [\"dist/**\"]", some("some files", globs(["**/*"], ["dist/**"]))],
    ["ignores it cannot read", "files: [\"src/**\"], ignores: shared.ignores", some("[\"src/**\"]")],
    ["a base path", "basePath: \"packages/a\", files: [\"src/**\"]", some("[\"src/**\"]")],
  ])("reads %s", (_, properties, scope) => {
    const text = `import shared from "shared-config";\nexport default [{ ${properties}, rules: { "no-console": "off" } }];`;
    expect(entriesOf(text)).toEqual([read("no-console", "off", scope as FlatConfigScope)]);
  });

  it("reads an entry an extends of an object for some files holds as for the files both hold, which it does not map onto folders", () => {
    const text = "export default [{ files: [\"src/**\"], extends: [{ files: [\"src/a/**\"], rules: { \"no-console\": \"off\" } }] }];";
    expect(entriesOf(text)).toEqual([read("no-console", "off", some("[\"src/a/**\"]", { files: [["src/**"], ["src/a/**"]], ignores: [] }))]);
  });

  it("reads an entry an extends of an object holds for files it cannot read as for some files", () => {
    const text = "export default [{ files: [\"src/**\", [\"lib/**\"]], extends: [{ files: [\"src/a/**\"], rules: { \"no-console\": \"off\" } }] }];";
    expect(entriesOf(text)).toEqual([read("no-console", "off", some("[\"src/a/**\"]"))]);
  });
});

describe("readEslintFlatConfig: shared configs", () => {
  /** The keys, scopes and presets of `entries`, with the value of `key`. */
  const summary = (entries: readonly FlatConfigEntry[] | undefined, key: string) => ({
    count: entries?.length,
    presets: [...new Set(entries?.map(entry => entry.preset))],
    scopes: [...new Set(entries?.map(entry => JSON.stringify(entry.scope)))],
    value: entries?.find(entry => entry.key === key)?.value,
  });

  it.each([
    ["a default export named by string", "import { \"default\" as js } from \"@eslint/js\";", "js.configs.recommended"],
    ["a namespace's default", "import * as js from \"@eslint/js\";", "js.default.configs.recommended"],
    ["an optional member path", "import js from \"@eslint/js\";", "js?.configs.recommended"],
  ])("reads @eslint/js's recommended config through %s", (_, imported, path) => {
    expect(summary(entriesOf(`${imported}\nexport default [${path}];`), "no-debugger"))
      .toEqual({ count: 64, presets: [path], scopes: [JSON.stringify(ALL)], value: { ok: true, value: "error" } });
  });

  it("names a namespace, or a member of it outside the module's named exports, as not read", () => {
    const text = "import * as js from \"@eslint/js\";\nexport default [js.configs.recommended, js];";
    expect(outcome(text)).toMatchObject({ ok: true, value: { entries: [], unread: ["js.configs.recommended", "js"] } });
  });

  it("reads typescript-eslint's configs through its namespace and named exports, and names other exports as not read", () => {
    const namespace = "import * as tseslint from \"typescript-eslint\";\nexport default tseslint.config(...tseslint.configs.recommended);";
    expect(summary(entriesOf(namespace, "eslint.config.mts"), "no-var")).toEqual({
      count: 46,
      presets: ["tseslint.configs.recommended"],
      scopes: [JSON.stringify({ kind: "some", files: TS_FILES, patterns: globs(TS_GLOBS) }), JSON.stringify(ALL)],
      value: { ok: true, value: "error" },
    });
    const named = "import { configs, parser, plugin } from \"typescript-eslint\";\nimport { other } from \"typescript-eslint\";\nexport default [...configs.recommended, parser, other.configs.recommended];";
    expect(outcome(named, "eslint.config.mts")).toMatchObject({ ok: true, value: { unread: ["parser", "other.configs.recommended"] } });
    expect(entriesOf(named, "eslint.config.mts")).toHaveLength(46);
  });

  it("reads a shared config object spread into a config object, for its own files", () => {
    const text = "import tseslint from \"typescript-eslint\";\nexport default [{ ...tseslint.configs.eslintRecommended }];";
    expect(summary(entriesOf(text, "eslint.config.mts"), "getter-return")).toEqual({
      count: 23,
      presets: ["tseslint.configs.eslintRecommended"],
      scopes: [JSON.stringify({ kind: "some", files: TS_FILES, patterns: globs(TS_GLOBS) })],
      value: { ok: true, value: "off" },
    });
  });

  it("reads a shared config for its own files in an extends of an object for some files as for both", () => {
    const text = "import tseslint from \"typescript-eslint\";\nexport default [{ files: [\"src/**\"], extends: [tseslint.configs.eslintRecommended] }];";
    expect(summary(entriesOf(text, "eslint.config.mts"), "getter-return").scopes)
      .toEqual([JSON.stringify({ kind: "some", files: `["src/**"] and ${TS_FILES}`, patterns: { files: [["src/**"], TS_GLOBS], ignores: [] } })]);
  });
});

describe("readEslintFlatConfig: plugins and string extends", () => {
  const read = (text: string) => outcome(text, "eslint.config.mjs");
  const keysOf = (text: string): string[] => [...new Set(entriesOf(text, "eslint.config.mjs")?.map(entry => entry.key))];
  const IMPORTS = [
    "import js from \"@eslint/js\";",
    "import tseslint from \"typescript-eslint\";",
    "import react from \"eslint-plugin-react\";",
    "import local from \"./local-plugin.mjs\";",
    "import shared from \"shared-config\";",
    "import { defineConfig } from \"eslint/config\";",
  ].join("\n");

  it("binds each namespace of plugins to the namespace its recognised plugin's rules are named under, and others to none", () => {
    const text = `${IMPORTS}\nexport default [{ plugins: { ts: tseslint.plugin, js, "@typescript-eslint": tseslint.plugin, local, r: react } }, { plugins: { r: react } }];`;
    expect(read(text)).toMatchObject({
      ok: true,
      value: { namespaces: new Map([["ts", "@typescript-eslint"], ["js", "js"], ["@typescript-eslint", "@typescript-eslint"], ["local", undefined], ["r", "react"]]) },
    });
  });

  it("binds a namespace two objects bind to different plugins to none", () => {
    const text = `${IMPORTS}\nexport default [{ plugins: { p: tseslint.plugin } }, { plugins: { p: react } }, { plugins: { q: local } }, { plugins: { q: react } }];`;
    expect(read(text)).toMatchObject({ ok: true, value: { namespaces: new Map([["p", undefined], ["q", undefined]]) } });
  });

  it("binds a namespace to none for a member of a recognised module that is no plugin", () => {
    const text = "import tseslint, { parser } from \"typescript-eslint\";\nimport { other } from \"typescript-eslint\";\nexport default [{ plugins: { a: tseslint.parser, b: parser, c: other, d: { rules: {} }, e: undefined } }];";
    expect(read(text)).toMatchObject({ ok: true, value: { namespaces: new Map([["a", undefined], ["b", undefined], ["c", undefined], ["d", undefined], ["e", undefined]]) } });
  });

  it.each([
    ["plugins it cannot read", "{ plugins: shared.plugins }", "shared.plugins"],
    ["plugins spread from what it cannot read", "{ plugins: { ...shared.plugins, js } }", "...shared.plugins"],
  ])("names %s as what may bind any namespace", (_, element, unread) => {
    expect(read(`${IMPORTS}\nexport default [${element}];`)).toMatchObject({ ok: true, value: { pluginsUnread: unread } });
  });

  it("reads a string extends through the plugin the object binds, as defineConfig finds its config", () => {
    const scaffold = `${IMPORTS}\nexport default defineConfig([{ files: ["**/*.js"], plugins: { js }, extends: ["js/recommended"] }]);`;
    expect(entriesOf(scaffold, "eslint.config.mjs")).toHaveLength(64);
    expect(entriesOf(scaffold, "eslint.config.mjs")?.[0]).toMatchObject({ preset: "\"js/recommended\"", scope: { kind: "some", patterns: globs(["**/*.js"]) } });
  });

  it("renames a string extends' rules to the namespace the object binds a plugin with its own namespace under", () => {
    const renamed = keysOf(`${IMPORTS}\nexport default defineConfig([{ plugins: { ts: tseslint.plugin }, extends: ["ts/recommended"] }]);`);
    expect(renamed).toContain("ts/no-explicit-any");
    expect(renamed).toContain("no-var");
    expect(renamed.some(key => key.startsWith("@typescript-eslint/"))).toBe(false);
    const own = keysOf(`${IMPORTS}\nexport default defineConfig([{ plugins: { "@typescript-eslint": tseslint.plugin }, extends: ["@typescript-eslint/strict"] }]);`);
    expect(own).toContain("@typescript-eslint/no-explicit-any");
  });

  it("reads config objects and arrays in extends as elements", () => {
    expect(keysOf(`${IMPORTS}\nexport default defineConfig([{ extends: [js.configs.recommended, ...[{ rules: { eqeqeq: "error" } }]] }]);`)).toContain("eqeqeq");
    expect(read(`${IMPORTS}\nexport default defineConfig([{ extends: shared.configs }]);`)).toMatchObject({ ok: true, value: { unread: ["shared.configs"] } });
  });

  it.each([
    ["a namespace the object does not bind", "{ extends: [\"js/recommended\"] }", "\"js/recommended\""],
    ["a plugin it does not recognise", "{ plugins: { local }, extends: [\"local/recommended\"] }", "\"local/recommended\""],
    ["a config the plugin does not have", "{ plugins: { js }, extends: [\"js/missing\"] }", "\"js/missing\""],
    ["an eslintrc config with no flat one", "{ plugins: { react }, extends: [\"react/recommended\"] }", "\"react/recommended\""],
    ["a name with no namespace", "{ plugins: { js }, extends: [\"recommended\"] }", "\"recommended\""],
    ["a scoped namespace of two parts", "{ plugins: { \"@ts/x\": tseslint.plugin }, extends: [\"@ts/x/missing\"] }", "\"@ts/x/missing\""],
  ])("names a string extends of %s as not read, and what it may replace", (_, element, unread) => {
    const text = `${IMPORTS}\nexport default defineConfig([{ rules: { eqeqeq: "error" } }, ${element}]);`;
    expect(read(text)).toMatchObject({ ok: true, value: { entries: [{ key: "eqeqeq", shadowedBy: unread }], unread: [unread] } });
  });
});

describe("presetScope", () => {
  const folders: FlatConfigScope = { kind: "folders", folders: ["app"], files: "[\"app/**\"]" };
  it.each([
    ["an entry for every file keeps the element's scope", { rules: {} }, folders, folders],
    ["an entry's folder files are its folders", { files: ["src/**"], rules: {} }, ALL, { kind: "folders", folders: ["src"], files: "[\"src/**\"]", patterns: globs(["src/**"]) }],
    ["an entry's ignores alone narrow it to some files", { ignores: ["dist/**"], rules: {} }, ALL, { kind: "some", files: "some files", patterns: globs(["**/*"], ["dist/**"]) }],
    ["an entry's ignores keep its folder files from mapping onto folders", { files: ["src/**"], ignores: ["src/gen/**"], rules: {} }, ALL, { kind: "some", files: "[\"src/**\"]", patterns: globs(["src/**"], ["src/gen/**"]) }],
    ["an entry's ignores in an element for some files narrow both", { ignores: ["dist/**"], rules: {} }, folders, { kind: "some", files: "[\"app/**\"] and some files" }],
    ["an entry's ignores in an element for files it reads narrow both", { ignores: ["dist/**"], rules: {} }, { ...folders, patterns: globs(["app/**"]) }, { kind: "some", files: "[\"app/**\"] and some files", patterns: { files: [["app/**"], ["**/*"]], ignores: ["dist/**"] } }],
  ])("%s", (_, entry, scope, expected) => {
    expect(presetScope(entry, scope)).toEqual(expected);
  });
});
