// Shared by the core release scripts in this directory (docs/distribution-model.md, "Release channels"). The core
// package @neralabs/rerail is versioned by the Release pull request, not by changesets: when anything core ships has
// changed since its last release, the pull request bumps its patch version and dates the root CHANGELOG.md's
// unreleased section. Each released core version is marked by the git tag `v<version>`, which release.yml publishes.
import semver from "semver";

export const CORE_NAME = "@neralabs/rerail";

/** The git tag that releases core's `version`: release.yml publishes on it. */
export const coreTag = (version) => `v${version}`;

/**
 * Paths whose change never changes what core ships, so they alone never release it: the packs (released on their
 * own), documentation, CI, changesets, tests, fixtures, corpora and benchmarks, development and release scripts, and
 * repository files npm does not pack (a generator's output is committed where it ships, so it counts there). Everything else is a core change, `package-lock.json` included unless only its
 * pack workspace entries moved (`lockChangesCore`). A directory ends in `/`.
 */
export const NOT_SHIPPED = [
  ".changeset/",
  ".github/",
  ".rerail/",
  "benchmark/",
  "conformance/",
  "docs/",
  "fixtures/",
  "integration/",
  "LICENSES/",
  "packs/",
  "playground/",
  "scripts/check/",
  "scripts/dev/",
  "scripts/dsl/",
  "scripts/generate/",
  "scripts/host/",
  "scripts/release/",
  "scripts/review/",
  "scripts/README.md",
  "test/",
  ".gitattributes",
  ".gitignore",
  "CHANGELOG.md",
  "CONTEXT.md",
  "CONTRIBUTING.md",
  "Justfile",
  "REUSE.toml",
  "REVIEW.md",
  "SECURITY.md",
  "vitest.config.ts",
];

/** Whether a change to repository path `path` can change what core ships. */
export const shipsWithCore = (path) => !NOT_SHIPPED.some((entry) => (entry.endsWith("/") ? path.startsWith(entry) : path === entry));

/** `package-lock.json` without its pack workspace entries, which the pack release moves. */
const withoutPacks = (lock) => ({ ...lock, packages: Object.fromEntries(Object.entries(lock.packages ?? {}).filter(([key]) => !key.startsWith("packs/"))) });

/** Whether two `package-lock.json` texts differ anywhere but their pack workspace entries. */
export const lockChangesCore = (before, after) => JSON.stringify(withoutPacks(JSON.parse(before))) !== JSON.stringify(withoutPacks(JSON.parse(after)));

/**
 * The paths among `changed` (repository paths changed since core's last release) that change what core ships.
 * `readLock(ref)` reads `package-lock.json` at the release (`"release"`) or now (`"head"`), called only when it changed.
 */
export function coreChanges(changed, readLock) {
  return changed.filter((path) => shipsWithCore(path) && (path !== "package-lock.json" || lockChangesCore(readLock("release"), readLock("head"))));
}

/** The first `## ` heading of a changelog and where it is, or null. */
function firstSection(text) {
  const match = /^## (.*)$/mu.exec(text);
  return match === null ? null : { heading: match[1].trim(), index: match.index, length: match[0].length };
}

/** The version an unreleased heading (`Unreleased`, `<version> (unreleased)`) names, `""` for none; null if released. */
function unreleasedVersion(heading) {
  if (/^unreleased$/iu.test(heading)) return "";
  const match = /^(\S+) \(unreleased\)$/iu.exec(heading);
  return match === null ? null : match[1];
}

/**
 * The version core's next release takes after `current`: a patch bump (`0.1.0-next.4` -> `0.1.0`, `0.1.0` -> `0.1.1`),
 * unless the changelog's unreleased heading names a later version (`## 0.2.0 (unreleased)`), which is how a release
 * other than a patch is asked for. Throws on a named version that is not valid or not later than `current`.
 */
export function nextCoreVersion(current, changelog) {
  const patch = semver.inc(current, "patch");
  const section = firstSection(changelog);
  const named = section === null ? null : unreleasedVersion(section.heading);
  if (named === null || named === "") return patch;
  if (semver.valid(named) === null) throw new Error(`CHANGELOG.md's unreleased heading names ${named}, which is not a version`);
  if (!semver.gt(named, current)) throw new Error(`CHANGELOG.md's unreleased heading names ${named}, which is not later than core's ${current}`);
  return named;
}

/**
 * The root CHANGELOG.md released as `version` on `date` (YYYY-MM-DD): its unreleased section's heading becomes
 * `## <version> (<date>)`. Without an unreleased section, a dated section saying so is added above the last release.
 * `notes` is whether the release had notes of its own.
 */
export function releaseChangelog(text, version, date) {
  const heading = `## ${version} (${date})`;
  const section = firstSection(text);
  if (section !== null && unreleasedVersion(section.heading) !== null) {
    return { text: `${text.slice(0, section.index)}${heading}${text.slice(section.index + section.length)}`, notes: true };
  }
  const added = `${heading}\n\nNo changelog entries were written for this release.\n`;
  if (section === null) return { text: `${text.replace(/\n*$/u, "")}\n\n${added}`, notes: false };
  return { text: `${text.slice(0, section.index)}${added}\n${text.slice(section.index)}`, notes: false };
}

/** The body of the changelog section headed `## <version>` (to the next `## `), trimmed; `""` when there is none. */
export function changelogSection(text, version) {
  const lines = text.split("\n");
  const start = lines.findIndex((line) => line === `## ${version}` || line.startsWith(`## ${version} (`));
  if (start === -1) return "";
  const end = lines.findIndex((line, index) => index > start && line.startsWith("## "));
  return lines.slice(start + 1, end === -1 ? undefined : end).join("\n").trim();
}

/** `package.json`'s text with its own `version` set to `version`, and nothing else changed. */
export function setPackageVersion(text, version) {
  const updated = text.replace(/^( {2}"version": ")[^"]*(")/mu, `$1${version}$2`);
  if (updated === text && JSON.parse(text).version !== version) throw new Error("package.json has no top-level version to set");
  return updated;
}

/** `package-lock.json` with the root package's recorded version set to `version`, and nothing else changed. */
export function setLockVersion(lock, version) {
  if (lock.packages?.[""] === undefined) throw new Error('package-lock.json has no root package (""); run npm install');
  return { ...lock, version, packages: { ...lock.packages, "": { ...lock.packages[""], version } } };
}

/**
 * Whether core `version` may be tagged for release, given the `v*` tags that exist: it must not be tagged yet and must
 * be later than every released version, so a version moved backwards by mistake is never published. Returns the
 * reason it may not, or null.
 */
export function coreTagRefusal(version, tags) {
  if (semver.valid(version) === null) return `package.json's version ${version} is not a version`;
  const released = tags.filter((tag) => tag.startsWith("v")).map((tag) => tag.slice(1)).filter((tagged) => semver.valid(tagged) !== null);
  if (released.includes(version)) return `${coreTag(version)} already exists`;
  const later = released.filter((tagged) => semver.gte(tagged, version)).sort(semver.rcompare);
  return later.length === 0 ? null : `${version} is not later than the released ${later[0]}`;
}
