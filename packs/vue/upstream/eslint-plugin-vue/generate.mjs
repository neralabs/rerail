// Records eslint-plugin-vue 10.11.1 for the rules the Vue pack ports (../../conformance.json). Run by
// `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI), with
// the tools of ../../../../fixtures/vue/tools.mjs installed in `<checkout>/.rerail/tools/vue`:
//
//   node generate.mjs fixtures <eslint-plugin-vue checkout at tag v10.11.1 (ec9c53a6fbed637d7b0fbcd6893b0206ae32839f)>
//
// Pins (refused when different): eslint 10.11.0, eslint-plugin-vue 10.11.1, vue-eslint-parser 10.4.1,
// @typescript-eslint/parser 8.70.1, typescript 6.0.3.
//
// One ESLint configuration lints everything (`CONFIG` below): eslint-plugin-vue's `flat/base` (vue-eslint-parser,
// the `vue/vue` processor and `vue/comment-directive`, so `<!-- eslint-disable -->` in a template works as it does
// for the plugin's users) and every rule of ../../conformance.json at its default options. `<script lang="ts">` and
// `lang="tsx"` go through @typescript-eslint/parser (JSX only for tsx, so `<T>x` assertions parse), other scripts
// through espree with JSX on (Options API render functions).
//
// `fixtures` reads each rule's own RuleTester cases (`read_tests.mjs`) and keeps those this configuration
// reproduces: a case with the rule's default options (or options `DEFAULT_EQUIVALENT_OPTIONS` lists as spelling
// them), no settings and no type information, whose findings for its
// rule here are exactly the upstream expectation (the number of errors of an invalid case, at the expected line and
// column where the case gives them; none for a valid one). A case's code is written verbatim as
// `fixtures/<rule>/<valid|invalid>-<index>.vue` when the case is a .vue file (a `.vue` filename, or none, which
// vue-eslint-parser reads as an SFC); a script-only case (`filename: "*.js"`) is wrapped as
// `<script>\n<code>\n</script>` and kept only if it still reproduces. `cases.json` lists every case with its fate.
// `authored.mjs` holds Rerail's own cases, inputs a port once misread; each is written as
// `fixtures/<rule>/rerail-<name>.vue` and listed in `UPSTREAM.md` beside this file. Then every fixture is linted by every
// rule, and `fixtures.expected.json` records all findings (a fixture of one rule may also be reported by another).
// Its `source.units` names each upstream case's origin: the rule whose RuleTester cases it came from (Rerail's own
// cases have none). Last, upstream's LICENSE is copied to ../../licenses/eslint-plugin-vue/, and provenance.json and
// LICENSES.md record each kept case's upstream test file and case.
//
// Findings are `{ rule, path, line, column }`: ESLint's line, and its 1-based column converted from UTF-16 code
// units to code points, as the conformance standard counts them.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { writeProvenance } from "../../../../scripts/pack-data/provenance.mjs";
import { packFixturesWritten, expectedFindingsWritten } from "../../../../scripts/s3/pack-fixtures.mjs";
import { load, toolsDirectory, TOOLS } from "../../../../fixtures/vue/tools.mjs";
import { AUTHORED } from "./authored.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOLS_DIR = toolsDirectory(HERE);
const RULES = JSON.parse(readFileSync(join(HERE, "../../conformance.json"), "utf8")).rules;

/**
 * Per upstream rule, option lists a case may carry and still be reproduced by `CONFIG`, because they spell the rule's
 * defaults (any other non-empty options make `fixtures` refuse the case as "non-default options").
 */
const DEFAULT_EQUIVALENT_OPTIONS = {
  "vue/no-duplicate-attributes": [[{ allowCoexistStyle: true }]],
  "vue/valid-v-for": [[{ allowEmptyAlias: false }]],
  "vue/no-v-text-v-html-on-component": [[{ ignoreElementNamespaces: false }]],
  "vue/v-on-event-hyphenation": [["always", { autofix: true }]],
  "vue/attribute-hyphenation": [["always"], ["always", { ignore: [] }]],
  "vue/return-in-computed-property": [[{ treatUndefinedAsUnspecified: true }]],
};
const UPSTREAM_TAG = "v10.11.1";
const UPSTREAM_COMMIT = "ec9c53a6fbed637d7b0fbcd6893b0206ae32839f";
const ORIGIN = `https://github.com/vuejs/eslint-plugin-vue@${UPSTREAM_COMMIT}`;

