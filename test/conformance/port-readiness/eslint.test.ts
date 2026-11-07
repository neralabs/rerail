import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "vitest";
import { isDeepStrictEqual } from "node:util";

import ts from "typescript";

import {
  bareRuleName, declaredVersion, definingModule, directiveValuesFrom, helperPackage, implementationPath, importedModules, inHelperPackage,
  packageName, resolveModule, ruleDocumentation, testFixtureFiles,
} from "../../../conformance/recorders/port-readiness/adapters/eslint/eslint.js";
import { eslintLineStarts, eslintPoint, messageSpan } from "../../../conformance/port-readiness/span-units/eslint.js";
import {
  caseFixture, casePath, caseWords, configNamed, declaresEnvironment, deprecationText, fileType, firstApproved, frozenFileType, isOff, modulePackage, parsedAs, parserLanguage,
  withoutEnvironment, withSetting, writtenExpectation,
} from "../../../conformance/recorders/port-readiness/adapters/eslint/eslint-upstream.js";
import { babelErrors, literalNames, type BabelParser, type TypeScript } from "../../../conformance/recorders/port-readiness/languages/ecmascript.js";

const bytes = (text: string) => Buffer.byteLength(text, "utf8");

test("positions are 0-based lines and UTF-16 columns, as ESLint counts them", () => {
  const source = 'const s = "\u{1F600}"; let x: any;';
  // The astral character is two UTF-16 code units and four UTF-8 bytes: `any` starts at UTF-16 column 23.
  const start = bytes(source.slice(0, source.indexOf("any")));
  assert.deepEqual(eslintPoint(source, start), { line: 0, column: 23 });
  assert.deepEqual(eslintPoint(source, start + 3), { line: 0, column: 26 });
});

test("lines break at CRLF, CR, LF, LS and PS, and CRLF counts once", () => {
  const source = "a\r\nb\rc\nd e f";
  assert.deepEqual(eslintLineStarts(source), [0, 3, 5, 7, 9, 11]);
  for (const [letter, line] of [["b", 1], ["c", 2], ["d", 3], ["e", 4], ["f", 5]] as const) {
    assert.deepEqual(eslintPoint(source, bytes(source.slice(0, source.indexOf(letter)))), { line, column: 0 });
  }
  // A point between CR and LF is still on the line the CRLF ends.
  assert.deepEqual(eslintPoint(source, 2), { line: 0, column: 2 });
});

test("a leading BOM is stripped before positions are counted, as ESLint does", () => {
  const source = "﻿debugger;\nfoo";
  assert.deepEqual(eslintPoint(source, bytes("﻿")), { line: 0, column: 0 });
  assert.deepEqual(eslintPoint(source, bytes("﻿debugger;\n")), { line: 1, column: 0 });
  assert.deepEqual(eslintPoint(source, 0), { line: 0, column: 0 });
});

test("ESLint positions become 0-based spans with an exclusive end, and without ends when the rule is compared on starts", () => {
  assert.deepEqual(messageSpan([1, 10, 1, 18], true), { startLine: 0, startCol: 9, endLine: 0, endCol: 17 });
  assert.deepEqual(messageSpan([3, 1, 4, 2], false), { startLine: 2, startCol: 0, endLine: null, endCol: null });
  assert.deepEqual(messageSpan([3, 1, null, null], true), { startLine: 2, startCol: 0, endLine: null, endCol: null });
});

test("a namespaced rule name and its bare name are the same plugin rule", () => {
  const plugin = { package: "@typescript-eslint/eslint-plugin", namespace: "@typescript-eslint" };
  assert.equal(bareRuleName(plugin, "@typescript-eslint/no-namespace"), "no-namespace");
  assert.equal(bareRuleName(plugin, "no-namespace"), "no-namespace");
  assert.equal(bareRuleName(null, "no-debugger"), "no-debugger");
  assert.equal(bareRuleName(null, "@typescript-eslint/no-namespace"), "@typescript-eslint/no-namespace");
});

test("a rule takes the options of the first approved configuration that turns it on", () => {
  const approved = [
    { name: "recommended", configs: [{ rules: { a: "error" as const, b: "off" as const } }, { rules: { c: ["warn", { x: 1 }] as ["warn", { x: number }] } }] },
    { name: "strict", configs: [{ rules: { b: ["error", "always"] as ["error", string], a: ["error", "never"] as ["error", string] } }] },
  ];
  assert.deepEqual(firstApproved(approved, "a"), { config: "recommended", options: [] });
  assert.deepEqual(firstApproved(approved, "b"), { config: "strict", options: ["always"] });
  assert.deepEqual(firstApproved(approved, "c"), { config: "recommended", options: [{ x: 1 }] });
  assert.equal(firstApproved(approved, "d"), null);
  // Later entries of one configuration override earlier ones, as ESLint merges a flat config array.
  assert.equal(firstApproved([{ name: "r", configs: [{ rules: { a: "error" } }, { rules: { a: 0 } }] }], "a"), null);
  assert.equal(isOff(["off", 1]), true);
  assert.equal(isOff(2), false);
});

