// Records eslint-plugin-unicorn for the JavaScript pack's unicorn ports (packs/javascript/conformance.json), in the
// format rerail/conformance/languages/pack.ts reads: eslint-plugin-unicorn/fixtures.expected.json.
// Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release or a re-snapshot):
//   node generate.mjs --tools <dir> --unicorn <checkout>
// <dir> holds node_modules with the runtime nera-rule-validation pins for eslint-plugin-unicorn 76.0.0
// (validation/upstream/eslint-plugin-unicorn-76.0.0): eslint 10.11.0, eslint-plugin-unicorn 76.0.0,
// @typescript-eslint/parser 8.70.0 and the test-time packages unicorn's tests import. The checkout is
// github.com/sindresorhus/eslint-plugin-unicorn at v76.0.0 (f70155ebe24dbb35b9814c3dbb3ab3620e09c3e1); only its
// test/<rule>.js files are read, and the plugin linted is the published one in <dir>.
//
// Each rule's test file is imported with test/utils/test.js and ava replaced by stubs that record the cases its
// `test(…)`, `test.snapshot(…)` and `test.typescript(…)` calls declare. Every case is linted with every pack rule of
// the plugin on at its default options (a case's own options are ignored, as the pack's ports ignore
// them), under ESLint's defaults plus the case's own ecmaVersion, sourceType and parserOptions;
// typescript-eslint's parser where the case names it, and cases for any other parser (vue, svelte, html) are left
// out. The case's globals and settings are left out. A case's code is written to
// eslint-plugin-unicorn/fixtures/<rule>/<n>.<ext>, or, for a case with a `filename` (filename-case's tests are file
// names), to <rule>/<n>/<filename>; a case named outside the ecma family's extensions (`.vue`, `.md`) is left out,
// since Rerail does not read it as JavaScript. Code that does not parse is left out. Each kept file is one analysis unit.
// Tests that build a `filename` from process.cwd() (filename-case's `path.join(cwd, …)` and `outsideCwd`) are imported
// under `withUpstreamTestCwd`, so the expected findings never carry the maintainer's own directory, neither in a fixture path (which
// must stay short enough to check out on Windows) nor in the case's code; `assertPortableCase` refuses one that would.
// `source.units` names each file's origin, the rule whose test file it came from; each such file also gets a rename
// variant under renamed/ (scripts/pack-data/eslint-expected.mjs `writeRenameVariants`), linted in the original's
// language and marked `renamedFrom` it.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { dirname, join, normalize, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  assertCommit,
  assertPortableCase,
  isVariantPath,
  lint,
  pinnedRequire,
  withOrigins,
  withUpstreamTestCwd,
  writeExpectedFindings,
  writeRenameVariants,
} from "../../../../scripts/pack-data/eslint-expected.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = JSON.parse(readFileSync(join(HERE, "..", "..", "conformance.json"), "utf8"));
const TOOL = "eslint-plugin-unicorn";
const PINS = { eslint: "10.11.0", "eslint-plugin-unicorn": "76.0.0", "@typescript-eslint/parser": "8.70.0" };
const CHECKOUT = { repository: "https://github.com/sindresorhus/eslint-plugin-unicorn", commit: "f70155ebe24dbb35b9814c3dbb3ab3620e09c3e1" };
const PREFIX = "unicorn/";
/** The ecma family's extensions (`CODE_RULE_DIALECTS`): a named case outside them is not JavaScript to Rerail. */
const ECMA_EXTENSIONS = [".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (["--tools", "--unicorn"].includes(flag)) options[flag.slice(2)] = resolve(argv[++index]);
    else throw new Error(`unknown argument ${flag}`);
  }
  for (const key of ["tools", "unicorn"]) if (options[key] === undefined) throw new Error(`--${key} is required`);
  return options;
}

const options = parseArguments(process.argv.slice(2));
const toolsRequire = pinnedRequire(options.tools, PINS);
assertCommit(options.unicorn, CHECKOUT.commit);
const { ESLint } = toolsRequire("eslint");
const tsParser = toolsRequire("@typescript-eslint/parser");
const plugin = (await import(pathToFileURL(toolsRequire.resolve("eslint-plugin-unicorn")).href)).default;

