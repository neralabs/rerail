/**
 * The ESLint recorders' shared helpers (conformance/recorders/): finding ends in recordings, and the source facts
 * (`source.json`) they read from an ESLint tool's checkout and installed packages. The recorders run offline against
 * pinned upstreams; these tests run the helpers over small trees written here, with this repository's TypeScript.
 */
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import ts from "typescript";
import { afterEach, describe, expect, it } from "vitest";
import { codePointColumn, compareFindings, findingOf, withReportedEnds, writeRecording } from "../conformance/recorders/eslint-record.mjs";
import {
  caseWords,
  checkoutFiles,
  commentVocabulary,
  configNamed,
  declaredVersion,
  deprecationText,
  directiveValuesFrom,
  firstApproved,
  helperPackage,
  implementationPath,
  isOff,
  keywords,
  literalNames,
  literalRegexes,
  localDirectory,
  moduleGraph,
  packageName,
  regexWords,
  resolveModule,
  ruleDocumentation,
  sourceWords,
} from "../conformance/recorders/eslint-source.mjs";

const temporary = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});
/** A new temporary directory, by its real path (module resolution reports real paths). */
const scratch = prefix => {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  temporary.push(directory);
  return directory;
};
const write = (path, text) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
};
/** A package under `root/node_modules`, from its files by relative path. */
const writePackage = (root, name, files) => {
  const directory = join(root, "node_modules", ...name.split("/"));
  for (const [path, text] of Object.entries({ "package.json": JSON.stringify({ name, main: "index.js" }), ...files })) write(join(directory, path), text);
  return directory;
};
/** The module graph with nothing as the checkout: every module resolves from where it is, as Node resolves it. */
const graph = root => moduleGraph(ts, join(root, "no-checkout"), createRequire(join(root, "package.json")));

describe("recordings", () => {
  it("a finding carries ESLint's end, its column in code points and exclusive, as its start does", () => {
    // "𝒳" is two UTF-16 code units and one code point.
    const text = "const 𝒳 = 1;\nlet a𝒳b = 2;\n";
    const message = { ruleId: "r", line: 2, column: 5, endLine: 2, endColumn: 9 };
    expect(findingOf(message, text, "f.js")).toEqual({ rule: "r", path: "f.js", line: 2, column: 5, endLine: 2, endColumn: 8 });
    expect(codePointColumn(text, 1, 10)).toBe(9);
    // A rule that reports a start location only has no end.
    expect(findingOf({ ruleId: "r", line: 1, column: 7 }, text, "f.js")).toEqual({ rule: "r", path: "f.js", line: 1, column: 7 });
  });

  it("a rule with any finding without an end keeps only starts; other rules keep their ends", () => {
    const findings = [
      { rule: "a", path: "x.js", line: 1, column: 1, endLine: 1, endColumn: 4 },
      { rule: "a", path: "y.js", line: 2, column: 3 },
      { rule: "b", path: "x.js", line: 1, column: 1, endLine: 2, endColumn: 1 },
    ];
    expect(withReportedEnds(findings)).toEqual([
      { rule: "a", path: "x.js", line: 1, column: 1 },
      { rule: "a", path: "y.js", line: 2, column: 3 },
      { rule: "b", path: "x.js", line: 1, column: 1, endLine: 2, endColumn: 1 },
    ]);
  });

  it("findings are written by rule, path, start and end, with a one-space indent", () => {
    const root = scratch("eslint-recording-");
    const findings = [
      { rule: "b", path: "a.js", line: 1, column: 1, endLine: 1, endColumn: 2 },
      { rule: "a", path: "b.js", line: 1, column: 1, endLine: 1, endColumn: 5 },
      { rule: "a", path: "b.js", line: 1, column: 1, endLine: 1, endColumn: 3 },
      { rule: "a", path: "a.js", line: 2, column: 1, endLine: 2, endColumn: 2 },
    ];
    expect([...findings].sort(compareFindings).map(finding => `${finding.rule} ${finding.path} ${finding.line} ${finding.endColumn}`)).toEqual(["a a.js 2 2", "a b.js 1 3", "a b.js 1 5", "b a.js 1 2"]);
    const path = join(root, "tool", "fixtures.expected.json");
    writeRecording(path, "tool", "1.0.0", { kind: "fixtures", origin: "https://example.com/tool@0", rules: ["a", "b"], files: ["a.js", "b.js"] }, findings);
    const text = readFileSync(path, "utf8");
    expect(text.startsWith('{\n "tool": "tool",\n "version": "1.0.0",')).toBe(true);
    expect(JSON.parse(text).findings.map(finding => finding.endColumn)).toEqual([2, 3, 5, 2]);
  });
});