test("deprecation notices read ESLint's DeprecatedInfo and plain flags", () => {
  assert.equal(deprecationText(undefined), null);
  assert.equal(deprecationText({ deprecated: false }), null);
  assert.equal(deprecationText({ deprecated: true }), "deprecated");
  assert.equal(
    deprecationText({ deprecated: { message: "Formatting rules are being moved out of ESLint core.", deprecatedSince: "8.53.0", replacedBy: [{ rule: { name: "semi" } }] } }),
    "Formatting rules are being moved out of ESLint core (since 8.53.0; replaced by semi)",
  );
});

test("a case's file type follows the language its parser reads and the options ESLint lints it with", () => {
  assert.equal(fileType({ language: "javascript", jsx: false, sourceType: "module" }), ".js");
  assert.equal(fileType({ language: "javascript", jsx: false, sourceType: "script" }), ".js");
  assert.equal(fileType({ language: "javascript", jsx: false, sourceType: "commonjs" }), ".cjs");
  assert.equal(fileType({ language: "javascript", jsx: true, sourceType: "commonjs" }), ".jsx");
  assert.equal(fileType({ language: "typescript", jsx: false, sourceType: "module" }), ".ts");
  assert.equal(fileType({ language: "typescript", jsx: true, sourceType: "module" }), ".tsx");
});

test("a parser's language is the one its name declares, or the one the manifest gives its package", () => {
  const parsers = { "@typescript-eslint/parser-v5": "typescript", "@babel/eslint-parser": "javascript" } as const;
  assert.equal(parserLanguage("espree", undefined, {}), "javascript");
  assert.equal(parserLanguage("typescript-eslint/parser", "@typescript-eslint/parser", {}), "typescript");
  // A declared name wins over the package map.
  assert.equal(parserLanguage("espree", "@typescript-eslint/parser-v5", parsers), "javascript");
  assert.equal(parserLanguage(undefined, "@typescript-eslint/parser-v5", parsers), "typescript");
  assert.equal(parserLanguage(undefined, "@babel/eslint-parser", parsers), "javascript");
  assert.equal(parserLanguage(undefined, "hermes-eslint", parsers), undefined);
  assert.equal(parserLanguage("hermes-eslint", undefined, parsers), undefined);
  assert.equal(parserLanguage(undefined, undefined, parsers), undefined);
});

test("a module file belongs to the package after its last node_modules, aliases and scopes included", () => {
  assert.equal(modulePackage("/r/node_modules/@typescript-eslint/parser-v5/dist/index.js"), "@typescript-eslint/parser-v5");
  assert.equal(modulePackage("/r/node_modules/a/node_modules/hermes-eslint/dist/index.js"), "hermes-eslint");
  assert.equal(modulePackage("C:\\r\\node_modules\\babel-eslint\\lib\\index.js"), "babel-eslint");
  assert.equal(modulePackage("/r/checkout/tests/fixtures/parsers/stub-parser.js"), undefined);
  assert.equal(modulePackage("/r/node_modules"), undefined);
});

test("a configuration name reads one key, or a dotted path into nested configurations", () => {
  const recommended = { rules: { "react/jsx-key": 2 } };
  const configs = { recommended: { plugins: ["react"] }, "flat/recommended": recommended, flat: { recommended, "jsx-runtime": {} } };
  assert.equal(configNamed(configs, "flat/recommended"), recommended);
  assert.equal(configNamed(configs, "flat.recommended"), recommended);
  assert.deepEqual(configNamed(configs, "flat.jsx-runtime"), {});
  assert.deepEqual(configNamed(configs, "recommended"), { plugins: ["react"] });
  assert.equal(configNamed(configs, "flat.all"), undefined);
  assert.equal(configNamed(configs, "recommended.plugins.0.x"), undefined);
});

test("a case is linted without the globals and settings it or its RuleTester declares, its language kept", () => {
  const parser = { meta: { name: "espree" } };
  const item = { languageOptions: { parser, ecmaVersion: 2018, globals: { window: "readonly" } }, settings: { react: { pragma: "Act" } } };
  assert.deepEqual(withoutEnvironment(item), { languageOptions: { parser, ecmaVersion: 2018 } });
  assert.deepEqual(withoutEnvironment([item, { rules: { a: 1 } }]), [{ languageOptions: { parser, ecmaVersion: 2018 } }, { rules: { a: 1 } }]);
  // An entry that declares neither is the same object.
  const plain = { languageOptions: { sourceType: "module" } };
  assert.equal(withoutEnvironment(plain), plain);
  assert.equal(withoutEnvironment("var a;"), "var a;");
  assert.equal(declaresEnvironment(item), true);
  assert.equal(declaresEnvironment([plain, { settings: { react: {} } }]), true);
  assert.equal(declaresEnvironment([plain, { languageOptions: { globals: {} } }]), true);
  // Empty settings set nothing.
  assert.equal(declaresEnvironment([plain, { settings: {} }]), false);
  assert.equal(declaresEnvironment(null), false);
});

