/**
 * `npm run upstream:releases`: whether every upstream a tool-pack ports is still pinned at its latest stable release.
 * The nightly (upstream-releases.yml) runs it on main, so a release upstream publishes after the pin shows up in its
 * summary and in the night's bump pull request, rather than as rules quietly ported from an old version.
 *
 * Each gated tool-pack (`packs/<pack>/upstream/<tool>/gates.yaml`) pins its upstream twice: `upstream.version` in
 * gates.yaml, and the package its runtime installs (`conformance/recorders/port-readiness/runtimes/<upstream.runtime>/`:
 * package.json, pyproject.toml or composer.json). The two must agree, and the version is held to the latest stable
 * release the package's registry (npm, PyPI or Packagist) publishes. The Rust pack's Clippy ships with Rust: its pin is
 * the Rust toolchain its generators and its recorder's runtime refuse any other than (`packs/rust/upstream/common.py`,
 * `runtimes/clippy-<release>/rust-toolchain.toml`), held to the latest stable Rust's major.minor (clippyPin, releaseGap).
 *
 * Each tool-pack's gap to upstream is `current`, `patch`, `minor`, `major` or `breaking-minor` (a minor release of a 0.x
 * version); `major` and `breaking-minor` are breaking. It reports and never fails on what upstream published: a gap of
 * any kind, pins that disagree, and a registry that could not be read are each a row of the table and, under GitHub
 * Actions, a `::warning::` annotation. Exit 0 whatever it found, 2 on an exception. With GITHUB_STEP_SUMMARY set, the
 * table is appended to it too. `--behind <file>` writes every tool-pack behind upstream, breaking or not, there as JSON,
 * for the nightly to bump in its one bump pull request (scripts/upstream/bump.ts).
 */
import { appendFileSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { parse as parseToml } from "smol-toml";
import { z } from "zod";

import { gatedToolPacks, readGatesManifest, type GatesManifest } from "../../conformance/languages/manifest.js";
import { RUNTIMES_DIRECTORY } from "../../conformance/recorders/port-readiness/recorded-adapter.js";

/** The registries a gated tool-pack's runtime installs its upstream from. */
export type PackageEcosystem = "npm" | "pypi" | "packagist";
export type Ecosystem = PackageEcosystem | "rust";

/** How each ecosystem is named in the table. */
export const ECOSYSTEM_NAMES: Record<Ecosystem, string> = { npm: "npm", pypi: "PyPI", packagist: "Packagist", rust: "Rust" };

/** What one tool-pack pins of its upstream. */
export interface UpstreamPin {
  /** `<pack>/<tool>`. */
  toolPack: string;
  ecosystem: Ecosystem;
  /** The package as its registry names it; `rust` for the toolchain Clippy ships with. */
  package: string;
  /** The version the tool-pack's manifest pins (gates.yaml `upstream.version`, or common.py's toolchain). */
  version: string;
  /** Where the other pins of the same upstream say something else, or cannot be read; empty when they agree. */
  disagreements: string[];
  /** What to do once upstream has a newer release. */
  bump: string;
}

/**
 * The registry package of each non-ESLint adapter's upstream. gates.yaml names upstream by its repository, which is not
 * always the name its releases are published under, so the adapter (one per upstream linter) decides. The ESLint
 * adapter serves several upstreams; each names its package in `eslint.release.installed` (eslintPackage).
 */
const ADAPTER_PACKAGES: Record<Exclude<GatesManifest["adapter"], "eslint" | "clippy">, { ecosystem: PackageEcosystem; package: string }> = {
  ruff: { ecosystem: "pypi", package: "ruff" },
  bandit: { ecosystem: "pypi", package: "bandit" },
  sqlfluff: { ecosystem: "pypi", package: "sqlfluff" },
  // PHP_CodeSniffer moved to the PHPCSStandards organisation and still publishes under its old name.
  phpcs: { ecosystem: "packagist", package: "squizlabs/php_codesniffer" },
  // phpstan/phpstan-src is the source; phpstan/phpstan is the release Composer installs, tagged with the same versions.
  phpstan: { ecosystem: "packagist", package: "phpstan/phpstan" },
  psalm: { ecosystem: "packagist", package: "vimeo/psalm" },
  // tighten/tlint publishes as tightenco/tlint.
  tlint: { ecosystem: "packagist", package: "tightenco/tlint" },
  // Its runtime is the Blade toolkit (`blade`), which installs it beside laravel/framework.
  "blade-parser": { ecosystem: "packagist", package: "stillat/blade-parser" },
  // TomasVotruba/bladestan publishes as tomasvotruba/bladestan (Packagist names are lower case).
  bladestan: { ecosystem: "packagist", package: "tomasvotruba/bladestan" },
};

/**
 * The npm package an ESLint-family upstream's release is installed as: the package part of `eslint.release.installed`
 * (`@next/eslint-plugin-next/dist` is @next/eslint-plugin-next, `eslint/lib` is eslint itself).
 */
export function eslintPackage(installed: string): string {
  const parts = installed.split("/");
  return (installed.startsWith("@") ? parts.slice(0, 2) : parts.slice(0, 1)).join("/");
}

/** A requirement `name==version` in a pyproject.toml's `project.dependencies`, extras allowed. */
export const PYTHON_PIN = /^([A-Za-z0-9._-]+)(?:\[[^\]]*\])?\s*==\s*(\S+)$/u;

