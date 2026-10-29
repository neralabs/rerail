#!/usr/bin/env node
// Files that pin a version in step with a dependency in package.json, brought back in step with it. Dependabot bumps
// package.json alone; .github/workflows/dependabot-pins.yml runs this on Dependabot's pull request and commits what it
// changes, so a bump arrives with everything that follows it:
//
//   @oh-my-pi/pi-coding-agent (the OMP SDK)
//     scripts/host/install-omp-host.mjs   the standalone OMP host's version, and the SHA-256 digest of each release asset
//                                         it pins, read from the GitHub release. The asset set itself stays as pinned.
//   @ast-grep/cli
//     test/engine/code-rule/code-rule-dsl-upstream-differential.test.ts   the reference version (UPSTREAM_VERSION).
//
// `--check` changes nothing and fails when a pin is out of step; it compares versions only, so it needs no network.
// GH_TOKEN or GITHUB_TOKEN, when set, authenticates the release lookup.
//
//   node scripts/deps/sync-pins.mjs [--check]   (npm run deps:sync; --check is npm run check:pins, part of npm run check)
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const OMP_SDK = "@oh-my-pi/pi-coding-agent";
export const OMP_RELEASES = "can1357/oh-my-pi";
export const AST_GREP_CLI = "@ast-grep/cli";
export const OMP_HOST = "scripts/host/install-omp-host.mjs";
export const DIFFERENTIAL_TEST = "test/engine/code-rule/code-rule-dsl-upstream-differential.test.ts";

const VERSION_LINE = /^const version = "([^"]+)";$/mu;
const DIGESTS_BLOCK = /^(const digests = \{\n)([\s\S]*?)(\n\};)$/mu;
const DIGEST_ENTRY = /^ {2}"([^"]+)": "([0-9a-f]{64})",$/u;

/** The OMP host pin in install-omp-host.mjs: its version, and each pinned release asset with its digest. */
export function ompHostPin(text) {
  const version = VERSION_LINE.exec(text)?.[1];
  const block = DIGESTS_BLOCK.exec(text)?.[2];
  if (version === undefined || block === undefined) throw new Error(`${OMP_HOST}: no \`const version\` or \`const digests\` block`);
  const digests = {};
  for (const line of block.split("\n")) {
    const entry = DIGEST_ENTRY.exec(line);
    if (entry === null) throw new Error(`${OMP_HOST}: unexpected line in the digests block: ${line}`);
    digests[entry[1]] = entry[2];
  }
  return { version, digests };
}

/** install-omp-host.mjs pinned at `version`, each of its pinned assets at its digest in `digests` (asset → hex). */
export function withOmpHostPin(text, version, digests) {
  const { digests: pinned } = ompHostPin(text);
  const missing = Object.keys(pinned).filter((asset) => !/^[0-9a-f]{64}$/u.test(digests[asset] ?? ""));
  if (missing.length > 0) throw new Error(`OMP ${version}'s release has no SHA-256 digest for ${missing.join(", ")}`);
  const block = Object.keys(pinned).map((asset) => `  "${asset}": "${digests[asset]}",`).join("\n");
  return text.replace(VERSION_LINE, `const version = "${version}";`).replace(DIGESTS_BLOCK, `$1${block}$3`);
}

/** The differential test's reference ast-grep version. */
export function astGrepReference(text) {
  const version = /^const UPSTREAM_VERSION = "([^"]+)";$/mu.exec(text)?.[1];
  if (version === undefined) throw new Error(`${DIFFERENTIAL_TEST}: no \`const UPSTREAM_VERSION\``);
  return version;
}

/** The differential test with its reference ast-grep version set to `version`. */
export function withAstGrepReference(text, version) {
  astGrepReference(text);
  return text.replace(/^const UPSTREAM_VERSION = "[^"]+";$/mu, `const UPSTREAM_VERSION = "${version}";`);
}

/** A version a package.json names exactly (`18.12.2`), or an error: a pin follows one version, never a range. */
export function exactVersion(manifest, name) {
  const version = manifest.devDependencies?.[name] ?? manifest.dependencies?.[name];
  if (version === undefined || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/u.test(version)) {
    throw new Error(`package.json: ${name} must be an exact version for the files that follow it, not ${JSON.stringify(version)}`);
  }
  return version;
}

/** Each pin out of step with package.json, as `{ file, pin, has, wants }`. */
export function pinDrift({ manifest, ompHost, differentialTest }) {
  const sdk = exactVersion(manifest, OMP_SDK);
  const cli = exactVersion(manifest, AST_GREP_CLI);
  const drift = [];
  const host = ompHostPin(ompHost).version;
  if (host !== sdk) drift.push({ file: OMP_HOST, pin: "OMP host", has: host, wants: sdk });
  const reference = astGrepReference(differentialTest);
  if (reference !== cli) drift.push({ file: DIFFERENTIAL_TEST, pin: "ast-grep reference", has: reference, wants: cli });
  return drift;
}

/** The SHA-256 digest of each asset of OMP release `version`, from the GitHub API: asset → hex. */
async function ompReleaseDigests(version) {
  const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
  const response = await fetch(`https://api.github.com/repos/${OMP_RELEASES}/releases/tags/v${version}`, {
    headers: { accept: "application/vnd.github+json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error(`OMP release v${version}: GitHub answered HTTP ${response.status}`);
  const { assets } = await response.json();
  return Object.fromEntries(assets.filter((asset) => asset.digest?.startsWith("sha256:")).map((asset) => [asset.name, asset.digest.slice(7)]));
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const read = (path) => readFileSync(join(root, path), "utf8");
  const files = {
    manifest: JSON.parse(read("package.json")),
    ompHost: read(OMP_HOST),
    differentialTest: read(DIFFERENTIAL_TEST),
  };
  const drift = pinDrift(files);
  if (process.argv.includes("--check")) {
    for (const { file, pin, has, wants } of drift) console.error(`${file}: ${pin} is ${has}, package.json has ${wants}`);
    console.log(drift.length === 0 ? "check-pins: every pin is in step with package.json" : `check-pins: ${drift.length} pin(s) out of step; run npm run deps:sync`);
    process.exit(drift.length === 0 ? 0 : 1);
  }
  const sdk = exactVersion(files.manifest, OMP_SDK);
  if (drift.some((entry) => entry.file === OMP_HOST)) {
    writeFileSync(join(root, OMP_HOST), withOmpHostPin(files.ompHost, sdk, await ompReleaseDigests(sdk)));
  }
  if (drift.some((entry) => entry.file === DIFFERENTIAL_TEST)) {
    writeFileSync(join(root, DIFFERENTIAL_TEST), withAstGrepReference(files.differentialTest, exactVersion(files.manifest, AST_GREP_CLI)));
  }
  for (const { file, pin, has, wants } of drift) console.log(`${file}: ${pin} ${has} -> ${wants}`);
  if (drift.length === 0) console.log("Every pin is already in step with package.json.");
}
