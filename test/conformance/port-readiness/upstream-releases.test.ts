/**
 * The nightly upstream-release check (conformance/port-readiness/upstream-releases.ts): each registry's answer read as
 * its latest stable release, and a tool-pack that is behind, whose pins disagree, or whose registry cannot be read
 * failing the run. The registry answers are trimmed copies of real ones, recorded on 2026-10-21; nothing reaches the
 * network.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test, vi } from "vitest";

import { RUNTIMES_DIRECTORY } from "../../../conformance/port-readiness/recorded-adapter.js";
import {
  checkUpstreamReleases,
  eslintPackage,
  latestNpm,
  latestPackagist,
  latestPypi,
  latestRust,
  main,
  registry,
  runtimePin,
  upstreamPins,
  type Fetch,
  type UpstreamPin,
} from "../../../conformance/port-readiness/upstream-releases.js";

const ROOT = join(import.meta.dirname, "..", "..", "..");

/** registry.npmjs.org/@next%2feslint-plugin-next, abbreviated: the dist-tags, which carry pre-release lines too. */
const NPM = JSON.stringify({
  name: "@next/eslint-plugin-next",
  "dist-tags": { latest: "16.4.2", canary: "16.5.0-canary.21", rc: "15.0.0-rc.1", beta: "16.0.0-beta.0", backport: "15.5.28" },
  versions: { "16.3.8": {}, "16.4.2": {} },
});

/** pypi.org/pypi/ruff/json with one file per release: a later release candidate, a yanked release and one with no files. */
const PYPI = JSON.stringify({
  info: { name: "ruff", version: "0.18.0rc1" },
  releases: {
    "0.16.9": [{ filename: "ruff-0.16.9.tar.gz", yanked: false }],
    "0.17.1": [{ filename: "ruff-0.17.1.tar.gz", yanked: false }, { filename: "ruff-0.17.1-py3-none-any.whl", yanked: false }],
    "0.17.2": [{ filename: "ruff-0.17.2.tar.gz", yanked: true, yanked_reason: "broken wheel" }],
    "0.17.10.dev0": [{ filename: "ruff-0.17.10.dev0.tar.gz", yanked: false }],
    "0.18.0rc1": [{ filename: "ruff-0.18.0rc1.tar.gz", yanked: false }],
    "0.19.0": [],
  },
});

/** repo.packagist.org/p2/tightenco/tlint.json, minified (composer/2.0): each entry after the first repeats only what changed. */
const PACKAGIST = JSON.stringify({
  minified: "composer/2.0",
  packages: {
    "tightenco/tlint": [
      { name: "tightenco/tlint", version: "v10.0.0-RC1", version_normalized: "10.0.0.0-RC1", license: ["MIT"] },
      { version: "v10.0.0-beta.2", version_normalized: "10.0.0.0-beta2" },
      { version: "v9.7.0", version_normalized: "9.7.0.0" },
      { version: "v9.6.10", version_normalized: "9.6.10.0" },
      { version: "dev-main", version_normalized: "dev-main" },
      { version: "v9.10.0-alpha1", version_normalized: "9.10.0.0-alpha1" },
    ],
  },
});

/** static.rust-lang.org/dist/channel-rust-stable.toml, with one target of rust and none of the other packages. */
const RUST = `manifest-version = "2"
date = "2026-10-15"

[pkg.cargo]
version = "0.100.1 (fe5a5e6a8 2026-10-08)"
git_commit_hash = "58cf8f9c35b355199727627d07ae34a4e2846d67"

[pkg.rust]
version = "1.99.1 (58cf8f9c3 2026-10-13)"
git_commit_hash = "58cf8f9c35b355199727627d07ae34a4e2846d67"

[pkg.rust.target.aarch64-apple-darwin]
available = true
url = "https://static.rust-lang.org/dist/2026-10-15/rust-1.99.1-aarch64-apple-darwin.tar.gz"
`;

