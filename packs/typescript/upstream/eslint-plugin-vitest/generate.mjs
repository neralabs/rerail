// Records @vitest/eslint-plugin for the TypeScript pack's vitest ports (packs/typescript/conformance.json), in the
// format rerail/conformance/languages/pack.ts reads: eslint-plugin-vitest/fixtures.expected.json (the rules' own
// RuleTester cases).
// Run by `npm run fixtures:derive` (a maintainer, the bump workflow, or a pull request's check that a changed
// test-data.lock reproduces):
//   node generate.mjs --tools <dir> --vitest <checkout>
// <dir> holds node_modules with the runtime nera-rule-validation pins for @vitest/eslint-plugin 1.6.27
// (validation/upstream/vitest-eslint-plugin-1.6.27): eslint 10.0.0, @vitest/eslint-plugin 1.6.27,
// @typescript-eslint/parser 8.58.0 and tsx (to load the plugin's .ts tests). The checkout is
// github.com/vitest-dev/eslint-plugin-vitest at v1.6.27 (aad57d121647efe764463c37414a8cd67ec9cf92); only its tests/
// are read, and the plugin linted is the published one in <dir>.
//
// Fixtures: each rule's test file is loaded with a stub RuleTester that records its cases instead of linting. Every
// case is linted with every pack rule of the plugin on at its default options (a case's own options are ignored, as
// the V1 gates' upstream-tests gate ignores them) under typescript-eslint's parser, as the plugin's RuleTester
// (@typescript-eslint/rule-tester) parses every case, plus the case's own parserOptions; its globals and settings are
// left out. A case with another parser is left out. A case's code is written to a test file, fixtures/<rule>/<n>.test.ts
// (.test.tsx with JSX, or the TypeScript-family extension of its filename after `.test`). Code that does not parse is left out. Each kept file is
// one analysis unit. Rerail's own cases (AUTHORED, below) are written beside them as fixtures/<rule>/rerail-<name>.test.ts,
// linted the same way, and listed as Rerail's in UPSTREAM.md beside this recorder.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire, registerHooks } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertCommit, assertKnownReportRules, lint, pinnedRequire, writeRecording } from "../../../../conformance/recorders/eslint-record.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = JSON.parse(readFileSync(join(HERE, "..", "..", "conformance.json"), "utf8"));
const TOOL = "eslint-plugin-vitest";
const PINS = { eslint: "10.0.0", "@vitest/eslint-plugin": "1.6.27", "@typescript-eslint/parser": "8.58.0", tsx: "4.21.0" };
const CHECKOUT = { repository: "https://github.com/vitest-dev/eslint-plugin-vitest", commit: "aad57d121647efe764463c37414a8cd67ec9cf92" };
const PREFIX = "vitest/";
const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
const TYPESCRIPT_EXTENSIONS = [".d.ts", ".ts", ".tsx", ".mts", ".cts"];

/**
 * Cases Rerail wrote for the plugin to score: inputs the ports once misread (a call was
 * Vitest's by its name alone). Each is linted with every pack rule, like the plugin's own cases. Upstream classifies a
 * call by its head's binding (`resolveVitestFn` in src/utils/parse-vitest-fn-call.ts): a global; a named import, or a
 * key destructured from `require`/`await import`, of `vitest`; a `.extend` of a test function so bound; or a test
 * context. `reports` is what the plugin reports on each, by rule (none when absent); the recorder fails when the
 * recording differs, so a case cannot silently stop proving what it was written for.
 */
