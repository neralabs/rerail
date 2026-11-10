// Shared by the pack release scripts in this directory (docs/pack-format.md, "Versioning and release"). Each
// released pack version is marked by the git tag `<npm name>@<version>`, changesets' own tag form, and is released
// once it is tagged and on npm. The tag comes first and npm after, so a release never waits on npm; a version tagged
// but not on npm is planned again by every later run (planPacks), so any re-run or the next push publishes it, which
// npm's refusal to publish over a version it has makes safe.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import { compareVersions, NEXT_TAG, raiseNext } from "./dist-tags.mjs";
import { changeType, changeTypeRefusal, releaseNoteEntry } from "./release-notes.mjs";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
export const PACKS = join(ROOT, "packs");
export const REGISTRY = "https://registry.npmjs.org";
/** Every pack version is published under `latest`, where set-up and `rerail packs add` fetch it. */
export const PUBLISH_TAG = "latest";
/** Where a pack held at an older `latest` (put back on purpose) is published instead, so the hold stays. */
export const HELD_PUBLISH_TAG = NEXT_TAG;

/** The git tag that marks `name`'s `version` as released. */
export const releaseTag = (name, version) => `${name}@${version}`;

/** The npm names of the packs whose ids `held` lists (space-separated, as packs-plan.mjs writes them). */
export function heldPackNames(held = "", packs = listPacks()) {
  const ids = held.split(/\s+/u).filter((id) => id !== "");
  return packs.filter((pack) => ids.includes(pack.id)).map((pack) => pack.name);
}

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

/** The dist-tag `name` is published under: `next` for a pack in `held` (held at an older `latest`), else `latest`. */
const publishTag = (name, held) => (held.includes(name) ? HELD_PUBLISH_TAG : PUBLISH_TAG);

/**
 * Tags each `{ name, version }` of a packs-pack.ts manifest `<name>@<version>` with `steps.tag(tag)`, before anything
 * is published, skipping a tag in `tags` (one that exists already), so a re-run tags only what the first run did not.
 * Returns every entry as `{ name, version, distTag }`, the dist-tag it is published under (publishTag), for its
 * GitHub Release, which a re-run rewrites.
 */
export function tagPacks(manifest, tags, steps, held = []) {
  const existing = new Set(tags);
  return manifest.map(({ name, version }) => {
    const tag = releaseTag(name, version);
    if (existing.has(tag)) {
      steps.log(`${tag} is tagged already`);
    } else {
      steps.tag(tag);
      steps.log(`tagged ${tag}`);
    }
    return { name, version, distTag: publishTag(name, held) };
  });
}

/**
 * Publishes each `{ name, version, file }` of a packs-pack.ts manifest, tagged already (tagPacks), waiting on nothing
 * npm does after a publish, so any failed run can be run again: a version the registry already has is not published
 * again, only taken through the steps after the publish. A pack named in `held` (held at an older `latest`) is
 * published under `next`, every other under `latest`, and then `next` is pointed at it too unless `next` is already
 * at a newer version (raiseNext in dist-tags.mjs), so `next` never trails `latest`. A version older than the pack's
 * `latest` (an old run run again after a newer version shipped) is `superseded`: publishing it under `latest` would
 * move `latest` back, so it is left unpublished and the rest go on. `steps` does the I/O: `publish(file, distTag)`
 * returns `{ ok, output }`, `readLatest(name)` and `readNext(name)` resolve to that dist-tag or null, and
 * `distTag(name, version, tag)` throws on failure. Returns `{ released, superseded }`: each `{ name, version, distTag }`
 * this run released, and each `{ name, version, latest }` it left unpublished.
 */