test("a rule's implementation path takes its name, or its name in PascalCase", () => {
  assert.equal(implementationPath("lib/rules/{rule}.js", "jsx-key"), "lib/rules/jsx-key.js");
  assert.equal(implementationPath("packages/eslint-plugin-react-hooks/src/rules/{Rule}.ts", "rules-of-hooks"), "packages/eslint-plugin-react-hooks/src/rules/RulesOfHooks.ts");
  assert.equal(implementationPath("src/{Rule}.ts", "exhaustive-deps"), "src/ExhaustiveDeps.ts");
});

test("a fixture parser that froze another parser's tree for the case's code gives that code's file type; others give none", () => {
  const fixtureParsers = { "tests/fixtures/parsers/typescript-parsers/": ".ts" };
  assert.equal(frozenFileType("tests/fixtures/parsers/typescript-parsers/global-await.js", fixtureParsers), ".ts");
  assert.equal(frozenFileType("tests/fixtures/parsers/typescript-parsers/no-lone-blocks/using.js", fixtureParsers), ".ts");
  assert.equal(frozenFileType("tests/fixtures/parsers/stub-parser.js", fixtureParsers), undefined);
  assert.equal(frozenFileType("tests/fixtures/parsers/typescript-parsers/x.js", {}), undefined);
});

test("upstream's written expectation is compared only where its options are the approved ones and every error is positioned", () => {
  const errors = [{ line: 1, column: 2, endLine: 1, endColumn: 5 }];
  assert.deepEqual(writtenExpectation("invalid", errors, undefined, [], isDeepStrictEqual), [[1, 2, 1, 5]]);
  assert.deepEqual(writtenExpectation("valid", undefined, [], [], isDeepStrictEqual), []);
  assert.equal(writtenExpectation("invalid", errors, ["always"], [], isDeepStrictEqual), null);
  assert.deepEqual(writtenExpectation("invalid", errors, ["always"], ["always"], isDeepStrictEqual), [[1, 2, 1, 5]]);
  assert.equal(writtenExpectation("invalid", [{ line: 1, column: 2 }], undefined, [], isDeepStrictEqual), null);
  assert.equal(writtenExpectation("invalid", 2, undefined, [], isDeepStrictEqual), null);
});

test("every case has its own key, and is evaluated where upstream lints it", () => {
  assert.equal(caseFixture("tests/lib/rules/eqeqeq.js", 0, "invalid", 3), "tests/lib/rules/eqeqeq.js#0/invalid[3]");
  const cache = "/cache";
  const checkout = `${cache}/upstream/next.js/b0fad0d4`;
  // Upstream names no file: a directory of the case's own, with the case's file type.
  assert.equal(casePath("eqeqeq", "tests/lib/rules/eqeqeq.js", 0, "invalid", 3, undefined, checkout, cache, ".js"), "eqeqeq/eqeqeq.0.invalid-3/input.js");
  assert.equal(casePath("no-img-element", "test/unit/eslint-plugin-next/no-img-element.test.ts", 0, "valid", 1, "", checkout, cache, ".jsx"), "no-img-element/no-img-element.0.valid-1/input.jsx");
  // Upstream names one: exactly that path, directories kept, so rules anchored at its start read it as upstream does.
  assert.equal(casePath("no-img-element", "x.test.ts", 0, "valid", 3, "src/app/icon.js", checkout, cache, ".jsx"), "src/app/icon.js");
  assert.equal(casePath("no-head-element", "x.test.ts", 0, "invalid", 0, "./pages/index.js", checkout, cache, ".jsx"), "pages/index.js");
  assert.equal(casePath("r", "x.test.ts", 0, "valid", 0, "foo.d.ts", checkout, cache, ".ts"), "foo.d.ts");
  // An absolute name, relative to the filesystem root; a Windows name, as the same directories.
  assert.equal(casePath("r", "x.test.ts", 0, "valid", 0, "/Users/user_name/projects/project-name/app/layout.tsx", checkout, cache, ".jsx"), "Users/user_name/projects/project-name/app/layout.tsx");
  assert.equal(casePath("r", "x.test.ts", 0, "valid", 4, "C:\\Users\\username\\projects\\project-name\\app\\layout.tsx", checkout, cache, ".jsx"), "Users/username/projects/project-name/app/layout.tsx");
  // A file of the upstream checkout (typescript-eslint's fixtures), relative to the checkout.
  assert.equal(casePath("r", "x.test.ts", 1, "valid", 0, `${checkout}/packages/eslint-plugin/tests/fixtures/react.tsx`, checkout, cache, ".ts"), "packages/eslint-plugin/tests/fixtures/react.tsx");
  // A name built from the directory the tests run in, elsewhere in the cache (above the checkout, or in the runtime):
  // its base name, in the case's own directory, since those directories are this machine's.
  assert.equal(casePath("filename-case", "test/filename-case.js", 0, "invalid", 58, `${cache}/upstream/next.js/Src/fooBar.js`, checkout, cache, ".js"), "filename-case/filename-case.0.invalid-58/fooBar.js");
  assert.equal(casePath("filename-case", "test/filename-case.js", 0, "invalid", 57, `${cache}/runtimes/r/1960ac1d/src/FooBar/file.js`, checkout, cache, ".js"), "filename-case/filename-case.0.invalid-57/file.js");
});

