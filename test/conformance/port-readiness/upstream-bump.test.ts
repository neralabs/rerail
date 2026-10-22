/**
 * The bump the nightly opens a pull request for (conformance/port-readiness/upstream/bump.ts): each pin of a tool-pack
 * moved to a newer release, in a throwaway checkout. GitHub's, npm's and Rust's answers are trimmed copies of real ones,
 * recorded on 2026-10-22 (typescript-eslint's annotated tags, Rust 1.99.0 and 1.99.1); nothing reaches the network.
 */
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test, vi } from "vitest";

import { RUNTIMES_DIRECTORY } from "../../../conformance/port-readiness/recorded-adapter.js";
import { applyBump, main, planBump, renderBump, type Sources } from "../../../conformance/port-readiness/upstream/bump.js";
import { upstreamPins, type Fetch } from "../../../conformance/port-readiness/upstream/releases.js";

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** A checkout holding `files`. */
function checkout(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "upstream-bump-"));
  roots.push(root);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

/** Answers each URL in `answers` with its JSON (or text), and every other URL with a 404, as GitHub and the registries do. */
function recorded(answers: Record<string, unknown>): { sources: Sources; asked: string[] } {
  const asked: string[] = [];
  const fetch: Fetch = async (url) => {
    asked.push(url);
    const body = answers[url];
    const text = body === undefined ? JSON.stringify({ message: "Not Found", status: "404" }) : typeof body === "string" ? body : JSON.stringify(body);
    return { ok: body !== undefined, status: body === undefined ? 404 : 200, text: async () => text };
  };
  return { sources: { fetch }, asked };
}

const GITHUB = "https://api.github.com/repos";

/** GitHub's answer for a lightweight tag: the ref names the commit. */
function lightweight(repository: string, tag: string, commit: string): Record<string, unknown> {
  return {
    [`${GITHUB}/${repository}/git/ref/tags/${encodeURIComponent(tag)}`]: {
      ref: `refs/tags/${tag}`,
      url: `${GITHUB}/${repository}/git/refs/tags/${tag}`,
      object: { sha: commit, type: "commit", url: `${GITHUB}/${repository}/git/commits/${commit}` },
    },
  };
}

function popularity(): string {
  return "popularity:\n  observed_on: 2026-10-06\n  evidence:\n    - a fake upstream\nknown_gaps: []\n";
}

const BANDIT_COMMIT = "92ae8b82fb422a639f0ed8d99e96cea769594e08";
const BANDIT_NEXT = "0123456789abcdef0123456789abcdef01234567";

/** demo/bandit: a PyPI upstream tagged `<version>`, its runtime named for the release. */
function banditCheckout(): string {
  return checkout({
    "packs/demo/upstream/bandit/gates.yaml": `adapter: bandit
rerail:
  pack: demo
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  # The release the demo pack records; tag 1.9.4.
  commit: ${BANDIT_COMMIT}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: bandit-1.9.4
${popularity()}`,
    "packs/demo/upstream/bandit/generate.py": `"""Records Bandit 1.9.4 for the demo pack."""\nVERSION = "1.9.4"\nORIGIN = "https://github.com/PyCQA/bandit@${BANDIT_COMMIT}"\nPYYAML = "1.9.4.1"\n`,
    "packs/demo/upstream/bandit/recording.lock": '{"pins": {"version": "1.9.4"}}\n',
    "packs/demo/conformance.json": `{\n  "tools": [\n    { "name": "bandit", "version": "1.9.4" },\n    { "name": "other", "version": "1.9.4" }\n  ],\n  "rules": []\n}\n`,
    "packs/demo/README.md": "Rules ported from Bandit 1.9.4 (tag v1.9.4), not 11.9.4 or 1.9.40.\n",
    [`${RUNTIMES_DIRECTORY}/bandit-1.9.4/pyproject.toml`]: '[project]\nname = "runtime"\ndependencies = ["Bandit[toml] == 1.9.4", "pyyaml==1.9.4"]\n\n[tool.uv]\npackage = false\n',
    [`${RUNTIMES_DIRECTORY}/bandit-1.9.4/.python-version`]: "3.13.12\n",
    "packs/rust/upstream/common.py": 'TOOLCHAIN = "1.99.0"\n',
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
  });
}

