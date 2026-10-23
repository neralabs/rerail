#!/usr/bin/env node
// The port readiness legs port-readiness.yml runs (docs/port-readiness.md, "CI"): one per gated tool-pack, a pack's
// upstream tool with packs/<pack>/upstream/<tool>/gates.yaml, or one per shard of a tool-pack in SHARDS. Given a diff,
// only the legs whose verdicts the changed paths can move; a tool-pack left out keeps main's verdict. Its plan job runs
// this on plain Node, before any npm ci, so it imports nothing but node: builtins.
//
//   node conformance/port-readiness/plan.mjs                      every leg
//   node conformance/port-readiness/plan.mjs --diff <from> <to>   the legs the paths changed from <from> to <to> select
//
// Prints one line per planned tool-pack and why. When $GITHUB_OUTPUT is set, appends `legs` (the gates job's matrix
// include list) and `tool-packs` ("<pack>/<tool>" strings), each as compact JSON, and `conformance` (true or false):
// whether the change can move a verdict of the conformance gates, which judge every rule (the report job posts the
// comment when it can).
import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, readdirSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * Legs per tool-pack too slow for one; every other gated tool-pack runs in one. Shard i of n judges the rules whose
 * sha256(id) mod n is i - 1, and the report job merges the shards (docs/port-readiness.md, "Sharding"). Cold,
 * javascript/eslint took over an hour, on upstream's tests alone. Since no rule runs over the real-code corpus, Ruff and
 * typescript-eslint each run in one leg: cold on a laptop, 7 min and 2 min. The leg's job name, report artifact and
 * results-cache key all carry the count.
 */
export const SHARDS = Object.freeze({ "javascript/eslint": 4 });

/** What a leg reads: ci.yml's `port-readiness` paths filter, glob for glob. `x/**` is a path prefix, anything else a path. */
export const ROOTS = Object.freeze([
  "packs/**",
  "conformance/**",
  "native/**",
  "languages/**",
  "src/engine/**",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "package.json",
  "package-lock.json",
  ".github/workflows/ci.yml",
  ".github/workflows/port-readiness.yml",
  ".github/actions/**",
  "scripts/s3/**",
]);

/**
 * What no leg reads: the recording diff runs only in the recorder, and the conformance gates of the rules no tool-pack
 * judges in their own job on every run. A change to one plans no leg. The shard merge,
 * the summary and the comment move no leg's verdict either, but a change to them should run on real reports, so it
 * plans every leg; each replays main's results, since they are out of the key.
 */
export const REPORT_ONLY = Object.freeze([
  "conformance/port-readiness/README.md",
  "conformance/port-readiness/recording/diff.ts",
  "conformance/port-readiness/conformance-gates.ts",
]);

const nameOf = ({ pack, tool }) => `${pack}/${tool}`;
const matches = (path, glob) => (glob.endsWith("/**") ? path.startsWith(glob.slice(0, -2)) : path === glob);

/** Every gated tool-pack, packs and tools sorted, as run.ts gatedToolPacks() finds them. */
export function gatedToolPacks(root = ROOT) {
  const found = [];
  const packs = join(root, "packs");
  for (const pack of existsSync(packs) ? readdirSync(packs).sort() : []) {
    const upstream = join(packs, pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream).sort()) {
      if (existsSync(join(upstream, tool, "gates.yaml"))) found.push({ pack, tool });
    }
  }
  return found;
}

/** The matrix legs of these tool-packs: `{pack, tool}`, or `{pack, tool, shard, shards}` for each shard of one in SHARDS. */
export function legsOf(toolPacks) {
  return toolPacks.flatMap(({ pack, tool }) => {
    const shards = Object.hasOwn(SHARDS, nameOf({ pack, tool })) ? SHARDS[nameOf({ pack, tool })] : undefined;
    return shards === undefined ? [{ pack, tool }] : Array.from({ length: shards }, (_, index) => ({ pack, tool, shard: index + 1, shards }));
  });
}

/**
 * The tool-packs among `toolPacks` whose verdicts a change to `path` can move. A path under packs/<pack>/ moves only
 * that pack's; anything else a leg reads (the engine, the gate code, the package files, the workflows, this file, or a
 * path nobody classified) moves every one. An engine change judges every leg even when it touches one language family:
 * the results key (engine.ts) keeps the other families' rules warm, so their legs replay main's results.
 */
