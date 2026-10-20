// Shared by the pack release scripts in this directory (docs/pack-format.md, "Versioning and release"). Each
// released pack version is marked by the git tag `<npm name>@<version>`, changesets' own tag form: a version without
// its tag has not been published yet.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const PACKS = join(ROOT, "packs");
export const REGISTRY = "https://registry.npmjs.org";
/** Every pack is published under `next` first; `latest` moves only by promotion. */
export const PUBLISH_TAG = "next";

/** The git tag that marks `name`'s `version` as published. */
export const releaseTag = (name, version) => `${name}@${version}`;

/** `gh api` against this repository (`GH_REPO`, else the checkout's remote), its output as text. */
const ghApi = (args) => execFileSync("gh", ["api", ...args], { cwd: ROOT, encoding: "utf8" });

/** Every pack release tag on GitHub, without `refs/tags/`. */
export const releaseTags = () =>
  ghApi(["--paginate", "repos/{owner}/{repo}/git/matching-refs/tags/@neralabs/rerail-pack-", "--jq", ".[].ref"])
    .split("\n")
    .filter((ref) => ref !== "")
    .map((ref) => ref.replace(/^refs\/tags\//u, ""));

/** Creates release tag `tag` at commit `sha` on GitHub. */
export const createReleaseTag = (tag, sha) => ghApi(["repos/{owner}/{repo}/git/refs", "-f", `ref=refs/tags/${tag}`, "-f", `sha=${sha}`, "--silent"]);

/** Fails unless commit `sha` is on GitHub and gh can read the repository: a tag can only name a pushed commit. */
export function assertCommitOnGitHub(sha) {
  try {
    ghApi([`repos/{owner}/{repo}/commits/${sha}`, "--silent"]);
  } catch {
    throw new Error(`commit ${sha} is not on GitHub, or gh cannot read the repository: push it, and run gh auth login with write access`);
  }
}

/** Whether npm publish's output says the registry already has this version. */
export const alreadyOnRegistry = (output) => /cannot publish over the previously published versions?|EPUBLISHCONFLICT/iu.test(output);

/**
 * Releases each `{ name, version, file }` of a packs-pack.ts manifest, so that any failed run can be run again:
 * a version whose tag exists is skipped; one the registry already has without its tag (a run that failed after
 * publishing) is not published again, only checked and tagged. `steps` does the I/O: `publish(file)` returns
 * `{ ok, output }`, `assertRestricted(name)` and `tag(tag)` throw on failure.
 */
export async function releasePacks(manifest, tags, steps) {
  const existing = new Set(tags);
  for (const { name, version, file } of manifest) {
    const tag = releaseTag(name, version);
    if (existing.has(tag)) {
      steps.log(`${tag} is already released; skipped`);
      continue;
    }
    const published = steps.publish(file);
    if (!published.ok && !alreadyOnRegistry(published.output)) throw new Error(`npm publish ${tag} failed`);
    if (!published.ok) steps.log(`${tag} is already on the registry without its tag; checking and tagging it`);
    await steps.assertRestricted(name);
    steps.tag(tag);
    steps.log(`released ${tag} under ${PUBLISH_TAG}`);
  }
}

/** Every pack under `packsDirectory`: `{ id, name, version }`, sorted by id. */
export function listPacks(packsDirectory = PACKS) {
  return readdirSync(packsDirectory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const { name, version } = JSON.parse(readFileSync(join(packsDirectory, entry.name, "package.json"), "utf8"));
      return { id: entry.name, name, version };
    })
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** Every pending changeset in `directory`: `{ file, text }`, sorted by file name. */
export function readChangesets(directory = join(ROOT, ".changeset")) {
  return readdirSync(directory)
    .filter((file) => file.endsWith(".md") && file !== "README.md")
    .sort()
    .map((file) => ({ file, text: readFileSync(join(directory, file), "utf8") }));
}

/**
 * What the pending `changesets` bump past a patch on a pack below 1.0.0. A pack is pre-release until 1.0.0, so every
 * change to it is a patch (docs/pack-format.md, "Versioning and release"); `changeset version` would otherwise take
 * 0.1.x to 0.2.0 on a minor and to 1.0.0 on a major.
 */
export function changesetBumpFailures(changesets, packs) {
  const versions = new Map(packs.map((pack) => [pack.name, pack.version]));
  const failures = [];
  for (const { file, text } of changesets) {
    const front = /^---\r?\n([\s\S]*?)\r?\n---/u.exec(text)?.[1] ?? "";
    for (const line of front.split(/\r?\n/u)) {
      const entry = /^\s*["']?([^"':]+)["']?\s*:\s*(patch|minor|major)\s*$/u.exec(line);
      if (entry === null) continue;
      const [, name, bump] = entry;
      const version = versions.get(name);
      if (version === undefined || bump === "patch" || !version.startsWith("0.")) continue;
      failures.push(`${file}: ${name} is ${version}, below 1.0.0, so its bump is patch, not ${bump}`);
    }
  }
  return failures;
}

/**
 * Which packs a release run publishes, given the git tags that exist. A pack whose version has its tag is already
 * published. One with no tag for its package at all has never been published, and its first publish is made by hand
 * (trusted publishing can only be configured for a package that exists): it is `first`, not `publish`.
 */
export function planPacks(packs, tags) {
  const existing = new Set(tags);
  const publish = [];
  const first = [];
  for (const pack of packs) {
    if (existing.has(releaseTag(pack.name, pack.version))) continue;
    if (tags.some((tag) => tag.startsWith(`${pack.name}@`))) publish.push(pack.id);
    else first.push(pack.id);
  }
  return { publish, first };
}

/**
 * Which packs a promotion moves to `latest`, given the release tags and `latestOf(name)`, the version npm's `latest`
 * names (null when it names none): each pack whose package.json version is released (its tag exists) and is not
 * `latest` already. package.json holds the newest version a release run published, so a `latest` that differs from it
 * is older. A version without its tag has not been published, so it is `unreleased`, not promoted.
 */
export function planPromotions(packs, tags, latestOf) {
  const existing = new Set(tags);
  const promote = [];
  const unreleased = [];
  for (const pack of packs) {
    if (!existing.has(releaseTag(pack.name, pack.version))) {
      unreleased.push(pack);
      continue;
    }
    const latest = latestOf(pack.name);
    if (latest !== pack.version) promote.push({ ...pack, latest });
  }
  return { promote, unreleased };
}

/**
 * Points `latest` at each `{ name, version }` in `promotions`, refusing a version this repository never released (no
 * tag). One failure does not stop the others; returns each failure, so the run reports them all and can be run again,
 * which skips the packs already promoted. `steps.promote(name, version)` throws on failure.
 */
export function promotePacks(promotions, tags, steps) {
  const existing = new Set(tags);
  const failures = [];
  for (const { name, version } of promotions) {
    const tag = releaseTag(name, version);
    try {
      if (!existing.has(tag)) throw new Error("it was never released from this repository (no such git tag)");
      steps.promote(name, version);
      steps.log(`promoted ${tag} to latest`);
    } catch (error) {
      failures.push(`${tag}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  return failures;
}

/**
 * `package-lock.json` with each pack workspace's recorded version set to its package.json's, and nothing else
 * changed: `changeset version` bumps the packs but leaves the lockfile behind.
 */
export function syncPackLockfile(lock, packs) {
  const packages = { ...lock.packages };
  for (const { id, version } of packs) {
    const key = `packs/${id}`;
    if (packages[key] === undefined) throw new Error(`package-lock.json has no ${key}; run npm install`);
    if (packages[key].version !== version) packages[key] = { ...packages[key], version };
  }
  return { ...lock, packages };
}

/**
 * What a vitest JSON report of test/engine/catalog/packs.test.ts says against publishing `ids`: each pack's tests
 * ran, none failed, and at least `checkCount` checks passed (a filter matching nothing would otherwise pass).
 */
export function gateFailures(report, ids, checkCount) {
  const results = report.testResults.flatMap((file) => file.assertionResults);
  const failures = [];
  for (const id of ids) {
    const own = results.filter((result) => result.ancestorTitles[0] === `pack ${id}`);
    for (const result of own) if (result.status === "failed") failures.push(`${id}: ${result.title} failed`);
    const checks = own.filter((result) => result.status === "passed" && result.title.startsWith("check ")).length;
    if (checks < checkCount) failures.push(`${id}: ${checks} of ${checkCount} pack checks passed`);
  }
  return failures;
}

/** The pack ids a command line names, every one a pack directory; `--all` names every pack. */
export function packIds(args, packs = listPacks()) {
  if (args.includes("--all")) return packs.map((pack) => pack.id);
  const known = new Set(packs.map((pack) => pack.id));
  const unknown = args.filter((id) => !known.has(id));
  if (unknown.length > 0) throw new Error(`not a pack under packs/: ${unknown.join(", ")}`);
  return args;
}