test("a case keeps its name, and is read in the language upstream's parser read it in", () => {
  // JSX that espree reads at pages/index.ts is read as JSX, not TypeScript.
  assert.equal(parsedAs("pages/index.ts", ".jsx"), ".jsx");
  assert.equal(parsedAs("app/layout.tsx", ".jsx"), ".jsx");
  assert.equal(parsedAs("src/a.mts", ".js"), ".js");
  // A path whose file type is the parser's language needs nothing.
  assert.equal(parsedAs("src/a.tsx", ".tsx"), undefined);
  assert.equal(parsedAs("src/a.ts", ".ts"), undefined);
  assert.equal(parsedAs("pages/index.js", ".jsx"), undefined);
  assert.equal(parsedAs("src/a.js", ".ts"), undefined);
});

test("a rule delegates to what it imports by relative path, transitively, through barrels to the defining modules, and not to other rules", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-imports-"));
  try {
    mkdirSync(join(root, "rules", "helpers"), { recursive: true });
    mkdirSync(join(root, "util"), { recursive: true });
    writeFileSync(join(root, "util", "index.ts"), "import { x } from 'external';\nexport * from './names';\nexport * from './unused';\nexport { renamed as alias } from './renamed';\nexport const { fromObject } = x;\n");
    writeFileSync(join(root, "util", "names.ts"), "export function isName(): boolean { return 'name' === 'name'; }\nexport type Kind = 'a';\n");
    writeFileSync(join(root, "util", "unused.ts"), "export const unused = 'unused';\n");
    writeFileSync(join(root, "util", "renamed.ts"), "export const renamed = 1;\n");
    writeFileSync(join(root, "rules", "helpers", "index.ts"), "const table = require('../../conf/table');\nconst rules = require('../other-rule');\nexport const helper = 'helper';\n");
    mkdirSync(join(root, "conf"), { recursive: true });
    writeFileSync(join(root, "conf", "table.js"), "module.exports = { Array: false };\n");
    writeFileSync(join(root, "rules", "other-rule.ts"), "export default {};\n");
    writeFileSync(join(root, "rules", "rule.ts"), [
      "import { isName, alias, fromObject } from '../util';",
      "import type { Kind } from '../util';",
      "import * as helpers from './helpers';",
      "import other from './other-rule';",
      "import external from 'external';",
      "const shared = require('./helpers/index');",
      "helpers.helper;",
    ].join("\n"));
    assert.equal(resolveModule(join(root, "rules"), "../util"), join(root, "util", "index.ts"));
    assert.equal(definingModule(join(root, "util", "index.ts"), "isName"), join(root, "util", "names.ts"));
    assert.equal(definingModule(join(root, "util", "index.ts"), "alias"), join(root, "util", "renamed.ts"));
    assert.equal(definingModule(join(root, "util", "index.ts"), "fromObject"), join(root, "util", "index.ts"));
    assert.equal(definingModule(join(root, "util", "index.ts"), "missing"), undefined);
    // What a helper imports in turn is followed too, but never into another rule; no package is a helper here.
    assert.deepEqual(importedModules(join(root, "rules", "rule.ts")), {
      own: [
        join(root, "conf", "table.js"),
        join(root, "rules", "helpers", "index.ts"),
        join(root, "util", "index.ts"),
        join(root, "util", "names.ts"),
        join(root, "util", "renamed.ts"),
      ],
      helpers: [],
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a rule checks the words of its whitespace-free strings, its messages and defaults, not its prose or the rest of its meta block", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-names-"));
  try {
    const file = join(root, "no-script-url.js");
    writeFileSync(file, [
      "module.exports = {",
      "  meta: { messages: { unexpectedScriptURL: 'hidden' }, docs: { url: 'https://eslint.org/docs/rules/no-script-url' }, defaultOptions: [{ terms: ['todo'] }] },",
      "  create(context) {",
      "    if (value.toLowerCase().indexOf('javascript:') === 0) context.report('Script URL is a form of eval.');",
      "    if (node.type === 'TemplateLiteral' || directive === 'eslint-disable-line') return `${name}.prototype`;",
      "    report(\"Use 'strictEqual' here\");",
      "    tracker.iterateGlobalReferences({ Math: { pow: { [CALL]: true } } });",
      "  },",
      "};",
    ].join("\n"));
    const isMeta = ruleDocumentation(ts as unknown as TypeScript);
    const names = literalNames(ts as unknown as TypeScript, [file], isMeta);
    for (const word of ["javascript", "TemplateLiteral", "eslint", "disable", "line", "prototype", "strictEqual"]) assert.ok(names.has(word), word);
    // Prose with spaces names nothing but its quoted words, and the rest of the meta block is documentation.
    for (const word of ["Script", "URL", "form", "eval", "Use", "here", "https", "no"]) assert.ok(!names.has(word), word);
    // Where every prose string sits in the documentation block, every word of the strings left counts.
    const every = literalNames(ts as unknown as TypeScript, [file], isMeta, { everyStringWord: true });
    for (const word of ["javascript", "Script", "URL", "form", "eval", "Use", "here", "strictEqual"]) assert.ok(every.has(word), word);
    for (const word of ["https", "no"]) assert.ok(!every.has(word), word);
    // The messages in meta are what the rule reports, not documentation.
    assert.ok(names.has("hidden") && every.has("hidden"));
    // The defaults in meta are the options the rule runs with, not documentation.
    assert.ok(names.has("todo"));
    // Object keys count only when asked for: the lookup tables a rule checks names against.
    assert.ok(!every.has("terms") && !every.has("Math"));
    const keyed = literalNames(ts as unknown as TypeScript, [file], isMeta, { objectKeys: true });
    for (const word of ["terms", "Math", "pow", "create"]) assert.ok(keyed.has(word), word);
    for (const word of ["docs", "url", "CALL", "toLowerCase"]) assert.ok(!keyed.has(word), word);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a bare specifier names its package, scoped or not, without the subpath", () => {
  assert.equal(packageName("@typescript-eslint/utils/ast-utils"), "@typescript-eslint/utils");
  assert.equal(packageName("@eslint-community/eslint-utils"), "@eslint-community/eslint-utils");
  assert.equal(packageName("esutils/lib/keyword"), "esutils");
});

/** A package under `root/node_modules`, from its files by relative path. */
function writePackage(root: string, name: string, files: Record<string, string>): string {
  const directory = join(root, "node_modules", ...name.split("/"));
  for (const [path, text] of Object.entries({ "package.json": JSON.stringify({ name, main: "index.js" }), ...files })) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    writeFileSync(join(directory, path), text);
  }
  return directory;
}

