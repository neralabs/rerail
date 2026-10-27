/**
 * The nightly upstream-release check (scripts/upstream/releases.ts): each registry's answer read as its latest stable
 * release; each tool-pack's gap to it (Rust's on major.minor only) reported and listed for the bump; and every finding,
 * a breaking gap, pins that disagree or a registry that cannot be read, a warning that never fails the run. The registry
 * answers are trimmed copies of real ones, recorded on 2026-10-21; nothing reaches the network.
 */
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test, vi } from "vitest";

import { RUNTIMES_DIRECTORY } from "../../../scripts/pack-data/runtimes.js";
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
} from "../../../scripts/upstream/releases.js";

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

/** demo/bandit's gates.yaml, pinning bandit `version`. */
function banditGates(version: string): string {
  return `adapter: bandit
rerail:
  pack: demo
  project: bandit
upstream:
  name: PyCQA/bandit
  repository: https://github.com/PyCQA/bandit
  commit: ${"a".repeat(40)}
  version: ${version}
  license: Apache-2.0
  recommended_configs:
    - default
  runtime: bandit-${version}
  lints: [".py"]
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
rules: {}
`;
}

const CLIPPY_COMMIT = "c".repeat(40);

/** The Rust pack's Clippy tool-pack: its pins are Rust 1.99.0's (Clippy 0.1.99). */
const CLIPPY_GATES = `adapter: clippy
rerail:
  pack: rust
  project: clippy
upstream:
  name: rust-lang/rust-clippy
  repository: https://github.com/rust-lang/rust-clippy
  commit: ${CLIPPY_COMMIT}
  version: 0.1.99
  license: MIT OR Apache-2.0
  recommended_configs:
    - default
  runtime: clippy-1.99.0
  lints: [".py"]
popularity:
  observed_on: 2026-10-06
  evidence:
    - a fake upstream
rules: {}
`;

/**
 * A checkout holding two gated tool-packs: demo/bandit at `bandit` (1.9.4 unless given), whose runtime pins `pyproject`,
 * and the Rust pack's Clippy.
 */