export function classify(path, toolPacks) {
  if (!ROOTS.some((glob) => matches(path, glob)) || REPORT_ONLY.includes(path)) return [];
  const [top, pack, ...rest] = path.split("/");
  if (top === "packs") return rest.length === 0 ? [] : toolPacks.filter((entry) => entry.pack === pack);
  return [...toolPacks];
}

/**
 * Whether a change to `path` can move a verdict of the conformance gates: anything a leg reads, and conformance-gates.ts
 * itself, except a file at the top of packs/. The other report-only files move none, so a change to one alone posts no
 * comment unless a check fails, as before. The conformance job judges every rule no tool-pack does on every run, so
 * this decides only whether the change is worth a comment.
 */
export function movesConformance(path) {
  if (!ROOTS.some((glob) => matches(path, glob))) return false;
  if (REPORT_ONLY.includes(path)) return path === "conformance/port-readiness/conformance-gates.ts";
  const [top, , ...rest] = path.split("/");
  return top !== "packs" || rest.length > 0;
}

/**
 * The legs to run, the tool-packs they judge and one line per tool-pack saying why: every gated tool-pack when `paths`
 * is undefined (no diff), otherwise the ones the changed paths select. `conformance` is whether the change can move a
 * conformance verdict (movesConformance): always, with no diff.
 */
export function plan(toolPacks, paths) {
  const why = new Map();
  if (paths === undefined) {
    for (const toolPack of toolPacks) why.set(nameOf(toolPack), "every tool-pack is judged");
  } else {
    const changed = new Map();
    for (const path of paths) {
      for (const toolPack of classify(path, toolPacks)) changed.set(nameOf(toolPack), [...(changed.get(nameOf(toolPack)) ?? []), path]);
    }
    for (const [name, [first, ...more]] of changed) why.set(name, `${first} changed${more.length === 0 ? "" : ` (and ${more.length} more)`}`);
  }
  const selected = toolPacks.filter((toolPack) => why.has(nameOf(toolPack)));
  return {
    legs: legsOf(selected),
    toolPacks: selected.map(nameOf),
    conformance: paths === undefined || paths.some(movesConformance),
    lines: selected.map((toolPack) => {
      const legs = legsOf([toolPack]).length;
      return `${nameOf(toolPack)} (${legs} leg${legs === 1 ? "" : "s"}): ${why.get(nameOf(toolPack))}`;
    }),
  };
}

/** The paths changed from `from` to `to`. --no-renames lists a moved file at both paths, so a move selects both packs. */
export function changedPaths(from, to, root = ROOT) {
  const output = execFileSync("git", ["diff", "--no-renames", "--name-only", "-z", from, to], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 256 * 1024 * 1024,
  });
  return output.split("\0").filter((path) => path !== "");
}

/** Appends the plan to a GitHub Actions output file: exactly `legs=<json>`, `tool-packs=<json>` and `conformance=<bool>`. */
export function writeOutputs(file, planned) {
  appendFileSync(file, `legs=${JSON.stringify(planned.legs)}\ntool-packs=${JSON.stringify(planned.toolPacks)}\nconformance=${planned.conformance}\n`);
}

function main(args) {
  let paths;
  if (args.length === 3 && args[0] === "--diff") {
    try {
      paths = changedPaths(args[1], args[2]);
    } catch (error) {
      // Failing open would skip legs a change needs; every leg is the safe plan.
      const stderr = typeof error?.stderr === "string" ? error.stderr.trim() : "";
      const reason = (stderr !== "" ? stderr : error instanceof Error ? error.message : String(error)).split("\n")[0];
      console.log(`::warning title=Port readiness plan::git diff ${args[1]} ${args[2]} failed, so every leg runs: ${reason}`);
    }
  } else if (args.length !== 0) {
    process.stderr.write("usage: node conformance/port-readiness/plan.mjs [--diff <from> <to>]\n");
    return 2;
  }
  const planned = plan(gatedToolPacks(), paths);
  for (const line of planned.lines) console.log(line);
  if (planned.legs.length === 0) console.log("No gated tool-pack reads a changed path: no leg runs.");
  console.log(planned.conformance ? "The conformance gates read a changed path." : "The conformance gates read no changed path.");
  if (process.env.GITHUB_OUTPUT !== undefined && process.env.GITHUB_OUTPUT !== "") writeOutputs(process.env.GITHUB_OUTPUT, planned);
  return 0;
}

// Imported by its tests, run as a script by CI: only the second plans.
const invoked = (() => {
  try {
    return process.argv[1] !== undefined && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invoked) process.exitCode = main(process.argv.slice(2));
