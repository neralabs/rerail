// Shared by the React pack's recorders (eslint-plugin-react/generate.mjs, eslint-plugin-react-hooks/generate.mjs):
// the pins, the one ESLint configuration every recording is made with, fixture extraction from the upstream rule
// tests (a stub RuleTester captures `run(name, rule, { valid, invalid })`), and the recording runs.
// Run offline by a maintainer, never in CI.
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { assertCommit, assertPortableCase, corpusFiles, lint, pack, pinnedRequire, REPOSITORY, writeRecording } from "../../../conformance/recorders/eslint-record.mjs";

export const UPSTREAM_DIRECTORY = dirname(fileURLToPath(import.meta.url));

/** Exact versions every recording is made with (`.rerail/tools/react/package.json`). */
export const PINS = {
  eslint: "9.39.5",
  "eslint-plugin-react": "7.37.5",
  "eslint-plugin-react-hooks": "7.1.1",
  "@typescript-eslint/parser": "8.70.1",
  typescript: "5.9.3",
};

/** The chosen upstream rules, per tool (the pack's `tools[].name`). Every recording enables all of them. */
export const RULES = {
  "eslint-plugin-react": [
    "react/button-has-type",
    "react/jsx-key",
    "react/jsx-no-comment-textnodes",
    "react/jsx-no-duplicate-props",
    "react/jsx-no-target-blank",
    "react/jsx-no-undef",
    "react/no-array-index-key",
    "react/no-children-prop",
    "react/no-danger",
    "react/no-danger-with-children",
    "react/no-direct-mutation-state",
    "react/no-find-dom-node",
    "react/no-is-mounted",
    "react/no-render-return-value",
    "react/no-string-refs",
    "react/no-unescaped-entities",
    "react/react-in-jsx-scope",
    "react/self-closing-comp",
    "react/void-dom-elements-no-children",
  ],
  "eslint-plugin-react-hooks": ["react-hooks/rules-of-hooks", "react-hooks/exhaustive-deps"],
};
export const ALL_RULES = Object.values(RULES).flat();

/** The upstream checkouts the fixtures are extracted from, at the pinned commits. */
export const UPSTREAM = {
  "eslint-plugin-react": {
    title: "eslint-plugin-react 7.37.5",
    repository: "https://github.com/jsx-eslint/eslint-plugin-react",
    reference: "tag `v7.37.5`",
    commit: "2c98b83c451a4297edf1787d9a616e50687e27e8",
    checkout: "upstream-eslint-plugin-react",
    project: "eslint-plugin-react",
    license: "MIT (Copyright (c) 2014 Yannick Croissant)",
  },
  "eslint-plugin-react-hooks": {
    title: "eslint-plugin-react-hooks 7.1.1",
    repository: "https://github.com/facebook/react",
    reference: "the `gitHead` of the published eslint-plugin-react-hooks 7.1.1",
    commit: "1ddff43c41147b880c22eb363e07aade5a71c5d9",
    checkout: "upstream-react",
    project: "react",
    license: "MIT (Copyright (c) Meta Platforms, Inc. and affiliates.)",
  },
};

/** The nearest ancestor of the repository holding `.rerail/<kind>/react` (sub-worktrees live under `.rerail/wt/`). */
function checkoutDirectory(kind) {
  for (let directory = REPOSITORY; ; directory = dirname(directory)) {
    const candidate = join(directory, ".rerail", kind, "react");
    if (existsSync(candidate)) return candidate;
    if (dirname(directory) === directory) throw new Error(`no .rerail/${kind}/react above ${REPOSITORY}`);
  }
}

/** `--tools <dir>` / `--corpora <dir>` override the checkout's `.rerail/tools/react` / `.rerail/corpora/react`. */
export function options(argv) {
  const value = flag => {
    const index = argv.indexOf(flag);
    return index === -1 ? undefined : resolve(argv[index + 1]);
  };
  return { tools: value("--tools") ?? checkoutDirectory("tools"), corpora: value("--corpora") ?? checkoutDirectory("corpora") };
}

export const JS_FILES = ["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"];
export const TS_FILES = ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"];

