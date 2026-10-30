// Reading npm dist-tags, and keeping `next` from trailing `latest`, for the release scripts in this directory. Core and
// the packs are published with `--access public`, so an anonymous reader gets their dist-tags: the publish, promote
// and put-back jobs, which hold no read token, read them that way (readPublicDistTags, `npm dist-tag ls`). The packs'
// plan and read-back jobs still read with NODE_AUTH_TOKEN set to the workflows' NPM_PACKS_READ_TOKEN in ~/.npmrc.
// Nothing here needs node_modules: the publish jobs install none.
import { execFile, execFileSync } from "node:child_process";
import { promisify } from "node:util";

const REGISTRY = "https://registry.npmjs.org";
/** The preview channel's dist-tag. It never trails `latest`: a version published under `latest` moves it up too. */
export const NEXT_TAG = "next";

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/u;

/** `version`'s release numbers and prerelease identifiers; throws on one that is not a semantic version. */
function parseVersion(version) {
  const match = SEMVER.exec(version);
  if (match === null) throw new Error(`${version} is not a semantic version`);
  return { release: match.slice(1, 4).map(Number), prerelease: match[4] === undefined ? [] : match[4].split(".") };
}

/**
 * Semantic-version precedence: negative when `a` is older than `b`, positive when newer, 0 when equal (build metadata
 * ignored). A prerelease is older than its release (`0.1.0-next.4` < `0.1.0`); its identifiers compare one by one,
 * numbers numerically and below words. Throws on a version that is not semantic.
 */
export function compareVersions(a, b) {
  const [left, right] = [parseVersion(a), parseVersion(b)];
  for (let index = 0; index < 3; index += 1) {
    if (left.release[index] !== right.release[index]) return left.release[index] - right.release[index];
  }
  if (left.prerelease.length === 0 || right.prerelease.length === 0) return right.prerelease.length - left.prerelease.length;
  for (let index = 0; index < Math.max(left.prerelease.length, right.prerelease.length); index += 1) {
    const [x, y] = [left.prerelease[index], right.prerelease[index]];
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    if (x === y) continue;
    const [xNumber, yNumber] = [/^\d+$/u.test(x), /^\d+$/u.test(y)];
    if (xNumber && yNumber) return Number(x) - Number(y);
    if (xNumber !== yNumber) return xNumber ? -1 : 1;
    return x < y ? -1 : 1;
  }
  return 0;
}

/** Whether `next` (null or undefined when the package has none) is older than `version`, so must be moved up to it. */
export const nextTrails = (next, version) => next === null || next === undefined || compareVersions(next, version) < 0;

/**
 * Why a `beta` or `next` preview `version` must not be published, given the package's `latest` (undefined when it has
 * none), or null when it may be: one older than `latest` would leave its channel behind the stable one.
 */
export const previewRefusal = (version, channel, latest) =>
  latest !== undefined && compareVersions(version, latest) < 0 ? `${version} is older than latest ${latest}: a ${channel} preview must not trail latest` : null;

/**
 * `name`'s dist-tags (`{ tag: version }`), read anonymously from the registry's dist-tags endpoint (the one
 * `npm dist-tag ls` reads, which sees a publish straight away, where `npm view` can lag it by minutes and a first
 * publish answers 404). Throws when the registry does not answer with the package's dist-tags.
 */
export async function readPublicDistTags(name) {
  const response = await fetch(`${REGISTRY}/-/package/${name.replace("/", "%2f")}/dist-tags`, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`${name}'s dist-tags answer an anonymous request with ${response.status}`);
  return await response.json();
}

/** `name`'s `next` dist-tag, null when it has none (readPublicDistTags). */
export const readNextTag = async (name) => (await readPublicDistTags(name))[NEXT_TAG] ?? null;

/** Points `name`'s dist-tag `tag` at `version`, with npm's output on the terminal; throws when npm fails. */
export const addDistTag = (name, version, tag) => execFileSync("npm", ["dist-tag", "add", `${name}@${version}`, tag, "--registry", REGISTRY], { stdio: "inherit" });

