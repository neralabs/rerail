// Shared by the React pack's generators (eslint-plugin-react/generate.mjs, eslint-plugin-react-hooks/generate.mjs):
// the pins, the one ESLint configuration every expected-findings run is made with, fixture extraction from the upstream rule
// tests (a stub RuleTester captures `run(name, rule, { valid, invalid })`), and the expected-findings runs.
// Run, through them, by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never
// pull request CI).
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { assertPortableCase, isVariantPath, lint, pinnedRequire, REPOSITORY, withOrigins, writeExpectedFindings, writeRenameVariants } from "../../../scripts/pack-data/eslint-expected.mjs";
import { variantEntries, writeProvenance } from "../../../scripts/pack-data/provenance.mjs";

export const UPSTREAM_DIRECTORY = dirname(fileURLToPath(import.meta.url));

/** Exact versions every expected-findings run is made with (`.rerail/tools/react/package.json`). */
export const PINS = {
  eslint: "9.39.5",
  "eslint-plugin-react": "7.37.5",
  "eslint-plugin-react-hooks": "7.1.1",
  "@typescript-eslint/parser": "8.70.1",
  typescript: "5.9.3",
};

/**
 * The chosen upstream rules, per tool (the pack's `tools[].name`). Every expected-findings run enables all of them. The hooks
 * plugin's React Compiler rules have no RuleTester cases of their own here (upstream tests them through the compiler's
 * fixtures), so their answers come from the hooks rules' cases, and the real-code samples.
 */
