#!/usr/bin/env node
// `node scripts/check-scrub.mjs <base-ref>`: what the pack validation contract excuses may only shrink against a pull
// request's base (docs/pack-format.md, "The validation contract"). It reads the working tree's packs/ and the base
// commit's git objects, and refuses:
// - a packs/<id>/scrub.json entry the base's list does not have, unless its sub-check is new (no `id: "<sub-check>"`
//   in the base's conformance/checks.ts) and the rule's document is byte-identical to the base's at the same path;
// - a packs/<id>/divergences.json entry the base does not have, on (rule, source, path, line, kind). A
//   divergences.json with a `format` is another harness's (TSLint's audit) and is not read;
// - a rule pack.json did not list at the base, which checks 4 and 7 to 9 cannot judge: any rule of a pack whose
//   conformance.json is another harness's, or a port whose tool declares no source facts (`tools[].recommended`). A
//   rule without a port is proven by its examples, and checks 4 and 7 to 9 do not apply to it;
// - an existing rule losing its port entry, an existing rule those checks judged at the base and no longer judge, an
//   existing rule becoming a port they cannot judge, and a tool dropping its `recommended`.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, realpathSync } from "node:fs";
import { join, posix } from "node:path";
import { fileURLToPath } from "node:url";

const PACKS = "packs";
const CHECKS = "conformance/checks.ts";
const SCRUB = "scrub.json";
const DIVERGENCES = "divergences.json";
const PACK_FILES = ["pack.json", "conformance.json", SCRUB, DIVERGENCES];
const USAGE = "usage: node scripts/check-scrub.mjs <base-ref>";

export class ScrubGuardError extends Error {}