test("a CommonJS build defines a name where it assigns it, and re-exports it through getters, __exportStar and namespaces", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-cjs-"));
  try {
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
      "tracker.js": "var Kinds;\n(function (Kinds) { Kinds[\"A\"] = \"A\"; })(Kinds || (exports.Kinds = Kinds = {}));\nexports.Tracker = class {};\n",
      "utils/index.js": "exports.helper = 1;\n",
    });
    writePackage(root, "other", { "index.js": "exports.External = 1;\n" });
    const helpers = new Set(["@scope/helper"]);
    const entry = join(helper, "index.js");
    assert.equal(definingModule(entry, "analyze", helpers), join(helper, "analyze.js"));
    assert.equal(definingModule(entry, "Tracker", helpers), join(helper, "tracker.js"));
    // An enum's `exports.Kinds = Kinds = {}` defines it.
    assert.equal(definingModule(entry, "Kinds", helpers), join(helper, "tracker.js"));
    // A namespace export is the module it exports.
    assert.equal(definingModule(entry, "Utils", helpers), join(helper, "utils", "index.js"));
    // Re-exports are followed into helper packages only.
    assert.equal(definingModule(entry, "External", helpers), undefined);
    assert.equal(definingModule(entry, "External", new Set([...helpers, "other"])), realpathSync(join(root, "node_modules", "other", "index.js")));
    assert.equal(definingModule(entry, "missing", helpers), undefined);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a rule delegates to the modules of helper packages that define the names it imports, and not to their own package's internals", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-helpers-"));
  try {
    const utils = writePackage(root, "@eslint-community/eslint-utils", {
      "index.js": "const globalObjectNames = ['global', 'globalThis', 'self', 'window'];\nclass ReferenceTracker {}\nconst CALL = Symbol('call');\nrequire('./internal');\nexports.ReferenceTracker = ReferenceTracker;\nexports.CALL = CALL;\nexports.getStaticValue = getStaticValue;\nfunction getStaticValue() {}\n",
      "internal.js": "exports.internal = 'internal';\n",
    });
    const scope = writePackage(root, "@scope/scope-manager", {
      "index.js": 'var analyze_1 = require("./analyze");\nObject.defineProperty(exports, "analyze", { enumerable: true, get: function () { return analyze_1.analyze; } });\n',
      "analyze.js": "exports.analyze = analyze;\nfunction analyze() { return { jsxPragma: 'React' }; }\n",
    });
    writePackage(root, "espree", { "index.js": "exports.tokenize = function () {};\n" });
    mkdirSync(join(root, "lib", "rules", "utils"), { recursive: true });
    writeFileSync(join(root, "lib", "rules", "utils", "ast-utils.js"), "const espree = require('espree');\nconst { getStaticValue } = require('@eslint-community/eslint-utils');\nmodule.exports = {};\n");
    writeFileSync(join(root, "lib", "rules", "no-obj-calls.js"), [
      "const { CALL, ReferenceTracker } = require('@eslint-community/eslint-utils');",
      "const astUtils = require('./utils/ast-utils');",
      "module.exports = {};",
    ].join("\n"));
    // TypeScript's CommonJS output reads named imports from the binding that holds the module.
    writeFileSync(join(root, "lib", "parser.js"), 'const scope_manager_1 = require("@scope/scope-manager");\nconst x = (0, scope_manager_1.analyze)(ast);\n');
    writeFileSync(join(root, "lib", "rules", "uses-analyze.ts"), "import { analyze } from '@scope/scope-manager';\nimport type { Scope } from '@scope/scope-manager';\nexport default analyze;\n");
    const helpers = new Set(["@eslint-community/eslint-utils", "@scope/scope-manager"]);
    assert.ok(inHelperPackage(join(utils, "index.js"), helpers));
    assert.ok(!inHelperPackage(join(root, "lib", "rules", "no-obj-calls.js"), helpers));
    // A helper's module counts once however it is reached; espree is not a helper; internal.js is the helper's own.
    assert.deepEqual(importedModules(join(root, "lib", "rules", "no-obj-calls.js"), helpers), {
      own: [join(root, "lib", "rules", "utils", "ast-utils.js")],
      helpers: [realpathSync(join(utils, "index.js"))],
    });
    assert.deepEqual(importedModules(join(root, "lib", "parser.js"), helpers, null).helpers, [realpathSync(join(scope, "analyze.js"))]);
    assert.deepEqual(importedModules(join(root, "lib", "rules", "uses-analyze.ts"), helpers).helpers, [realpathSync(join(scope, "analyze.js"))]);
    // Without helpers, packages are not read at all.
    assert.deepEqual(importedModules(join(root, "lib", "rules", "no-obj-calls.js")).helpers, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("ESLint's directive values are its global settings and its severities, read from its sources", () => {
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
  assert.deepEqual(directiveValuesFrom(sourceCode, severity), ["error", "false", "off", "readable", "readonly", "true", "warn", "writable", "writeable"]);
  assert.throws(() => directiveValuesFrom("function renamed() {}", severity), /normalizeConfigGlobal/u);
});

test("test fixtures are the checkout files a manifest glob matches, in path order", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-fixtures-"));
  try {
    for (const path of ["packages/p/tests/fixtures/file.ts", "packages/p/tests/fixtures/nested/react.tsx", "packages/p/tests/fixtures/tsconfig.json", "packages/p/tests/rules/a.test.ts"]) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), "");
    }
    assert.deepEqual(testFixtureFiles(root, ["packages/p/tests/fixtures/**"]), [
      join(root, "packages/p/tests/fixtures/file.ts"),
      join(root, "packages/p/tests/fixtures/nested/react.tsx"),
      join(root, "packages/p/tests/fixtures/tsconfig.json"),
    ]);
    assert.deepEqual(testFixtureFiles(root, []), []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a case's test words are its code and expected outputs as code, and its configuration and expected reports as words", () => {
  const words = caseWords({
    code: "a()",
    output: ["b()", "c()"],
    filename: "file.tsx",
    options: [{ allow: ["zz"] }],
    languageOptions: { globals: { G1: "readonly" } },
    errors: [{ message: "Bad foo.", data: { name: "fooName", list: ["x1"] }, suggestions: [{ desc: "Use bar", output: "bar()", data: { n: "q" } }] }, "Legacy message", 3],
  });
  assert.deepEqual(words, {
    code: "a()",
    outputs: ["b()", "c()", "bar()"],
    jsx: true,
    configuration: JSON.stringify([[{ allow: ["zz"] }], null, ["G1"], ["Bad foo.", "fooName", "x1", "Legacy message", "Use bar", "q"]]),
  });
  assert.deepEqual(caseWords("x;"), { code: "x;", outputs: [], jsx: false, configuration: JSON.stringify([null, null, [], []]) });
  // A null output (no fix) and a count of errors add nothing.
  assert.deepEqual(caseWords({ code: "y", output: null, errors: 2 }).outputs, []);
});

