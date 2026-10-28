/**
 * The nightly report (scripts/upstream/watch.ts, watch-compare.ts, github.ts): what changed upstream in a tool-pack's
 * watched files between two commits, read from a fake GitHub (support/fake-github.ts), and each tool-pack's status in a
 * throwaway checkout whose registry and tag answers are recorded. Nothing reaches the network.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test } from "vitest";

import type { ExpandedWatch, WatchPatternSet } from "../../../conformance/languages/watch.js";
import { RUNTIMES_DIRECTORY } from "../../../scripts/pack-data/runtimes.js";
import { directoryReads } from "../../../scripts/upstream/github.js";
import type { Fetch } from "../../../scripts/upstream/releases.js";
import { compareWatch } from "../../../scripts/upstream/watch-compare.js";
import { releasesBetween, renderWatch, watchUpstreams } from "../../../scripts/upstream/watch.js";
import { fakeGitHub } from "./support/fake-github.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function patterns(paths: string[], globs: string[] = [], byRule: Record<string, string[]> = {}): WatchPatternSet {
  return { paths, globs, byRule };
}

/** A watch with nothing in it but `parts`. */
function watchOf(parts: Partial<ExpandedWatch> & { rules: string[] }): ExpandedWatch {
  return { tests: patterns([]), code: { ...patterns([]), indexes: [] }, shared: { paths: [], globs: [] }, newTests: patterns([]), ...parts };
}

/** An ESLint RuleTester file for `rule` with `valid` cases. */
function ruleTest(rule: string, valid: string[]): string {
  return `ruleTester.run(${JSON.stringify(rule)}, rule, { valid: ${JSON.stringify(valid)}, invalid: [] });\n`;
}

test("an ESLint-family release: a read test with its case diff, a ported rule's code, shared code, and a test the generator does not read", async () => {
  const { github } = fakeGitHub({
    pin: {
      "tests/lib/rules/no-foo.js": ruleTest("no-foo", ["a;"]),
      "tests/lib/rules/no-bar.js": ruleTest("no-bar", ["b;"]),
      "lib/rules/no-foo.js": "module.exports = 1;",
      "lib/rules/no-bar.js": "module.exports = 2;",
      "lib/rules/not-ported.js": "module.exports = 3;",
      "lib/rules/utils/ast.js": "old",
      "docs/no-foo.md": "old",
    },
    next: {
      "tests/lib/rules/no-foo.js": ruleTest("no-foo", ["a;", "c;"]),
      "tests/lib/rules/no-bar.js": ruleTest("no-bar", ["b;"]),
      "tests/lib/rules/no-foo/extra.js": ruleTest("no-foo", ["d;"]),
      "lib/rules/no-foo.js": "module.exports = 10;",
      "lib/rules/no-bar.js": "module.exports = 2;",
      "lib/rules/not-ported.js": "module.exports = 30;",
      "lib/rules/utils/ast.js": "new",
      "docs/no-foo.md": "new",
    },
  });
  const watch = watchOf({
    rules: ["no-bar", "no-foo"],
    tests: patterns(["tests/lib/rules/no-bar.js", "tests/lib/rules/no-foo.js"], [], { "no-bar": ["tests/lib/rules/no-bar.js"], "no-foo": ["tests/lib/rules/no-foo.js"] }),
    code: { ...patterns(["lib/rules/no-bar.js", "lib/rules/no-foo.js"], [], { "no-bar": ["lib/rules/no-bar.js"], "no-foo": ["lib/rules/no-foo.js"] }), indexes: [] },
    shared: { paths: [], globs: ["lib/rules/utils/**"] },
    newTests: patterns([], ["tests/lib/rules/no-bar/**", "tests/lib/rules/no-foo/**"], { "no-bar": ["tests/lib/rules/no-bar/**"], "no-foo": ["tests/lib/rules/no-foo/**"] }),
  });
  const changes = await compareWatch(github, { repository: "demo/eslint", ruleTester: true, watch, from: "pin", to: "next" });
  assert.deepEqual(changes, {
    tests: { files: [{ path: "tests/lib/rules/no-foo.js", change: "modified", rule: "no-foo", cases: { added: 1, removed: 0, changed: 0, unresolved: 0 } }], cases: { added: 1, removed: 0, changed: 0, unresolved: 0 } },
    code: { rules: ["no-foo"], files: [{ path: "lib/rules/no-foo.js", change: "modified", rules: ["no-foo"] }] },
    shared: { files: [{ path: "lib/rules/utils/ast.js", change: "modified" }] },
    newTests: [{ rule: "no-foo", path: "tests/lib/rules/no-foo/extra.js" }],
  });
});

