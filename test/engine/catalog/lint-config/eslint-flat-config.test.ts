import { describe, expect, it } from "vitest";
import { presetScope, readEslintFlatConfig, type FlatConfigEntry, type FlatConfigScope } from "../../../../src/engine/catalog/lint-config/eslint-flat-config.js";
import { readFormat } from "../../../../src/engine/catalog/lint-config/formats/lint-config-formats.js";

const ALL: FlatConfigScope = { kind: "all" };
const TS_FILES = "[\"**/*.ts\",\"**/*.tsx\",\"**/*.mts\",\"**/*.cts\"]";

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
      .toEqual([read("no-console", "off", { kind: "folders", folders: ["src"], files: "[\"src/**\"]" })]);
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
      },
    });
  });

  it("sets no rule for an object of only a name and ignores", () => {
    expect(entriesOf("export default [{ name: \"ignored\", ignores: [\"dist/**\"] }];")).toEqual([]);
  });
});

describe("readEslintFlatConfig: where an entry applies", () => {
  const some = (files: string): FlatConfigScope => ({ kind: "some", files });
  it.each([
    ["a folder glob with ./", "files: [\"./src/**\"]", { kind: "folders", folders: ["src"], files: "[\"./src/**\"]" }],
    ["a folder's files glob", "files: [\"src/**/*\", \"lib/**\"]", { kind: "folders", folders: ["src", "lib"], files: "[\"src/**/*\", \"lib/**\"]" }],
    ["a folder outside the repository", "files: [\"../x/**\"]", some("[\"../x/**\"]")],
    ["a folder through a dot segment", "files: [\"src/./x/**\"]", some("[\"src/./x/**\"]")],
    ["a pattern that is not a folder's", "files: [\"src/*.ts\"]", some("[\"src/*.ts\"]")],
    ["a nested pattern list", "files: [\"src/**\", [\"lib/**\"]]", some("[\"src/**\", [\"lib/**\"]]")],
    ["no patterns", "files: []", some("[]")],
    ["a string", "files: \"src/**\"", some("\"src/**\"")],
    ["patterns it cannot read", "files: shared.files", some("shared.files")],
    ["ignores and no files", "ignores: [\"dist/**\"]", some("some files")],
    ["a base path", "basePath: \"packages/a\", files: [\"src/**\"]", some("[\"src/**\"]")],
  ])("reads %s", (_, properties, scope) => {
    const text = `import shared from "shared-config";\nexport default [{ ${properties}, rules: { "no-console": "off" } }];`;
    expect(entriesOf(text)).toEqual([read("no-console", "off", scope as FlatConfigScope)]);
  });

  it("reads an entry an extends of an object for some files holds as for some files, which it does not map", () => {
    const text = "export default [{ files: [\"src/**\"], extends: [{ files: [\"src/a/**\"], rules: { \"no-console\": \"off\" } }] }];";
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
      scopes: [JSON.stringify({ kind: "some", files: TS_FILES }), JSON.stringify(ALL)],
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
      scopes: [JSON.stringify({ kind: "some", files: TS_FILES })],
      value: { ok: true, value: "off" },
    });
  });

  it("reads a shared config for its own files in an extends of an object for some files as for both", () => {
    const text = "import tseslint from \"typescript-eslint\";\nexport default [{ files: [\"src/**\"], extends: [tseslint.configs.eslintRecommended] }];";
    expect(summary(entriesOf(text, "eslint.config.mts"), "getter-return").scopes)
      .toEqual([JSON.stringify({ kind: "some", files: `["src/**"] and ${TS_FILES}` })]);
  });
});

describe("presetScope", () => {
  const folders: FlatConfigScope = { kind: "folders", folders: ["app"], files: "[\"app/**\"]" };
  it.each([
    ["an entry for every file keeps the element's scope", { rules: {} }, folders, folders],
    ["an entry's folder files are its folders", { files: ["src/**"], rules: {} }, ALL, { kind: "folders", folders: ["src"], files: "[\"src/**\"]" }],
    ["an entry's ignores alone narrow it to some files", { ignores: ["dist/**"], rules: {} }, ALL, { kind: "some", files: "some files" }],
    ["an entry's ignores keep its folder files from mapping onto folders", { files: ["src/**"], ignores: ["src/gen/**"], rules: {} }, ALL, { kind: "some", files: "[\"src/**\"]" }],
    ["an entry's ignores in an element for some files narrow both", { ignores: ["dist/**"], rules: {} }, folders, { kind: "some", files: "[\"app/**\"] and some files" }],
  ])("%s", (_, entry, scope, expected) => {
    expect(presetScope(entry, scope)).toEqual(expected);
  });
});