const BANDIT_TAGS = { ...lightweight("PyCQA/bandit", "1.9.4", BANDIT_COMMIT), ...lightweight("PyCQA/bandit", "1.9.5", BANDIT_NEXT) };

test("a PyPI tool-pack's bump moves gates.yaml, renames the runtime, and moves conformance.json and the generator's constants", async () => {
  const root = banditCheckout();
  const { sources, asked } = recorded(BANDIT_TAGS);
  const bump = (await planBump(root, "demo", "bandit", "1.9.5", sources))!;
  // v1.9.4 is not a tag: 1.9.4 is, and names the pinned commit, so 1.9.5 is the new release's tag.
  assert.deepEqual(asked, [`${GITHUB}/PyCQA/bandit/git/ref/tags/v1.9.4`, `${GITHUB}/PyCQA/bandit/git/ref/tags/1.9.4`, `${GITHUB}/PyCQA/bandit/git/ref/tags/1.9.5`]);
  assert.deepEqual(bump.from, { version: "1.9.4", tag: "1.9.4", commit: BANDIT_COMMIT });
  assert.deepEqual(bump.to, { version: "1.9.5", tag: "1.9.5", commit: BANDIT_NEXT });
  assert.deepEqual(bump.renames, [{ from: `${RUNTIMES_DIRECTORY}/bandit-1.9.4`, to: `${RUNTIMES_DIRECTORY}/bandit-1.9.5` }]);
  assert.deepEqual(bump.record, ["demo/bandit"]);
  applyBump(root, bump);

  const gates = readFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), "utf8");
  assert.match(gates, new RegExp(`\n  commit: ${BANDIT_NEXT}\n  version: 1.9.5\n`, "u"));
  assert.match(gates, /\n {2}runtime: bandit-1\.9\.5\n/u);
  assert.ok(!existsSync(join(root, RUNTIMES_DIRECTORY, "bandit-1.9.4")));
  // The extras and the spacing stay; another package at the same version does not move.
  assert.equal(readFileSync(join(root, RUNTIMES_DIRECTORY, "bandit-1.9.5/pyproject.toml"), "utf8"), '[project]\nname = "runtime"\ndependencies = ["Bandit[toml] == 1.9.5", "pyyaml==1.9.4"]\n\n[tool.uv]\npackage = false\n');
  assert.equal(readFileSync(join(root, RUNTIMES_DIRECTORY, "bandit-1.9.5/.python-version"), "utf8"), "3.13.12\n");
  assert.equal(readFileSync(join(root, "packs/demo/conformance.json"), "utf8"), `{\n  "tools": [\n    { "name": "bandit", "version": "1.9.5" },\n    { "name": "other", "version": "1.9.4" }\n  ],\n  "rules": []\n}\n`);
  assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/generate.py"), "utf8"), `"""Records Bandit 1.9.4 for the demo pack."""\nVERSION = "1.9.5"\nORIGIN = "https://github.com/PyCQA/bandit@${BANDIT_NEXT}"\nPYYAML = "1.9.4.1"\n`);
  // The recorder rewrites the lock; the bump leaves it.
  assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/recording.lock"), "utf8"), '{"pins": {"version": "1.9.4"}}\n');
  // The pins now agree at 1.9.5, as the nightly's check reads them.
  const pin = upstreamPins(root).find((entry) => entry.toolPack === "demo/bandit")!;
  assert.deepEqual([pin.version, pin.disagreements], ["1.9.5", []]);

  // What the bump leaves for the reviewer: prose, another tool's entry and another package at the same version.
  assert.deepEqual(bump.leftovers, [
    "packs/demo/upstream/bandit/gates.yaml:8: # The release the demo pack records; tag 1.9.4.",
    'packs/demo/upstream/bandit/generate.py:1: """Records Bandit 1.9.4 for the demo pack."""',
    'packs/demo/conformance.json:4: { "name": "other", "version": "1.9.4" }',
    "packs/demo/README.md:1: Rules ported from Bandit 1.9.4 (tag v1.9.4), not 11.9.4 or 1.9.40.",
    `${RUNTIMES_DIRECTORY}/bandit-1.9.5/pyproject.toml:3: dependencies = ["Bandit[toml] == 1.9.5", "pyyaml==1.9.4"]`,
  ]);
  const markdown = renderBump(bump);
  assert.match(markdown, /- Release: \[PyCQA\/bandit 1\.9\.5\]\(https:\/\/github\.com\/PyCQA\/bandit\/releases\/tag\/1\.9\.5\) \(PyPI: \[1\.9\.5\]\(https:\/\/pypi\.org\/project\/bandit\/1\.9\.5\/\)\)/u);
  assert.match(markdown, /\[1\.9\.4\.\.\.1\.9\.5\]\(https:\/\/github\.com\/PyCQA\/bandit\/compare\/1\.9\.4\.\.\.1\.9\.5\)/u);
  assert.ok(markdown.includes(`rename ${RUNTIMES_DIRECTORY}/bandit-1.9.4/ → ${RUNTIMES_DIRECTORY}/bandit-1.9.5/`), markdown);
  assert.ok(markdown.includes(`--- packs/demo/upstream/bandit/gates.yaml\n@@ 9\n-  commit: ${BANDIT_COMMIT}\n+  commit: ${BANDIT_NEXT}\n@@ 10\n-  version: 1.9.4\n+  version: 1.9.5\n`), markdown);
});

/** An ESLint-family gates.yaml: upstream, its release tag, and optionally the environment the pack is linted in. */
function eslintGates(options: { pack: string; tool: string; repository: string; commit: string; version: string; runtime: string; installed: string; environment?: string }): string {
  return `adapter: eslint
rerail:
  pack: ${options.pack}
  project: ${options.tool}
upstream:
  name: ${options.repository}
  repository: https://github.com/${options.repository}
  commit: ${options.commit}
  version: ${options.version}
  license: MIT
  recommended_configs:
    - recommended
  runtime: ${options.runtime}
${popularity()}eslint:
  plugin:
    package: "${options.installed.split("/").slice(0, 2).join("/")}"
    namespace: "@demo"
  release:
    tag: v${options.version}
    path: src
    installed: "${options.installed}"
    verbatim: false
${options.environment === undefined ? "" : `  environment:
    package: demo-config
    version: 1.0.0
    runtime: ${options.environment}
    source:
      repository: https://github.com/demo/config
      commit: ${"b".repeat(40)}
      path: packages/config
`}  implementation: src/rules/{rule}.ts
  tests: ["tests/*.test.ts"]
  helpers: []
  checkout: ["/src/", "/tests/"]
`;
}

const TSE = "typescript-eslint/typescript-eslint";
/** typescript-eslint's annotated tags v8.70.1 and v8.72.0, as GitHub answers them. */
const TSE_TAGS = {
  [`${GITHUB}/${TSE}/git/ref/tags/v8.70.1`]: {
    ref: "refs/tags/v8.70.1",
    url: `${GITHUB}/${TSE}/git/refs/tags/v8.70.1`,
    object: { sha: "9176cb8e806fa1a2c1ff643a5e9804dc399065df", type: "tag", url: `${GITHUB}/${TSE}/git/tags/9176cb8e806fa1a2c1ff643a5e9804dc399065df` },
  },
  [`${GITHUB}/${TSE}/git/tags/9176cb8e806fa1a2c1ff643a5e9804dc399065df`]: {
    sha: "9176cb8e806fa1a2c1ff643a5e9804dc399065df",
    tag: "v8.70.1",
    object: { sha: "23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7", type: "commit", url: `${GITHUB}/${TSE}/git/commits/23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7` },
  },
  [`${GITHUB}/${TSE}/git/ref/tags/v8.72.0`]: {
    ref: "refs/tags/v8.72.0",
    url: `${GITHUB}/${TSE}/git/refs/tags/v8.72.0`,
    object: { sha: "a2fbe657102826e34cb73a450b261a23f9cf0e37", type: "tag", url: `${GITHUB}/${TSE}/git/tags/a2fbe657102826e34cb73a450b261a23f9cf0e37` },
  },
  [`${GITHUB}/${TSE}/git/tags/a2fbe657102826e34cb73a450b261a23f9cf0e37`]: {
    sha: "a2fbe657102826e34cb73a450b261a23f9cf0e37",
    tag: "v8.72.0",
    object: { sha: "a4c2603d3e4fd43b33bc25b82457716e9f7cef71", type: "commit", url: `${GITHUB}/${TSE}/git/commits/a4c2603d3e4fd43b33bc25b82457716e9f7cef71` },
  },
  // npm records no gitHead for these releases.
  "https://registry.npmjs.org/@typescript-eslint%2feslint-plugin/8.70.1": { name: "@typescript-eslint/eslint-plugin", version: "8.70.1" },
  "https://registry.npmjs.org/@typescript-eslint%2feslint-plugin/8.72.0": { name: "@typescript-eslint/eslint-plugin", version: "8.72.0" },
};

test("an npm tool-pack's bump follows annotated tags, moves the scope's packages at that release, and renames its runtime wherever a gates.yaml names it", async () => {
  const root = checkout({
    "packs/ts/upstream/typescript-eslint/gates.yaml": eslintGates({ pack: "ts", tool: "typescript-eslint", repository: TSE, commit: "23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7", version: "8.70.1", runtime: "typescript-eslint-8.70.1", installed: "@typescript-eslint/eslint-plugin/dist" }),
    // Another tool-pack lints in that runtime: it is renamed there too, and that tool-pack is recorded again.
    "packs/ts/upstream/other/gates.yaml": eslintGates({ pack: "ts", tool: "other", repository: "demo/other", commit: "c".repeat(40), version: "1.0.0", runtime: "other-1.0.0", installed: "@demo/other", environment: "typescript-eslint-8.70.1" }),
    "packs/ts/conformance.json": JSON.stringify({ tools: [{ name: "typescript-eslint", version: "8.70.1" }, { name: "other", version: "1.0.0" }] }, null, 2),
    "packs/ts/upstream/generate.mjs": [
      "// The checkout is github.com/typescript-eslint/typescript-eslint at v8.70.1 (23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7).",
      'const VERSIONS = { eslint: "8.70.1", "@typescript-eslint/parser": "8.70.1", "@typescript-eslint/eslint-plugin": "8.70.1", "@typescript-eslint/utils": "8.70.0" };',
      'const CHECKOUT = { commit: "23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7" };',
      "",
    ].join("\n"),
    [`${RUNTIMES_DIRECTORY}/typescript-eslint-8.70.1/package.json`]: `${JSON.stringify({
      name: "runtime",
      private: true,
      description: "typescript-eslint 8.70.1",
      dependencies: { "@typescript-eslint/eslint-plugin": "8.70.1", "@typescript-eslint/parser": "8.70.1", "@typescript-eslint/utils": "8.70.0", eslint: "8.70.1", typescript: "6.0.3" },
      overrides: { acorn: "8.18.0" },
    }, null, 2)}\n`,
    "packs/rust/upstream/common.py": 'TOOLCHAIN = "1.99.0"\n',
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
  });
  const bump = (await planBump(root, "ts", "typescript-eslint", "8.72.0", recorded(TSE_TAGS).sources))!;
  assert.deepEqual(bump.to, { version: "8.72.0", tag: "v8.72.0", commit: "a4c2603d3e4fd43b33bc25b82457716e9f7cef71" });
  assert.deepEqual(bump.record, ["ts/typescript-eslint", "ts/other"]);
  applyBump(root, bump);

  const runtime = JSON.parse(readFileSync(join(root, RUNTIMES_DIRECTORY, "typescript-eslint-8.72.0/package.json"), "utf8")) as unknown;
  assert.deepEqual(runtime, {
    name: "runtime",
    private: true,
    description: "typescript-eslint 8.70.1",
    // The scope's packages at 8.70.1 move; @typescript-eslint/utils at another release and the unscoped eslint do not.
    dependencies: { "@typescript-eslint/eslint-plugin": "8.72.0", "@typescript-eslint/parser": "8.72.0", "@typescript-eslint/utils": "8.70.0", eslint: "8.70.1", typescript: "6.0.3" },
    overrides: { acorn: "8.18.0" },
  });
  const gates = readFileSync(join(root, "packs/ts/upstream/typescript-eslint/gates.yaml"), "utf8");
  assert.match(gates, /\n {2}commit: a4c2603d3e4fd43b33bc25b82457716e9f7cef71\n {2}version: 8\.72\.0\n/u);
  assert.match(gates, /\n {2}runtime: typescript-eslint-8\.72\.0\n/u);
  assert.match(gates, /\n {4}tag: v8\.72\.0\n/u);
  assert.match(readFileSync(join(root, "packs/ts/upstream/other/gates.yaml"), "utf8"), /\n {4}runtime: typescript-eslint-8\.72\.0\n/u);
  assert.equal(readFileSync(join(root, "packs/ts/upstream/generate.mjs"), "utf8"), [
    "// The checkout is github.com/typescript-eslint/typescript-eslint at v8.72.0 (a4c2603d3e4fd43b33bc25b82457716e9f7cef71).",
    'const VERSIONS = { eslint: "8.70.1", "@typescript-eslint/parser": "8.72.0", "@typescript-eslint/eslint-plugin": "8.72.0", "@typescript-eslint/utils": "8.70.0" };',
    'const CHECKOUT = { commit: "a4c2603d3e4fd43b33bc25b82457716e9f7cef71" };',
    "",
  ].join("\n"));
  assert.ok(bump.leftovers.includes(`${RUNTIMES_DIRECTORY}/typescript-eslint-8.72.0/package.json:4: "description": "typescript-eslint 8.70.1",`));
});

test("a generator that pins npm's gitHead of the release moves to the new release's gitHead", async () => {
  const commit = "d1727fbf987392749b374f3afe62227d2c70fc41";
  const root = checkout({
    "packs/react/upstream/hooks/gates.yaml": eslintGates({ pack: "react", tool: "hooks", repository: "facebook/react", commit, version: "7.1.1", runtime: "hooks-7.1.1", installed: "eslint-plugin-react-hooks" }),
    "packs/react/conformance.json": JSON.stringify({ tools: [{ name: "hooks", version: "7.1.1" }] }),
    "packs/react/upstream/common.mjs": 'export const UPSTREAM = { hooks: { title: "eslint-plugin-react-hooks 7.1.1", commit: "1ddff43c41147b880c22eb363e07aade5a71c5d9" } };\n',
    [`${RUNTIMES_DIRECTORY}/hooks-7.1.1/package.json`]: JSON.stringify({ dependencies: { "eslint-plugin-react-hooks": "7.1.1" } }),
    "packs/rust/upstream/common.py": 'TOOLCHAIN = "1.99.0"\n',
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
  });
  const { sources } = recorded({
    ...lightweight("facebook/react", "v7.1.1", commit),
    ...lightweight("facebook/react", "v7.2.0", "e".repeat(40)),
    "https://registry.npmjs.org/eslint-plugin-react-hooks/7.1.1": { version: "7.1.1", gitHead: "1ddff43c41147b880c22eb363e07aade5a71c5d9" },
    "https://registry.npmjs.org/eslint-plugin-react-hooks/7.2.0": { version: "7.2.0", gitHead: "f".repeat(40) },
  });
  const bump = (await planBump(root, "react", "hooks", "7.2.0", sources))!;
  applyBump(root, bump);
  // The line naming the gitHead is the pin's: its release moves with it.
  assert.equal(readFileSync(join(root, "packs/react/upstream/common.mjs"), "utf8"), `export const UPSTREAM = { hooks: { title: "eslint-plugin-react-hooks 7.2.0", commit: "${"f".repeat(40)}" } };\n`);
  assert.deepEqual(bump.notes, [`The generators pin npm's \`gitHead\` of eslint-plugin-react-hooks: 1ddff43c41147b880c22eb363e07aade5a71c5d9 for 7.1.1, ${"f".repeat(40)} for 7.2.0.`]);
});

test("a Packagist upstream in a shared runtime keeps the directory and the tag's v, and every tool-pack installing that runtime is recorded again", async () => {
  const gates = (adapter: string, tool: string, repository: string, commit: string, version: string, runtime: string) => `adapter: ${adapter}
rerail:
  pack: blade
  project: ${tool}
upstream:
  name: ${repository}
  repository: https://github.com/${repository}
  commit: ${commit}
  version: ${version}
  license: MIT
  recommended_configs:
    - default
  runtime: ${runtime}
${popularity()}`;
  const root = checkout({
    "packs/blade/upstream/blade-parser/gates.yaml": gates("blade-parser", "blade-parser", "stillat/blade-parser", "23a962f99bca451f8738ed2475a70129fb33f07c", "2.1.0", "blade"),
    // tlint installs the Blade toolkit too (manifest.ts's EXTRA_RUNTIMES).
    "packs/blade/upstream/tlint/gates.yaml": gates("tlint", "tlint", "tighten/tlint", "e768bedce2bd84930712f9412ecbf2d4c586735c", "9.7.0", "tlint-9.7.0"),
    "packs/blade/conformance.json": JSON.stringify({ tools: [{ name: "blade-parser", version: "2.1.0" }, { name: "tlint", version: "9.7.0" }] }, null, 2),
    [`${RUNTIMES_DIRECTORY}/blade/composer.json`]: `${JSON.stringify({ name: "toolkit", require: { "laravel/framework": "13.34.0", "Stillat/Blade-Parser": "v2.1.0" } }, null, 2)}\n`,
    [`${RUNTIMES_DIRECTORY}/tlint-9.7.0/composer.json`]: JSON.stringify({ require: { "tightenco/tlint": "9.7.0" } }),
    "packs/rust/upstream/common.py": 'TOOLCHAIN = "1.99.0"\n',
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
  });
  const { sources } = recorded({
    ...lightweight("stillat/blade-parser", "v2.1.0", "23a962f99bca451f8738ed2475a70129fb33f07c"),
    ...lightweight("stillat/blade-parser", "v2.2.0", "9a".repeat(20)),
  });
  const bump = (await planBump(root, "blade", "blade-parser", "2.2.0", sources))!;
  assert.deepEqual(bump.renames, []);
  assert.deepEqual(bump.record, ["blade/blade-parser", "blade/tlint"]);
  applyBump(root, bump);
  assert.equal(readFileSync(join(root, RUNTIMES_DIRECTORY, "blade/composer.json"), "utf8"), `${JSON.stringify({ name: "toolkit", require: { "laravel/framework": "13.34.0", "Stillat/Blade-Parser": "v2.2.0" } }, null, 2)}\n`);
  assert.match(readFileSync(join(root, "packs/blade/upstream/blade-parser/gates.yaml"), "utf8"), /\n {2}version: 2\.2\.0\n {2}license: MIT\n(?:.*\n)* {2}runtime: blade\n/u);
});

/** Rust's channel manifests for 1.99.0 and 1.99.1, trimmed to the packages the bump reads. */
const CHANNEL_1_99_0 = `manifest-version = "2"
date = "2026-10-01"

[pkg.clippy-preview]
version = "0.1.99"
git_commit_hash = "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4"

[pkg.rust]
version = "1.99.0 (b940084d7 2026-09-28)"
git_commit_hash = "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4"
`;
const CHANNEL_1_99_1 = `manifest-version = "2"
date = "2026-10-15"

[pkg.clippy-preview]
version = "0.1.99"
git_commit_hash = "58cf8f9c35b355199727627d07ae34a4e2846d67"

[pkg.rust]
version = "1.99.1 (58cf8f9c3 2026-10-13)"
git_commit_hash = "58cf8f9c35b355199727627d07ae34a4e2846d67"
`;

const COMMON_PY = `"""Pins (each generator refuses anything else):
- Rust toolchain 1.99.0: \`rustc 1.99.0 (b940084d7 2026-09-28)\`, \`clippy 0.1.99 (b940084d7e 2026-09-28)\`, invoked as
  \`rustc +1.99.0\` / \`cargo +1.99.0 clippy\`.
- Clippy source: https://github.com/rust-lang/rust-clippy at tag \`rust-1.99.0\`,
  commit 49e2f89a2605a9a340e04955fdbefa9803e6027a.
Python 3.9 or newer."""

TOOLCHAIN = "1.99.0"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"
CLIPPY_VERSION = "clippy 0.1.99 (b940084d7e 2026-09-28)"
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
`;