/** PEP 503's normal form of a Python package name, which is how two spellings of one package compare. */
export function normalPythonName(name: string): string {
  return name.toLowerCase().replace(/[-_.]+/gu, "-");
}

/** The parts of a runtime's pin files this check reads; the recorder holds the rest to its own rules (runtimes.ts). */
const npmManifestSchema = z.object({ dependencies: z.record(z.string(), z.string()).default({}) });
const composerManifestSchema = z.object({ require: z.record(z.string(), z.string()).default({}) });
const pyprojectSchema = z.object({ project: z.object({ dependencies: z.array(z.string()).default([]) }) });

/** The version a runtime's pin file pins `name` at, or why it cannot say. */
export function runtimePin(ecosystem: PackageEcosystem, file: string, text: string, name: string): { version: string } | { problem: string } {
  if (ecosystem === "npm") {
    const version = npmManifestSchema.parse(JSON.parse(text)).dependencies[name];
    return version === undefined ? { problem: `${file} has no dependency ${name}` } : { version };
  }
  if (ecosystem === "packagist") {
    const entry = Object.entries(composerManifestSchema.parse(JSON.parse(text)).require).find(([key]) => key.toLowerCase() === name);
    return entry === undefined ? { problem: `${file} does not require ${name}` } : { version: entry[1].replace(/^v/u, "") };
  }
  for (const dependency of pyprojectSchema.parse(parseToml(text)).project.dependencies) {
    const match = PYTHON_PIN.exec(dependency.trim());
    if (match !== null && normalPythonName(match[1]!) === normalPythonName(name)) return { version: match[2]! };
  }
  return { problem: `${file} pins no ${name}==<version>` };
}

/** The pin file of each ecosystem's runtimes. */
export const PIN_FILES: Record<PackageEcosystem, string> = { npm: "package.json", pypi: "pyproject.toml", packagist: "composer.json" };

/** A gated tool-pack's pins: gates.yaml's `upstream.version` and its runtime's pin of the same package. */
export function gatedPin(root: string, pack: string, tool: string): UpstreamPin {
  const manifest = readGatesManifest(root, pack, tool);
  if (manifest.adapter === "clippy") return clippyPin(root, pack, tool);
  const { ecosystem, package: name }: { ecosystem: PackageEcosystem; package: string } = manifest.adapter === "eslint" ? { ecosystem: "npm", package: eslintPackage(manifest.eslint!.release.installed) } : ADAPTER_PACKAGES[manifest.adapter];
  const file = `${RUNTIMES_DIRECTORY}/${manifest.upstream.runtime}/${PIN_FILES[ecosystem]}`;
  let pinned: { version: string } | { problem: string };
  try {
    pinned = runtimePin(ecosystem, file, readFileSync(join(root, file), "utf8"), name);
  } catch (error) {
    pinned = { problem: `${file} cannot be read: ${error instanceof Error ? error.message : String(error)}` };
  }
  const version = manifest.upstream.version;
  const disagreements = "problem" in pinned ? [pinned.problem] : pinned.version === version ? [] : [`gates.yaml upstream.version is ${version} but ${file} pins ${name} ${pinned.version}`];
  return {
    toolPack: manifest.id,
    ecosystem,
    package: name,
    version,
    disagreements,
    bump: `npm run port-readiness:bump -- --pack ${pack} --tool ${tool}, then record it again (the nightly opens this as a pull request)`,
  };
}

