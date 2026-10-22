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
/** Every pack version is published under `latest`, where set-up and `rerail packs add` fetch it. */
export const PUBLISH_TAG = "latest";
/** Where a pack held at an older `latest` (put back on purpose) is published instead, so the hold stays. */
export const HELD_PUBLISH_TAG = "next";

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
 * publishing) is not published again, only checked and tagged. A pack named in `held` (held at an older `latest`) is
 * published under `next`, every other under `latest`. `steps` does the I/O: `publish(file, distTag)` returns
 * `{ ok, output }`, `assertRestricted(name)` and `tag(tag)` throw on failure. Returns each `{ name, version, distTag }`
 * this run released.
 */
export async function releasePacks(manifest, tags, steps, held = []) {
  const existing = new Set(tags);
  const holding = new Set(held);
  const released = [];
  for (const { name, version, file } of manifest) {
    const tag = releaseTag(name, version);
    if (existing.has(tag)) {
      steps.log(`${tag} is already released; skipped`);
      continue;
    }
    const distTag = holding.has(name) ? HELD_PUBLISH_TAG : PUBLISH_TAG;
    const published = steps.publish(file, distTag);
    if (!published.ok && !alreadyOnRegistry(published.output)) throw new Error(`npm publish ${tag} failed`);
    if (!published.ok) steps.log(`${tag} is already on the registry without its tag; checking and tagging it`);
    await steps.assertRestricted(name);
    steps.tag(tag);
    steps.log(`released ${tag} under ${distTag}${holding.has(name) ? ` (held at an older ${PUBLISH_TAG})` : ""}`);
    released.push({ name, version, distTag });
  }
  return released;
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

/** The dist-tag that holds a pack at a version put back on purpose, so a sweep and a publish leave it there. */
export const HOLD_TAG = "held";

/**
 * Which of `names` are held, given `distTagsOf(name)` (null when npm could not be read): a pack whose `held` dist-tag
 * is its `latest`, put back on purpose. A publish leaves such a pack's `latest` alone, as a promotion sweep does.
 */
export function heldPacks(names, distTagsOf) {
  const held = [];
  const unreadable = [];
  for (const name of names) {
    const distTags = distTagsOf(name);
    if (distTags === null) unreadable.push(name);
    else if (distTags[HOLD_TAG] !== undefined && distTags[HOLD_TAG] === distTags.latest) held.push(name);
  }
  return { held, unreadable };
}

/**
 * What one promotion run was asked for. With no `id`, every pack at its package.json version: the sweep. With one,
 * that pack alone, at `version` or else its package.json one. package.json holds the newest released version, so
 * naming another puts an older one back on purpose: a `hold`. Throws on an unknown id, or a version without an id.
 */
export function promotionRequest(packs, id, version) {
  if (id === undefined) {
    if (version !== undefined) throw new Error(`version ${version} needs the pack it is for`);
    return { packs, named: false };
  }
  const pack = packs.find((candidate) => candidate.id === id);
  if (pack === undefined) throw new Error(`${id} is not a pack under packs/`);
  const wanted = version ?? pack.version;
  return { packs: [{ ...pack, version: wanted, hold: wanted !== pack.version }], named: true };
}

/**
 * Which packs a promotion moves to `latest`, given the release tags and `distTagsOf(name)`: the pack's `latest` and
 * `held` dist-tags (absent when npm has none), or null when npm could not be read. A pack is promoted when its
 * version is released (its tag exists) and is not `latest` already; one without its tag is `unreleased`. A sweep
 * leaves out a pack held at its `latest` (put back on purpose); naming that pack promotes it. A promotion that is
 * not itself a hold releases any hold the pack has (`release`).
 */
export function planPromotions(packs, tags, distTagsOf, { named = false } = {}) {
  const existing = new Set(tags);
  const plan = { promote: [], unreleased: [], held: [], unreadable: [] };
  for (const pack of packs) {
    if (!existing.has(releaseTag(pack.name, pack.version))) {
      plan.unreleased.push(pack);
      continue;
    }
    const distTags = distTagsOf(pack.name);
    if (distTags === null) {
      plan.unreadable.push(pack);
      continue;
    }
    const latest = distTags.latest ?? null;
    const held = distTags[HOLD_TAG] ?? null;
    const hold = pack.hold === true;
    if (!named && held !== null && held === latest) {
      plan.held.push({ ...pack, latest });
      continue;
    }
    if (latest === pack.version && held === (hold ? pack.version : null)) continue;
    plan.promote.push({ ...pack, latest, hold, release: !hold && held !== null });
  }
  return plan;
}

/**
 * Points `latest` at each `{ name, version }` in `promotions`, refusing a version this repository never released (no
 * tag); a `hold` also sets the `held` dist-tag there, and a `release` removes it. One failure does not stop the
 * others; returns each failure, so the run reports them all and can be run again. `steps.promote(name, version)`,
 * `steps.hold(name, version)` and `steps.release(name)` throw on failure.
 */
export function promotePacks(promotions, tags, steps) {
  const existing = new Set(tags);
  const failures = [];
  for (const { name, version, hold = false, release = false } of promotions) {
    const tag = releaseTag(name, version);
    try {
      if (!existing.has(tag)) throw new Error("it was never released from this repository (no such git tag)");
      steps.promote(name, version);
      if (hold) steps.hold(name, version);
      if (release) steps.release(name);
      steps.log(`promoted ${tag} to latest${hold ? `, held there (${HOLD_TAG})` : ""}${release ? `, ${HOLD_TAG} released` : ""}`);
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