/** rust-lang/rust's src/tools at a tag, trimmed to two entries. */
function tools(clippy: string): unknown {
  return [
    { name: "cargo", path: "src/tools/cargo", sha: "5f94df4789f005f9a352888e8355ffc645b7ed0e", type: "file" },
    { name: "clippy", path: "src/tools/clippy", sha: clippy, type: "dir" },
  ];
}

const CLIPPY_ANSWERS = {
  "https://static.rust-lang.org/dist/channel-rust-1.99.0.toml": CHANNEL_1_99_0,
  "https://static.rust-lang.org/dist/channel-rust-1.99.1.toml": CHANNEL_1_99_1,
  ...lightweight("rust-lang/rust-clippy", "rust-1.99.0", "49e2f89a2605a9a340e04955fdbefa9803e6027a"),
  [`${GITHUB}/rust-lang/rust/contents/src/tools?ref=1.99.0`]: tools("f3bfc2411549eab24928aeb8b373bf1f6290f943"),
};

function clippyCheckout(): string {
  return checkout({
    "packs/rust/upstream/common.py": COMMON_PY,
    "packs/rust/conformance.json": '{\n  "tools": [\n    { "name": "clippy", "version": "0.1.99" }\n  ]\n}\n',
    "packs/rust/README.md": "Clippy lints, recorded with Rust 1.99.0.\n",
  });
}

