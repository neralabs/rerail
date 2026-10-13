// Records eslint-plugin-vue 10.11.1 for the rules the Vue pack will port (../../conformance.json). Run offline by a
// maintainer, never in CI, with the tools of ../../../fixtures/vue/tools.mjs installed in `<checkout>/.rerail/tools/vue`:
//
//   node generate.mjs fixtures <eslint-plugin-vue checkout at tag v10.11.1 (ec9c53a6fbed637d7b0fbcd6893b0206ae32839f)>
//   node generate.mjs corpus [<name>...]            # corpora of ../../../fixtures/vue/corpora.json, under .rerail/corpora/vue
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
// Then every fixture is linted by every rule, and `fixtures.expected.json` records all findings (a fixture of one
// rule may also be reported by another).
//
// `corpus` lints every `.vue` file of each corpus (the files T1 enumerates) and writes `corpus-<name>.expected.json`;
// a file ESLint cannot parse (a fatal message) is `skipped`.
//
// Findings are `{ rule, path, line, column }`: ESLint's line, and its 1-based column converted from UTF-16 code
// units to code points, as the conformance standard counts them.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { corporaDirectory, load, toolsDirectory, TOOLS } from "../../../../fixtures/vue/tools.mjs";

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
// `conformance.json` `tools`; a recording names its tool alone, as `recordingSchema` requires.
function write(file, source, findings) {
  const out = {
    tool: "eslint-plugin-vue",
    version: TOOLS["eslint-plugin-vue"],
    source,
    findings: findings.map(strip).sort(byPosition),
  };
  writeFileSync(join(HERE, file), `${JSON.stringify(out, null, 1)}\n`);
  console.log(`wrote ${file}: ${out.findings.length} findings`);
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
          cases.push({ ...entry, kept: true, file: `fixtures/${path}`,
            copy: outcome.lang === undefined ? "verbatim" : `derived: wrapped in <script${outcome.lang === "ts" ? ' lang="ts"' : ""}>` });
        });
      }
    });
  }
  const findings = files.flatMap(path => lint(readFileSync(join(root, path), "utf8"), path).findings);
  writeFileSync(join(HERE, "cases.json"), `${JSON.stringify({ origin: ORIGIN, cases }, null, 1)}\n`);
  writeLicense(checkout, cases.filter(c => c.kept));
  write("fixtures.expected.json", { kind: "fixtures", origin: ORIGIN, rules: ruleIds, files }, findings);
  const kept = cases.filter(c => c.kept).length;
  console.log(`cases: ${kept} kept of ${cases.length}`);
}

/** `packs/vue/licenses/eslint-plugin-vue/`: the upstream LICENSE, verbatim, and a manifest of every copy. */
function writeLicense(checkout, kept) {
  const dir = join(HERE, "../../licenses/eslint-plugin-vue");
  mkdirSync(dir, { recursive: true });
  copyFileSync(join(checkout, "LICENSE"), join(dir, "LICENSE"));
  const rows = kept.map(c => `| \`upstream/eslint-plugin-vue/${c.file}\` | \`${c.upstream}\` case \`${c.case}\` | ${c.copy} |`);
  writeFileSync(join(dir, "UPSTREAM.md"), `# eslint-plugin-vue ${TOOLS["eslint-plugin-vue"]}

Upstream: [vuejs/eslint-plugin-vue](https://github.com/vuejs/eslint-plugin-vue/tree/${UPSTREAM_COMMIT}) at tag \`${UPSTREAM_TAG}\` (commit \`${UPSTREAM_COMMIT}\`). License: MIT (Copyright (c) 2017 Toru Nagashima); [\`LICENSE\`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the code of RuleTester cases of the rules the Vue pack will port (\`../../conformance.json\`), and remain under eslint-plugin-vue's license. Each is one case's \`code\`, copied verbatim, or wrapped in a \`<script>\` element when the case is a script (a non-\`.vue\` filename, a parser other than vue-eslint-parser, or code with no SFC block and no filename), so that it is a \`.vue\` file. Every other file under \`upstream/\` is Rerail's, under the Apache License 2.0 (this repository's \`LICENSE\`): \`read_tests.mjs\`, \`generate.mjs\`, \`cases.json\` and the recordings they write by running ESLint ${TOOLS.eslint} with eslint-plugin-vue ${TOOLS["eslint-plugin-vue"]}.

This file is written by \`upstream/eslint-plugin-vue/generate.mjs fixtures\`. Paths are relative to the pack directory (\`packs/vue/\`).

| File | Upstream case | Copy |
|---|---|---|
${rows.join("\n")}
`);
}

// --- corpora -----------------------------------------------------------------------------------------------------

function vueFiles(root, paths) {
  const out = [];
  const walk = dir => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === "node_modules" || entry.name === "dist" || entry.name === ".git") continue;
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile() && entry.name.endsWith(".vue")) out.push(relative(root, path));
    }
  };
  for (const path of paths) {
    const full = join(root, path);
    if (statSync(full, { throwIfNoEntry: false })?.isDirectory()) walk(full);
  }
  return out.sort();
}

function corpus(names) {
  const corpora = JSON.parse(readFileSync(join(HERE, "../../../../fixtures/vue/corpora.json"), "utf8"));
  const base = corporaDirectory(HERE);
  for (const corpus of corpora) {
    if (names.length > 0 && !names.includes(corpus.name)) continue;
    const root = join(base, corpus.name);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    if (head !== corpus.commit) throw new Error(`${root} is at ${head}; corpora.json pins ${corpus.commit}`);
    const files = vueFiles(root, corpus.paths);
    const findings = [];
    const skipped = [];
    for (const path of files) {
      const result = lint(readFileSync(join(root, path), "utf8"), path);
      if (result.fatal !== undefined) skipped.push(path);
      else findings.push(...result.findings);
    }
    write(`corpus-${corpus.name}.expected.json`, {
      kind: "corpus", name: corpus.name, repository: corpus.repository, commit: corpus.commit, rules: ruleIds, skipped,
    }, findings);
  }
}

const [mode, ...rest] = process.argv.slice(2).filter((arg, index, all) => arg !== "--tools" && all[index - 1] !== "--tools"
  && arg !== "--corpora" && all[index - 1] !== "--corpora");
if (mode === "fixtures") fixtures(resolve(rest[0]));
else if (mode === "corpus") corpus(rest);
else throw new Error("usage: node generate.mjs fixtures <eslint-plugin-vue checkout> | corpus [<name>...]");