/** The pack's unicorn rules, as ESLint reports them (`unicorn/no-null`). */
const RULES = PACK.rules.filter(rule => rule.upstream.tool === TOOL).map(rule => rule.upstream.rule).sort();
const VERSION = PACK.tools.find(entry => entry.name === TOOL).version;

// ---------------------------------------------------------------------------------------------------------------
// Stubs: unicorn's test/utils/test.js and ava, which record the cases instead of running them.

const RUNS = [];
const STUB = Symbol.for("rerail.javascriptPack.unicornTester");
globalThis[STUB] = {
  getTester(importMeta) {
    const rule = importMeta.url.slice(importMeta.url.lastIndexOf("/") + 1).replace(/\.js$/u, "");
    const record = parser => tests => RUNS.push({ rule, parser, tests });
    const test = record(undefined);
    test.snapshot = record(undefined);
    test.typescript = record("typescript");
    for (const name of ["vue", "svelte", "html"]) test[name] = record(name);
    test.only = item => (typeof item === "string" ? { code: item } : item);
    return { test };
  },
};

function installHooks() {
  const shim = (name, body) => {
    const path = join(options.tools, ".generate-shims", name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, body);
    return pathToFileURL(path).href;
  };
  const testerShim = shim("unicorn-test-utils.mjs", `export const getTester = globalThis[Symbol.for("rerail.javascriptPack.unicornTester")].getTester;\n`);
  const avaShim = shim("ava.mjs", `const test = () => undefined;\nfor (const name of ["serial", "skip", "only", "todo", "failing"]) test[name] = () => undefined;\nexport default test;\n`);
  const testUtils = pathToFileURL(join(options.unicorn, "test", "utils", "test.js")).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "ava") return { url: avaShim, format: "module", shortCircuit: true };
      let resolved;
      try {
        resolved = nextResolve(specifier, context);
      } catch (error) {
        // The checkout has no node_modules: a bare specifier resolves from the tools directory.
        if (specifier.startsWith(".") || specifier.startsWith("/")) throw error;
        resolved = { url: pathToFileURL(toolsRequire.resolve(specifier)).href, shortCircuit: true };
      }
      if (resolved.url === testUtils) return { url: testerShim, format: "module", shortCircuit: true };
      return resolved;
    },
  });
}

/** The case's own language beyond its extension: ecmaVersion, sourceType and parserOptions, or nothing. */
function languageOf(languageOptions) {
  const own = {};
  for (const key of ["ecmaVersion", "sourceType", "parserOptions"]) if (languageOptions[key] !== undefined) own[key] = languageOptions[key];
  return Object.keys(own).length === 0 ? undefined : own;
}

/** A case's `filename` as a relative path under its fixture directory, undefined without one, null when unusable. */
function relativeFilename(filename) {
  if (typeof filename !== "string" || filename.length === 0) return undefined;
  // A name with a control character or a line separator (filename-case tests line breaks, U+2028 and U+2029) is no path
  // a repository lists.
  if (/[\u0000-\u001f\u007f\u2028\u2029]/u.test(filename)) return null;
  const path = normalize(filename.replace(/\\/gu, "/").replace(/^[A-Za-z]:/u, "")).replace(/^(\.\.\/|\/)+/u, "");
  return path.length === 0 || path === "." ? undefined : path;
}