/** A registry answer whose latest stable release of `pin`'s package is `version`, in that registry's format. */
function answer(pin: UpstreamPin, version: string): string {
  switch (pin.ecosystem) {
    case "npm":
      return JSON.stringify({ "dist-tags": { latest: version } });
    case "pypi":
      return JSON.stringify({ releases: { [version]: [{ yanked: false }] } });
    case "packagist":
      return JSON.stringify({ packages: { [pin.package]: [{ version }] } });
    case "rust":
      return `[pkg.rust]\nversion = "${version} (0000000 2026-10-01)"\n`;
  }
}

/** A fetch that answers each pin's registry URL with `latest(pin)` as its latest release, and refuses any other URL. */
function fakeFetch(pins: UpstreamPin[], latest: (pin: UpstreamPin) => string): Fetch {
  const answers = new Map(pins.map((pin) => [registry(pin.ecosystem, pin.package).url, answer(pin, latest(pin))]));
  return async (url) => {
    const body = answers.get(url);
    return { ok: body !== undefined, status: body === undefined ? 404 : 200, text: async () => body ?? "" };
  };
}

const GATES = `adapter: bandit
rerail:
  pack: demo
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  commit: ${"a".repeat(40)}
  version: 1.9.4
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: bandit-1.9.4
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
known_gaps: []
`;

