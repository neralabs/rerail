#!/usr/bin/env node
// The engine packages' part of a core release (docs/release.md, "Engine packages"). Core names the six engine packages
// as optional dependencies, each at core's version and at the integrity of the tarball the release packed
// (packages.mjs). npm skips an optional dependency the registry lacks, or whose bytes are not the ones core pins, and
// exits 0: an install with no engine. So release.yml publishes the six first, reads each one's integrity back from the
// registry, and publishes core only once npm has all six as packed (`publish`).
//
// The six are published under core's dist-tag, and promoted and put back with core: `specs` names them for those
// steps. A core version from before ENGINE_PACKAGES_SINCE has none.
//
// Nothing here needs node_modules: the jobs that publish and promote install none.
//
//   RELEASE_VERSION=<version> RELEASE_CHANNEL=<dist-tag> node scripts/release/engine/release.mjs publish <folder>
//   node scripts/release/engine/release.mjs specs <version> [<dist-tag>]
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { pathToFileURL } from "node:url";
import { compareVersions, readPublishedManifestOrNone } from "../dist-tags.mjs";
import { alreadyOnRegistry, REGISTRY } from "../packs.mjs";
import { absentEnginePackages, ENGINE_PACKAGES, engineTarballsProblem, readEngineManifest } from "./packages.mjs";

/**
 * The first core release whose engines are packages of their own. Every release before it carries all six engines in
 * core's own tarball; 0.2.0, the last built that way, was refused by npm's registry for its size and never published.
 */
export const ENGINE_PACKAGES_SINCE = "0.2.1";

/** Whether core `version` has engine packages: ENGINE_PACKAGES_SINCE, its previews included, or a later version. */
export const hasEnginePackages = (version) => compareVersions(version, `${ENGINE_PACKAGES_SINCE}-0`) >= 0;

/** The engine package for `platform` and `architecture` (this machine's by default); undefined where Rerail has no engine. */
export const enginePackageFor = (platform = process.platform, architecture = process.arch) => ENGINE_PACKAGES.find(({ os, cpu }) => os === platform && cpu === architecture);

/**
 * The engine packages released with core `version`, as the dist-tag steps name a package: `<name>@<version>`, or
 * `<name>@<version>:<dist-tag>` with `distTag`. None for a version from before the engine packages, which a put-back
 * can still name.
 */
export function engineSpecs(version, distTag) {
  if (!hasEnginePackages(version)) return [];
  return ENGINE_PACKAGES.map(({ name }) => `${name}@${version}${distTag === undefined ? "" : `:${distTag}`}`);
}

/**
 * The tarballs a registry must serve for the core tarball at `coreTarball` to install with an engine: the engine
 * packages `pack` wrote beside it (packages.mjs), then core. The install checks publish these to a registry of their
 * own. Throws when the folder has no manifest, or a tarball its manifest lists is absent or not the one packed.
 */
export function releaseTarballs(coreTarball) {
  const core = resolve(coreTarball);
  const folder = dirname(core);
  const manifest = readEngineManifest(folder);
  const problem = engineTarballsProblem(folder, manifest);
  if (problem !== undefined) throw new Error(problem);
  return [...manifest.packages.map(({ file }) => join(folder, file)), core];
}

/**
 * `name`'s dist-tags, read anonymously, or null when npm has no public package of that name. For a scoped name it has
 * never had, npm answers 401 here, not the 404 an unscoped name gets (asked of registry.npmjs.org on 2026-11-23), so
 * both mean none. Any other answer throws: a registry that could not be read is never taken for an absent package.
 */
async function readEngineDistTags(name) {
  const response = await fetch(`${REGISTRY}/-/package/${name.replace("/", "%2f")}/dist-tags`, { headers: { accept: "application/json" } });
  if (response.status === 401 || response.status === 404) return null;
  if (!response.ok) throw new Error(`${name}'s dist-tags answer an anonymous request with ${response.status}`);
  return await response.json();
}

/** How this script reads the registry; a test passes its own. */
const REGISTRY_READS = { distTags: readEngineDistTags, manifest: readPublishedManifestOrNone };

