// Which repository paths change what core (@neralabs/rerail) ships. With no dependencies, so a workflow can ask before
// `npm ci`: the Release pull request asks it of the changes since core's last release (release-version.mjs), the
// environment matrix of the changes a push to main made (scripts/host/azure/matrix.mjs `affected`, `matrixChanges`).

/**
 * Paths whose change never changes what core ships, so they alone never release it: the packs (released on their
 * own), documentation, CI, changesets, tests, fixtures, corpora and benchmarks, development and release scripts, and
 * repository files npm does not pack (a generator's output is committed where it ships, so it counts there). Everything
 * else is a core change, `package-lock.json` included unless only its pack workspace entries moved (`lockChangesCore`).
 * A directory ends in `/`.
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
  "plugins/",
  "playground/",
  "scripts/check/",
  "scripts/dev/",
  "scripts/dsl/",
  "scripts/feedback/",
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
 * The paths among `changed` (repository paths changed since a base: core's last release, or a push's previous head)
 * that change what core ships. `readLock(ref)` reads `package-lock.json` at the base (`"release"`) or now (`"head"`),
 * called only when it changed.
 */
export function coreChanges(changed, readLock) {
  return changed.filter((path) => shipsWithCore(path) && (path !== "package-lock.json" || lockChangesCore(readLock("release"), readLock("head"))));
}

/** What the environment matrix itself runs on: a change to one of these is tested even though core does not ship it. */
export const MATRIX_INPUTS = [
  ".github/workflows/env-matrix.yml",
  ".github/workflows/package.yml",
  "scripts/host/azure/",
  "scripts/host/windows-host.ps1",
  "scripts/release/core-paths.mjs",
];

/**
 * Why a push should run the environment matrix: the paths among `changed` that change what core ships (`coreChanges`)
 * and those that change the matrix itself. Both empty: packs, tests, docs and other CI changes leave the package as the
 * previous run tested it.
 */
export function matrixChanges(changed, readLock) {
  return {
    core: coreChanges(changed, readLock),
    matrix: changed.filter((path) => MATRIX_INPUTS.some((input) => (input.endsWith("/") ? path.startsWith(input) : path === input))),
  };
}
