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