function checkout(pyproject: string | null, bandit = "1.9.4"): string {
  const root = mkdtempSync(join(tmpdir(), "upstream-releases-"));
  const files: Record<string, string> = {
    "packs/demo/upstream/bandit/gates.yaml": banditGates(bandit),
    "packs/rust/upstream/common.py": `TOOLCHAIN = "1.99.0"\nCLIPPY_COMMIT = "${CLIPPY_COMMIT}"\n`,
    "packs/rust/conformance.json": JSON.stringify({ tools: [{ name: "clippy", version: "0.1.99" }] }),
    "packs/rust/upstream/clippy/gates.yaml": CLIPPY_GATES,
    [`${RUNTIMES_DIRECTORY}/clippy-1.99.0/rust-toolchain.toml`]: '[toolchain]\nchannel = "1.99.0"\ncomponents = ["clippy"]\nprofile = "minimal"\n',
    ...(pyproject === null ? {} : { [`${RUNTIMES_DIRECTORY}/bandit-${bandit}/pyproject.toml`]: pyproject }),
  };
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

/** A runtime's pyproject.toml pinning bandit `version`. */
function banditRuntime(version: string): string {
  return `[project]\nname = "runtime"\ndependencies = ["bandit==${version}"]\n`;
}

const BANDIT_1_9_4 = banditRuntime("1.9.4");

/**
 * Run the CLI over `root` as the nightly does, under GitHub Actions with a step summary and `--behind`; its exit code,
 * output, and the tool-packs it lists behind.
 */
async function nightly(root: string, fetch: Fetch): Promise<{ code: number; stdout: string; summary: string; behind: unknown }> {
  const summaryFile = join(root, "step-summary.md");
  const behindFile = join(root, "behind.json");
  writeFileSync(summaryFile, "");
  const written: string[] = [];
  vi.spyOn(process.stdout, "write").mockImplementation((chunk: string | Uint8Array) => {
    written.push(String(chunk));
    return true;
  });
  const code = await main(root, fetch, { GITHUB_ACTIONS: "true", GITHUB_STEP_SUMMARY: summaryFile }, { behind: behindFile });
  vi.restoreAllMocks();
  return { code, stdout: written.join(""), summary: readFileSync(summaryFile, "utf8"), behind: JSON.parse(readFileSync(behindFile, "utf8")) };
}

/** A fetch whose latest release of each pin is its pinned one, except `latest`'s for the tool-packs it names. */
function releasing(root: string, latest: Record<string, string>): Fetch {
  return fakeFetch(upstreamPins(root), (pin) => latest[pin.toolPack] ?? pin.version);
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
  assert.ok(pins.length >= 18, `${pins.length} pins`);
  assert.deepEqual(pins.filter((pin) => pin.disagreements.length > 0).map((pin) => [pin.toolPack, pin.disagreements]), []);
  const byToolPack = Object.fromEntries(pins.map((pin) => [pin.toolPack, `${pin.ecosystem}:${pin.package}`]));
  assert.equal(byToolPack["blade/blade-parser"], "packagist:stillat/blade-parser");
  assert.equal(byToolPack["blade/tlint"], "packagist:tightenco/tlint");
  assert.equal(byToolPack["php/phpcs"], "packagist:squizlabs/php_codesniffer");
  assert.equal(byToolPack["next/eslint-plugin-next"], "npm:@next/eslint-plugin-next");
  assert.equal(byToolPack["python/ruff"], "pypi:ruff");
  // #141's tool-packs: Clippy follows Rust's own releases, once, as a gated tool-pack.
  assert.equal(byToolPack["rust/clippy"], "rust:rust");
  assert.equal(pins.filter((pin) => pin.toolPack === "rust/clippy").length, 1);
  assert.equal(byToolPack["sql/sqlfluff"], "pypi:sqlfluff");
  assert.equal(byToolPack["vue/eslint-plugin-vue"], "npm:eslint-plugin-vue");

  const checks = await checkUpstreamReleases(pins, fakeFetch(pins, (pin) => pin.version));
  assert.deepEqual(checks.filter((check) => check.problems.length > 0).map((check) => [check.toolPack, check.problems]), []);
  assert.ok(checks.every((check) => check.statuses.join() === "current" && check.latest === check.version));
});

test("a current pin passes and writes the step summary", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const { code, stdout, summary, behind } = await nightly(root, releasing(root, {}));
  assert.equal(code, 0, stdout);
  assert.doesNotMatch(stdout, /::(?:error|warning)/u);
  assert.match(summary, /^\| Tool-pack \| Ecosystem \| Package \| Pinned \| Latest stable \| Gap \| Status \|$/mu);
  assert.match(summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.9\.4 \| current \| current \|$/mu);
  assert.match(summary, /Every one of the 2 upstreams is pinned at its latest stable release/u);
  assert.deepEqual(behind, []);
});

test("a patch or minor release upstream is reported and listed for the bump as a warning", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);

  const patch = await nightly(root, releasing(root, { "demo/bandit": "1.9.5" }));
  assert.equal(patch.code, 0, patch.stdout);
  assert.match(patch.summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.9\.5 \| patch \| behind \|$/mu);
  assert.match(patch.summary, /^1 of 2 upstreams are behind their latest stable release; the nightly's bump pull request moves them:$/mu);
  assert.match(patch.summary, /^- `demo\/bandit`: bandit 1\.9\.4 is pinned and PyPI's latest stable release is 1\.9\.5, a patch release: npm run upstream:bump -- --pack demo --tool bandit, /mu);
  assert.match(patch.stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit behind \(gap: patch\): /mu);
  assert.doesNotMatch(patch.stdout, /::error/u);
  assert.deepEqual(patch.behind, [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "1.9.4", latest: "1.9.5", gap: "patch", breaking: false }]);

  const minor = await nightly(root, releasing(root, { "demo/bandit": "1.10.0" }));
  assert.equal(minor.code, 0, minor.stdout);
  assert.match(minor.summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.10\.0 \| minor \| behind \|$/mu);
  assert.match(minor.stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit behind \(gap: minor\): /mu);
  assert.doesNotMatch(minor.stdout, /::error/u);
  assert.deepEqual(minor.behind, [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "1.9.4", latest: "1.10.0", gap: "minor", breaking: false }]);
});