describe("configurations and rule metadata", () => {
  it("a rule takes the options of the first approved configuration that turns it on", () => {
    const approved = [
      { name: "recommended", configs: [{ rules: { a: "error", b: "off" } }, { rules: { c: ["warn", { x: 1 }] } }] },
      { name: "strict", configs: [{ rules: { b: ["error", "always"], a: ["error", "never"] } }] },
    ];
    expect(firstApproved(approved, "a")).toEqual({ configuration: "recommended", options: [] });
    expect(firstApproved(approved, "b")).toEqual({ configuration: "strict", options: ["always"] });
    expect(firstApproved(approved, "c")).toEqual({ configuration: "recommended", options: [{ x: 1 }] });
    expect(firstApproved(approved, "d")).toBeNull();
    // Later entries of one configuration override earlier ones, as ESLint merges a flat config array.
    expect(firstApproved([{ name: "r", configs: [{ rules: { a: "error" } }, { rules: { a: 0 } }] }], "a")).toBeNull();
    expect(isOff(["off", 1])).toBe(true);
    expect(isOff(2)).toBe(false);
  });

  it("a configuration name reads one key, or a dotted path into nested configurations", () => {
    const recommended = { rules: { "react/jsx-key": 2 } };
    const configs = { recommended: { plugins: ["react"] }, "flat/recommended": recommended, flat: { recommended, "jsx-runtime": {} } };
    expect(configNamed(configs, "flat/recommended")).toBe(recommended);
    expect(configNamed(configs, "flat.recommended")).toBe(recommended);
    expect(configNamed(configs, "flat.jsx-runtime")).toEqual({});
    expect(configNamed(configs, "recommended")).toEqual({ plugins: ["react"] });
    expect(configNamed(configs, "flat.all")).toBeUndefined();
    expect(configNamed(configs, "recommended.plugins.0.x")).toBeUndefined();
  });

  it("deprecation notices read ESLint's DeprecatedInfo and plain flags", () => {
    expect(deprecationText(undefined)).toBeNull();
    expect(deprecationText({ deprecated: false })).toBeNull();
    expect(deprecationText({ deprecated: true })).toBe("deprecated");
    expect(
      deprecationText({ deprecated: { message: "Formatting rules are being moved out of ESLint core.", deprecatedSince: "8.53.0", replacedBy: [{ rule: { name: "semi" } }] } }),
    ).toBe("Formatting rules are being moved out of ESLint core (since 8.53.0; replaced by semi)");
  });

  it("a rule's implementation path takes its name, or its name in PascalCase", () => {
    expect(implementationPath("lib/rules/{rule}.js", "jsx-key")).toBe("lib/rules/jsx-key.js");
    expect(implementationPath("packages/eslint-plugin-react-hooks/src/rules/{Rule}.ts", "rules-of-hooks")).toBe("packages/eslint-plugin-react-hooks/src/rules/RulesOfHooks.ts");
    expect(implementationPath("lib/rules/{rule}.js", "prefer-global/buffer")).toBe("lib/rules/prefer-global/buffer.js");
  });

  it("a version file declares a release's version as the string it maps the package's name to", () => {
    // facebook/react's ReactVersions.js at eslint-plugin-react-hooks@7.1.1, abridged.
    const versions = "const ReactVersion = '19.3.0';\nconst stablePackages = {\n  'eslint-plugin-react-hooks': '7.1.1',\n  react: ReactVersion,\n  scheduler: '0.28.0',\n};\n";
    expect(declaredVersion(ts, versions, "eslint-plugin-react-hooks")).toBe("7.1.1");
    expect(declaredVersion(ts, versions, "scheduler")).toBe("0.28.0");
    // A name mapped to anything but a string literal, or not mapped at all, declares no version.
    expect(declaredVersion(ts, versions, "react")).toBeUndefined();
    expect(declaredVersion(ts, versions, "eslint-plugin-react")).toBeUndefined();
  });
});