export async function releasePacks(manifest, steps, held = []) {
  const released = [];
  const superseded = [];
  for (const { name, version, file } of manifest) {
    const spec = releaseTag(name, version);
    const distTag = publishTag(name, held);
    if (distTag === PUBLISH_TAG) {
      const latest = await steps.readLatest(name);
      if (latest !== null && compareVersions(version, latest) < 0) {
        steps.log(`${spec} is older than latest ${latest}: not published, since publishing it under ${PUBLISH_TAG} would move ${PUBLISH_TAG} back`);
        superseded.push({ name, version, latest });
        continue;
      }
    }
    const published = steps.publish(file, distTag);
    if (!published.ok && !alreadyOnRegistry(published.output)) throw new Error(`npm publish ${spec} failed`);
    if (!published.ok) steps.log(`${spec} is already on the registry; not published again`);
    if (distTag === PUBLISH_TAG) steps.log(await raiseNext(name, version, steps));
    steps.log(`released ${spec} under ${distTag}${distTag === PUBLISH_TAG ? "" : ` (held at an older ${PUBLISH_TAG})`}`);
    released.push({ name, version, distTag });
  }
  return { released, superseded };
}

/**
 * Whether a pack's parsed package.json says `"private": true`: a pack never published, which the release steps, the
 * generated pack list and the conformance gate leave out (docs/pack-format.md, "package.json"). `private` is a boolean
 * or absent, as `rulePackPackageSchema` (src/engine/catalog/rule-packs.ts) has it; any other value throws, naming
 * `where`, rather than being read as a published pack. The one reading of the flag: the release scripts run on plain
 * Node, so this lives here, and the TypeScript checks import it.
 */
export function isPrivatePack(manifest, where = "package.json") {
  const value = typeof manifest === "object" && manifest !== null ? manifest.private : undefined;
  if (value === undefined || typeof value === "boolean") return value === true;
  throw new TypeError(`${where}: "private" must be true or false, not ${JSON.stringify(value)}`);
}

/**
 * Every pack under `packsDirectory`: `{ id, name, version, private }`, sorted by id. A pack whose package.json says
 * `"private": true` is never published; release steps take `publishedPacks()` instead.
 */
export function listPacks(packsDirectory = PACKS) {
  return readdirSync(packsDirectory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const path = join(packsDirectory, entry.name, "package.json");
      const manifest = JSON.parse(readFileSync(path, "utf8"));
      return { id: entry.name, name: manifest.name, version: manifest.version, private: isPrivatePack(manifest, path) };
    })
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** Every pack under `packsDirectory` that is published: not `"private": true`. */
export function publishedPacks(packsDirectory = PACKS) {
  return listPacks(packsDirectory).filter((pack) => !pack.private);
}

/** Every pending changeset in `directory`: `{ file, text }`, sorted by file name. */
export function readChangesets(directory = join(ROOT, ".changeset")) {
  return readdirSync(directory)
    .filter((file) => file.endsWith(".md") && file !== "README.md")
    .sort()
    .map((file) => ({ file, text: readFileSync(join(directory, file), "utf8") }));
}

/** A changeset's summary: its text after the front matter, trimmed, which becomes the pack's changelog entry. */
export const changesetSummary = (text) => text.replace(/^---\r?\n(?:[\s\S]*?\r?\n)?---/u, "").trim();

/** Each package a changeset's front matter names, with its bump: `{ name, bump }`. */
export function changesetEntries(text) {
  const front = /^---\r?\n([\s\S]*?)\r?\n---/u.exec(text)?.[1] ?? "";
  return front
    .split(/\r?\n/u)
    .map((line) => /^\s*["']?([^"':]+)["']?\s*:\s*(patch|minor|major)\s*$/u.exec(line))
    .filter((entry) => entry !== null)
    .map(([, name, bump]) => ({ name, bump }));
}

/** Why the changeset `file` can't go under a heading of a pack's release notes, or null: its name gives no type. */
export const changesetTypeRefusal = (file) => changeTypeRefusal(file, ".changeset/README.md");

/** The change types whose notes are about one rule, which a summary opens with (changesetRuleRefusal). */
const RULE_LED_TYPES = ["Added", "Removed", "Fixed"];

/** Why a note of `type` about rule `id` doesn't fit what `names`' packs ship (changesetRuleRefusal), or null. */
function ruleShippedRefusal(type, id, names, ruleIds) {
  if (type === "Removed") {
    const still = names.filter((name) => ruleIds(name).has(id));
    return still.length === 0 ? null : `says \`${id}\` was removed, but ${still.join(" and ")} still ships it`;
  }
  const without = names.filter((name) => !ruleIds(name).has(id));
  return without.length === 0 ? null : `opens with \`${id}\`, which ${without.join(" and ")} does not ship`;
}