const AUTHORED = [
  // Not Vitest's: another runner's import, a local, a parameter, or a binding of `vitest` that is not a test function.
  {
    rule: "no-focused-tests",
    name: "node-test",
    code: `import { describe, it } from 'node:test';

describe.only('suite', () => {
  it.only('focused', t => {
    t.assert.ok(true);
  });
  it.skip('skipped', () => {});
  it('todo');
});
`,
  },
  {
    rule: "no-focused-tests",
    name: "node-test-default",
    code: `import test from 'node:test';

test.only('focused', t => {
  t.assert.ok(true);
});
`,
  },
  {
    rule: "no-focused-tests",
    name: "bun-test",
    code: `import { describe, it } from 'bun:test';

describe.only('suite', () => {
  it.skip('skipped', () => {});
  it('todo');
});
`,
  },
  {
    rule: "expect-expect",
    name: "ava",
    code: `import test from 'ava';

test('filters', t => {
  t.deepEqual([1, 0].filter(Boolean), [1]);
});
`,
  },
  {
    rule: "no-focused-tests",
    name: "local-binding",
    code: `const test = { only() {}, skip() {} };

test.only();
test.skip();
`,
  },
  {
    rule: "no-focused-tests",
    name: "parameter",
    code: `export function suite(it: any) {
  it.only('focused', () => {});
  it('todo');
}
`,
  },
  {
    rule: "no-focused-tests",
    name: "global-extend",
    code: `const it = test.extend({});

it.only('focused', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "vitest-type-import",
    code: `import type { it } from 'vitest';

it.only('focused', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "vitest-default-import",
    code: `import test from 'vitest';

test.only('focused', () => {});
`,
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-require-whole",
    code: `const it = require('vitest');

it.only('focused', () => {});
it.skip('skipped', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "vitest-require-default-value",
    code: `const { it = { only() {} } } = require('vitest');

it.only('focused', () => {});
`,
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-alias-not-a-test",
    code: `import { expect as it, describe as test } from 'vitest';

it.only('focused', () => {});
test('no function');
`,
  },
  {
    rule: "no-focused-tests",
    name: "vitest-extend-not-a-test",
    code: `import { describe as base } from 'vitest';

const describe = base.extend({});

describe.only('suite', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "suite-only",
    code: `suite.only('suite', () => {});
bench.only('bench', () => {
  expect(1).toBe(1);
});
`,
  },
  // Vitest's: reported as the globals are.
  {
    rule: "no-disabled-tests",
    name: "vitest-import",
    code: `import { describe, it, test as check } from 'vitest';

describe.only('suite', () => {
  it.skip('skipped', () => {});
  check('asserts nothing', () => {});
  it('todo');
});
`,
    reports: { "expect-expect": 2, "no-disabled-tests": 2, "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-alias",
    code: `import { test as it } from 'vitest';

it.only('focused', () => {});
it('no function');
`,
    reports: { "expect-expect": 2, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-require",
    code: `const { it } = require('vitest');

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-require-renamed",
    code: `const { test: it, describe } = require('vitest');

describe.skip('skipped', () => {});
it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-dynamic-import",
    code: `const { it } = await import('vitest');

it.skip('skipped', () => {});
`,
    reports: { "no-disabled-tests": 1 },
  },
  {
    rule: "expect-expect",
    name: "vitest-extend",
    code: `import { test as base } from 'vitest';

const it = base.extend({});

it.only('focused', () => {});
it('asserts nothing', () => {});
`,
    reports: { "expect-expect": 2, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-extend-chain",
    code: `const { test: base } = require('vitest');

const extended = base.extend({});
const it = extended.extend({});

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "test-context",
    code: `test('suite', (context, it) => {
  it.only('focused', () => {});
  it.skip('skipped', () => {});
});
`,
    reports: { "expect-expect": 2, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "test-context-property",
    code: `test('suite', ({ it }) => {
  it.only('focused', () => {});
});
`,
    reports: { "expect-expect": 2, "no-focused-tests": 1 },
  },
  // Syntax upstream's ESTree resolver reads through: comments, trailing commas, template literals
  // and escapes in the module name; parentheses around an initializer; a test context named by an explicit property.
  {
    rule: "no-focused-tests",
    name: "vitest-require-comment",
    code: `const { it } = require(/* the runner */ 'vitest',);

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-dynamic-import-trailing-comma",
    code: `const { it } = await import('vitest',);

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "vitest-require-template",
    code: `const { test } = require(\`vitest\`);

test.skip('skipped', () => {});
`,
    reports: { "no-disabled-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-import-escaped",
    code: `import { it } from 'vit\\u0065st';

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-require-parenthesized",
    code: `const { it } = (require('vitest'));

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-extend-parenthesized",
    code: `import { test as base } from 'vitest';

const it = (base.extend({}));

it.only('focused', () => {});
it.skip('skipped', () => {});
it('asserts nothing', () => {});
`,
    reports: { "expect-expect": 2, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "test-context-explicit-property",
    code: `test('suite', ({ it: it }) => {
  it.only('focused', () => {});
});
`,
    reports: { "expect-expect": 2, "no-focused-tests": 1 },
  },
  // Not Vitest's: a submodule of vitest, and a context property whose key is not the name.
  {
    rule: "no-focused-tests",
    name: "vitest-submodule",
    code: `const { it } = require('vitest/config');

it.only('focused', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "test-context-renamed-property",
    code: `test('suite', ({ test: it }) => {
  it.only('focused', () => {});
});
`,
    reports: { "expect-expect": 1 },
  },
  // A mode option (`{ only: true }`, `{ skip: true }`), which upstream reports at its key: Vitest's, and node:test's.
  {
    rule: "no-focused-tests",
    name: "vitest-mode-options",
    code: `import { describe, test } from 'vitest';

test('focused', { only: true }, () => {
  expect(1).toBe(1);
});
test('skipped', { only: false, skip: true }, () => {});
describe('focused later', { only: false, only: true }, () => {});
`,
    reports: { "expect-expect": 1, "no-disabled-tests": 1, "no-focused-tests": 2 },
  },
  {
    rule: "no-focused-tests",
    name: "node-test-mode-options",
    code: `import { describe, test } from 'node:test';

test('focused', { only: true }, t => {
  t.assert.ok(true);
});
describe('skipped', { skip: true }, () => {});
`,
  },
  // Key and module-name literals as upstream reads them, valid chains only, the last mode
  // option property, and a test callback's own name.
  {
    rule: "no-disabled-tests",
    name: "vitest-require-computed-key",
    code: `const { ["it"]: it } = require('vitest');

it.only('focused', () => {});
it.skip('skipped', () => {});
`,
    reports: { "expect-expect": 1, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-require-template-escape",
    code: `const { it } = require(\`vit\\u0065st\`);

it.only('focused', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "vitest-default-import-named-type",
    code: `import type, { it } from 'vitest';

it.only('focused', () => {});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "mode-option-overridden",
    code: `test('a', { only: true, only() {} }, () => {
  expect(1).toBe(1);
});
test('b', { skip: true, get skip() { return false; } }, () => {
  expect(1).toBe(1);
});
`,
  },
  {
    rule: "no-focused-tests",
    name: "invalid-chains",
    code: `describe.notVitest.only('suite', () => {});
test.notVitest('focused', { only: true }, () => {
  expect(1).toBe(1);
});
it.only.only('twice', () => {});
`,
  },
  {
    rule: "no-focused-tests",
    name: "test-context-named-callback",
    code: `test('suite', function it(context) {
  it.only('focused', () => {});
});
`,
    reports: { "expect-expect": 2, "no-focused-tests": 1 },
  },
  // Self-review round 2: chains read by their accessors (line breaks), each function's own grammar whatever its local
  // name, test contexts and `vi`/`expect` whose chains upstream does not check, and expect-expect's test shapes.
  {
    rule: "no-focused-tests",
    name: "multiline-chain",
    code: `import { describe, it } from 'vitest';

it.only
  .concurrent('focused', () => {});
describe
  .skip('skipped', () => {});
`,
    reports: { "expect-expect": 1, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "vitest-alias-across-grammars",
    code: `import { test as describe } from 'vitest';

describe.fails.only('focused', () => {});
describe.fails.skip('skipped', () => {});
`,
    reports: { "expect-expect": 1, "no-disabled-tests": 1, "no-focused-tests": 1 },
  },
  {
    rule: "no-focused-tests",
    name: "test-context-unchecked-chain",
    code: `test('suite', (context, it) => {
  expect(context).toBeDefined();
  it.only.only('focused', () => {});
});
`,
    reports: { "expect-expect": 1, "no-focused-tests": 1 },
  },
  {
    rule: "expect-expect",
    name: "test-without-callback",
    code: `import { it, test } from 'vitest';

it('todo');
test('options only', { timeout: 1 });
it('named', check);
it('made', make());

function check() {
  expect(1).toBe(1);
}
`,
    reports: { "expect-expect": 3, "no-disabled-tests": 1 },
  },
  {
    rule: "expect-expect",
    name: "vitest-table-calls",
    code: `import { it } from 'vitest';

it.for([1])('for %s', () => {});
it.only.each\`a\`('tagged', () => {});
`,
    reports: { "expect-expect": 2, "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "unchecked-heads",
    code: `vi.skip('vi');
expect.not.skip('expect');
expect.foo.skip('no matcher chain');
`,
    reports: { "no-disabled-tests": 2 },
  },
  // Self-review round 3: comments between links, expect's modifier rules, and assertions as getNodeName names them.
  {
    rule: "no-focused-tests",
    name: "comment-between-links",
    code: `it.only /* keep */ .concurrent('focused', () => {
  expect(1).toBe(1);
});
`,
    reports: { "no-focused-tests": 1 },
  },
  {
    rule: "no-disabled-tests",
    name: "expect-modifier-rules",
    code: `expect.not.not.skip('two nots');
expect.resolves.rejects.skip('two promise modifiers');
expect.resolves.not.skip('one of each');
expect.to.be.skip('language chains');
expectTypeOf.returns.not.skip('expectTypeOf modifiers');
`,
    reports: { "no-disabled-tests": 3 },
  },
  {
    rule: "expect-expect",
    name: "assertion-names",
    code: `it('parenthesized', check);
it('case', () => {
  Expect(1).toBe(1);
});
it('tagged template', () => {
  expect\`x\`;
});

function check() {
  (expect)(1).toBe(1);
}
`,
    reports: { "expect-expect": 1 },
  },
  // Self-review round 4: computed accessors in an assertion's name, and a parenthesized helper argument.
  {
    rule: "expect-expect",
    name: "assertion-computed-names",
    code: `it('string key', () => {
  assert['ok'](true);
});
it('identifier key', () => {
  assert[method](true);
});
it('parenthesized helper', (check));

function check() {
  expect(1).toBe(1);
}
`,
  },
];

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (["--tools", "--vitest"].includes(flag)) options[flag.slice(2)] = resolve(argv[++index]);
    else throw new Error(`unknown argument ${flag}`);
  }
  for (const key of ["tools", "vitest"]) if (options[key] === undefined) throw new Error(`--${key} is required`);
  return options;
}

const options = parseArguments(process.argv.slice(2));
const toolsRequire = pinnedRequire(options.tools, PINS);
assertCommit(options.vitest, CHECKOUT.commit);
const { ESLint } = toolsRequire("eslint");
const tsParser = toolsRequire("@typescript-eslint/parser");
const pluginModule = toolsRequire("@vitest/eslint-plugin");
const plugin = pluginModule.default ?? pluginModule;

/** The pack's vitest rules, as ESLint reports them (`vitest/no-focused-tests`). */
const RULES = PACK.rules.filter(rule => rule.upstream.tool === TOOL).map(rule => rule.upstream.rule).sort();
const VERSION = PACK.tools.find(entry => entry.name === TOOL).version;

/** The pack's configuration for the plugin: typescript-eslint's parser for every file, every pack rule on. */
function config(languages = []) {
  return [
    {
      files: ECMA_EXTENSIONS.map(extension => `**/*${extension}`),
      plugins: { vitest: plugin },
      languageOptions: { parser: tsParser },
      linterOptions: { reportUnusedDisableDirectives: "off" },
      rules: Object.fromEntries(RULES.map(rule => [rule, "error"])),
    },
    { files: ["**/*.tsx", "**/*.jsx"], languageOptions: { parserOptions: { ecmaFeatures: { jsx: true } } } },
    ...languages.map(({ files, languageOptions }) => ({ files, languageOptions })),
  ];
}

// ---------------------------------------------------------------------------------------------------------------
// Fixture extraction: a stub RuleTester records each test file's runs.

const RUNS = [];
class RuleTesterStub {
  constructor(testerConfig = {}) {
    this.config = testerConfig;
  }
  static afterAll = () => undefined;
  static describe = (_text, method) => method();
  static describeSkip = () => undefined;
  static it = () => undefined;
  static itOnly = () => undefined;
  static itSkip = () => undefined;
  static only = item => (typeof item === "object" ? { ...item, only: true } : { code: item, only: true });
  static setDefaultConfig() {}
  defineRule() {}
  run(name, _rule, tests) {
    RUNS.push({ name, config: this.config, tests });
  }
}
const STUB = Symbol.for("rerail.typescriptPack.vitestRuleTester");
globalThis[STUB] = { RuleTester: RuleTesterStub, noFormat: (raw, ...keys) => String.raw({ raw }, ...keys) };
for (const name of ["describe", "it", "test"]) globalThis[name] ??= Object.assign((_text, method) => (name === "describe" ? method() : undefined), { skip: () => undefined, only: () => undefined, each: () => () => undefined, for: () => () => undefined });

function installHooks() {
  const shim = (name, body) => {
    const path = join(options.tools, ".generate-shims", name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
    return pathToFileURL(path).href;
  };
  const testerShim = shim("vitest-rule-tester.cjs", `module.exports = globalThis[Symbol.for("rerail.typescriptPack.vitestRuleTester")];\n`);
  const ruleDirectory = pathToFileURL(join(options.vitest, "src", "rules")).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@typescript-eslint/rule-tester") return { url: testerShim, format: "commonjs", shortCircuit: true };
      let resolved;
      try {
        resolved = nextResolve(specifier, context);
      } catch (error) {
        if (specifier.startsWith(".") && context.parentURL?.startsWith(pathToFileURL(join(options.vitest, "tests")).href)) {
          // A rule's source: only its name is read (`rule.name`), so it is a stub named after the file.
          const target = new URL(specifier, context.parentURL).href;
          if (target.startsWith(`${ruleDirectory}/`)) {
            const name = target.slice(ruleDirectory.length + 1);
            return { url: shim(`vitest-rule-${name}.cjs`, `module.exports = { default: { name: ${JSON.stringify(name)}, meta: {}, create() { return {}; } } };\n`), format: "commonjs", shortCircuit: true };
          }
        }
        // The checkout has no node_modules: a bare specifier resolves from the tools directory.
        if (specifier.startsWith(".") || specifier.startsWith("/")) throw error;
        resolved = { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true };
      }
      return resolved;
    },
  });
}

const isObject = value => typeof value === "object" && value !== null && !Array.isArray(value);
const merge = (...layers) =>
  layers.reduce((merged, layer) => {
    if (!isObject(layer)) return merged;
    const out = { ...merged };
    for (const [key, value] of Object.entries(layer)) out[key] = isObject(value) && isObject(out[key]) ? merge(out[key], value) : value;
    return out;
  }, {});

/** Every kept case of the pack's vitest rules, written under `fixtures/<rule>/`, with each file's own parserOptions. */
function extractFixtures() {
  const fixtures = join(HERE, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  const files = [];
  const languages = new Map();
  const counts = {};
  for (const rule of RULES) {
    const name = rule.slice(PREFIX.length);
    const path = join(options.vitest, "tests", `${name}.test.ts`);
    RUNS.length = 0;
    createRequire(path)(path);
    const runs = RUNS.filter(run => run.name === name);
    if (runs.length === 0) throw new Error(`${rule}: ${path} ran no RuleTester for ${name}`);
    const seen = new Set();
    const count = (counts[rule] = { cases: 0, withOptions: 0, skipped: {}, written: 0, duplicate: 0 });
    for (const run of runs) {
      for (const kind of ["valid", "invalid"]) {
        for (const entry of run.tests[kind] ?? []) {
          const test = typeof entry === "string" ? { code: entry } : entry;
          count.cases += 1;
          if (test.skip === true) {
            count.skipped.skip = (count.skipped.skip ?? 0) + 1;
            continue;
          }
          if (Array.isArray(test.options) && test.options.length > 0) count.withOptions += 1;
          const languageOptions = merge(run.config.languageOptions, test.languageOptions);
          if (languageOptions.parser !== undefined && languageOptions.parser !== tsParser) {
            count.skipped["custom parser"] = (count.skipped["custom parser"] ?? 0) + 1;
            continue;
          }
          const parserOptions = languageOptions.parserOptions;
          const jsx = parserOptions?.ecmaFeatures?.jsx === true;
          const filename = typeof test.filename === "string" ? test.filename : "";
          const named = TYPESCRIPT_EXTENSIONS.find(extension => filename.endsWith(extension));
          const extension = named !== undefined && !(jsx && named !== ".tsx") ? named : jsx ? ".tsx" : ".ts";
          const key = `${extension}\0${JSON.stringify(parserOptions ?? null)}\0${test.code}`;
          if (seen.has(key)) {
            count.duplicate += 1;
            continue;
          }
          seen.add(key);
          // A test file's name, as the rules' cases are tests (the ports read `*.test.ts` and `*.spec.ts` files).
          const file = `${name}/${String(seen.size).padStart(3, "0")}${extension === ".d.ts" ? extension : `.test${extension}`}`;
          mkdirSync(join(fixtures, name), { recursive: true });
          writeFileSync(join(fixtures, file), test.code);
          files.push(file);
          if (parserOptions !== undefined) {
            const group = JSON.stringify(parserOptions);
            if (!languages.has(group)) languages.set(group, []);
            languages.get(group).push(file);
          }
          count.written += 1;
        }
      }
    }
  }
  const authored = [];
  for (const { rule, name, code, reports = {} } of AUTHORED) {
    if (!RULES.includes(`${PREFIX}${rule}`)) throw new Error(`AUTHORED ${name}: ${rule} is not a pack rule`);
    assertKnownReportRules(name, reports, RULES.map(entry => entry.slice(PREFIX.length)));
    const file = `${rule}/rerail-${name}.test.ts`;
    mkdirSync(join(fixtures, rule), { recursive: true });
    writeFileSync(join(fixtures, file), code);
    files.push(file);
    authored.push({ file, reports });
  }
  return { fixtures, files, authored, languages: [...languages].map(([group, members]) => ({ files: members, languageOptions: { parserOptions: JSON.parse(group) } })), counts };
}

/** UPSTREAM.md beside this recorder: the fixtures Rerail wrote, which are Rerail's, not the plugin's. */
function writeAuthoredRecord(authored) {
  const rows = authored.map(({ file }) => `| \`upstream/${TOOL}/fixtures/${file}\` | written by Rerail |`);
  const text = `# ${TOOL} ${VERSION}: cases Rerail wrote

The fixtures listed below were written by Rerail for the pinned plugin to score, beside the cases extracted from the plugin's own tests (\`licenses/${TOOL}/UPSTREAM.md\`). Each is an input the ports once misread; \`generate.mjs\` holds them (\`AUTHORED\`) and records the plugin's findings on them in \`fixtures.expected.json\`. They are Rerail's (Apache-2.0, the pack's licence), so nothing goes under \`licenses/\`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(HERE, "UPSTREAM.md"), text);
}

/** The AUTHORED cases whose recorded findings differ from their `reports`, as `<file> <rule>: expected n, recorded m`. */
function authoredMismatches(authored, findings) {
  const mismatches = [];
  for (const { file, reports } of authored) {
    for (const rule of RULES) {
      const expected = reports[rule.slice(PREFIX.length)] ?? 0;
      const recorded = findings.filter(finding => finding.path === file && finding.rule === rule).length;
      if (recorded !== expected) mismatches.push(`${file} ${rule}: expected ${expected}, recorded ${recorded}`);
    }
  }
  return mismatches;
}

async function recordFixtures() {
  const { fixtures, files, authored, languages, counts } = extractFixtures();
  const { findings, skipped } = await lint(ESLint, fixtures, config(languages), files, RULES);
  for (const { path, reason } of skipped) {
    if (authored.some(({ file }) => file === path)) throw new Error(`AUTHORED ${path} does not parse: ${reason}`);
    rmSync(join(fixtures, path));
    const count = counts[`${PREFIX}${path.split("/")[0]}`];
    count.unparsed = (count.unparsed ?? 0) + 1;
  }
  const mismatches = authoredMismatches(authored, findings);
  if (mismatches.length > 0) throw new Error(`AUTHORED cases the plugin reports otherwise:\n  ${mismatches.join("\n  ")}`);
  const unparsed = new Set(skipped.map(({ path }) => path));
  const kept = files.filter(file => !unparsed.has(file)).sort();
  writeAuthoredRecord([...authored].sort((a, b) => (a.file < b.file ? -1 : a.file > b.file ? 1 : 0)));
  writeRecording(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${CHECKOUT.repository}@${CHECKOUT.commit}`, rules: RULES, files: kept }, findings);
  console.log(`${TOOL} fixtures: ${kept.length} files (${authored.length} Rerail's), ${findings.length} findings`);
  for (const [rule, count] of Object.entries(counts)) console.log(`  ${rule}: ${JSON.stringify(count)}`);
}

installHooks();
const { register } = await import(pathToFileURL(toolsRequire.resolve("tsx/cjs/api")).href);
register();
await recordFixtures();