/** A checkout holding one gated tool-pack, demo/bandit, whose runtime pins `pyproject`, and the Rust pack's Clippy pins. */
function checkout(pyproject: string | null): string {
  const root = mkdtempSync(join(tmpdir(), "upstream-releases-"));
  const files: Record<string, string> = {
    "packs/demo/upstream/bandit/gates.yaml": GATES,
    "packs/rust/upstream/common.py": 'TOOLCHAIN = "1.99.0"\n',
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
    ...(pyproject === null ? {} : { [`${RUNTIMES_DIRECTORY}/bandit-1.9.4/pyproject.toml`]: pyproject }),
  };
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

const BANDIT_1_9_4 = '[project]\nname = "runtime"\ndependencies = ["bandit==1.9.4"]\n';

/** Run the CLI over `root` as the nightly does, under GitHub Actions with a step summary; its exit code and output. */
async function nightly(root: string, fetch: Fetch): Promise<{ code: number; stdout: string; summary: string }> {
  const summaryFile = join(root, "step-summary.md");
  writeFileSync(summaryFile, "");
  const written: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation((chunk: string | Uint8Array) => {
    written.push(String(chunk));
    return true;
  });
  const code = await main(root, fetch, { GITHUB_ACTIONS: "true", GITHUB_STEP_SUMMARY: summaryFile });
  vi.restoreAllMocks();
  return { code, stdout: written.join(""), summary: readFileSync(summaryFile, "utf8") };
}

const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

test("npm's latest is the latest dist-tag, and a pre-release there is refused", () => {
  assert.equal(latestNpm(NPM), "16.4.2");
  assert.throws(() => latestNpm(JSON.stringify({ "dist-tags": { latest: "17.0.0-canary.1" } })), /not a release/u);
  assert.equal(registry("npm", "@next/eslint-plugin-next").url, "https://registry.npmjs.org/@next%2feslint-plugin-next");
});

test("PyPI's latest stable release skips pre-releases, development releases, yanked releases and releases without files", () => {
  assert.equal(latestPypi(PYPI), "0.17.1");
  assert.throws(() => latestPypi(JSON.stringify({ releases: { "1.0.0rc1": [{ yanked: false }], "0.9.0": [{ yanked: true }] } })), /no final release/u);
});

test("Packagist's latest stable release skips dev, alpha, beta and RC versions and drops the tag's v", () => {
  assert.equal(latestPackagist(PACKAGIST, "tightenco/tlint"), "9.7.0");
  assert.throws(() => latestPackagist(PACKAGIST, "tighten/tlint"), /no stable release of tighten\/tlint/u);
});

test("Rust's latest stable release is [pkg.rust]'s version in the stable channel manifest", () => {
  assert.equal(latestRust(RUST), "1.99.1");
  assert.throws(() => latestRust("[pkg.cargo]\nversion = \"0.100.1\"\n"));
});

test("an ESLint-family upstream's package is the package part of eslint.release.installed", () => {
  assert.equal(eslintPackage("eslint/lib"), "eslint");
  assert.equal(eslintPackage("eslint-plugin-react-hooks"), "eslint-plugin-react-hooks");
  assert.equal(eslintPackage("@typescript-eslint/eslint-plugin/dist"), "@typescript-eslint/eslint-plugin");
});

test("a runtime's pin is read from its package.json, composer.json or pyproject.toml", () => {
  assert.deepEqual(runtimePin("npm", "package.json", JSON.stringify({ dependencies: { eslint: "9.37.0", "@next/eslint-plugin-next": "16.3.8" } }), "@next/eslint-plugin-next"), { version: "16.3.8" });
  assert.deepEqual(runtimePin("packagist", "composer.json", JSON.stringify({ require: { "tightenco/tlint": "v9.7.0" } }), "tightenco/tlint"), { version: "9.7.0" });
  assert.deepEqual(runtimePin("pypi", "pyproject.toml", '[project]\ndependencies = ["Bandit[toml] == 1.9.4"]\n', "bandit"), { version: "1.9.4" });
  assert.deepEqual(runtimePin("pypi", "pyproject.toml", '[project]\ndependencies = ["bandit>=1.9"]\n', "bandit"), { problem: "pyproject.toml pins no bandit==<version>" });
});

test("this checkout's pins agree, and pass when every registry's latest is the pinned release", async () => {
  const pins = upstreamPins(ROOT);
  assert.ok(pins.length >= 16, `${pins.length} pins`);
  assert.equal(pins.at(-1)!.toolPack, "rust/clippy");
  assert.deepEqual(pins.filter((pin) => pin.disagreements.length > 0).map((pin) => [pin.toolPack, pin.disagreements]), []);
  const byToolPack = Object.fromEntries(pins.map((pin) => [pin.toolPack, `${pin.ecosystem}:${pin.package}`]));
  assert.equal(byToolPack["blade/blade-parser"], "packagist:stillat/blade-parser");
  assert.equal(byToolPack["blade/tlint"], "packagist:tightenco/tlint");
  assert.equal(byToolPack["php/phpcs"], "packagist:squizlabs/php_codesniffer");
  assert.equal(byToolPack["next/eslint-plugin-next"], "npm:@next/eslint-plugin-next");
  assert.equal(byToolPack["python/ruff"], "pypi:ruff");

  const checks = await checkUpstreamReleases(pins, fakeFetch(pins, (pin) => pin.version));
  assert.deepEqual(checks.filter((check) => check.problems.length > 0).map((check) => [check.toolPack, check.problems]), []);
  assert.ok(checks.every((check) => check.statuses.join() === "current" && check.latest === check.version));
});

test("a current pin passes and writes the step summary", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const { code, stdout, summary } = await nightly(root, fakeFetch(upstreamPins(root), (pin) => pin.version));
  assert.equal(code, 0, stdout);
  assert.doesNotMatch(stdout, /::error/u);
  assert.match(summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.9\.4 \| current \|$/mu);
  assert.match(summary, /Every one of the 2 upstreams is pinned at its latest stable release/u);
});