/**
 * Why the changeset `{ file, text }` is not grounded in a rule, or null. A non-empty summary under Added, Removed or Fixed
 * opens with one rule's id in backticks: `ruleIds(name)` is what pack `name` ships now, which names the rule for Added
 * and Fixed and must not for Removed. An empty summary has no note to ground, and other types need no rule.
 */
export function changesetRuleRefusal({ file, text }, ruleIds) {
  const type = changeType(file);
  const summary = changesetSummary(text);
  if (!RULE_LED_TYPES.includes(type) || summary === "") return null;
  const id = /^`([^`\s]+)`/u.exec(summary)?.[1];
  if (id === undefined) return `opens with no rule id: a note under ${type} starts with the one rule it is about, in backticks; a change to no one rule is \`changed-\``;
  return ruleShippedRefusal(type, id, changesetEntries(text).map(({ name }) => name), ruleIds);
}

/**
 * What the `changesets` (readChangesets) say to the people who upgrade `name`, in order, for its release notes:
 * `{ type, entry }` each, leaving out those that do not name it and those with an empty summary.
 */
export function changesetNotes(changesets, name) {
  return changesets
    .filter(({ text }) => changesetEntries(text).some((entry) => entry.name === name) && changesetSummary(text) !== "")
    .map(({ file, text }) => ({ type: changeType(file), entry: releaseNoteEntry(changesetSummary(text)) }));
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
    for (const { name, bump } of changesetEntries(text)) {
      const version = versions.get(name);
      if (version === undefined || bump === "patch" || !version.startsWith("0.")) continue;
      failures.push(`${file}: ${name} is ${version}, below 1.0.0, so its bump is patch, not ${bump}`);
    }
  }
  return failures;
}

/**
 * Each pending changeset naming a private pack. Changesets never version a private pack, so a changeset naming only
 * private packs stays pending for ever, and one naming a private pack beside a published one makes `changeset version`
 * fail ("mixed changeset").
 */
export function privateChangesetFailures(changesets, packs) {
  const privateNames = new Set(packs.filter((pack) => pack.private).map((pack) => pack.name));
  return changesets.flatMap(({ file, text }) =>
    changesetEntries(text)
      .filter(({ name }) => privateNames.has(name))
      .map(({ name }) => `${file}: ${name} is private and never released, so no changeset names it`),
  );
}

/**
 * Whether a pack's package.json changed what installing the pack gives, from its text `before` (undefined: the file is
 * new) to `after`. `publishConfig` only steers `npm publish`, so a change to it alone needs no release.
 */
export function manifestChanged(before, after) {
  if (before === undefined) return true;
  const { publishConfig: _before, ...was } = JSON.parse(before);
  const { publishConfig: _after, ...is } = JSON.parse(after);
  return !isDeepStrictEqual(was, is);
}

/**
 * Each pack a pull request changes what it publishes of, with no changeset of that pull request naming it. A pack
 * publishes its `package.json` and what its `files` list (a file, or everything under a directory); its expected findings,
 * candidates and `CHANGELOG.md` are not published, and changing them needs no changeset. `changed` lists the pull
 * request's changed paths (old and new paths of a rename both), `changesets` the changesets it adds or edits, and
 * `packs` each pack's `{ id, name, files, private }`. A private pack (judged on its package.json after the change) is
 * never published and needs no changeset.
 */
export function missingChangesets(changed, changesets, packs) {
  const named = new Set(changesets.flatMap(({ text }) => changesetEntries(text).map((entry) => entry.name)));
  const failures = [];
  for (const { id, name, files, private: isPrivate } of packs) {
    if (isPrivate === true) continue;
    const prefix = `packs/${id}/`;
    const published = changed
      .filter((path) => path.startsWith(prefix))
      .map((path) => path.slice(prefix.length))
      .filter((path) => path === "package.json" || files.some((entry) => path === entry || path.startsWith(`${entry}/`)));
    if (published.length === 0 || named.has(name)) continue;
    const shown = published.slice(0, 3).map((path) => `${prefix}${path}`).join(", ");
    failures.push(`${name}: ${shown}${published.length > 3 ? ` and ${published.length - 3} more` : ""} changed, and no changeset names it`);
  }
  return failures;
}