const { Linter } = await load(TOOLS_DIR, "eslint");
const pluginVue = (await load(TOOLS_DIR, "eslint-plugin-vue")).default;
const vueParser = await load(TOOLS_DIR, "vue-eslint-parser");
const tsParser = await load(TOOLS_DIR, "@typescript-eslint/parser");
await load(TOOLS_DIR, "typescript");

const typescript = jsx => ({
  meta: { name: `@typescript-eslint/parser (jsx: ${jsx})` },
  parseForESLint: (code, options) => tsParser.parseForESLint(code, { ...options, ecmaFeatures: { ...options?.ecmaFeatures, jsx } }),
});

const ruleIds = RULES.map(rule => rule.upstream.rule);
const CONFIG = [
  ...pluginVue.configs["flat/base"],
  {
    files: ["**/*.vue"],
    languageOptions: {
      parser: vueParser,
      ecmaVersion: "latest",
      sourceType: "module",
      parserOptions: { parser: { ts: typescript(false), tsx: typescript(true) }, ecmaFeatures: { jsx: true } },
    },
    rules: Object.fromEntries(ruleIds.map(id => [id, "error"])),
  },
];

const linter = new Linter({ configType: "flat" });

/** ESLint's messages for `code` as `path`, split into findings and the fatal message, if any. */
function lint(code, path) {
  const messages = linter.verify(code, CONFIG, path);
  const fatal = messages.find(message => message.fatal);
  const lines = code.split(/\r\n|[\r\n\u2028\u2029]/);
  const findings = messages.filter(message => !message.fatal && ruleIds.includes(message.ruleId)).map(message => ({
    rule: message.ruleId,
    path,
    line: message.line,
    column: [...(lines[message.line - 1] ?? "").slice(0, message.column - 1)].length + 1,
    utf16Column: message.column,
  }));
  return { findings, fatal };
}

