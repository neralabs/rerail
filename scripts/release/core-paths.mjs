// Which repository paths change what core (@neralabs/rerail) ships. With no dependencies, so a workflow can ask before
// `npm ci`: the Release pull request asks it of the changes since core's last release (release-version.mjs), the
// environment matrix of the changes a push to main made (scripts/host/azure/matrix.mjs `affected`, `matrixChanges`).

/**
 * Paths whose change never changes what core ships, so they alone never release it: the packs (released on their
 * own), documentation and its site's build, CI, changesets and core's pending changelog entries, tests, fixtures,
 * corpora and benchmarks, development and release scripts, and repository files npm does not pack (a generator's
 * output is committed where it ships, so it counts there). Everything else is a core change, `package-lock.json`
 * included unless only its pack workspace entries moved (`lockChangesCore`).
 * A directory ends in `/`.
 */
export const NOT_SHIPPED = [
  ".changelog/",
  ".changeset/",
  ".cursor-plugin/marketplace.json",
  ".github/",
  ".rerail/",
  "benchmark/",
  "conformance/",
  "docs/",
  "e2e/",
  "fixtures/",
  "integration/",
  "LICENSES/",
  "packs/",
  "plugins/",
  "playground/",
  "scripts/build/docs-site.mjs",
  "scripts/check/",
  "scripts/deps/",
  "scripts/dev/",
  "scripts/dsl/",
  "scripts/feedback/",
  "scripts/generate/",
  "scripts/host/",
  "scripts/pack-data/",
  "scripts/release/",
  "scripts/review/",
  "scripts/s3/",
  "scripts/samples/",
  "scripts/upstream/",
  "scripts/README.md",
  "test/",
  ".gitattributes",
  ".gitignore",
  "CHANGELOG.md",
  "CODE_OF_CONDUCT.md",
  "CONTEXT.md",
  "CONTRIBUTING.md",
  "context7.json",
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

/**
 * Why release.yml's `publish` input can't publish core's `version`, or null. It retries a release that failed before
 * publishing, once whatever broke it is fixed on main, so a failed release never blocks its version: run from main
 * (`ref`), with no put-back asked for in the same run (`latest`), for the version main's package.json names (`packageVersion`), whose tag exists (`tagged`), with no path
 * that ships changed since that tag (`shipped`, as `coreChanges` gives). So core's own code is the tag's; how the
 * package is built and its changelog can differ (PACKAGE_INPUTS), so that a fix to the build can be retried too, and
 * retrySummary shows them to whoever approves the publish. A fix to what ships takes the next version, which the Release
 * pull request already offers.
 */
export function publishRetryRefusal({ version, ref, latest = "", packageVersion, tagged, shipped }) {
  const refusals = [
    [latest !== "", `publish retries ${version} and latest puts core back to ${latest}: run them separately`],
    [ref !== "refs/heads/main", `a retry runs from main, not ${ref}`],
    [!/^\d+\.\d+\.\d+(?:-(?:beta|next)\.\d+)?$/u.test(version), `${version} is not X.Y.Z, X.Y.Z-beta.N or X.Y.Z-next.N`],
    [version !== packageVersion, `main's package.json says ${packageVersion}, so ${version} can't be retried from it`],
    [!tagged, `v${version} has no tag: a release starts from the Release pull request, and a retry only repeats one`],
    [shipped.length > 0, `what core ships changed since v${version} (${shipped.join(", ")}): release it as the next version, through the Release pull request`],
  ];
  return refusals.find(([refused]) => refused)?.[1] ?? null;
}

/**
 * Paths `coreChanges` doesn't count that can still change the tarball: what builds it, what packing runs, how Git checks
 * out the large files it packs, and the changelog it packs. A directory ends in `/`.
 */
export const PACKAGE_INPUTS = [
  ".gitattributes",
  ".github/actions/",
  ".github/workflows/package.yml",
  "CHANGELOG.md",
  "scripts/release/shrinkwrap-pack.mjs",
];

/** Whether a change to repository path `path` can change the tarball though `coreChanges` doesn't count it. */
export const changesPackage = (path) => PACKAGE_INPUTS.some((entry) => (entry.endsWith("/") ? path.startsWith(entry) : path === entry));

/**
 * The run summary of a retry of `version`, for whoever approves its publish: every path in `changed` (all changed since
 * the tag), those that can change the tarball (`changesPackage`) first.
 */
export function retrySummary(version, changed) {
  const list = (paths) => paths.map((path) => `- \`${path}\``);
  const inputs = changed.filter(changesPackage);
  const others = changed.filter((path) => !changesPackage(path));
  return [
    `### Retrying ${version} from main`,
    "",
    `Core's own code is the same as at \`v${version}\`, but the tarball is built from main. ${changed.length} path(s) changed since the tag.`,
    ...(inputs.length === 0 ? ["", "Nothing that builds or is packed into the tarball changed."] : ["", `**Can change the tarball, so check these before approving (${inputs.length}):**`, "", ...list(inputs)]),
    ...(others.length === 0 ? [] : ["", `**Other changes (${others.length}):**`, "", ...list(others)]),
    "",
  ].join("\n");
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