test("a pin behind upstream's latest stable release fails and names the tool-pack", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const { code, stdout, summary } = await nightly(root, fakeFetch(upstreamPins(root), (pin) => (pin.toolPack === "demo/bandit" ? "1.10.0" : pin.version)));
  assert.equal(code, 1);
  assert.match(summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.10\.0 \| behind \|$/mu);
  assert.match(summary, /^- `demo\/bandit`: bandit 1\.9\.4 is pinned and PyPI's latest stable release is 1\.10\.0: npm run port-readiness:bump -- --pack demo --tool bandit, then record it again \(the nightly opens this as a pull request\)$/mu);
  assert.match(stdout, /^::error title=Upstream release \(demo\/bandit\)::demo\/bandit behind: /mu);
  assert.doesNotMatch(stdout, /::error title=Upstream release \(rust\/clippy\)/u);
});

test("--behind lists the tool-packs behind upstream for the nightly to bump, and only those", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const behind = join(root, "behind.json");
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  const code = await main(root, fakeFetch(upstreamPins(root), (pin) => (pin.toolPack === "demo/bandit" ? "1.10.0" : pin.version)), {}, { behind });
  vi.restoreAllMocks();
  assert.equal(code, 1);
  assert.deepEqual(JSON.parse(readFileSync(behind, "utf8")), [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "1.9.4", latest: "1.10.0" }]);

  // A failing check whose pins are not behind upstream (here they disagree) lists nothing to bump.
  const disagreeing = checkout('[project]\nname = "runtime"\ndependencies = ["bandit==1.9.3"]\n');
  roots.push(disagreeing);
  vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  await main(disagreeing, fakeFetch(upstreamPins(disagreeing), (pin) => pin.version), {}, { behind });
  vi.restoreAllMocks();
  assert.deepEqual(JSON.parse(readFileSync(behind, "utf8")), []);
});

test("gates.yaml and the runtime disagreeing about the pin fails, as does a runtime that pins no release of upstream", async () => {
  const root = checkout('[project]\nname = "runtime"\ndependencies = ["bandit==1.9.3"]\n');
  roots.push(root);
  const { code, stdout } = await nightly(root, fakeFetch(upstreamPins(root), (pin) => pin.version));
  assert.equal(code, 1);
  assert.match(stdout, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.9\.4 \| pins disagree \|$/mu);
  assert.match(stdout, /gates\.yaml upstream\.version is 1\.9\.4 but conformance\/recorders\/port-readiness\/runtimes\/bandit-1\.9\.4\/pyproject\.toml pins bandit 1\.9\.3/u);

  const missing = checkout(null);
  roots.push(missing);
  const unread = await nightly(missing, fakeFetch(upstreamPins(missing), (pin) => pin.version));
  assert.equal(unread.code, 1);
  assert.match(unread.stdout, /`demo\/bandit`: conformance\/recorders\/port-readiness\/runtimes\/bandit-1\.9\.4\/pyproject\.toml cannot be read/u);
});

test("a registry that cannot be read fails rather than passing", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const pins = upstreamPins(root);
  const unavailable: Fetch = async (url) => (url.startsWith("https://pypi.org/") ? { ok: false, status: 503, text: async () => "" } : await fakeFetch(pins, (pin) => pin.version)(url, { headers: {} }));
  const answered = await nightly(root, unavailable);
  assert.equal(answered.code, 1);
  assert.match(answered.stdout, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| unknown \| registry error \|$/mu);
  assert.match(answered.stdout, /`demo\/bandit`: https:\/\/pypi\.org\/pypi\/bandit\/json: HTTP 503/u);

  const offline = await checkUpstreamReleases(pins, async () => {
    throw new Error("getaddrinfo ENOTFOUND");
  });
  assert.deepEqual(offline.map((check) => [check.toolPack, check.statuses]), [["demo/bandit", ["registry error"]], ["rust/clippy", ["registry error"]]]);
  assert.match(offline[1]!.problems[0]!, /channel-rust-stable\.toml: getaddrinfo ENOTFOUND/u);

  const garbled = await checkUpstreamReleases(pins, async () => ({ ok: true, status: 200, text: async () => "<html>maintenance</html>" }));
  assert.ok(garbled.every((check) => check.statuses.includes("registry error")));
});