/**
 * Points `name`'s `next` at `version`, just published under `latest`, unless `next` is already there or at a newer
 * version (a preview past it), so the preview channel never trails the stable one. `steps.readNext(name)` resolves to
 * `next` or null, and `steps.distTag(name, version, tag)` moves it; both throw on failure. Returns what it did, for
 * the log. Running it again is safe: a `next` already moved is left where it is.
 */
export async function raiseNext(name, version, steps) {
  const next = await steps.readNext(name);
  if (!nextTrails(next, version)) return `${name}@${version}: ${NEXT_TAG} is ${next}, not older; left there`;
  steps.distTag(name, version, NEXT_TAG);
  return `pointed ${NEXT_TAG} at ${name}@${version} (it was ${next ?? "not set"})`;
}

/** `npm dist-tag ls` output (`<tag>: <version>` lines) as `{ tag: version }`. */
export const parseDistTagList = (output) =>
  Object.fromEntries(output.split("\n").map((line) => /^(\S+): (\S+)$/u.exec(line.trim())).filter((match) => match !== null).map((match) => [match[1], match[2]]));

/**
 * Each name's dist-tags, read in parallel; null for one npm could not give, with why on stderr. `fresh` reads them
 * with `npm dist-tag ls`, which sees a publish straight away, where `npm view` can lag it by minutes.
 */
export async function readDistTags(names, { fresh = false } = {}) {
  const read = async (name) => {
    try {
      const args = fresh ? ["dist-tag", "ls", name] : ["view", name, "dist-tags", "--json"];
      const { stdout } = await promisify(execFile)("npm", [...args, "--registry", REGISTRY]);
      return [name, fresh ? parseDistTagList(stdout) : JSON.parse(stdout)];
    } catch (error) {
      console.error(`${name}: ${error instanceof Error ? error.message : String(error)}`);
      return [name, null];
    }
  };
  return new Map(await Promise.all(names.map(read)));
}

/** `<name>@<version>` or `<name>@<version>:<dist-tag>` (default `latest`), as a release run names what it published. */
export function parseExpectation(argument) {
  const [spec, tag = "latest"] = argument.split(":");
  const at = spec.lastIndexOf("@");
  if (at <= 0 || at === spec.length - 1 || tag === "") throw new Error(`${argument} is not <name>@<version>[:<dist-tag>]`);
  return { name: spec.slice(0, at), version: spec.slice(at + 1), tag };
}

/**
 * What each `{ name, version, tag }` a release published has on npm, given `distTagsOf(name)` (null when unread):
 * a markdown table of every dist-tag, and a failure for each package whose `tag` does not point at `version`, or,
 * for a release under `latest` or `next` (the two that set `next`), whose `next` is unset or older than its `latest`
 * (the preview channel must never trail the stable one). A release under another tag (`beta`) never moves `next`, so
 * is not failed for it.
 */
export function distTagReport(expected, distTagsOf) {
  const failures = [];
  const rows = expected.map(({ name, version, tag }) => {
    const distTags = distTagsOf(name);
    if (distTags === null) {
      failures.push(`${name}: its dist-tags could not be read from npm`);
      return `| \`${name}\` | ${version} | \`${tag}\` | could not be read |`;
    }
    if (distTags[tag] !== version) failures.push(`${name}: ${tag} is ${distTags[tag] ?? "not set"}, not the ${version} just published`);
    const { latest, [NEXT_TAG]: next } = distTags;
    if ((tag === "latest" || tag === NEXT_TAG) && latest !== undefined && nextTrails(next, latest)) {
      const fix = tag === "latest" ? `a release under latest must move ${NEXT_TAG} up to it` : `a ${NEXT_TAG} preview must not be older than latest`;
      failures.push(`${name}: ${NEXT_TAG} is ${next === undefined ? "not set" : `${next}, older than latest ${latest}`}; ${fix}`);
    }
    const listed = Object.entries(distTags).map(([distTag, at]) => `\`${distTag}\`: ${at}`).join(", ");
    return `| \`${name}\` | ${version} | \`${tag}\` ${distTags[tag] === version ? "✅" : "❌"} | ${listed} |`;
  });
  return { table: ["| Package | Published | Under | npm dist-tags now |", "|---|---|---|---|", ...rows].join("\n"), failures };
}