/**
 * Whether npm has `version`, given the package's `distTags` and its published `versions` (each null when npm could not
 * give them), or null when that can't be told. A dist-tag pointing at the version settles it at once: `npm dist-tag
 * ls` sees a publish straight away, where the version list (`npm view`) can lag it by minutes.
 */
export function onRegistry(version, distTags, versions) {
  if (distTags !== null && Object.values(distTags).includes(version)) return true;
  if (distTags === null || versions === null) return null;
  return versions.includes(version);
}

/**
 * Which packs a release run tags and publishes, given the git tags that exist and `npmHas(name, version)` (whether npm
 * has it, null when that could not be read). A pack whose version is tagged and on npm is released. One tagged but not
 * on npm (a run whose publish failed or was never approved) is published again, `unpublished`; tagging and publishing
 * are both safe to repeat, so any later run, a re-run of any of its jobs or the next push, finishes it. One npm could
 * not be read for is `unreadable`, and the plan fails rather than call it released. A pack with no tag at all has
 * never been published, and its first publish is made by hand (trusted publishing can only be configured for a
 * package that exists): it is `first`, not `publish`.
 */
export function planPacks(packs, tags, npmHas) {
  const existing = new Set(tags);
  const plan = { publish: [], first: [], unpublished: [], unreadable: [] };
  for (const pack of packs) {
    if (!tags.some((tag) => tag.startsWith(`${pack.name}@`))) {
      plan.first.push(pack.id);
      continue;
    }
    if (!existing.has(releaseTag(pack.name, pack.version))) {
      plan.publish.push(pack.id);
      continue;
    }
    const has = npmHas(pack.name, pack.version);
    if (has === null) {
      plan.unreadable.push(pack.id);
    } else if (!has) {
      plan.publish.push(pack.id);
      plan.unpublished.push(pack.id);
    }
  }
  return plan;
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
 * tag); a `hold` also sets the `held` dist-tag there, and a `release` removes it. Then `next` is pointed at the version
 * too unless it is already at a newer one (raiseNext in dist-tags.mjs), so `next` never trails `latest`; a put-back to
 * an older version leaves it where it is. One failure does not stop the others. Returns each pack fully promoted, and
 * each failure, so the run reports them all and can be run again. `steps.promote(name, version)`,
 * `steps.hold(name, version)`, `steps.release(name)` and `steps.distTag(name, version, tag)` throw on failure, and
 * `steps.readNext(name)` resolves to `next` or null.
 */
export async function promotePacks(promotions, tags, steps) {
  const existing = new Set(tags);
  const promoted = [];
  const failures = [];
  for (const { name, version, hold = false, release = false } of promotions) {
    const tag = releaseTag(name, version);
    try {
      if (!existing.has(tag)) throw new Error("it was never released from this repository (no such git tag)");
      steps.promote(name, version);
      if (hold) steps.hold(name, version);
      if (release) steps.release(name);
      steps.log(`promoted ${tag} to latest${hold ? `, held there (${HOLD_TAG})` : ""}${release ? `, ${HOLD_TAG} released` : ""}`);
      steps.log(await raiseNext(name, version, steps));
      promoted.push({ name, version });
    } catch (error) {
      failures.push(`${tag}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  return { promoted, failures };
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

/**
 * The pack ids a command line names, every one a published pack directory; `--all` names every published pack. A
 * private pack is never packed for npm.
 */
export function packIds(args, packs = listPacks()) {
  if (args.includes("--all")) return packs.filter((pack) => !pack.private).map((pack) => pack.id);
  const privateIds = args.filter((id) => packs.some((pack) => pack.id === id && pack.private));
  if (privateIds.length > 0) throw new Error(`private, never packed for npm: ${privateIds.join(", ")}`);
  const known = new Set(packs.map((pack) => pack.id));
  const unknown = args.filter((id) => !known.has(id));
  if (unknown.length > 0) throw new Error(`not a pack under packs/: ${unknown.join(", ")}`);
  return args;
}