test("Clippy's bump reads the toolchain from the release's channel manifest and keeps rust-clippy's .0 tag when the point release left src/tools/clippy alone", async () => {
  const root = clippyCheckout();
  const { sources } = recorded({ ...CLIPPY_ANSWERS, [`${GITHUB}/rust-lang/rust/contents/src/tools?ref=1.99.1`]: tools("f3bfc2411549eab24928aeb8b373bf1f6290f943") });
  const bump = (await planBump(root, "rust", "clippy", "1.99.1", sources))!;
  assert.deepEqual(bump.record, []);
  assert.deepEqual(bump.to, { version: "1.99.1", tag: "rust-1.99.0", commit: "49e2f89a2605a9a340e04955fdbefa9803e6027a" });
  applyBump(root, bump);
  assert.equal(readFileSync(join(root, "packs/rust/upstream/common.py"), "utf8"), `"""Pins (each generator refuses anything else):
- Rust toolchain 1.99.1: \`rustc 1.99.1 (58cf8f9c3 2026-10-13)\`, \`clippy 0.1.99 (58cf8f9c35 2026-10-13)\`, invoked as
  \`rustc +1.99.1\` / \`cargo +1.99.1 clippy\`.
- Clippy source: https://github.com/rust-lang/rust-clippy at tag \`rust-1.99.0\`,
  commit 49e2f89a2605a9a340e04955fdbefa9803e6027a.
Python 3.9 or newer."""

TOOLCHAIN = "1.99.1"
RUSTC_VERSION = "rustc 1.99.1 (58cf8f9c3 2026-10-13)"
CLIPPY_VERSION = "clippy 0.1.99 (58cf8f9c35 2026-10-13)"
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
`);
  // Clippy is still 0.1.99.
  assert.equal(readFileSync(join(root, "packs/rust/conformance.json"), "utf8"), '{\n  "tools": [\n    { "name": "clippy", "version": "0.1.99" }\n  ]\n}\n');
  assert.deepEqual(bump.leftovers, ["packs/rust/README.md:1: Clippy lints, recorded with Rust 1.99.0."]);
  const markdown = renderBump(bump);
  assert.match(markdown, /- Release: \[rust-lang\/rust 1\.99\.1\]\(https:\/\/github\.com\/rust-lang\/rust\/releases\/tag\/1\.99\.1\)/u);
  assert.match(markdown, /src\/tools\/clippy is the tree 1\.99\.0 has/u);
});