/**
 * Whether npm has `name` at `version`. A dist-tag at the version settles it at once: the dist-tags see a publish
 * straight away, where the version's own document can answer 404 for minutes. `read.distTags(name)` resolves to the
 * package's dist-tags, or null when npm has no such package, and `read.manifest(name, version)` to the version's
 * document or null; both throw on an answer that is neither.
 */
async function onNpm(name, version, read) {
  if (Object.values((await read.distTags(name)) ?? {}).includes(version)) return true;
  return (await read.manifest(name, version)) !== null;
}

/** The names of the engine packages npm has at `version` (onNpm). */
export async function publishedEnginePackages(version, read = REGISTRY_READS) {
  const published = [];
  for (const { name } of ENGINE_PACKAGES) {
    if (await onNpm(name, version, read)) published.push(name);
  }
  return published;
}

/**
 * Why release.yml's `publish` input can't retry core `version` while npm has the engine packages `published` at it
 * (publishedEnginePackages), or null when it has none. A retry builds the engines again, and nothing makes a rebuilt
 * engine the bytes npm already has under that version, which it never replaces: core would pin an integrity no
 * published engine package has.
 */
export function rebuildRefusal(version, published) {
  if (published.length === 0) return null;
  return `npm already has ${published.join(", ")} at ${version}, and a retry builds the engines again, to other bytes than those: re-run the failed publish job of the run that published them, which still holds their tarballs, or release the next version, through the Release pull request`;
}

/** How often the read-back asks the registry for a version it does not serve yet, and how long it waits between. */
const READ_BACK_ATTEMPTS = 20;
const READ_BACK_PAUSE_MS = 15_000;

/** A manifest entry as npm names a version: `<name>@<version>`. */
const specOf = ({ name, version }) => `${name}@${version}`;

/** Throws unless npm knows every package of `packages`, each asked before anything is published. */
async function requirePublishedNames(packages, steps) {
  const first = [];
  for (const { name } of packages) {
    if (!(await steps.exists(name))) first.push(name);
  }
  if (first.length === 0) return;
  throw new Error(`First publish needed: npm has no public package named ${first.join(", ")}. Publish each by hand and configure its trusted publisher (docs/publishing-the-engine-packages.md), then run this job again`);
}

/** Publishes one manifest entry under `distTag`; a version npm already has is left as it is. */
function publishEntry(entry, distTag, steps) {
  const published = steps.publish(entry.file, distTag);
  if (!published.ok && !alreadyOnRegistry(published.output)) throw new Error(`npm publish ${specOf(entry)} failed`);
  steps.log(published.ok ? `published ${specOf(entry)} under ${distTag}` : `${specOf(entry)} is already on npm (a re-run): not published again`);
}

/** The entries of `entries` npm does not serve yet. Throws on one it serves at another integrity than the manifest's. */
async function unserved(entries, steps) {
  const pending = [];
  for (const entry of entries) {
    const served = await steps.readIntegrity(entry.name, entry.version);
    if (served !== null && served !== entry.integrity) {
      throw new Error(`npm has ${specOf(entry)} as ${served}, and this run packed ${entry.integrity}: an earlier run or attempt published other bytes, which npm never replaces. Core pins this run's, so its install would have no engine there. Release the next version: re-running a run's failed jobs keeps its tarballs, while re-running all of its jobs, or retrying from main, builds them again`);
    }
    if (served === null) pending.push(entry);
    else steps.log(`npm serves ${specOf(entry)} as packed`);
  }
  return pending;
}

/** Reads each entry's integrity back until npm serves every one as packed; throws when it never does (unserved). */
async function readBack(entries, steps) {
  let pending = entries;
  for (let attempt = 1; ; attempt += 1) {
    pending = await unserved(pending, steps);
    if (pending.length === 0) return;
    const names = pending.map(specOf).join(", ");
    if (attempt === READ_BACK_ATTEMPTS) throw new Error(`npm does not serve ${names} yet, so whether it has ${pending.length === 1 ? "it" : "them"} as packed is unknown: run this job again`);
    steps.log(`reading ${attempt}: npm does not serve ${names} yet; reading again`);
    await steps.wait();
  }
}