/**
 * The pack's one ESLint configuration: espree (latest, module, JSX) for JavaScript, @typescript-eslint/parser for
 * TypeScript (JSX by extension), every chosen rule at its default options, no settings (eslint-plugin-react then
 * assumes the latest React and the `React` pragma) and no globals.
 */
export function config(require) {
  const react = require("eslint-plugin-react");
  const hooks = require("eslint-plugin-react-hooks");
  const typescript = require("@typescript-eslint/parser");
  return [
    { files: JS_FILES, languageOptions: { ecmaVersion: "latest", sourceType: "module", parserOptions: { ecmaFeatures: { jsx: true } } } },
    { files: TS_FILES, languageOptions: { ecmaVersion: "latest", sourceType: "module", parser: typescript } },
    {
      files: [...JS_FILES, ...TS_FILES],
      plugins: { react, "react-hooks": hooks.default ?? hooks },
      rules: Object.fromEntries(ALL_RULES.map(rule => [rule, "error"])),
    },
  ];
}

export function toolsRequire(tools) {
  return pinnedRequire(tools, PINS);
}

/** Posix path of `path` relative to `root`. */
export function posix(root, path) {
  return relative(root, path).split(sep).join("/");
}

/**
 * Runs a copied upstream test file with `requireStub(id)` answering its `require` calls (the stub RuleTester among
 * them) and `globals` (jest's `describe`/`it`) in scope.
 */
export function execute(path, requireStub, globals = {}) {
  const source = readFileSync(path, "utf8");
  const module = { exports: {} };
  const localRequire = Object.assign(id => requireStub(id), { resolve: id => id });
  const parameters = ["require", "module", "exports", "__filename", "__dirname", ...Object.keys(globals)];
  // eslint-disable-next-line no-new-func -- executing the copied test file is the extraction
  new Function(...parameters, source)(localRequire, module, module.exports, path, dirname(path), ...Object.values(globals));
}

/** Writes one fixture case: `<dir>/<name>.<ext>`, or `<dir>/<name>/<filename>` when the case names its file. */
export function writeCase(fixtures, directory, name, extension, code, filename) {
  const path = filename === undefined ? join(fixtures, directory, `${name}.${extension}`) : join(fixtures, directory, name, filename);
  assertPortableCase(posix(fixtures, path), code);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, code);
  return posix(fixtures, path);
}

export function resetDirectory(path) {
  rmSync(path, { recursive: true, force: true });
  mkdirSync(path, { recursive: true });
}

/**
 * Writes the extracted cases (`{ rule, directory, kind, index, code, errors, origin, filename?, extension?, drop? }`)
 * under `<directory>/fixtures/`, records the pack configuration over them (every chosen rule on every file; the
 * recording holds `tool`'s rules, over the files ESLint parses) and checks each case: the rule's own finding count
 * under the pack configuration against the test's `errors`. The check's report goes to `<tools>/scratch/`.
 */
export async function finishFixtures(require, tool, directory, cases, tools) {
  const { ESLint } = require("eslint");
  const fixtures = join(directory, "fixtures");
  resetDirectory(fixtures);
  const written = [];
  const dropped = [];
  for (const entry of cases) {
    if (entry.drop !== undefined) {
      dropped.push({ origin: entry.origin, why: entry.drop });
      continue;
    }
    const name = `${entry.kind}-${String(entry.index).padStart(3, "0")}`;
    const filename = entry.filename === undefined ? undefined : entry.filename.replace(/^\/+/u, "");
    written.push({ ...entry, path: writeCase(fixtures, entry.directory, name, entry.extension, entry.code, filename) });
  }
  const { findings, skipped } = await lint(ESLint, fixtures, config(require), written.map(entry => entry.path), ALL_RULES);
  const unparsable = new Map(skipped.map(entry => [entry.path, entry.reason]));
  for (const path of unparsable.keys()) rmSync(join(fixtures, path));
  const kept = written.filter(entry => !unparsable.has(entry.path));
  const upstream = UPSTREAM[tool];
  writeRecording(
    join(directory, "fixtures.expected.json"),
    tool,
    PINS[tool],
    { kind: "fixtures", origin: `${upstream.repository}@${upstream.commit}`, rules: RULES[tool], files: kept.map(entry => entry.path) },
    findings.filter(finding => RULES[tool].includes(finding.rule)),
  );
  const mismatches = [];
  for (const entry of kept) {
    const own = findings.filter(finding => finding.path === entry.path && finding.rule === entry.rule).length;
    if (own !== entry.errors) mismatches.push({ origin: entry.origin, path: entry.path, expected: entry.errors, recorded: own, options: entry.test?.options, settings: entry.test?.settings });
  }
  const report = {
    tool,
    extracted: cases.length,
    written: written.length,
    kept: kept.length,
    dropped,
    unparsable: written.filter(entry => unparsable.has(entry.path)).map(entry => ({ origin: entry.origin, path: entry.path, reason: unparsable.get(entry.path) })),
    mismatches,
  };
  mkdirSync(join(tools, "scratch"), { recursive: true });
  writeFileSync(join(tools, "scratch", `${tool}-fixtures-report.json`), `${JSON.stringify(report, null, 1)}\n`);
  console.log(`${tool}: ${cases.length} cases, ${dropped.length} dropped (syntax), ${report.unparsable.length} unparsable, ${kept.length} kept, ${mismatches.length} count mismatches`);
  return { kept, report };
}