function git(root, args) {
  return execFileSync("git", ["-C", root, ...args], { maxBuffer: 1024 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
}

/** The base commit's blobs under packs/ and conformance/checks.ts, read on demand. */
function baseTree(root, ref) {
  let commit;
  try {
    commit = git(root, ["rev-parse", "--verify", "--quiet", `${ref}^{commit}`]).toString("utf8").trim();
  } catch {
    throw new ScrubGuardError(`${ref} is not a commit in ${root}; fetch the base first`);
  }
  const blobs = new Map();
  for (const entry of git(root, ["ls-tree", "-r", "-z", commit, "--", PACKS, CHECKS]).toString("utf8").split("\0")) {
    const tab = entry.indexOf("\t");
    const [, type, object] = entry.slice(0, tab).split(" ");
    if (tab !== -1 && type === "blob") blobs.set(entry.slice(tab + 1), object);
  }
  const contents = new Map();
  const read = path => {
    if (!blobs.has(path)) return undefined;
    if (!contents.has(path)) contents.set(path, git(root, ["cat-file", "blob", blobs.get(path)]));
    return contents.get(path);
  };
  return { name: commit.slice(0, 12), read, paths: [...blobs.keys()] };
}

function workingTree(root) {
  const read = path => {
    try {
      return readFileSync(join(root, path));
    } catch (error) {
      if (error.code === "ENOENT" || error.code === "ENOTDIR") return undefined;
      throw error;
    }
  };
  let directories = [];
  try {
    directories = readdirSync(join(root, PACKS), { withFileTypes: true }).filter(entry => entry.isDirectory());
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const paths = directories.flatMap(entry => PACK_FILES.map(file => `${PACKS}/${entry.name}/${file}`)).filter(path => read(path) !== undefined);
  return { name: "the working tree", read, paths };
}

/** Pack ids with a pack file at `paths`. */
function packIds(paths) {
  const ids = new Set();
  for (const path of paths) {
    const [top, id, file, extra] = path.split("/");
    if (top === PACKS && extra === undefined && PACK_FILES.includes(file)) ids.add(id);
  }
  return ids;
}

const list = value => (Array.isArray(value) ? value.filter(item => typeof item === "object" && item !== null) : []);
const isObject = value => typeof value === "object" && value !== null && !Array.isArray(value);
const anotherHarness = value => isObject(value) && typeof value.format === "string";
const declaresFacts = tool => Array.isArray(tool?.recommended) && tool.recommended.length > 0;

/** pack.json's rules: id to document. */
function listedRules(pack) {
  return new Map(list(pack?.rules).filter(rule => typeof rule.id === "string" && typeof rule.document === "string").map(rule => [rule.id, rule.document]));
}

/** The port entry conformance.json has for `document`, if any; none in another harness's conformance.json. */
function portOf(conformance, document) {
  if (anotherHarness(conformance)) return undefined;
  return list(conformance?.rules).find(port => typeof port.policy === "string" && port.policy.replace(/^\.\//u, "") === document);
}

/**
 * Why checks 4 and 7 to 9 cannot judge the rule with `document` under `conformance` (conformance.json), or undefined
 * when nothing escapes them: a port of a tool with source facts, or a rule without a port, which its examples prove
 * and to which they do not apply.
 */
function unjudged(conformance, document) {
  if (anotherHarness(conformance)) return `conformance.json is ${conformance.format}, another harness's, so checks 4 and 7 to 9 do not judge its rules`;
  const port = portOf(conformance, document);
  if (port === undefined) return undefined;
  const tool = list(conformance.tools).find(tool => tool.name === port.upstream?.tool);
  return declaresFacts(tool) ? undefined : `no source facts recorded for ${port.upstream?.tool} (conformance.json tools[].recommended)`;
}

const scrubKey = entry => `${entry.rule}\u0000${entry.check}`;
const divergenceKey = entry => JSON.stringify([entry.rule, entry.source ?? null, entry.path, entry.line, entry.kind]);

/** Every way the working tree's packs/ excuses more than `base` does; empty when it excuses nothing new. */
export function checkScrub({ root, base }) {
  const before = baseTree(root, base);
  const after = workingTree(root);
  const problems = [];
  const json = (tree, path) => {
    const bytes = tree.read(path);
    if (bytes === undefined) return undefined;
    try {
      return JSON.parse(bytes.toString("utf8"));
    } catch (error) {
      problems.push(`${path}: does not parse in ${tree.name}: ${error.message}`);
      return undefined;
    }
  };
  const checksText = before.read(CHECKS)?.toString("utf8") ?? "";
  const baseSubChecks = new Set([...checksText.matchAll(/\bid:\s*["'`]([^"'`]+)["'`]/gu)].map(match => match[1]));

  const ids = [...new Set([...packIds(before.paths), ...packIds(after.paths)])].sort();
  for (const id of ids) {
    const path = file => `${PACKS}/${id}/${file}`;
    const basePack = json(before, path("pack.json"));
    const headPack = json(after, path("pack.json"));
    const baseRules = listedRules(basePack);
    const headRules = listedRules(headPack);
    const baseConformance = json(before, path("conformance.json"));
    const headConformance = json(after, path("conformance.json"));

    // scrub.json: shrink only; an addition needs a sub-check the base did not have, and the rule's document as it was.
    const baseScrub = json(before, path(SCRUB));
    const headScrub = json(after, path(SCRUB));
    if (headScrub !== undefined && !Array.isArray(headScrub)) problems.push(`${path(SCRUB)}: not a list of {rule, check} entries`);
    const known = new Set(list(baseScrub).map(scrubKey));
    for (const entry of list(headScrub)) {
      if (known.has(scrubKey(entry))) continue;
      const refusal = (() => {
        if (baseSubChecks.has(entry.check)) return `scrub.json may only shrink, and ${entry.check} was a sub-check at ${before.name}`;
        const document = headRules.get(entry.rule);
        if (document === undefined) return "pack.json does not list the rule";
        const baseDocument = baseRules.get(entry.rule);
        if (baseDocument === undefined) return `${before.name} does not list the rule: a new rule must pass every check`;
        if (baseDocument !== document) return `its document moved from ${baseDocument} to ${document}; an entry needs the document unchanged at the same path`;
        const baseBytes = before.read(posix.normalize(path(document)));
        const headBytes = after.read(posix.normalize(path(document)));
        if (headBytes === undefined) return `${document} does not exist`;
        if (baseBytes === undefined || !baseBytes.equals(headBytes)) return `${document} differs from ${before.name}'s; an entry needs the document byte-identical to the base's`;
        return undefined;
      })();
      if (refusal !== undefined) problems.push(`${path(SCRUB)}: new entry (${entry.rule}, ${entry.check}): ${refusal}`);
    }

    // divergences.json: shrink only, on what an entry excuses.
    const headDivergences = json(after, path(DIVERGENCES));
    if (headDivergences !== undefined && !anotherHarness(headDivergences)) {
      if (!Array.isArray(headDivergences)) problems.push(`${path(DIVERGENCES)}: neither a list of divergences nor another harness's file with a format`);
      const baseDivergences = json(before, path(DIVERGENCES));
      const excused = new Set(anotherHarness(baseDivergences) ? [] : list(baseDivergences).map(divergenceKey));
      for (const entry of list(headDivergences))
        if (!excused.has(divergenceKey(entry)))
          problems.push(
            `${path(DIVERGENCES)}: new divergence (${entry.rule}, ${entry.kind}, ${entry.source ?? "every source"} ${entry.path}:${entry.line}); divergences.json may only shrink: fix the port, or remove the rule`,
          );
    }

    // New rules face every check; an existing port keeps its entry, and an existing rule keeps being judged.
    for (const [rule, document] of headRules) {
      const reason = unjudged(headConformance, document);
      const baseDocument = baseRules.get(rule);
      if (baseDocument === undefined) {
        if (reason !== undefined) problems.push(`${path("pack.json")}: new rule ${rule}: ${reason}; a new rule must face every check`);
        continue;
      }
      const basePort = portOf(baseConformance, baseDocument);
      if (basePort !== undefined && !anotherHarness(headConformance) && portOf(headConformance, document) === undefined)
        problems.push(`${path("pack.json")}: ${rule} has a port entry in conformance.json at ${before.name} and none now; an existing port keeps facing checks 4 and 7 to 9`);
      else if (reason !== undefined && unjudged(baseConformance, baseDocument) === undefined)
        problems.push(
          basePort === undefined
            ? `${path("pack.json")}: ${rule} becomes a port: ${reason}; a new port must face every check`
            : `${path("pack.json")}: ${rule} was judged by checks 4 and 7 to 9 at ${before.name}, and no longer is: ${reason}`,
        );
    }
    if (!anotherHarness(baseConformance) && !anotherHarness(headConformance)) {
      const baseTools = new Map(list(baseConformance?.tools).map(tool => [tool.name, tool]));
      for (const tool of list(headConformance?.tools))
        if (declaresFacts(baseTools.get(tool.name)) && !declaresFacts(tool))
          problems.push(`${path("conformance.json")}: ${tool.name} no longer declares source facts (tools[].recommended), which ${before.name} records`);
    }
  }
  return { base: before.name, packs: ids.length, problems };
}

export function main(argv) {
  if (argv.length !== 1 || argv[0].startsWith("-")) {
    process.stderr.write(`${USAGE}\n`);
    return 2;
  }
  const root = git(process.cwd(), ["rev-parse", "--show-toplevel"]).toString("utf8").trim();
  const { base, packs, problems } = checkScrub({ root, base: argv[0] });
  if (problems.length > 0) {
    process.stderr.write(`check-scrub: ${problems.length} problem(s) against ${base}:\n${problems.map(problem => `  ${problem}\n`).join("")}`);
    return 1;
  }
  process.stdout.write(`check-scrub: ${packs} pack(s) excuse nothing new against ${base}\n`);
  return 0;
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof ScrubGuardError)) throw error;
    process.stderr.write(`check-scrub: ${error.message}\n`);
    process.exitCode = 2;
  }
}