const RUFF_REGISTRY = '    #[prefix = "B"]\n    Flake8Bugbear,\n';
const RUFF_FIXTURE = "crates/ruff_linter/resources/test/fixtures/flake8_bugbear";
const RUFF_PLUGIN = "crates/ruff_linter/src/rules/flake8_bugbear";

test("Ruff: a ported code's rule file is found through codes.rs, and a test_case added for it is a test not read", async () => {
  const codes = '        (Flake8Bugbear, "005") => rules::flake8_bugbear::rules::StripWithMultiCharacters,\n        (Flake8Bugbear, "006") => rules::flake8_bugbear::rules::MutableArgumentDefault,\n';
  const common = { "crates/ruff_linter/src/codes.rs": codes, "crates/ruff_linter/src/registry.rs": RUFF_REGISTRY };
  const { github } = fakeGitHub({
    pin: {
      ...common,
      [`${RUFF_FIXTURE}/B005.py`]: "s.strip('ab')\n",
      [`${RUFF_PLUGIN}/mod.rs`]: '#[test_case(Rule::StripWithMultiCharacters, Path::new("B005.py"))]\n',
      [`${RUFF_PLUGIN}/rules/strip_with_multi_characters.rs`]: "pub(crate) struct StripWithMultiCharacters;\n",
      [`${RUFF_PLUGIN}/rules/mutable_argument_default.rs`]: "pub(crate) struct MutableArgumentDefault;\n",
    },
    next: {
      ...common,
      [`${RUFF_FIXTURE}/B005.py`]: "s.strip('abc')\n",
      [`${RUFF_PLUGIN}/mod.rs`]: '#[test_case(Rule::StripWithMultiCharacters, Path::new("B005.py"))]\n#[test_case(Rule::StripWithMultiCharacters, Path::new("B005_1.py"))]\n#[test_case(Rule::MutableArgumentDefault, Path::new("B006_9.py"))]\n',
      [`${RUFF_PLUGIN}/rules/strip_with_multi_characters.rs`]: "pub(crate) struct StripWithMultiCharacters { x: u8 }\n",
      [`${RUFF_PLUGIN}/rules/mutable_argument_default.rs`]: "pub(crate) struct MutableArgumentDefault { y: u8 }\n",
    },
  });
  const watch = watchOf({ rules: ["B005"], tests: patterns([`${RUFF_FIXTURE}/B005.py`], [], { B005: [`${RUFF_FIXTURE}/B005.py`] }), code: { ...patterns([]), indexes: ["ruff-codes"] } });
  const changes = await compareWatch(github, { repository: "astral-sh/ruff", ruleTester: false, watch, from: "pin", to: "next" });
  assert.deepEqual(changes.tests, { files: [{ path: `${RUFF_FIXTURE}/B005.py`, change: "modified", rule: "B005", cases: null }], cases: null });
  assert.deepEqual(changes.code, { rules: ["B005"], files: [{ path: `${RUFF_PLUGIN}/rules/strip_with_multi_characters.rs`, change: "modified", rules: ["B005"] }] });
  // B006 is not ported: neither its code nor its new test is reported.
  assert.deepEqual(changes.newTests, [{ rule: "B005", path: `${RUFF_FIXTURE}/B005_1.py` }]);
});