describe("words", () => {
  it("a rule checks the words of its whitespace-free strings, its messages and defaults, not its prose or the rest of its meta block", () => {
    const root = scratch("eslint-names-");
    const file = join(root, "no-script-url.js");
    write(
      file,
      [
        "module.exports = {",
        "  meta: { messages: { unexpectedScriptURL: 'hidden' }, docs: { url: 'https://eslint.org/docs/rules/no-script-url' }, defaultOptions: [{ terms: ['todo'] }] },",
        "  create(context) {",
        "    if (value.toLowerCase().indexOf('javascript:') === 0) context.report('Script URL is a form of eval.');",
        "    if (node.type === 'TemplateLiteral' || directive === 'eslint-disable-line') return `${name}.prototype`;",
        "    report(\"Use 'strictEqual' here\");",
        "    tracker.iterateGlobalReferences({ Math: { pow: { [CALL]: true } } });",
        "    return /^(?:foo|barBaz)$/u.test(name);",
        "  },",
        "};",
      ].join("\n"),
    );
    const isMeta = ruleDocumentation(ts);
    const names = literalNames(ts, [file], isMeta);
    for (const word of ["javascript", "TemplateLiteral", "eslint", "disable", "line", "prototype", "strictEqual", "foo", "barBaz"]) expect(names.has(word), word).toBe(true);
    // Prose with spaces names nothing but its quoted words, and the rest of the meta block is documentation.
    for (const word of ["Script", "URL", "form", "eval", "Use", "here", "https", "no"]) expect(names.has(word), word).toBe(false);
    // Where every prose string sits in the documentation block, every word of the strings left counts.
    const every = literalNames(ts, [file], isMeta, { everyStringWord: true });
    for (const word of ["javascript", "Script", "URL", "form", "eval", "Use", "here", "strictEqual"]) expect(every.has(word), word).toBe(true);
    for (const word of ["https", "no"]) expect(every.has(word), word).toBe(false);
    // The messages and defaults in meta are what the rule reports and runs with, not documentation.
    expect(names.has("hidden") && every.has("hidden") && names.has("todo")).toBe(true);
    // Object keys count only when asked for: the lookup tables a rule checks names against.
    expect(every.has("terms") || every.has("Math")).toBe(false);
    const keyed = literalNames(ts, [file], isMeta, { objectKeys: true });
    for (const word of ["terms", "Math", "pow", "create"]) expect(keyed.has(word), word).toBe(true);
    for (const word of ["docs", "url", "CALL", "toLowerCase"]) expect(keyed.has(word), word).toBe(false);
    expect(literalRegexes(ts, [file], isMeta)).toEqual([{ source: "^(?:foo|barBaz)$", flags: "u" }]);
  });

  it("a regex names the words it spells, not its escapes, classes or group syntax", () => {
    expect(regexWords("^(?<name>foo)\\bbar[a-z]+\\u0041(?:baz)\\\\n$")).toEqual(["foo", "bar", "baz"]);
  });

  it("source words are identifiers, keywords and the words in strings and comments, not numbers", () => {
    expect(sourceWords(ts, "const #x = 'a b'; // c-d\n100n + 0x1F;", false)).toEqual(["const", "x", "a", "b", "c", "d"]);
    expect(sourceWords(ts, "<A b='c'>text</A>", true)).toEqual(["A", "b", "c", "text", "A"]);
  });

  it("the language's keywords and comment vocabulary come from the TypeScript compiler", () => {
    const found = keywords(ts);
    for (const word of ["const", "typeof", "satisfies", "readonly"]) expect(found.has(word), word).toBe(true);
    const vocabulary = commentVocabulary(ts, readFileSync(createRequire(import.meta.url).resolve("typescript"), "utf8"));
    for (const word of ["param", "returns", "template", "reference", "path", "amd"]) expect(vocabulary.has(word), word).toBe(true);
    expect(() => commentVocabulary(ts, "")).toThrow(/JSDoc tag names/u);
  });

  it("ESLint's directive values are its global settings and its severities, read from its sources", () => {
    const sourceCode = [
      "function normalizeConfigGlobal(configuredValue) {",
      "  switch (configuredValue) {",
      '    case "off": return "off";',
      '    case true: case "true": case "writeable": case "writable": return "writable";',
      '    case null: case false: case "false": case "readable": case "readonly": return "readonly";',
      "    default: throw new Error(`'${configuredValue}' is not a valid configuration for a global`);",
      "  }",
      "}",
      'function other(value) { switch (value) { case "elsewhere": return 1; } }',
    ].join("\n");
    const severity = '"use strict";\nfunction normalize(s) { if ([2, "2", "error"].includes(s)) return "error"; if ([1, "1", "warn"].includes(s)) return "warn"; if ([0, "0", "off"].includes(s)) return "off"; }\n';
    expect(directiveValuesFrom(ts, sourceCode, severity)).toEqual(["error", "false", "off", "readable", "readonly", "true", "warn", "writable", "writeable"]);
    expect(() => directiveValuesFrom(ts, "function renamed() {}", severity)).toThrow(/normalizeConfigGlobal/u);
  });

  it("a case's test words are its code and expected outputs as code, and its configuration and expected reports as words", () => {
    const words = caseWords({
      code: "a()",
      output: ["b()", "c()"],
      filename: "file.tsx",
      options: [{ allow: ["zz"] }],
      languageOptions: { globals: { G1: "readonly" } },
      errors: [{ message: "Bad foo.", data: { name: "fooName", list: ["x1"] }, suggestions: [{ desc: "Use bar", output: "bar()", data: { n: "q" } }] }, "Legacy message", 3],
    });
    expect(words).toEqual({
      code: "a()",
      outputs: ["b()", "c()", "bar()"],
      jsx: true,
      configuration: JSON.stringify([[{ allow: ["zz"] }], null, ["G1"], ["Bad foo.", "fooName", "x1", "Legacy message", "Use bar", "q"]]),
    });
    expect(caseWords("x;")).toEqual({ code: "x;", outputs: [], jsx: false, configuration: JSON.stringify([null, null, [], []]) });
    // A null output (no fix) and a count of errors add nothing.
    expect(caseWords({ code: "y", output: null, errors: 2 }).outputs).toEqual([]);
  });

  it("a case's configuration names paths under upstream's checkout checkout-relative; the recording machine's directories are found", () => {
    const checkout = "/machine/tmp/upstream-n";
    const real = "/private/machine/tmp/upstream-n";
    const found = caseWords(
      { code: "x", options: [{ resolvePaths: [`${real}/tests`], name: `${checkout}/tests/lib/rules/foo` }], settings: { root: real }, errors: [{ message: `'${real}/foo' is restricted.` }] },
      [checkout, real],
    );
    // The real path is the longer root, so it is read first and not as the shorter one inside it.
    expect(found.configuration).toBe(JSON.stringify([[{ resolvePaths: ["tests"], name: "tests/lib/rules/foo" }], { root: "." }, [], ["'foo' is restricted."]]));
    expect(localDirectory(found, [checkout, real, "/machine/tools"])).toBeUndefined();
    expect(localDirectory(caseWords({ code: 'require("/machine/tools/x")' }), ["/machine/tools"])).toBe("/machine/tools");
    expect(localDirectory(caseWords({ code: "x", options: ["/machine/tools/node_modules/y"] }, [checkout]), ["/machine/tools"])).toBe("/machine/tools");
  });

  it("test fixtures are the checkout files a glob matches, checkout-relative, in path order", () => {
    const root = scratch("eslint-fixtures-");
    for (const path of ["packages/p/tests/fixtures/file.ts", "packages/p/tests/fixtures/nested/react.tsx", "packages/p/tests/fixtures/tsconfig.json", "packages/p/tests/rules/a.test.ts"]) write(join(root, path), "");
    expect(checkoutFiles(root, ["packages/p/tests/fixtures/**"])).toEqual(["packages/p/tests/fixtures/file.ts", "packages/p/tests/fixtures/nested/react.tsx", "packages/p/tests/fixtures/tsconfig.json"]);
    expect(checkoutFiles(root, ["packages/p/tests/rules/*.test.ts"])).toEqual(["packages/p/tests/rules/a.test.ts"]);
    expect(checkoutFiles(root, [])).toEqual([]);
  });
});