test("Clippy's bump stops when the point release changed src/tools/clippy and rust-clippy has no tag for it", async () => {
  const { sources } = recorded({ ...CLIPPY_ANSWERS, [`${GITHUB}/rust-lang/rust/contents/src/tools?ref=1.99.1`]: tools("0".repeat(40)) });
  await assert.rejects(planBump(clippyCheckout(), "rust", "clippy", "1.99.1", sources), /src\/tools\/clippy changed between Rust 1\.99\.0 and 1\.99\.1/u);
});

test("the bump stops when no tag names the pinned commit, when the new release has no tag, or when the pins already disagree", async () => {
  await assert.rejects(planBump(banditCheckout(), "demo", "bandit", "1.9.5", recorded(lightweight("PyCQA/bandit", "1.9.4", "1".repeat(40))).sources), /no tag of PyCQA\/bandit names the pinned commit/u);
  await assert.rejects(planBump(banditCheckout(), "demo", "bandit", "1.9.5", recorded(lightweight("PyCQA/bandit", "1.9.4", BANDIT_COMMIT)).sources), /PyCQA\/bandit has no tag 1\.9\.5/u);
  const root = banditCheckout();
  writeFileSync(join(root, RUNTIMES_DIRECTORY, "bandit-1.9.4/pyproject.toml"), '[project]\ndependencies = ["bandit==1.9.3"]\n');
  await assert.rejects(planBump(root, "demo", "bandit", "1.9.5", recorded(BANDIT_TAGS).sources), /the pins disagree before the bump/u);
  await assert.rejects(planBump(banditCheckout(), "demo", "bandit", "1.9.3", recorded(BANDIT_TAGS).sources), /pins 1\.9\.4, newer than 1\.9\.3/u);
});