test("Clippy: a lint's module from declared_lints.rs, and an added UI test annotating a ported lint", async () => {
  const lints = "crate::double_parens::DOUBLE_PARENS_INFO,\ncrate::misc_early::UNNEEDED_FIELD_PATTERN_INFO,\ncrate::misc_early::ZERO_PREFIXED_LITERAL_INFO,\n";
  const { github } = fakeGitHub({
    pin: {
      "clippy_lints/src/declared_lints.rs": lints,
      "clippy_lints/src/double_parens.rs": "a",
      "clippy_lints/src/misc_early/mod.rs": "a",
      "clippy_lints/src/misc_early/unneeded_field_pattern.rs": "a",
      "tests/ui/double_parens.rs": "fn main() {} //~ double_parens",
    },
    next: {
      "clippy_lints/src/declared_lints.rs": lints,
      "clippy_lints/src/double_parens.rs": "a",
      "clippy_lints/src/misc_early/mod.rs": "b",
      "clippy_lints/src/misc_early/unneeded_field_pattern.rs": "b",
      "tests/ui/double_parens.rs": "fn main() {} //~ double_parens",
      "tests/ui/parens_in_macros.rs": "fn main() { ((1)); } //~^ double_parens",
      "tests/ui/unrelated.rs": "fn main() {} //~^ needless_return",
    },
  });
  const watch = watchOf({
    rules: ["double_parens", "zero_prefixed_literal"],
    tests: patterns(["tests/ui/double_parens.rs"], [], { double_parens: ["tests/ui/double_parens.rs"] }),
    code: { ...patterns([]), indexes: ["clippy-declared-lints"] },
    newTests: patterns([], ["tests/ui/**"]),
  });
  const changes = await compareWatch(github, { repository: "rust-lang/rust-clippy", ruleTester: false, watch, from: "pin", to: "next" });
  // mod.rs is every misc_early lint's code; unneeded_field_pattern.rs is only its own lint's, which is not ported.
  assert.deepEqual(changes.code, { rules: ["zero_prefixed_literal"], files: [{ path: "clippy_lints/src/misc_early/mod.rs", change: "modified", rules: ["zero_prefixed_literal"] }] });
  assert.deepEqual(changes.newTests, [{ rule: "double_parens", path: "tests/ui/parens_in_macros.rs" }]);
});

test("directory reads: one per directory, a directory below another folded into a recursive read of it", () => {
  const rules = ["no-a", "no-b"];
  const reads = directoryReads([
    ...rules.map((rule) => `tests/rules/${rule}.test.ts`),
    ...rules.map((rule) => `tests/rules/${rule}/${rule}.test.ts`),
    ...rules.map((rule) => `tests/rules/${rule}/**`),
    "package.json",
    "src/util/**",
  ]);
  assert.deepEqual(Object.fromEntries(reads), { "": false, "tests/rules": true, "src/util": true });
});

test("releases between the pin and the latest: stable ones only, from each registry's answer", () => {
  const npm = JSON.stringify({ "dist-tags": { latest: "10.13.0" }, versions: { "10.11.0": {}, "10.12.0": {}, "10.13.0-rc.0": {}, "10.13.0": {}, "9.39.5": {} } });
  assert.equal(releasesBetween("npm", "eslint", npm, "10.11.0", "10.13.0"), 2);
  const pypi = JSON.stringify({ releases: { "0.16.9": [{}], "0.17.0": [{}], "0.17.1": [{ yanked: true }], "0.17.2rc1": [{}], "0.17.3": [{}] } });
  assert.equal(releasesBetween("pypi", "ruff", pypi, "0.16.9", "0.17.3"), 2);
  assert.equal(releasesBetween("rust", "rust", "", "1.99.0", "1.101.2"), 2);
});

const PIN = "a".repeat(40);
const NEXT = "b".repeat(40);