test("a major release upstream, or a minor release of a 0.x pin, is breaking, and still only a warning", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const major = await nightly(root, releasing(root, { "demo/bandit": "2.0.0" }));
  assert.equal(major.code, 0, major.stdout);
  assert.match(major.summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 2\.0\.0 \| major \(breaking\) \| behind \|$/mu);
  assert.match(major.summary, /^- `demo\/bandit`: bandit 1\.9\.4 is pinned and PyPI's latest stable release is 2\.0\.0, a major release, which is breaking: /mu);
  assert.match(major.stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit behind \(gap: major \(breaking\)\): /mu);
  assert.doesNotMatch(major.stdout, /::error/u);
  assert.doesNotMatch(major.stdout, /::(?:error|warning) title=Upstream release \(rust\/clippy\)/u);
  assert.deepEqual(major.behind, [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "1.9.4", latest: "2.0.0", gap: "major", breaking: true }]);

  const zero = checkout(banditRuntime("0.9.4"), "0.9.4");
  roots.push(zero);
  const zeroMinor = await nightly(zero, releasing(zero, { "demo/bandit": "0.10.0" }));
  assert.equal(zeroMinor.code, 0, zeroMinor.stdout);
  assert.match(zeroMinor.summary, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 0\.9\.4 \| 0\.10\.0 \| breaking-minor \(breaking\) \| behind \|$/mu);
  assert.match(zeroMinor.stdout, /^::warning title=Upstream release \(demo\/bandit\)::/mu);
  assert.deepEqual(zeroMinor.behind, [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "0.9.4", latest: "0.10.0", gap: "breaking-minor", breaking: true }]);

  // A patch release of a 0.x pin is not breaking.
  const zeroPatch = await nightly(zero, releasing(zero, { "demo/bandit": "0.9.5" }));
  assert.equal(zeroPatch.code, 0, zeroPatch.stdout);
  assert.deepEqual(zeroPatch.behind, [{ pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "0.9.4", latest: "0.9.5", gap: "patch", breaking: false }]);
});

test("Rust's releases are compared on major.minor: a patch release is current, a minor one behind but not breaking", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const patch = await nightly(root, releasing(root, { "rust/clippy": "1.99.3" }));
  assert.equal(patch.code, 0, patch.stdout);
  assert.match(patch.summary, /^\| `rust\/clippy` \| Rust \| `rust` \| 1\.99\.0 \| 1\.99\.3 \| current \| current \|$/mu);
  assert.doesNotMatch(patch.stdout, /::(?:error|warning)/u);
  assert.deepEqual(patch.behind, []);

  const minor = await nightly(root, releasing(root, { "rust/clippy": "1.100.1" }));
  assert.equal(minor.code, 0, minor.stdout);
  assert.match(minor.summary, /^\| `rust\/clippy` \| Rust \| `rust` \| 1\.99\.0 \| 1\.100\.1 \| minor \| behind \|$/mu);
  assert.match(minor.stdout, /^::warning title=Upstream release \(rust\/clippy\)::/mu);
  assert.deepEqual(minor.behind, [{ pack: "rust", tool: "clippy", toolPack: "rust/clippy", pinned: "1.99.0", latest: "1.100.1", gap: "minor", breaking: false }]);
});

