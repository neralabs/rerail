// Reading npm dist-tags, for the release scripts in this directory. Rerail's packages are restricted, so reading
// needs an npm login, or NODE_AUTH_TOKEN with a read token in ~/.npmrc (the workflows' NPM_PACKS_READ_TOKEN).
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const REGISTRY = "https://registry.npmjs.org";

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
 * a markdown table of every dist-tag, and a failure for each package whose `tag` does not point at `version`.
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
    const listed = Object.entries(distTags).map(([distTag, at]) => `\`${distTag}\`: ${at}`).join(", ");
    return `| \`${name}\` | ${version} | \`${tag}\` ${distTags[tag] === version ? "✅" : "❌"} | ${listed} |`;
  });
  return { table: ["| Package | Published | Under | npm dist-tags now |", "|---|---|---|---|", ...rows].join("\n"), failures };
}