/** A demo pack with ESLint-family tool-packs, each pinned at 1.0.0 and watching `tests/{rule}.js`. */
function checkout(tools: string[]): string {
  const root = mkdtempSync(join(tmpdir(), "upstream-watch-"));
  roots.push(root);
  const files: Record<string, string> = {
    "packs/demo/conformance.json": JSON.stringify({ tools: tools.map((name) => ({ name, version: "1.0.0" })), rules: tools.map((tool) => ({ upstream: { tool, rule: "no-foo" } })) }),
  };
  for (const tool of tools) {
    files[`packs/demo/upstream/${tool}/gates.yaml`] = `adapter: eslint
rerail:
  pack: demo
  project: ${tool}
upstream:
  name: demo/${tool}
  repository: https://github.com/demo/${tool}
  commit: ${PIN}
  version: 1.0.0
  license: MIT
  recommended_configs:
    - recommended
  runtime: ${tool}-1.0.0
  lints: [".js"]
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
rules: {}
watch:
  tests:
    - "tests/{rule}.js"
  code:
    - "lib/{rule}.js"
  shared:
    - "README.md"
eslint:
  release:
    tag: v1.0.0
    installed: "${tool}"
`;
    files[`${RUNTIMES_DIRECTORY}/${tool}-1.0.0/package.json`] = JSON.stringify({ dependencies: { [tool]: "1.0.0" } });
  }
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

/** npm answering each package's latest release, and GitHub its tags v1.0.0 (the pin) and v1.1.0; any other URL is a 404. */
function registryAndTags(latest: Record<string, string>, tags: Record<string, string[]>): Fetch {
  const answers = new Map<string, unknown>();
  for (const [name, version] of Object.entries(latest)) answers.set(`https://registry.npmjs.org/${name}`, { "dist-tags": { latest: version }, versions: { "1.0.0": {}, "1.1.0": {} } });
  for (const [tool, names] of Object.entries(tags)) {
    for (const tag of names) answers.set(`https://api.github.com/repos/demo/${tool}/git/ref/tags/${tag}`, { object: { type: "commit", sha: tag === "v1.0.0" ? PIN : NEXT } });
  }
  return (url) => {
    const body = answers.get(url);
    return Promise.resolve({ ok: body !== undefined, status: body === undefined ? 404 : 200, text: () => Promise.resolve(JSON.stringify(body ?? { message: "Not Found" })) });
  };
}

test("each tool-pack's status, one tool-pack's error leaving the others' rows", async () => {
  const root = checkout(["changed", "current", "irrelevant", "untagged"]);
  const fetch = registryAndTags(
    { changed: "1.1.0", current: "1.0.0", irrelevant: "1.1.0", untagged: "1.1.0" },
    { changed: ["v1.0.0", "v1.1.0"], current: ["v1.0.0"], irrelevant: ["v1.0.0", "v1.1.0"], untagged: ["v1.0.0"] },
  );
  const { github, reads } = fakeGitHub({
    [PIN]: { "tests/no-foo.js": ruleTest("no-foo", ["a;"]), "lib/no-foo.js": "a", "README.md": "a" },
    [NEXT]: { "tests/no-foo.js": ruleTest("no-foo", ["a;"]), "lib/no-foo.js": "a", "README.md": "b" },
  });
  // Only the files of the tool-pack named `changed` differ: its fake tree is its own.
  const changedGitHub = fakeGitHub({ [PIN]: { "tests/no-foo.js": ruleTest("no-foo", ["a;"]), "lib/no-foo.js": "a" }, [NEXT]: { "tests/no-foo.js": ruleTest("no-foo", ["a;", "b;"]), "lib/no-foo.js": "b" } }).github;
  const routed = { tree: (repository: string, ...rest: [string, string, boolean]) => (repository === "demo/changed" ? changedGitHub : github).tree(repository, ...rest), file: (repository: string, ...rest: [string, string]) => (repository === "demo/changed" ? changedGitHub : github).file(repository, ...rest) };
  const reports = await watchUpstreams(root, ["changed", "current", "irrelevant", "untagged"].map((tool) => ({ pack: "demo", tool })), { fetch, github: routed });
  assert.deepEqual(reports.map((report) => [report.toolPack, report.status, report.releasesBehind]), [
    ["demo/changed", "changed", 1],
    ["demo/current", "current", 0],
    ["demo/irrelevant", "behind-irrelevant", 1],
    ["demo/untagged", "error", null],
  ]);
  const [changed, current, irrelevant, untagged] = reports;
  // Shared code alone is listed but does not make the tool-pack `changed`.
  assert.deepEqual(irrelevant!.shared.files.map((file) => file.path), ["README.md"]);
  assert.deepEqual(changed!.tests.cases, { added: 1, removed: 0, changed: 0, unresolved: 0 });
  assert.deepEqual(changed!.code.rules, ["no-foo"]);
  assert.deepEqual([changed!.pinned, changed!.latest], [{ version: "1.0.0", tag: "v1.0.0", commit: PIN }, { version: "1.1.0", tag: "v1.1.0", commit: NEXT }]);
  // An up-to-date tool-pack's files are never read.
  assert.deepEqual(current!.latest, { version: "1.0.0", tag: "v1.0.0", commit: PIN });
  assert.ok(reads.some((read) => read.includes("demo/irrelevant@")));
  assert.ok(reads.every((read) => !read.includes("demo/current@")));
  assert.match(untagged!.error!, /has no tag v1\.1\.0/u);
  const markdown = renderWatch(reports);
  assert.match(markdown, /\| `demo\/changed` \| changed \| 1\.0\.0 → 1\.1\.0 \| 1 \| 1 file, \+1 −0 ~0 \| `no-foo` \| — \| — \|/u);
  assert.match(markdown, /- Error: .*has no tag v1\.1\.0/u);
});