describe("the modules a rule delegates to", () => {
  it("a rule delegates to what it imports by relative path, transitively, through barrels to the defining modules, and not to other rules", () => {
    const root = scratch("eslint-imports-");
    write(join(root, "util", "index.ts"), "import { x } from 'external';\nexport * from './names';\nexport * from './unused';\nexport { renamed as alias } from './renamed';\nexport const { fromObject } = x;\n");
    write(join(root, "util", "names.ts"), "export function isName(): boolean { return 'name' === 'name'; }\nexport type Kind = 'a';\n");
    write(join(root, "util", "unused.ts"), "export const unused = 'unused';\n");
    write(join(root, "util", "renamed.ts"), "export const renamed = 1;\n");
    write(join(root, "rules", "helpers", "index.ts"), "const table = require('../../conf/table');\nconst rules = require('../other-rule');\nexport const helper = 'helper';\n");
    write(join(root, "conf", "table.js"), "module.exports = { Array: false };\n");
    write(join(root, "rules", "other-rule.ts"), "export default {};\n");
    write(
      join(root, "rules", "rule.ts"),
      [
        "import { isName, alias, fromObject } from '../util';",
        "import type { Kind } from '../util';",
        "import * as helpers from './helpers';",
        "import other from './other-rule';",
        "import external from 'external';",
        "const shared = require('./helpers/index');",
        "helpers.helper;",
      ].join("\n"),
    );
    const { definingModule, importedModules } = graph(root);
    expect(resolveModule(join(root, "rules"), "../util")).toBe(join(root, "util", "index.ts"));
    expect(definingModule(join(root, "util", "index.ts"), "isName")).toBe(join(root, "util", "names.ts"));
    expect(definingModule(join(root, "util", "index.ts"), "alias")).toBe(join(root, "util", "renamed.ts"));
    expect(definingModule(join(root, "util", "index.ts"), "fromObject")).toBe(join(root, "util", "index.ts"));
    expect(definingModule(join(root, "util", "index.ts"), "missing")).toBeUndefined();
    // What a helper imports in turn is followed too, but never into another rule; no package is a helper here.
    expect(importedModules(join(root, "rules", "rule.ts"))).toEqual({
      own: [join(root, "conf", "table.js"), join(root, "rules", "helpers", "index.ts"), join(root, "util", "index.ts"), join(root, "util", "names.ts"), join(root, "util", "renamed.ts")],
      helpers: [],
    });
  });

  it("a bare specifier names its package, scoped or not, without the subpath", () => {
    expect(packageName("@typescript-eslint/utils/ast-utils")).toBe("@typescript-eslint/utils");
    expect(packageName("@eslint-community/eslint-utils")).toBe("@eslint-community/eslint-utils");
    expect(packageName("esutils/lib/keyword")).toBe("esutils");
  });

  it("a CommonJS build defines a name where it assigns it, and re-exports it through getters, __exportStar and namespaces", () => {
    const root = scratch("eslint-cjs-");
    const helper = writePackage(root, "@scope/helper", {
      // TypeScript's CommonJS output: a declaring preamble, then re-exports.
      "index.js": [
        "exports.Tracker = exports.Utils = exports.analyze = void 0;",
        'var analyze_1 = require("./analyze");',
        'Object.defineProperty(exports, "analyze", { enumerable: true, get: function () { return analyze_1.analyze; } });',
        '__exportStar(require("./tracker"), exports);',
        'exports.Utils = __importStar(require("./utils"));',
        'var other_1 = require("other");',
        'Object.defineProperty(exports, "External", { enumerable: true, get: function () { return other_1.External; } });',
      ].join("\n"),
      "analyze.js": "exports.analyze = analyze;\nconst DEFAULTS = { jsxPragma: 'React' };\nfunction analyze() {}\n",
      "tracker.js": 'var Kinds;\n(function (Kinds) { Kinds["A"] = "A"; })(Kinds || (exports.Kinds = Kinds = {}));\nexports.Tracker = class {};\n',
      "utils/index.js": "exports.helper = 1;\n",
    });
    writePackage(root, "other", { "index.js": "exports.External = 1;\n" });
    const helpers = new Set(["@scope/helper"]);
    const entry = join(helper, "index.js");
    const { definingModule } = graph(root);
    expect(definingModule(entry, "analyze", helpers)).toBe(join(helper, "analyze.js"));
    expect(definingModule(entry, "Tracker", helpers)).toBe(join(helper, "tracker.js"));
    // An enum's `exports.Kinds = Kinds = {}` defines it.
    expect(definingModule(entry, "Kinds", helpers)).toBe(join(helper, "tracker.js"));
    // A namespace export is the module it exports.
    expect(definingModule(entry, "Utils", helpers)).toBe(join(helper, "utils", "index.js"));
    // Re-exports are followed into helper packages only.
    expect(definingModule(entry, "External", helpers)).toBeUndefined();
    expect(definingModule(entry, "External", new Set([...helpers, "other"]))).toBe(join(root, "node_modules", "other", "index.js"));
    expect(definingModule(entry, "missing", helpers)).toBeUndefined();
  });

  it("a default export is defined where it is declared, by the module whose import it exports, or by a JSON module's data", () => {
    const root = scratch("eslint-default-");
    const at = file => join(root, "utils", file);
    write(at("index.js"), "export {default as assertToken} from './assert-token.js';\nexport {default as list} from './list-entry.js';\n");
    write(at("assert-token.js"), "export default function assertToken() {}\n");
    write(at("list-entry.js"), "import list from './list.json' with {type: 'json'};\nexport default list;\n");
    write(at("list.json"), '["fs", "node:fs"]\n');
    write(at("config.js"), "const config = {create() {}};\nexport default config;\n");
    write(at("common.cjs"), "module.exports = function common() {};\n");
    write(at("named.js"), "export const named = 1;\n");
    const { definingModule } = graph(root);
    expect(definingModule(at("index.js"), "assertToken")).toBe(at("assert-token.js"));
    expect(definingModule(at("index.js"), "list")).toBe(at("list.json"));
    expect(definingModule(at("list-entry.js"), "default")).toBe(at("list.json"));
    expect(definingModule(at("config.js"), "default")).toBe(at("config.js"));
    // A CommonJS module's default export is what it assigns to module.exports.
    expect(definingModule(at("common.cjs"), "default")).toBe(at("common.cjs"));
    expect(definingModule(at("named.js"), "missing")).toBeUndefined();
  });

  it("a rule delegates to the modules of helper packages that define the names it imports, and not to their own package's internals", () => {
    const root = scratch("eslint-helpers-");
    const utils = writePackage(root, "@eslint-community/eslint-utils", {
      "index.js":
        "const globalObjectNames = ['global', 'globalThis', 'self', 'window'];\nclass ReferenceTracker {}\nconst CALL = Symbol('call');\nrequire('./internal');\nexports.ReferenceTracker = ReferenceTracker;\nexports.CALL = CALL;\nexports.getStaticValue = getStaticValue;\nfunction getStaticValue() {}\n",
      "internal.js": "exports.internal = 'internal';\n",
    });
    const scope = writePackage(root, "@scope/scope-manager", {
      "index.js": 'var analyze_1 = require("./analyze");\nObject.defineProperty(exports, "analyze", { enumerable: true, get: function () { return analyze_1.analyze; } });\n',
      "analyze.js": "exports.analyze = analyze;\nfunction analyze() { return { jsxPragma: 'React' }; }\n",
    });
    writePackage(root, "espree", { "index.js": "exports.tokenize = function () {};\n" });
    write(join(root, "lib", "rules", "utils", "ast-utils.js"), "const espree = require('espree');\nconst { getStaticValue } = require('@eslint-community/eslint-utils');\nmodule.exports = {};\n");
    write(join(root, "lib", "rules", "no-obj-calls.js"), ["const { CALL, ReferenceTracker } = require('@eslint-community/eslint-utils');", "const astUtils = require('./utils/ast-utils');", "module.exports = {};"].join("\n"));
    // TypeScript's CommonJS output reads named imports from the binding that holds the module.
    write(join(root, "lib", "parser.js"), 'const scope_manager_1 = require("@scope/scope-manager");\nconst x = (0, scope_manager_1.analyze)(ast);\n');
    write(join(root, "lib", "rules", "uses-analyze.ts"), "import { analyze } from '@scope/scope-manager';\nimport type { Scope } from '@scope/scope-manager';\nexport default analyze;\n");
    const helpers = new Set(["@eslint-community/eslint-utils", "@scope/scope-manager"]);
    const { importedModules } = graph(root);
    // A helper's module counts once however it is reached; espree is not a helper; internal.js is the helper's own.
    expect(importedModules(join(root, "lib", "rules", "no-obj-calls.js"), helpers)).toEqual({ own: [join(root, "lib", "rules", "utils", "ast-utils.js")], helpers: [join(utils, "index.js")] });
    expect(importedModules(join(root, "lib", "parser.js"), helpers, null).helpers).toEqual([join(scope, "analyze.js")]);
    expect(importedModules(join(root, "lib", "rules", "uses-analyze.ts"), helpers).helpers).toEqual([join(scope, "analyze.js")]);
    // Without helpers, packages are not read at all.
    expect(importedModules(join(root, "lib", "rules", "no-obj-calls.js")).helpers).toEqual([]);
  });

  it("a helper's imports of another helper package are followed to the data they define, not its own package's", () => {
    const root = scratch("eslint-helper-chain-");
    const isBuiltin = writePackage(root, "is-builtin-module", {
      "index.js": "import builtinModules from 'builtin-modules';\nimport {cache} from './cache.js';\nexport default function isBuiltinModule(name) { return new Set(builtinModules).has(name); }\n",
      "cache.js": "export const cache = 'internal';\n",
    });
    const builtins = writePackage(root, "builtin-modules", {
      "index.js": "import builtinModules from './builtin-modules.json' with {type: 'json'};\nexport default builtinModules;\n",
      "builtin-modules.json": '["node:fs", "fs", "child_process"]\n',
    });
    const rule = join(root, "rules", "prefer-node-protocol.js");
    write(rule, "import isBuiltinModule from 'is-builtin-module';\nexport default {create() { return isBuiltinModule('fs'); }};\n");
    const { importedModules } = graph(root);
    expect(importedModules(rule, new Set(["is-builtin-module", "builtin-modules"]))).toEqual({ own: [], helpers: [join(builtins, "builtin-modules.json"), join(isBuiltin, "index.js")].sort() });
    // A package that is not a helper is not followed from a helper either.
    expect(importedModules(rule, new Set(["is-builtin-module"])).helpers).toEqual([join(isBuiltin, "index.js")]);
  });

  it("a helper's re-exports of another helper package's code do not bring in that package's bundle", () => {
    const root = scratch("eslint-helper-bundle-");
    // A single-file bundle, as @eslint-community/eslint-utils ships: one module defines every name.
    const bundle = writePackage(root, "bundled-utils", {
      "index.js": "function isArrowToken(token) { return token.value === '=>'; }\nexports.isArrowToken = isArrowToken;\n",
    });
    // @typescript-eslint/utils's ast-utils/eslint-utils/predicates.js: a namespace of the bundle, re-exported by name.
    const commonjs = writePackage(root, "@scope/utils", { "index.js": 'const eslintUtils = __importStar(require("bundled-utils"));\nexports.isArrowToken = eslintUtils.isArrowToken;\n' });
    const esm = writePackage(root, "esm-helper", { "index.js": "import { isArrowToken } from 'bundled-utils';\nexport function isArrow(token) { return isArrowToken(token); }\n" });
    const helpers = new Set(["bundled-utils", "@scope/utils", "esm-helper"]);
    const rule = join(root, "rules", "uses-helpers.js");
    write(rule, "import { isArrowToken } from '@scope/utils';\nimport { isArrow } from 'esm-helper';\nexport default {create() { return isArrowToken(isArrow); }};\n");
    const { importedModules } = graph(root);
    expect(importedModules(rule, helpers)).toEqual({ own: [], helpers: [join(commonjs, "index.js"), join(esm, "index.js")].sort() });
    // The rule's own import of the bundle still counts it, through the module that defines the name.
    const direct = join(root, "rules", "uses-bundle.js");
    write(direct, "import { isArrowToken } from 'bundled-utils';\nexport default {create() { return isArrowToken; }};\n");
    expect(importedModules(direct, helpers).helpers).toEqual([join(bundle, "index.js")]);
  });

  it("a checkout file's helper imports resolve from the tools directory, which the checkout has no link to", () => {
    const root = scratch("eslint-checkout-");
    const tools = join(root, "tools");
    const utils = writePackage(tools, "@eslint-community/eslint-utils", { "index.js": "exports.ReferenceTracker = class {};\n" });
    const checkout = join(root, "checkout");
    const rule = join(checkout, "lib", "rules", "no-obj-calls.js");
    write(rule, "const { ReferenceTracker } = require('@eslint-community/eslint-utils');\nmodule.exports = {};\n");
    const helpers = new Set(["@eslint-community/eslint-utils"]);
    expect(moduleGraph(ts, checkout, createRequire(join(tools, "package.json"))).importedModules(rule, helpers)).toEqual({ own: [], helpers: [join(utils, "index.js")] });
    expect(graph(root).importedModules(rule, helpers).helpers).toEqual([]);
  });

  it("a module belongs to the innermost helper package on its path", () => {
    const nested = "/x/node_modules/is-builtin-module/node_modules/builtin-modules/index.js";
    expect(helperPackage(nested, new Set(["is-builtin-module", "builtin-modules"]))).toBe("/x/node_modules/is-builtin-module/node_modules/builtin-modules/");
    expect(helperPackage(nested, new Set(["is-builtin-module"]))).toBe("/x/node_modules/is-builtin-module/");
    expect(helperPackage("/x/node_modules/@scope/utils/dist/a.js", new Set(["@scope/utils"]))).toBe("/x/node_modules/@scope/utils/");
    expect(helperPackage("/x/lib/rules/a.js", new Set(["@scope/utils"]))).toBeUndefined();
  });
});