/**
 * Clippy's pin. It ships with the Rust toolchain, so the release to follow is Rust's: `TOOLCHAIN` in
 * packs/rust/upstream/common.py, which the pack's generators refuse any other toolchain than. The repository's own
 * rust-toolchain.toml pins the toolchain Rerail is built with, not the Clippy the pack ports, so it is not read here.
 * Every other pin must name the same release: the recorder's runtime (`runtimes/<upstream.runtime>/rust-toolchain.toml`,
 * its channel), gates.yaml's `upstream.version` and conformance.json's `tools[clippy].version` (Clippy's own version,
 * 0.1.<minor> for Rust 1.<minor>.<patch>), and gates.yaml's `upstream.commit` (common.py's `CLIPPY_COMMIT`).
 */
export function clippyPin(root: string, pack = "rust", tool = "clippy"): UpstreamPin {
  const manifest = readGatesManifest(root, pack, tool);
  const common = `packs/${pack}/upstream/common.py`;
  const conformance = `packs/${pack}/conformance.json`;
  const gates = `packs/${pack}/upstream/${tool}/gates.yaml`;
  const text = readFileSync(join(root, common), "utf8");
  const version = /^TOOLCHAIN = "([^"]+)"$/mu.exec(text)?.[1];
  if (version === undefined) throw new Error(`${common} sets no TOOLCHAIN = "<version>"`);
  const commit = /^CLIPPY_COMMIT = "([^"]+)"$/mu.exec(text)?.[1];
  const conformanceSchema = z.object({ tools: z.array(z.object({ name: z.string(), version: z.string() })) });
  const clippy = conformanceSchema.parse(JSON.parse(readFileSync(join(root, conformance), "utf8"))).tools.find((entry) => entry.name === "clippy")?.version;
  const expected = `0.1.${version.split(".")[1] ?? ""}`;
  const toolchainFile = `${RUNTIMES_DIRECTORY}/${manifest.upstream.runtime}/rust-toolchain.toml`;
  let channel: string | undefined;
  try {
    channel = z.object({ toolchain: z.object({ channel: z.string() }) }).parse(parseToml(readFileSync(join(root, toolchainFile), "utf8"))).toolchain.channel;
  } catch {
    channel = undefined;
  }
  const disagreements = [
    ...(clippy === expected ? [] : [`${common} pins Rust ${version}, whose Clippy is ${expected}, but ${conformance} tools[clippy].version is ${clippy ?? "missing"}`]),
    ...(manifest.upstream.version === expected ? [] : [`${common} pins Rust ${version}, whose Clippy is ${expected}, but ${gates} upstream.version is ${manifest.upstream.version}`]),
    ...(manifest.upstream.commit === commit ? [] : [`${gates} upstream.commit is ${manifest.upstream.commit}, but ${common} CLIPPY_COMMIT is ${commit ?? "missing"}`]),
    ...(channel === version ? [] : [`${common} pins Rust ${version}, but ${toolchainFile} ${channel === undefined ? "names no toolchain channel" : `pins ${channel}`}`]),
  ];
  return {
    toolPack: manifest.id,
    ecosystem: "rust",
    package: "rust",
    version,
    disagreements,
    bump: "npm run port-readiness:bump -- --pack rust --tool clippy, then record it again (the nightly opens this as a pull request)",
  };
}

/** Every pin this check follows: the gated tool-packs, in pack and tool order. */
export function upstreamPins(root: string): UpstreamPin[] {
  return gatedToolPacks(root).map(({ pack, tool }) => gatedPin(root, pack, tool));
}

/** A version's numeric components (`v9.7.0` is 9, 7, 0), or null when it is not a plain release (`4.0.0-RC1`, `0.17.0rc1`, `dev-main`). */
export function releaseParts(version: string): number[] | null {
  const match = /^v?(\d+(?:\.\d+)*)$/u.exec(version.trim());
  return match === null ? null : match[1]!.split(".").map(Number);
}