/** Every kept case of the pack's unicorn rules, written under `fixtures/<rule>/`. */
async function extractFixtures() {
  const fixtures = join(HERE, "fixtures");
  rmSync(fixtures, { recursive: true, force: true });
  const files = [];
  const languages = new Map();
  const counts = {};
  for (const rule of RULES) {
    const name = rule.slice(PREFIX.length);
    RUNS.length = 0;
    await withUpstreamTestCwd(() => import(pathToFileURL(join(options.unicorn, "test", `${name}.js`)).href));
    const runs = RUNS.filter(run => run.rule === name);
    if (runs.length === 0) throw new Error(`${rule}: test/${name}.js declared no cases`);
    const seen = new Set();
    const count = (counts[rule] = { cases: 0, withOptions: 0, skipped: {}, written: 0, duplicate: 0 });
    for (const run of runs) {
      for (const kind of ["valid", "invalid"]) {
        for (const entry of run.tests[kind] ?? []) {
          const test = typeof entry === "string" ? { code: entry } : entry;
          count.cases += 1;
          if (Array.isArray(test.options) && test.options.length > 0) count.withOptions += 1;
          const parser = run.parser ?? (test.languageOptions?.parser === undefined ? undefined : "custom");
          if (parser !== undefined && parser !== "typescript") {
            count.skipped["custom parser"] = (count.skipped["custom parser"] ?? 0) + 1;
            continue;
          }
          const { parser: _parser, globals: _globals, ...own } = { ...run.tests.testerOptions?.languageOptions, ...test.languageOptions };
          const language = languageOf(own);
          const jsx = own.parserOptions?.ecmaFeatures?.jsx === true;
          const named = relativeFilename(test.filename);
          if (named === null || (named !== undefined && !ECMA_EXTENSIONS.some(extension => named.endsWith(extension)))) {
            count.skipped["not an ecma file"] = (count.skipped["not an ecma file"] ?? 0) + 1;
            continue;
          }
          const extension = parser === "typescript" ? (jsx ? ".tsx" : ".ts") : jsx ? ".jsx" : ".js";
          const key = `${parser}\0${named}\0${JSON.stringify(language ?? null)}\0${test.code}`;
          if (seen.has(key)) {
            count.duplicate += 1;
            continue;
          }
          seen.add(key);
          const n = String(seen.size).padStart(3, "0");
          const file = named === undefined ? `${name}/${n}${extension}` : `${name}/${n}/${named}`;
          assertPortableCase(file, test.code);
          mkdirSync(dirname(join(fixtures, file)), { recursive: true });
          writeFileSync(join(fixtures, file), test.code);
          files.push(file);
          const group = JSON.stringify({ language: language ?? null, typescript: parser === "typescript" });
          if (!languages.has(group)) languages.set(group, []);
          languages.get(group).push(file);
          count.written += 1;
        }
      }
    }
  }
  return { fixtures, files, languages, counts };
}

installHooks();
const { fixtures, files, languages, counts } = await extractFixtures();
// Each file is a case of the rule whose test file it came from (`<rule>/…`); its rename variant joins its language group.
const origins = new Map(files.map(file => [file, [`${PREFIX}${file.split("/")[0]}`]]));
const { variants, skipped: unrenamed } = writeRenameVariants(fixtures, files, origins);
const variantOf = new Map(variants.map(variant => [variant.of, variant.unit]));
const config = [
  {
    files: ["**/*"],
    plugins: { unicorn: plugin },
    linterOptions: { reportUnusedDisableDirectives: "off" },
    rules: Object.fromEntries(RULES.map(rule => [rule, "error"])),
  },
  ...[...languages].map(([group, members]) => {
    const { language, typescript } = JSON.parse(group);
    return { files: [...members, ...members.flatMap(member => variantOf.get(member) ?? [])], languageOptions: { ...(typescript ? { parser: tsParser } : {}), ...(language ?? {}) } };
  }),
];
const linted = await lint(ESLint, fixtures, config, [...files, ...variants.map(variant => variant.unit)], RULES);
for (const { path } of linted.skipped.filter(({ path }) => !isVariantPath(path))) {
  rmSync(join(fixtures, path));
  const count = counts[`${PREFIX}${path.split("/")[0]}`];
  count.unparsed = (count.unparsed ?? 0) + 1;
}
const unparsed = new Set(linted.skipped.map(({ path }) => path));
const kept = files.filter(file => !unparsed.has(file)).sort();
const { files: units, units: origin, findings } = withOrigins(fixtures, kept, origins, variants, unparsed, linted.findings);
writeExpectedFindings(join(HERE, "fixtures.expected.json"), TOOL, VERSION, { kind: "fixtures", origin: `${CHECKOUT.repository}@${CHECKOUT.commit}`, rules: RULES, files: units, units: origin }, findings);
console.log(`${TOOL} fixtures: ${kept.length} files, ${units.length - kept.length} rename variants (${unrenamed.length} units without one), ${findings.length} findings`);
for (const [rule, count] of Object.entries(counts)) console.log(`  ${rule}: ${JSON.stringify(count)}`);