export const RULES = {
  "eslint-plugin-react": [
    "react/button-has-type",
    "react/iframe-missing-sandbox",
    "react/jsx-key",
    "react/jsx-no-comment-textnodes",
    "react/jsx-no-constructed-context-values",
    "react/jsx-no-duplicate-props",
    "react/jsx-no-leaked-render",
    "react/jsx-no-script-url",
    "react/jsx-no-target-blank",
    "react/jsx-no-undef",
    "react/no-access-state-in-setstate",
    "react/no-array-index-key",
    "react/no-children-prop",
    "react/no-danger",
    "react/no-danger-with-children",
    "react/no-deprecated",
    "react/no-did-mount-set-state",
    "react/no-did-update-set-state",
    "react/no-direct-mutation-state",
    "react/no-find-dom-node",
    "react/no-is-mounted",
    "react/no-object-type-as-default-prop",
    "react/no-render-return-value",
    "react/no-string-refs",
    "react/no-unescaped-entities",
    "react/no-unknown-property",
    "react/no-unsafe",
    "react/no-unstable-nested-components",
    "react/react-in-jsx-scope",
    "react/self-closing-comp",
    "react/void-dom-elements-no-children",
  ],
  "eslint-plugin-react-hooks": [
    "react-hooks/rules-of-hooks",
    "react-hooks/exhaustive-deps",
    "react-hooks/error-boundaries",
    "react-hooks/globals",
    "react-hooks/immutability",
    "react-hooks/purity",
    "react-hooks/refs",
    "react-hooks/set-state-in-effect",
    "react-hooks/set-state-in-render",
    "react-hooks/static-components",
  ],
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

/**
 * Ours: cases Rerail wrote for the plugins to score beside the upstream tests, per tool. Each is a file of another JSX
 * library (named by a pragma, an `h` import or a manifest depending on Vue), which the plugins, enabled on every file,
 * still report and the React ports skip (`divergences.json`), or a React file whose JSX goes through Emotion (its
 * `@jsxImportSource` pragma, or an `@jsx jsx` pragma in a package depending on react), which the plugins and the React
 * ports both report. `errors` is the case's own rule's finding count, checked as a test's is; a case with a `manifest`
 * is written as `<name>/<filename>` beside that `package.json`, and the two are one analysis unit.
 */
export const AUTHORED = {
  "eslint-plugin-react": [
    {
      rule: "react/jsx-key",
      directory: "jsx-key",
      name: "stencil-h",
      extension: "tsx",
      code: "import { h } from '@stencil/core';\nexport const rows = (items: string[]) => items.map(i => <div>{i}</div>);\n",
      errors: 1,
    },
    {
      rule: "react/jsx-key",
      directory: "jsx-key",
      name: "jsx-import-source-emotion",
      extension: "jsx",
      code: "/** @jsxImportSource @emotion/react */\nexport const rows = items => items.map(i => <div>{i}</div>);\n",
      errors: 1,
    },
    {
      rule: "react/react-in-jsx-scope",
      directory: "react-in-jsx-scope",
      name: "jsx-import-source-preact",
      extension: "jsx",
      code: "/** @jsxImportSource preact */\nexport const Greeting = () => <p>Hello</p>;\n",
      errors: 1,
    },
  ],
  "eslint-plugin-react-hooks": [
    {
      rule: "react-hooks/rules-of-hooks",
      directory: "rules-of-hooks",
      name: "vue-package",
      filename: "src/use-field.js",
      manifest: { name: "vue-app", private: true, dependencies: { vue: "3.5.22" } },
      code: "import { useId } from 'vue';\n\nexport function useField(labelled) {\n  if (labelled) {\n    return useId();\n  }\n  return undefined;\n}\n",
      errors: 1,
    },
    {
      rule: "react-hooks/rules-of-hooks",
      directory: "rules-of-hooks",
      name: "jsx-import-source-emotion",
      extension: "jsx",
      code: "/** @jsxImportSource @emotion/react */\nimport { useState } from 'react';\n\nexport function Panel({ open }) {\n  if (open) {\n    const [count] = useState(0);\n    return <div css={{ color: 'hotpink' }}>{count}</div>;\n  }\n  return null;\n}\n",
      errors: 1,
    },
    {
      rule: "react-hooks/rules-of-hooks",
      directory: "rules-of-hooks",
      name: "emotion-jsx-pragma",
      filename: "src/banner.jsx",
      manifest: { name: "emotion-app", private: true, dependencies: { "@emotion/react": "11.14.0", react: "19.2.0" } },
      code: "/** @jsx jsx */\nimport { jsx } from '@emotion/react';\nimport { useState } from 'react';\n\nexport function Banner({ visible }) {\n  if (visible) {\n    const [text] = useState('Hello');\n    return <div css={{ color: 'hotpink' }}>{text}</div>;\n  }\n  return null;\n}\n",
      errors: 1,
    },
  ],
};

/** The nearest ancestor of the repository holding `.rerail/<kind>/react` (sub-worktrees live under `.rerail/wt/`). */
function checkoutDirectory(kind) {
  for (let directory = REPOSITORY; ; directory = dirname(directory)) {
    const candidate = join(directory, ".rerail", kind, "react");
    if (existsSync(candidate)) return candidate;
    if (dirname(directory) === directory) throw new Error(`no .rerail/${kind}/react above ${REPOSITORY}`);
  }
}

/** `--tools <dir>` overrides the checkout's `.rerail/tools/react`. */
export function options(argv) {
  const value = flag => {
    const index = argv.indexOf(flag);
    return index === -1 ? undefined : resolve(argv[index + 1]);
  };
  return { tools: value("--tools") ?? checkoutDirectory("tools") };
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
 * Writes the extracted cases (`{ rule, directory, kind, index, code, errors, origin, from, detail, filename?,
 * extension?, drop? }`; `from` is the upstream test file, `detail` locates the case in it) and `tool`'s AUTHORED cases
 * under `<directory>/fixtures/`, a rename variant of each extracted case under `fixtures/renamed/`
 * (`writeRenameVariants`), records the pack configuration over them (every chosen rule on every source; the expected
 * findings hold `tool`'s rules, over the files ESLint parses, and each extracted case's origin, its rule), lists the
 * authored files in `<directory>/UPSTREAM.md` and checks each case: the rule's own finding count under the pack
 * configuration against the test's `errors`. The check's report goes to `<tools>/scratch/`. The kept entries it returns
 * hold the kept variants too (kind `renamed`, `of` the original's path), and `recorded` is withOrigins' result.
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
  const origins = new Map(written.map(entry => [entry.path, [entry.rule]]));
  for (const entry of AUTHORED[tool]) {
    const name = `rerail-${entry.name}`;
    const path = writeCase(fixtures, entry.directory, name, entry.extension, entry.code, entry.filename);
    const unit = entry.manifest === undefined ? path : [writeCase(fixtures, entry.directory, name, undefined, `${JSON.stringify(entry.manifest, null, 2)}\n`, "package.json"), path];
    written.push({ ...entry, kind: "authored", origin: "written by Rerail", path, unit });
  }
  const { variants, skipped: unrenamed } = writeRenameVariants(fixtures, [...origins.keys()], origins);
  const linted = await lint(ESLint, fixtures, config(require), [...written.map(entry => entry.path), ...variants.map(variant => variant.unit)], ALL_RULES);
  const unparsable = new Map(linted.skipped.map(entry => [entry.path, entry.reason]));
  for (const path of [...unparsable.keys()].filter(path => !isVariantPath(path))) rmSync(join(fixtures, path));
  const kept = written.filter(entry => !unparsable.has(entry.path));
  const recorded = withOrigins(fixtures, kept.map(entry => entry.unit ?? entry.path), origins, variants, new Set(unparsable.keys()), linted.findings);
  const findings = recorded.findings;
  const upstream = UPSTREAM[tool];
  writeExpectedFindings(
    join(directory, "fixtures.expected.json"),
    tool,
    PINS[tool],
    { kind: "fixtures", origin: `${upstream.repository}@${upstream.commit}`, rules: RULES[tool], files: recorded.files, units: recorded.units },
    findings.filter(finding => RULES[tool].includes(finding.rule)),
  );
  writeAuthoredRecord(tool, directory, kept);
  const mismatches = [];
  for (const entry of kept) {
    const own = findings.filter(finding => finding.path === entry.path && finding.rule === entry.rule).length;
    if (own !== entry.errors) mismatches.push({ origin: entry.origin, path: entry.path, expected: entry.errors, recorded: own, options: entry.test?.options, settings: entry.test?.settings });
  }
  const renamed = Object.entries(recorded.units).flatMap(([path, { renamedFrom }]) => (renamedFrom === undefined ? [] : [{ kind: "renamed", path, of: renamedFrom }]));
  const report = {
    tool,
    extracted: cases.length,
    written: written.length,
    kept: kept.length,
    renamed: renamed.length,
    unrenamed,
    dropped,
    unparsable: written.filter(entry => unparsable.has(entry.path)).map(entry => ({ origin: entry.origin, path: entry.path, reason: unparsable.get(entry.path) })),
    mismatches,
  };
  mkdirSync(join(tools, "scratch"), { recursive: true });
  writeFileSync(join(tools, "scratch", `${tool}-fixtures-report.json`), `${JSON.stringify(report, null, 1)}\n`);
  console.log(`${tool}: ${cases.length} cases, ${dropped.length} dropped (syntax), ${report.unparsable.length} unparsable, ${kept.length} kept, ${renamed.length} rename variants, ${mismatches.length} count mismatches`);
  return { kept: [...kept, ...renamed], recorded, report };
}

/** UPSTREAM.md beside the generator: the fixtures Rerail wrote, which are Rerail's, not the plugin's. */
function writeAuthoredRecord(tool, directory, kept) {
  const rows = kept.filter(entry => entry.kind === "authored").flatMap(entry => [entry.unit].flat().map(path => `| \`upstream/${tool}/fixtures/${path}\` | written by Rerail |`));
  const text = `# ${UPSTREAM[tool].title}: cases Rerail wrote

The fixtures listed below were written by Rerail for the pinned plugin to score, beside the cases extracted from the plugin's own tests (\`licenses/${UPSTREAM[tool].project}/UPSTREAM.md\`). Each is a file of another JSX library, which the plugin, enabled on every file, reports and the React ports skip (\`divergences.json\`), or a React file whose JSX goes through Emotion, which the plugin and the React ports both report. \`../common.mjs\` holds them (\`AUTHORED\`) and records the plugin's findings on them in \`fixtures.expected.json\`. They are Rerail's (Apache-2.0, the pack's licence), so nothing goes under \`licenses/\`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
${rows.join("\n")}
`;
  writeFileSync(join(directory, "UPSTREAM.md"), text);
}

/**
 * `licenses/<project>/LICENSE` (verbatim at the pinned commit) and the tool's provenance.json and LICENSES.md: every
 * copied test file (`copied`: `{ file, upstream }`), every extracted case kept (`kept`, its `from` and `detail`) and
 * every rename variant of one (`recorded`, withOrigins' result).
 */
export function writeLicenseAndProvenance(tool, upstream, copied, kept, recorded) {
  const meta = UPSTREAM[tool];
  const directory = join(UPSTREAM_DIRECTORY, "..", "licenses", meta.project);
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(upstream, "LICENSE"), join(directory, "LICENSE"));
  const originals = new Map(
    kept.filter(entry => entry.kind !== "authored" && entry.kind !== "renamed").map(entry => [`fixtures/${entry.path}`, { source: meta.project, from: entry.from, how: "extracted", detail: entry.detail }]),
  );
  const entries = new Map([
    ...copied.map(entry => [entry.file, { source: meta.project, from: entry.upstream, how: "verbatim" }]),
    ...originals,
    ...variantEntries(originals, recorded),
  ]);
  writeProvenance({ pack: "react", tool, entries, commits: { [meta.project]: meta.commit } });
}
