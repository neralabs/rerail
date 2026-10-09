// What every source-facts recorder shares: the pin checks it refuses a checkout on, the command runner, and the
// writer of `upstream/<tool>/source.json` (`upstreamSourceSchema`, rerail/conformance/languages/pack.ts).
// Run offline by a maintainer, never in CI.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { upstreamSourceSchema, type UnscoredTest, type UpstreamSource } from "../languages/pack.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/** A command's stdout; a non-zero exit throws with its stderr. */
export function run(command: string, args: string[], options: { cwd?: string; input?: string; env?: NodeJS.ProcessEnv } = {}): string {
  return execFileSync(command, args, { ...options, encoding: "utf8", maxBuffer: 1 << 30, stdio: ["pipe", "pipe", "pipe"] });
}

/** A set of strings in `compareDeterministicStrings` order, as source.json writes every set. */
export function sortedSet(values: Iterable<string>): string[] {
  return [...new Set(values)].sort(compareDeterministicStrings);
}

/** A rule's unscored tests as source.json records them: by test file, then case, each once. */
export function unscoredTests(entries: Iterable<UnscoredTest>): UnscoredTest[] {
  const key = (entry: UnscoredTest): string => `${entry.test}\u0000${entry.case ?? ""}`;
  return [...new Map([...entries].map(entry => [key(entry), entry])).entries()].sort(([left], [right]) => compareDeterministicStrings(left, right)).map(([, entry]) => entry);
}

/** The regexes as source.json records them, each once, in the order they were found. */
export function distinctRegexes(regexes: RegExp[]): { source: string; flags: string }[] {
  const seen = new Set<string>();
  return regexes
    .map(regex => ({ source: regex.source, flags: regex.flags }))
    .filter(regex => {
      const key = `${regex.flags}/${regex.source}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

/** Every file under a directory whose path satisfies `predicate`, in name order. */
export function filesUnder(directory: string, predicate: (path: string) => boolean): string[] {
  if (!existsSync(directory)) return [];
  const found: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => compareDeterministicStrings(a.name, b.name))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...filesUnder(path, predicate));
    else if (entry.isFile() && predicate(path)) found.push(path);
  }
  return found;
}

/** `path` relative to `root`, with `/` separators: how source.json names upstream files. */
export function upstreamPath(root: string, path: string): string {
  return relative(root, path).split(sep).join("/");
}

/** A digest of every file under a directory that `keep` accepts, by relative path and content. */
export function treeDigest(directory: string, keep: (path: string) => boolean = () => true): string {
  const sha256 = (data: string | Buffer) => createHash("sha256").update(data).digest("hex");
  return sha256(filesUnder(directory, keep).map(path => `${upstreamPath(directory, path)}\0${sha256(readFileSync(path))}`).join("\n"));
}

export interface UpstreamPin {
  repository: string;
  commit: string;
  /** The release tag the runtime was published from. */
  tag: string;
  /** The source tree the runtime is built from (`crates` for Ruff, `bandit` for Bandit). */
  releasePath: string;
}

/**
 * Refuses a checkout that is not upstream at the pinned commit with an unmodified working tree, or whose release tree
 * differs at the release tag (fetch the tag into the checkout first): the facts are then the code the runtime runs.
 */
export function assertPinnedCheckout(directory: string, pin: UpstreamPin): void {
  const git = (...args: string[]) => run("git", ["-C", directory, ...args]).trim();
  const head = git("rev-parse", "HEAD");
  if (head !== pin.commit) throw new Error(`${directory} is at ${head}; the pack pins ${pin.commit}`);
  if (git("status", "--porcelain") !== "") throw new Error(`${directory} has a modified working tree`);
  const tagged = (() => {
    try {
      return git("rev-parse", "-q", "--verify", `refs/tags/${pin.tag}^{commit}`);
    } catch {
      throw new Error(`${directory} lacks tag ${pin.tag}: git -C ${directory} fetch --depth 1 origin refs/tags/${pin.tag}:refs/tags/${pin.tag}`);
    }
  })();
  const commitTree = git("rev-parse", `${pin.commit}:${pin.releasePath}`);
  const releaseTree = git("rev-parse", `${tagged}:${pin.releasePath}`);
  if (commitTree !== releaseTree) throw new Error(`${pin.releasePath} is tree ${commitTree} at ${pin.commit} but ${releaseTree} at tag ${pin.tag}`);
}

/** The repository and commit of a fixtures recording's `origin` (`<repository>@<commit>`). */
export function parseOrigin(origin: string): { repository: string; commit: string } {
  const at = origin.lastIndexOf("@");
  return { repository: origin.slice(0, at), commit: origin.slice(at + 1) };
}

/** Validates the facts and writes them, so a recorder never leaves a source.json check 7 would refuse. */
export function writeUpstreamSource(path: string, source: UpstreamSource): void {
  const parsed = upstreamSourceSchema.safeParse(source);
  if (!parsed.success) throw new Error(`${path}: ${parsed.error.message}`);
  writeFileSync(path, `${JSON.stringify(parsed.data, null, 1)}\n`);
}