test("Babel reads a TypeScript file's accessor class members, as TypeScript does, and a JavaScript file's as the proposal they are", () => {
  const seen: unknown[][] = [];
  const babel: BabelParser = { parse: (_source, options) => (seen.push(options.plugins), { errors: [] }) };
  babelErrors(babel, "class A { accessor x = 1; }", { typescript: true, jsx: false, goal: "module" });
  babelErrors(babel, "class A { accessor x = 1; }", { typescript: false, jsx: true, goal: "module" });
  assert.deepEqual(seen, [[["typescript", {}], "decoratorAutoAccessors", "decorators-legacy"], ["jsx", "decorators-legacy"]]);
});

test("Babel reads a case with the goal upstream reads it with: a script is not strict mode, a .cjs file is CommonJS", () => {
  const seen: string[] = [];
  const babel: BabelParser = { parse: (_source, options) => (seen.push(options.sourceType), { errors: [] }) };
  for (const goal of ["module", "script", "commonjs"] as const) babelErrors(babel, "yield: x;", { typescript: false, jsx: false, goal });
  assert.deepEqual(seen, ["module", "script", "commonjs"]);
});

test("a version file declares a release's version as the string it maps the package's name to", () => {
  // facebook/react's ReactVersions.js at eslint-plugin-react-hooks@7.1.1, abridged.
  const versions = `
const ReactVersion = '19.3.0';
const stablePackages = {
  'eslint-plugin-react-hooks': '7.1.1',
  'jest-react': '0.18.0',
  react: ReactVersion,
  scheduler: '0.28.0',
};
module.exports = { ReactVersion, stablePackages };
`;
  assert.equal(declaredVersion(versions, "eslint-plugin-react-hooks"), "7.1.1");
  assert.equal(declaredVersion(versions, "scheduler"), "0.28.0");
  // A name mapped to anything but a string literal, or not mapped at all, declares no version.
  assert.equal(declaredVersion(versions, "react"), undefined);
  assert.equal(declaredVersion(versions, "eslint-plugin-react"), undefined);
});