test("--behind lists every tool-pack behind upstream, breaking or not, sorted, and only those", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const both = await nightly(root, releasing(root, { "demo/bandit": "2.0.0", "rust/clippy": "1.100.0" }));
  assert.equal(both.code, 0, both.stdout);
  assert.deepEqual(both.behind, [
    { pack: "demo", tool: "bandit", toolPack: "demo/bandit", pinned: "1.9.4", latest: "2.0.0", gap: "major", breaking: true },
    { pack: "rust", tool: "clippy", toolPack: "rust/clippy", pinned: "1.99.0", latest: "1.100.0", gap: "minor", breaking: false },
  ]);
  assert.match(both.stdout, /^::warning title=Upstream release \(demo\/bandit\)::/mu);
  assert.match(both.stdout, /^::warning title=Upstream release \(rust\/clippy\)::/mu);

  // A tool-pack whose pins are not behind upstream (here they disagree) lists nothing to bump.
  const disagreeing = checkout(banditRuntime("1.9.3"));
  roots.push(disagreeing);
  assert.deepEqual((await nightly(disagreeing, releasing(disagreeing, {}))).behind, []);
});

test("gates.yaml and the runtime disagreeing about the pin is reported, as is a runtime that pins no release of upstream", async () => {
  const root = checkout(banditRuntime("1.9.3"));
  roots.push(root);
  const { code, stdout } = await nightly(root, releasing(root, {}));
  assert.equal(code, 0, stdout);
  assert.match(stdout, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| 1\.9\.4 \| current \| pins disagree \|$/mu);
  assert.match(stdout, /gates\.yaml upstream\.version is 1\.9\.4 but scripts\/pack-data\/runtimes\/bandit-1\.9\.4\/pyproject\.toml pins bandit 1\.9\.3/u);
  assert.match(stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit pins disagree/mu);
  assert.match(stdout, /^1 of 2 upstreams need a maintainer \(pins that disagree, or a registry that cannot be read\):$/mu);

  // Pins that disagree need a maintainer even when upstream is ahead by a release the bump could take.
  const behind = await nightly(root, releasing(root, { "demo/bandit": "1.9.5" }));
  assert.equal(behind.code, 0, behind.stdout);
  assert.match(behind.stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit pins disagree, behind \(gap: patch\): /mu);

  const missing = checkout(null);
  roots.push(missing);
  const unread = await nightly(missing, releasing(missing, {}));
  assert.equal(unread.code, 0, unread.stdout);
  assert.match(unread.stdout, /`demo\/bandit`: scripts\/pack-data\/runtimes\/bandit-1\.9\.4\/pyproject\.toml cannot be read/u);
});

test("a registry that cannot be read is a warning, not a pass", async () => {
  const root = checkout(BANDIT_1_9_4);
  roots.push(root);
  const pins = upstreamPins(root);
  const unavailable: Fetch = async (url) => (url.startsWith("https://pypi.org/") ? { ok: false, status: 503, text: async () => "" } : await fakeFetch(pins, (pin) => pin.version)(url, { headers: {} }));
  const answered = await nightly(root, unavailable);
  assert.equal(answered.code, 0, answered.stdout);
  assert.match(answered.stdout, /^::warning title=Upstream release \(demo\/bandit\)::demo\/bandit registry error/mu);
  assert.match(answered.stdout, /^\| `demo\/bandit` \| PyPI \| `bandit` \| 1\.9\.4 \| unknown \| unknown \| registry error \|$/mu);
  assert.match(answered.stdout, /`demo\/bandit`: https:\/\/pypi\.org\/pypi\/bandit\/json: HTTP 503/u);

  const offline = await checkUpstreamReleases(pins, async () => {
    throw new Error("getaddrinfo ENOTFOUND");
  });
  assert.deepEqual(offline.map((check) => [check.toolPack, check.statuses]), [["demo/bandit", ["registry error"]], ["rust/clippy", ["registry error"]]]);
  assert.match(offline[1]!.problems[0]!, /channel-rust-stable\.toml: getaddrinfo ENOTFOUND/u);

  const garbled = await checkUpstreamReleases(pins, async () => ({ ok: true, status: 200, text: async () => "<html>maintenance</html>" }));
  assert.ok(garbled.every((check) => check.statuses.includes("registry error")));
});