/** `licenses/<project>/LICENSE` (verbatim) and `UPSTREAM.md`: every copied file and every extracted fixture. */
export function writeLicense(tool, upstream, copied, kept) {
  const meta = UPSTREAM[tool];
  const directory = join(UPSTREAM_DIRECTORY, "..", "licenses", meta.project);
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "LICENSE"), join(directory, "LICENSE"));
  const rows = [
    ...copied.map(entry => `| \`upstream/${tool}/${entry.file}\` | \`${entry.upstream}\` | verbatim |`),
    ...kept.map(entry => `| \`upstream/${tool}/fixtures/${entry.path}\` | \`${entry.origin}\` | extracted (its \`code\`, verbatim) |`),
  ];
  const text = `# ${meta.title}

Upstream: [${meta.repository.replace("https://github.com/", "")}](${meta.repository}/tree/${meta.commit}) at ${meta.reference} (commit \`${meta.commit}\`). License: ${meta.license}; [\`LICENSE\`](LICENSE) is the upstream license file, copied verbatim at that commit.

The files listed below are the upstream tests of the rules the React pack records (\`conformance.json\`, \`upstream/common.mjs\` \`RULES\`), and the test cases extracted from them, one case per file, and remain under ${meta.project}'s license. Every other file under \`upstream/${tool}/\` is Rerail's, under the Apache License 2.0 (the pack's \`LICENSE\`): \`generate.mjs\` (with its stub RuleTester), \`../common.mjs\` and the recordings they write by running the pinned plugin. The rule documents in \`rules/\` and \`candidates/rules/\` are Rerail's re-implementations of eslint-plugin-react's and eslint-plugin-react-hooks' rules, published under the pack's licence expression, \`Apache-2.0 AND MIT\`, as \`REUSE.toml\` annotates them.

Paths are relative to the pack directory, \`packs/react/\`. \`<file> valid[i]\` / \`invalid[i]\` is the i-th case of the \`valid\` / \`invalid\` list the test file passes to \`RuleTester.run\`.

| File | Upstream path | Copy |
|---|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(directory, "UPSTREAM.md"), text);
}

/** Records the pack configuration over each pinned corpus (`corpus-<name>.expected.json`). */
export async function recordCorpora(require, tool, directory, corpora) {
  const { ESLint } = require("eslint");
  const summary = [];
  for (const corpus of pack("react").corpora) {
    const root = join(corpora, corpus.name);
    assertCommit(root, corpus.commit);
    const files = corpusFiles(root, corpus);
    const { findings, skipped } = await lint(ESLint, root, config(require), files, RULES[tool]);
    writeRecording(
      join(directory, `corpus-${corpus.name}.expected.json`),
      tool,
      PINS[tool],
      { kind: "corpus", name: corpus.name, repository: corpus.repository, commit: corpus.commit, rules: RULES[tool], skipped: skipped.map(entry => entry.path).sort() },
      findings,
    );
    summary.push({ corpus: corpus.name, files: files.length, skipped: skipped.length, findings: findings.length, skippedFiles: skipped });
  }
  return summary;
}