/**
 * Publishes the engine packages `manifest` lists (packages.mjs) under `distTag`, then reads each one's integrity back
 * from the registry. It returns only when npm has all six at the integrity the manifest gives, which is the one core
 * pins; core is published after it, never before. It throws, and core stays unpublished, when:
 *
 * - the manifest lacks an engine package;
 * - a package was never published. Trusted publishing can only be configured for a package that exists, so its first
 *   publish is made by hand. Every name is asked before anything is published;
 * - a publish fails. A version npm already has is not a failure: this job run again publishes what is left;
 * - npm has a version at another integrity: an earlier run published other bytes, and npm never replaces them. Every
 *   version is read before anything is published, so this run then publishes nothing;
 * - npm does not serve a version within READ_BACK_ATTEMPTS readings.
 *
 * `steps` does the I/O: `exists(name)` resolves to whether npm knows the package, `publish(file, distTag)` returns
 * `{ ok, output }`, `readIntegrity(name, version)` resolves to the `dist.integrity` npm serves, or null while it serves
 * none, and `wait()` pauses between readings.
 */
export async function publishEnginePackages(manifest, distTag, steps) {
  const absent = absentEnginePackages(manifest);
  if (absent.length > 0) throw new Error(`the manifest lacks ${absent.map(({ name }) => name).join(", ")}: a release publishes all ${ENGINE_PACKAGES.length} engine packages`);
  await requirePublishedNames(manifest.packages, steps);
  // Read before anything is published: a version npm already has at another integrity ends the run here. Published
  // first, this run's other packages would be on npm for good beside it, and no core could pin all six.
  const pending = await unserved(manifest.packages, steps);
  for (const entry of pending) publishEntry(entry, distTag, steps);
  await readBack(manifest.packages, steps);
}

/**
 * Runs npm publish for one tarball; whether it succeeded, and its error output. `--provenance` makes npm fail the
 * publish when it cannot attach an attestation, as for core. Its `npm error` lines, the publish conflict included, go
 * to stderr, which is captured and shown when npm exits.
 */
function publish(path, distTag) {
  const run = spawnSync("npm", ["publish", path, "--registry", REGISTRY, "--access", "public", "--tag", distTag, "--provenance"], { stdio: ["inherit", "inherit", "pipe"], encoding: "utf8" });
  process.stderr.write(run.stderr ?? "");
  if (run.error !== undefined) throw run.error;
  return { ok: run.status === 0, output: run.stderr };
}

/** The `dist.integrity` npm serves for `name` at `version`, or null while it serves no such version. */
async function servedIntegrity(name, version) {
  const document = await readPublishedManifestOrNone(name, version);
  return document === null ? null : (document.dist?.integrity ?? "no integrity");
}

/** The `publish` command: the folder's manifest and tarballs checked against the release, then published and read back. */
async function publishFolder(directory) {
  const { RELEASE_VERSION: version = "", RELEASE_CHANNEL: channel = "" } = process.env;
  if (version === "" || channel === "") throw new Error("RELEASE_VERSION and RELEASE_CHANNEL name what the run releases, and one is empty");
  const folder = resolve(directory);
  const manifest = readEngineManifest(folder);
  if (manifest.version !== version) throw new Error(`${folder} holds the engine packages of ${manifest.version}, and the run releases ${version}`);
  const problem = engineTarballsProblem(folder, manifest);
  if (problem !== undefined) throw new Error(problem);
  await publishEnginePackages(manifest, channel, {
    exists: async (name) => (await readEngineDistTags(name)) !== null,
    publish: (file, distTag) => publish(join(folder, file), distTag),
    readIntegrity: servedIntegrity,
    wait: () => sleep(READ_BACK_PAUSE_MS),
    log: (line) => console.log(line),
  });
  console.log(`npm has the ${ENGINE_PACKAGES.length} engine packages of ${version} as packed: core can be published.`);
}

const USAGE = "usage: engine/release.mjs publish <folder written by engine/packages.mjs pack> | specs <version> [<dist-tag>]";

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [mode, ...rest] = process.argv.slice(2);
  if (mode === "specs" && rest[0] !== undefined) {
    console.log(engineSpecs(rest[0], rest[1]).join(" "));
  } else if (mode === "publish" && rest[0] !== undefined) {
    try {
      await publishFolder(rest[0]);
    } catch (error) {
      console.log(`::error title=Engine packages::${error instanceof Error ? error.message : String(error)}. Core is not published.`);
      process.exit(1);
    }
  } else {
    process.stderr.write(`${USAGE}\n`);
    process.exit(2);
  }
}