/** Run the CLI over `root`; its exit code and output. */
async function cli(args: string[], root: string, fetch: Fetch): Promise<{ code: number; stdout: string }> {
  const written: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation((chunk: string | Uint8Array) => {
    written.push(String(chunk));
    return true;
  });
  const code = await main(args, root, fetch, {});
  vi.restoreAllMocks();
  return { code, stdout: written.join("") };
}

test("the CLI: --dry-run writes nothing, a bump writes the summary and the JSON the nightly reads, and a second run is a no-op", async () => {
  const root = banditCheckout();
  const { sources } = recorded(BANDIT_TAGS);
  const gates = readFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), "utf8");
  const dry = await cli(["--pack", "demo", "--tool", "bandit", "--version", "1.9.5", "--dry-run"], root, sources.fetch);
  assert.equal(dry.code, 0);
  assert.match(dry.stdout, /^would bump demo\/bandit 1\.9\.4 → 1\.9\.5/u);
  assert.equal(readFileSync(join(root, "packs/demo/upstream/bandit/gates.yaml"), "utf8"), gates);

  const out = join(root, "out");
  const run = await cli(["--pack", "demo", "--tool", "bandit", "--version", "1.9.5", "--summary", join(out, "bump.md"), "--json", join(out, "bump.json")], root, sources.fetch);
  assert.equal(run.code, 0);
  assert.match(readFileSync(join(out, "bump.md"), "utf8"), /^## `demo\/bandit`: `bandit` 1\.9\.4 → 1\.9\.5\n/u);
  const json = JSON.parse(readFileSync(join(out, "bump.json"), "utf8")) as { changed: boolean; version: string; record: unknown; files: string[] };
  assert.equal(json.changed, true);
  assert.equal(json.version, "1.9.5");
  assert.deepEqual(json.record, [{ pack: "demo", tool: "bandit" }]);
  assert.ok(json.files.includes(`${RUNTIMES_DIRECTORY}/bandit-1.9.5/pyproject.toml`));

  const again = await cli(["--pack", "demo", "--tool", "bandit", "--version", "1.9.5", "--json", join(out, "again.json")], root, sources.fetch);
  assert.equal(again.code, 0);
  assert.equal(again.stdout, "demo/bandit already pins 1.9.5\n");
  assert.deepEqual(JSON.parse(readFileSync(join(out, "again.json"), "utf8")), { toolPack: "demo/bandit", version: "1.9.5", changed: false });
});

test("without --version the CLI bumps to the registry's latest stable release", async () => {
  const root = banditCheckout();
  const { sources } = recorded({ ...BANDIT_TAGS, "https://pypi.org/pypi/bandit/json": { releases: { "1.9.4": [{ yanked: false }], "1.9.5": [{ yanked: false }], "2.0.0rc1": [{ yanked: false }] } } });
  const run = await cli(["--pack", "demo", "--tool", "bandit"], root, sources.fetch);
  assert.equal(run.code, 0);
  assert.match(run.stdout, /^bumped demo\/bandit 1\.9\.4 → 1\.9\.5/u);
});