const strip = ({ utf16Column, ...finding }) => finding;
const byPosition = (a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.column - b.column || a.rule.localeCompare(b.rule);

// The parser stack's pins (eslint, vue-eslint-parser, @typescript-eslint/parser, typescript) are the pack's
// `conformance.json` `tools`; an expected-findings file names its tool alone, as `expectedFindingsSchema` requires.
function write(file, source, findings) {
  const out = {
    tool: "eslint-plugin-vue",
    version: TOOLS["eslint-plugin-vue"],
    source,
    findings: findings.map(strip).sort(byPosition),
  };
  writeFileSync(join(HERE, file), `${JSON.stringify(out, null, 1)}\n`);
  console.log(`wrote ${file}: ${out.findings.length} findings`);
  console.log(expectedFindingsWritten(join(HERE, file), source.kind));
}

// --- fixtures ----------------------------------------------------------------------------------------------------

/** Why a case cannot be reproduced by `CONFIG`, or undefined. */
function refusal(testCase, run, rule) {
  const options = testCase.options ?? [];
  const equivalent = (DEFAULT_EQUIVALENT_OPTIONS[rule.upstream.rule] ?? []).some(list => JSON.stringify(list) === JSON.stringify(options));
  if (options.length > 0 && !equivalent) return "non-default options";
  if (testCase.settings !== undefined) return "settings";
  const languageOptions = { ...run.config.languageOptions, ...testCase.languageOptions };
  if (JSON.stringify(languageOptions).includes("project")) return "type-aware (parserOptions.project)";
  if (languageOptions.globals !== undefined) return "globals";
  if (![undefined, "vue-eslint-parser", "espree", "@typescript-eslint/parser"].includes(languageOptions.parser)) {
    return `parser ${JSON.stringify(languageOptions.parser)}`;
  }
  if ((testCase.code ?? "").trim() === "") return "empty code";
  return undefined;
}

/**
 * The form a case's code takes as a `.vue` fixture: `[undefined]` (verbatim SFC) or `[lang]` of the `<script>` that
 * wraps it. vue-eslint-parser reads a `.vue` filename as an SFC; with no filename it reads code holding an SFC block
 * as an SFC and other code as a script (the rules' invalid cases show both). Any other parser (ESLint's default
 * espree, or @typescript-eslint/parser) or filename reads the code as one script, whose content the plugin treats as
 * a `.vue` file's `<script>` when the filename is `.vue`.
 */
function forms(testCase, run) {
  const parser = { ...run.config.languageOptions, ...testCase.languageOptions }.parser;
  const wrap = parser === "@typescript-eslint/parser" ? "ts" : "js";
  if (parser !== "vue-eslint-parser") return [wrap];
  if (testCase.filename?.endsWith(".vue")) return [undefined];
  if (testCase.filename !== undefined) return [wrap];
  return /<(template|script|style)\b/.test(testCase.code) ? [undefined] : [wrap];
}

/** Whether `findings` (of the case's rule) are exactly the case's expectation. */
function reproduces(testCase, kind, findings, lineOffset) {
  if (kind === "valid") return findings.length === 0;
  const expected = typeof testCase.errors === "number" ? Array.from({ length: testCase.errors }, () => ({})) : testCase.errors;
  if (findings.length !== expected.length) return false;
  const got = findings.map(f => [f.line - lineOffset, f.utf16Column]).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const want = expected.map(e => [e.line, e.column]).sort((a, b) => (a[0] ?? 0) - (b[0] ?? 0) || (a[1] ?? 0) - (b[1] ?? 0));
  return want.every(([line, column], index) => (line === undefined || line === got[index][0])
    && (column === undefined || column === got[index][1]));
}

function fixtures(checkout) {
  const head = execFileSync("git", ["-C", checkout, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (head !== UPSTREAM_COMMIT) throw new Error(`${checkout} is at ${head}; check out ${UPSTREAM_TAG} (${UPSTREAM_COMMIT})`);
  const read = JSON.parse(execFileSync(process.execPath, [join(HERE, "read_tests.mjs"), checkout, TOOLS_DIR,
    ...RULES.map(rule => rule.upstream.rule.replace(/^vue\//, ""))], { encoding: "utf8", maxBuffer: 1 << 28 }));
  const root = join(HERE, "fixtures");
  rmSync(root, { recursive: true, force: true });
  const cases = [];
  const files = [];
  const units = {};
  for (const rule of RULES) {
    const { upstream } = rule;
    const name = upstream.rule.replace(/^vue\//, "");
    read[name].forEach((run, runIndex) => {
      for (const kind of ["valid", "invalid"]) {
        run[kind].forEach((testCase, index) => {
          const id = `${kind}-${String(index).padStart(3, "0")}${read[name].length > 1 ? `-run${runIndex}` : ""}`;
          const entry = { rule: upstream.rule, case: `${kind}/${index}`, upstream: `tests/lib/rules/${name}.test.ts` };
          const why = refusal(testCase, run, rule);
          if (why !== undefined) {
            cases.push({ ...entry, kept: false, reason: why });
            return;
          }
          const path = `${name}/${id}.vue`;
          let outcome;
          for (const lang of forms(testCase, run)) {
            const open = lang === undefined ? "" : lang === "ts" ? `<script lang="ts">\n` : `<script>\n`;
            const code = lang === undefined ? testCase.code : `${open}${testCase.code}\n</script>\n`;
            const script = lang !== undefined;
            const { findings, fatal } = lint(code, path);
            if (fatal !== undefined) {
              outcome = { reason: `parse error here: ${fatal.message}` };
              continue;
            }
            const own = findings.filter(finding => finding.rule === upstream.rule);
            if (!reproduces(testCase, kind, own, script ? 1 : 0)) {
              outcome = { reason: `not reproduced: ${own.length} finding(s) here` };
              continue;
            }
            outcome = { code, lang };
            break;
          }
          if (outcome.code === undefined) {
            cases.push({ ...entry, kept: false, reason: outcome.reason });
            return;
          }
          mkdirSync(join(root, name), { recursive: true });
          writeFileSync(join(root, path), outcome.code);
          files.push(path);
          units[path] = { rules: [upstream.rule] };
          cases.push({ ...entry, kept: true, file: `fixtures/${path}`,
            copy: outcome.lang === undefined ? "verbatim" : `derived: wrapped in <script${outcome.lang === "ts" ? ' lang="ts"' : ""}>` });
        });
      }
    });
  }
  // Rerail's own cases (authored.mjs): inputs the port once misread, scored against what the plugin reports on them.
  const authored = [];
  for (const { rule, name, code } of AUTHORED) {
    const path = `${rule}/rerail-${name}.vue`;
    const { fatal } = lint(code, path);
    if (fatal !== undefined) throw new Error(`authored ${path} does not parse: ${fatal.message}`);
    mkdirSync(join(root, rule), { recursive: true });
    writeFileSync(join(root, path), code);
    files.push(path);
    authored.push(path);
  }
  const findings = files.flatMap(path => lint(readFileSync(join(root, path), "utf8"), path).findings);
  writeAuthoredRecord(authored);
  writeFileSync(join(HERE, "cases.json"), `${JSON.stringify({ origin: ORIGIN, cases }, null, 1)}\n`);
  console.log(packFixturesWritten([join(HERE, "cases.json")]));
  const sortedUnits = Object.fromEntries(Object.entries(units).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
  write("fixtures.expected.json", { kind: "fixtures", origin: ORIGIN, rules: ruleIds, files, units: sortedUnits }, findings);
  writeLicenseAndProvenance(checkout, cases.filter(c => c.kept));
  const kept = cases.filter(c => c.kept).length;
  console.log(`cases: ${kept} kept of ${cases.length}`);
}

/** UPSTREAM.md beside this generator: the fixtures Rerail wrote, which are Rerail's, not eslint-plugin-vue's. */
function writeAuthoredRecord(authored) {
  const rows = authored.map(path => `| \`upstream/eslint-plugin-vue/fixtures/${path}\` | written by Rerail |`);
  writeFileSync(join(HERE, "UPSTREAM.md"), `# eslint-plugin-vue ${TOOLS["eslint-plugin-vue"]}: cases Rerail wrote

The fixtures listed below were written by Rerail for the pinned plugin to score, beside the cases extracted from the plugin's own tests (\`licenses/eslint-plugin-vue/UPSTREAM.md\`). Each is an input a port once misread; \`authored.mjs\` holds them and \`generate.mjs fixtures\` records the plugin's findings on them in \`fixtures.expected.json\`. They are Rerail's (Apache-2.0, the pack's licence), so nothing goes under \`licenses/\`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
${rows.join("\n")}
`);
}

/**
 * `packs/vue/licenses/eslint-plugin-vue/LICENSE` (upstream's, verbatim) and the tool's provenance.json and
 * LICENSES.md: every kept case (a `cases.json` entry) with its upstream test file and case, copied verbatim or wrapped.
 */
function writeLicenseAndProvenance(checkout, kept) {
  const dir = join(HERE, "../../licenses/eslint-plugin-vue");
  mkdirSync(dir, { recursive: true });
  copyFileSync(join(checkout, "LICENSE"), join(dir, "LICENSE"));
  const source = "eslint-plugin-vue";
  const entries = new Map(
    kept.map(c => [
      c.file,
      c.copy === "verbatim"
        ? { source, from: c.upstream, how: "verbatim", detail: `case \`${c.case}\`` }
        : { source, from: c.upstream, how: "adapted", detail: `case \`${c.case}\`; ${c.copy}` },
    ]),
  );
  writeProvenance({ pack: "vue", tool: source, entries, commits: { [source]: UPSTREAM_COMMIT } });
}

const [mode, ...rest] = process.argv.slice(2).filter((arg, index, all) => arg !== "--tools" && all[index - 1] !== "--tools");
if (mode === "fixtures") fixtures(resolve(rest[0]));
else throw new Error("usage: node generate.mjs fixtures <eslint-plugin-vue checkout>");