test("a default export is defined where it is declared, by the module whose import it exports, or by a JSON module's data", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-default-"));
  try {
    mkdirSync(join(root, "utils"), { recursive: true });
    writeFileSync(join(root, "utils", "index.js"), "export {default as assertToken} from './assert-token.js';\nexport {default as list} from './list-entry.js';\n");
    writeFileSync(join(root, "utils", "assert-token.js"), "export default function assertToken() {}\n");
    writeFileSync(join(root, "utils", "list-entry.js"), "import list from './list.json' with {type: 'json'};\nexport default list;\n");
    writeFileSync(join(root, "utils", "list.json"), '["fs", "node:fs"]\n');
    writeFileSync(join(root, "utils", "config.js"), "const config = {create() {}};\nexport default config;\n");
    writeFileSync(join(root, "utils", "common.cjs"), "module.exports = function common() {};\n");
    writeFileSync(join(root, "utils", "named.js"), "export const named = 1;\n");
    const at = (file: string) => join(root, "utils", file);
    assert.equal(definingModule(at("index.js"), "assertToken"), at("assert-token.js"));
    assert.equal(definingModule(at("index.js"), "list"), at("list.json"));
    assert.equal(definingModule(at("list-entry.js"), "default"), at("list.json"));
    assert.equal(definingModule(at("config.js"), "default"), at("config.js"));
    // A CommonJS module's default export is what it assigns to module.exports.
    assert.equal(definingModule(at("common.cjs"), "default"), at("common.cjs"));
    assert.equal(definingModule(at("named.js"), "missing"), undefined);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a helper's imports of another helper package are followed to the data they define, not its own package's", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-helper-chain-"));
  try {
    const isBuiltin = writePackage(root, "is-builtin-module", {
      "index.js": "import builtinModules from 'builtin-modules';\nimport {cache} from './cache.js';\nexport default function isBuiltinModule(name) { return new Set(builtinModules).has(name); }\n",
      "cache.js": "export const cache = 'internal';\n",
    });
    const builtins = writePackage(root, "builtin-modules", {
      "index.js": "import builtinModules from './builtin-modules.json' with {type: 'json'};\nexport default builtinModules;\n",
      "builtin-modules.json": '["node:fs", "fs", "child_process"]\n',
    });
    mkdirSync(join(root, "rules"), { recursive: true });
    const rule = join(root, "rules", "prefer-node-protocol.js");
    writeFileSync(rule, "import isBuiltinModule from 'is-builtin-module';\nexport default {create() { return isBuiltinModule('fs'); }};\n");
    assert.deepEqual(importedModules(rule, new Set(["is-builtin-module", "builtin-modules"])), {
      own: [],
      helpers: [realpathSync(join(builtins, "builtin-modules.json")), realpathSync(join(isBuiltin, "index.js"))].sort(),
    });
    // A package that is not a helper is not followed from a helper either.
    assert.deepEqual(importedModules(rule, new Set(["is-builtin-module"])).helpers, [realpathSync(join(isBuiltin, "index.js"))]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a helper's re-exports of another helper package's code do not bring in that package's bundle", () => {
  const root = mkdtempSync(join(tmpdir(), "eslint-helper-bundle-"));
  try {
    // A single-file bundle, as @eslint-community/eslint-utils ships: one module defines every name, with strings no
    // token predicate checks.
    const bundle = writePackage(root, "bundled-utils", {
      "index.js": [
        "function isArrowToken(token) { return token.value === '=>'; }",
        "function getFunctionNameWithKind() { return 'method'; }",
        "const globals = ['Promise'];",
        "exports.isArrowToken = isArrowToken;",
        "exports.getFunctionNameWithKind = getFunctionNameWithKind;",
        "",
      ].join("\n"),
    });
    // @typescript-eslint/utils's ast-utils/eslint-utils/predicates.js: a namespace of the bundle, re-exported by name.
    const commonjs = writePackage(root, "@scope/utils", {
      "index.js": [
        "const eslintUtils = __importStar(require(\"bundled-utils\"));",
        "exports.isArrowToken = eslintUtils.isArrowToken;",
        "",
      ].join("\n"),
    });
    const esm = writePackage(root, "esm-helper", {
      "index.js": "import { isArrowToken } from 'bundled-utils';\nexport function isArrow(token) { return isArrowToken(token); }\n",
    });
    const helpers = new Set(["bundled-utils", "@scope/utils", "esm-helper"]);
    mkdirSync(join(root, "rules"), { recursive: true });
    const rule = join(root, "rules", "uses-helpers.js");
    writeFileSync(rule, "import { isArrowToken } from '@scope/utils';\nimport { isArrow } from 'esm-helper';\nexport default {create() { return isArrowToken(isArrow); }};\n");
    assert.deepEqual(importedModules(rule, helpers), {
      own: [],
      helpers: [realpathSync(join(commonjs, "index.js")), realpathSync(join(esm, "index.js"))].sort(),
    });
    // The rule's own import of the bundle still counts it, through the module that defines the name.
    const direct = join(root, "rules", "uses-bundle.js");
    writeFileSync(direct, "import { isArrowToken } from 'bundled-utils';\nexport default {create() { return isArrowToken; }};\n");
    assert.deepEqual(importedModules(direct, helpers).helpers, [realpathSync(join(bundle, "index.js"))]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a module belongs to the innermost helper package on its path", () => {
  const nested = "/x/node_modules/is-builtin-module/node_modules/builtin-modules/index.js";
  assert.equal(helperPackage(nested, new Set(["is-builtin-module", "builtin-modules"])), "/x/node_modules/is-builtin-module/node_modules/builtin-modules/");
  assert.equal(helperPackage(nested, new Set(["is-builtin-module"])), "/x/node_modules/is-builtin-module/");
  assert.equal(helperPackage("/x/node_modules/@scope/utils/dist/a.js", new Set(["@scope/utils"])), "/x/node_modules/@scope/utils/");
  assert.equal(helperPackage("/x/lib/rules/a.js", new Set(["@scope/utils"])), undefined);
});

test("a case tester's configuration takes the approved setting only where it sets the rule under test", () => {
  const config = { files: ["**"], rules: { "rule-to-test/no-null": ["error", { checkStrictEquality: true }] }, languageOptions: { sourceType: "module" } };
  assert.deepEqual(withSetting(config, "rule-to-test/no-null", ["error"]), { ...config, rules: { "rule-to-test/no-null": ["error"] } });
  assert.deepEqual(withSetting([{ rules: { other: "error" } }, config], "rule-to-test/no-null", ["error"]), [{ rules: { other: "error" } }, { ...config, rules: { "rule-to-test/no-null": ["error"] } }]);
  const unrelated = { rules: { other: "warn" } };
  assert.equal(withSetting(unrelated, "rule-to-test/no-null", ["error"]), unrelated);
  assert.equal(withSetting(null, "rule-to-test/no-null", ["error"]), null);
});