/** Two releases compared component by component, a missing component counting as 0 (`2.1` is `2.1.0`). */
export function compareReleases(a: number[], b: number[]): number {
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/** The highest of `versions` that is a plain release; null when none is. */
function highestRelease(versions: Iterable<string>): string | null {
  let best: { version: string; parts: number[] } | null = null;
  for (const version of versions) {
    const parts = releaseParts(version);
    if (parts !== null && (best === null || compareReleases(parts, best.parts) > 0)) best = { version: version.replace(/^v/u, ""), parts };
  }
  return best?.version ?? null;
}

/** The parts of each registry's answer the check reads. */
const npmPackumentSchema = z.object({ "dist-tags": z.object({ latest: z.string() }) });
const pypiProjectSchema = z.object({ releases: z.record(z.string(), z.array(z.object({ yanked: z.boolean().default(false) }))) });
const packagistSchema = z.object({ packages: z.record(z.string(), z.array(z.object({ version: z.string() }))) });
const rustChannelSchema = z.object({ pkg: z.object({ rust: z.object({ version: z.string() }) }) });

/** npm's latest release: the `latest` dist-tag, which `npm install <package>` installs. */
export function latestNpm(body: string): string {
  const latest = npmPackumentSchema.parse(JSON.parse(body))["dist-tags"].latest;
  if (releaseParts(latest) === null) throw new Error(`the latest dist-tag is ${latest}, not a release`);
  return latest;
}

/**
 * PyPI's latest stable release: the highest final release (no `a`, `b`, `rc`, `.dev` or `.post` part) with a file that is
 * not yanked. `info.version` is not used, since it can name a pre-release or a release whose files were all yanked.
 */
export function latestPypi(body: string): string {
  const { releases } = pypiProjectSchema.parse(JSON.parse(body));
  const latest = highestRelease(Object.entries(releases).filter(([, files]) => files.some((file) => !file.yanked)).map(([version]) => version));
  if (latest === null) throw new Error("no final release with a file that is not yanked");
  return latest;
}

/**
 * Packagist's latest stable release of `name`, from its Composer v2 metadata (`p2/<vendor>/<package>.json`, which holds
 * tagged versions only; branches are in `~dev.json`): the highest version that is a plain release, so `-alpha`, `-beta`,
 * `-RC` and `dev-` versions are skipped. Each entry of the minified list repeats `version`.
 */
export function latestPackagist(body: string, name: string): string {
  const latest = highestRelease((packagistSchema.parse(JSON.parse(body)).packages[name] ?? []).map((entry) => entry.version));
  if (latest === null) throw new Error(`no stable release of ${name}`);
  return latest;
}

/**
 * Rust's latest stable release, from the stable channel's manifest: `[pkg.rust] version = "1.99.1 (58cf8f9c3 2026-10-13)"`.
 * The manifest is what rustup installs as `stable`, so a release is in it exactly when it can be pinned; GitHub's
 * releases of rust-lang/rust are marked latest by hand and are read through an API that limits callers without a token.
 */
export function latestRust(body: string): string {
  const version = rustChannelSchema.parse(parseToml(body)).pkg.rust.version;
  const release = version.split(" ")[0]!;
  if (releaseParts(release) === null) throw new Error(`[pkg.rust] version is ${version}, not a release`);
  return release;
}

/** Where each ecosystem publishes a package's releases, the media type to ask for, and how to read the answer. */
export function registry(ecosystem: Ecosystem, name: string): { url: string; accept: string; latest: (body: string) => string } {
  switch (ecosystem) {
    case "npm":
      // The abbreviated document carries the dist-tags without every version's full metadata.
      return { url: `https://registry.npmjs.org/${name.replace("/", "%2f")}`, accept: "application/vnd.npm.install-v1+json", latest: latestNpm };
    case "pypi":
      return { url: `https://pypi.org/pypi/${name}/json`, accept: "application/json", latest: latestPypi };
    case "packagist":
      return { url: `https://repo.packagist.org/p2/${name}.json`, accept: "application/json", latest: (body) => latestPackagist(body, name) };
    case "rust":
      return { url: "https://static.rust-lang.org/dist/channel-rust-stable.toml", accept: "application/toml", latest: latestRust };
  }
}

/** The part of `fetch` the check uses; tests pass recorded answers instead of the network. */
export type Fetch = (url: string, init: { headers: Record<string, string>; signal?: AbortSignal }) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;

/** How long one registry request may take before the check fails it. */
const REQUEST_TIMEOUT_MS = 60_000;

export type ReleaseStatus = "current" | "behind" | "pins disagree" | "registry error";

/**
 * How far a pin is behind upstream's latest stable release: the first release component that differs. A `major` gap,
 * or a `minor` one while the pinned major is 0 (`breaking-minor`), is breaking; a `patch` gap is any later component.
 */
export type ReleaseGap = "current" | "patch" | "minor" | "major" | "breaking-minor";

/** Whether each gap fails the check. */
export const BREAKING_GAPS: Record<ReleaseGap, boolean> = { current: false, patch: false, minor: false, major: true, "breaking-minor": true };

/**
 * Whether an ecosystem's pins follow upstream's major.minor only. Clippy's version is 0.1.<minor> for every Rust
 * 1.<minor>.<patch>, so a Rust patch release ships no Clippy to port: it is not behind, and the nightly does not bump it.
 */
const FOLLOWS_MINOR: Record<Ecosystem, boolean> = { npm: false, pypi: false, packagist: false, rust: true };

/** The gap from `pinned` to `latest`, both plain releases, in `ecosystem`; `current` when latest is not newer. */
export function releaseGap(ecosystem: Ecosystem, pinned: number[], latest: number[]): ReleaseGap {
  const length = FOLLOWS_MINOR[ecosystem] ? 2 : Math.max(pinned.length, latest.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (latest[index] ?? 0) - (pinned[index] ?? 0);
    if (difference < 0) return "current";
    if (difference === 0) continue;
    if (index === 0) return "major";
    if (index === 1) return (pinned[0] ?? 0) === 0 ? "breaking-minor" : "minor";
    return "patch";
  }
  return "current";
}

/** How a gap reads in a problem: the kind of release the pin is behind by. */
const GAP_RELEASES: Record<Exclude<ReleaseGap, "current">, string> = {
  patch: "a patch release",
  minor: "a minor release",
  major: "a major release, which is breaking",
  "breaking-minor": "a minor release of a 0.x version, which is breaking",
};

/**
 * One tool-pack's verdict: its pin, upstream's latest stable release (null when the registry could not be read), the gap
 * between them (null when either is unknown or not a release), whether that gap is breaking, and what is wrong.
 */
export interface ReleaseCheck extends UpstreamPin {
  latest: string | null;
  gap: ReleaseGap | null;
  breaking: boolean;
  statuses: ReleaseStatus[];
  problems: string[];
}

/** Every tool-pack's pin against its registry's latest stable release, in `pins` order. */
export async function checkUpstreamReleases(pins: UpstreamPin[], fetch: Fetch): Promise<ReleaseCheck[]> {
  return await Promise.all(pins.map(async (pin): Promise<ReleaseCheck> => {
    const source = registry(pin.ecosystem, pin.package);
    const statuses: ReleaseStatus[] = [];
    const problems: string[] = [];
    if (pin.disagreements.length > 0) {
      statuses.push("pins disagree");
      problems.push(...pin.disagreements);
    }
    let latest: string | null = null;
    try {
      const response = await fetch(source.url, { headers: { accept: source.accept }, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      latest = source.latest(await response.text());
    } catch (error) {
      statuses.push("registry error");
      problems.push(`${source.url}: ${error instanceof Error ? error.message : String(error)}`);
    }
    const pinned = releaseParts(pin.version);
    let gap: ReleaseGap | null = null;
    if (pinned === null) {
      statuses.push("pins disagree");
      problems.push(`the pin ${pin.version} is not a release version`);
    } else if (latest !== null) {
      gap = releaseGap(pin.ecosystem, pinned, releaseParts(latest)!);
      if (gap !== "current") {
        statuses.push("behind");
        problems.push(`${pin.package} ${pin.version} is pinned and ${ECOSYSTEM_NAMES[pin.ecosystem]}'s latest stable release is ${latest}, ${GAP_RELEASES[gap]}: ${pin.bump}`);
      }
    }
    const breaking = gap !== null && BREAKING_GAPS[gap];
    return { ...pin, latest, gap, breaking, statuses: statuses.length === 0 ? ["current"] : [...new Set(statuses)], problems };
  }));
}

/** Whether a check needs a maintainer rather than a bump: its pins disagree, or its registry could not be read. */
export function needsAttention(check: ReleaseCheck): boolean {
  return check.statuses.some((status) => status === "pins disagree" || status === "registry error");
}

/** How a check's gap reads in the table: its kind, marked when breaking, or unknown. */
function gapCell(check: ReleaseCheck): string {
  if (check.gap === null) return "unknown";
  return check.breaking ? `${check.gap} (breaking)` : check.gap;
}

/** The checks as Markdown: one table row per tool-pack, then the ones needing attention, then the others behind. */
export function renderMarkdown(checks: ReleaseCheck[]): string {
  const attention = checks.filter(needsAttention);
  const behind = checks.filter((check) => !needsAttention(check) && check.problems.length > 0);
  const lines = [
    "## Upstream releases",
    "",
    "| Tool-pack | Ecosystem | Package | Pinned | Latest stable | Gap | Status |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...checks.map((check) => `| \`${check.toolPack}\` | ${ECOSYSTEM_NAMES[check.ecosystem]} | \`${check.package}\` | ${check.version} | ${check.latest ?? "unknown"} | ${gapCell(check)} | ${check.statuses.join(", ")} |`),
    "",
  ];
  if (attention.length === 0 && behind.length === 0) lines.push(`Every one of the ${checks.length} upstreams is pinned at its latest stable release.`);
  if (attention.length > 0) {
    lines.push(`${attention.length} of ${checks.length} upstreams need a maintainer (pins that disagree, or a registry that cannot be read):`);
    lines.push(...attention.flatMap((check) => check.problems.map((problem) => `- \`${check.toolPack}\`: ${problem}`)));
  }
  if (behind.length > 0) {
    if (attention.length > 0) lines.push("");
    lines.push(`${behind.length} of ${checks.length} upstreams are behind their latest stable release; the nightly's bump pull request moves them:`);
    lines.push(...behind.flatMap((check) => check.problems.map((problem) => `- \`${check.toolPack}\`: ${problem}`)));
  }
  return `${lines.join("\n")}\n`;
}

/** A tool-pack behind its upstream, as the nightly's bump-dispatch job reads it: one commit each in the nightly's bump pull request. */
export interface Behind {
  pack: string;
  tool: string;
  toolPack: string;
  pinned: string;
  latest: string;
  gap: Exclude<ReleaseGap, "current">;
  breaking: boolean;
}

/** Every tool-pack whose pin is behind upstream's latest stable release, breaking or not, sorted by tool-pack. */
export function behindToolPacks(checks: ReleaseCheck[]): Behind[] {
  const behind: Behind[] = [];
  for (const check of checks) {
    if (check.latest === null || check.gap === null || check.gap === "current") continue;
    const [pack, tool] = check.toolPack.split("/");
    behind.push({ pack: pack!, tool: tool!, toolPack: check.toolPack, pinned: check.version, latest: check.latest, gap: check.gap, breaking: check.breaking });
  }
  return behind.sort((a, b) => (a.toolPack < b.toolPack ? -1 : a.toolPack > b.toolPack ? 1 : 0));
}

/**
 * Run the check over this checkout and report it; the exit code, 0 whatever it found (the report is the result). With
 * `behind`, every tool-pack behind upstream is written there as JSON (behindToolPacks), so the nightly can bump them.
 */
export async function main(root: string, fetch: Fetch, env: NodeJS.ProcessEnv = process.env, options: { behind?: string } = {}): Promise<number> {
  const checks = await checkUpstreamReleases(upstreamPins(root), fetch);
  const markdown = renderMarkdown(checks);
  process.stdout.write(markdown);
  const step = env.GITHUB_STEP_SUMMARY;
  if (step !== undefined && step !== "") appendFileSync(step, markdown);
  if (options.behind !== undefined) writeFileSync(options.behind, `${JSON.stringify(behindToolPacks(checks))}\n`);
  if (env.GITHUB_ACTIONS === "true") {
    // One warning per tool-pack with anything to report. A workflow command's data escapes `%`, CR and LF.
    for (const check of checks.filter((entry) => entry.problems.length > 0)) {
      const data = `${check.toolPack} ${check.statuses.join(", ")} (gap: ${gapCell(check)}): ${check.problems.join("; ")}`;
      process.stdout.write(`::warning title=Upstream release (${check.toolPack})::${data.replace(/%/gu, "%25").replace(/\r/gu, "%0D").replace(/\n/gu, "%0A")}\n`);
    }
  }
  return 0;
}

// Imported by its tests, run as a script by the nightly: only the second checks.
const invoked = (() => {
  try {
    return process.argv[1] !== undefined && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invoked) {
  let behind: string | undefined;
  try {
    behind = parseArgs({ options: { behind: { type: "string" } }, strict: true }).values.behind;
  } catch {
    process.stderr.write("usage: npm run port-readiness:upstream-releases [-- --behind <file>]\n");
    process.exitCode = 2;
  }
  if (process.exitCode !== 2) {
    main(resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."), globalThis.fetch, process.env, behind === undefined ? {} : { behind: resolve(behind) }).then((code) => {
      process.exitCode = code;
    }, (error: unknown) => {
      process.stderr.write(`scripts/upstream/releases.ts: ${error instanceof Error ? error.message : String(error)}\n`);
      process.exitCode = 2;
    });
  }
}
