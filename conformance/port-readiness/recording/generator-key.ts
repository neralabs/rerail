/**
 * The key a tool-pack's generated answers are stored under: `<pack>/<tool>/<upstream version>/<inputs digest>`, where
 * the digest covers everything the generator reads to make them. The answers are not in the tree, so this key is what
 * ties a commit to its answers: the same inputs are the same answers (the generator is deterministic, and the nightly
 * `npm run generate -- --drift` checks that it is), and any change to an input is a new key, which CI generates. The
 * gates compute it for the tree they judge and read the answers the local cache holds under it.
 *
 * The inputs, each hashed from the working tree:
 * - the generator's code: every repository file the static import graph of expected-cli.ts reaches, read off an
 *   esbuild bundle as cache.test.ts reads the gates' own;
 * - the generator's data: every tracked file under conformance/recorders/port-readiness/ that is not TypeScript (the
 *   drivers it runs), except the runtimes, of which only the tool-pack's own (recordedRuntimes) count;
 * - the npm packages the bundle leaves external, by their locked closure in package-lock.json, and package.json's
 *   `type`;
 * - the CI actions that install the tools the runtimes do not pin (GENERATOR_ACTIONS, PHP_ACTION);
 * - the tool-pack's: its gates.yaml, goldens.tsv (the corpus files upstream answers on) and fixtures/ (the committed
 *   case sources the answers name), the pack's conformance manifests (the rules and corpora), its licenses/*\/UPSTREAM.md
 *   (the upstream paths nothing is copied from), the pinned corpus tasks, and the repository files its adapter reads
 *   by path (ADAPTER_REPOSITORY_FILES).
 *
 * The platform is not part of the key: Linux CI is the only uploader.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { isBuiltin } from "node:module";
import { join } from "node:path";
import { build } from "esbuild";

import { lockedClosure } from "../cache.js";
import { conformancePaths } from "../corpus.js";
import { GATES_MANIFEST, readGatesManifest, recordedRuntimes, type GatesManifest } from "../manifest.js";
import { GOLDENS_FILE } from "../goldens.js";
import { RUNTIMES_DIRECTORY } from "../upstream-files.js";

/** The generator's entry point, relative to the repository root: its import graph is the generator's code. */
export const GENERATOR_ENTRY = "conformance/recorders/port-readiness/expected-cli.ts";

/** Where the generator's code and the drivers it runs are, relative to the repository root. */
const GENERATOR_DIRECTORY = "conformance/recorders/port-readiness";

/** The pinned corpus tasks the generator fetches the goldens' files from (corpus.ts's CORPUS_TASKS). */
const CORPUS_TASKS_FILE = "conformance/port-readiness/corpus-tasks.json";

/**
 * Repository files an adapter reads by path rather than by import, relative to the repository root: Ruff's recorder
 * reads the engine's default Python minor version to say which target its answers are read at.
 */
export const ADAPTER_REPOSITORY_FILES: Partial<Record<GatesManifest["adapter"], readonly string[]>> = {
  ruff: ["native/src/code_rule_dsl/scope/options.rs"],
};

/**
 * The CI actions that install what the generator runs beside the committed runtimes, relative to the repository root:
 * uv and Clippy's toolchain for every tool-pack (generator-runtimes), and, for a tool-pack that installs the PHP
 * runtime, the pinned PHP build's Composer (php), whose version decides what a Composer runtime resolves.
 */
const GENERATOR_ACTIONS = ".github/actions/generator-runtimes";
const PHP_ACTION = ".github/actions/php";

/** A pack, tool or version as a key segment: a blob name's path segment on every store and file system. */
const KEY_SEGMENT = /^[0-9A-Za-z._+-]+$/u;

export interface GeneratorKey {
  pack: string;
  tool: string;
  /** gates.yaml's `upstream.version`. */
  version: string;
  /** sha256 over a `<sha256>  <input>\n` line per input, in input order. */
  digest: string;
  /** `<pack>/<tool>/<version>/<digest>`. */
  key: string;
  /** Every input, sorted: repository paths, `npm:<package-lock location>` and `package.json#type`. */
  inputs: string[];
}

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** The npm package a bare import names: `@scope/name` or `name`, without a subpath. */
function packageOf(specifier: string): string {
  const parts = specifier.split("/");
  return specifier.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0]!;
}

/** The generator's code, by repository path, and the npm packages it imports; one bundle per checkout and process. */
const graphs = new Map<string, Promise<{ files: string[]; packages: string[] }>>();

function generatorGraph(root: string): Promise<{ files: string[]; packages: string[] }> {
  let graph = graphs.get(root);
  if (graph === undefined) {
    graph = build({
      entryPoints: [join(root, GENERATOR_ENTRY)],
      absWorkingDir: root,
      bundle: true,
      write: false,
      metafile: true,
      platform: "node",
      format: "esm",
      packages: "external",
      logLevel: "silent",
    }).then(({ metafile, warnings }) => {
      // A require or import the bundle cannot follow is a warning, and a module loaded that way would escape the key.
      if (warnings.length > 0) throw new Error(`the generator's import graph cannot be read whole: ${warnings.map((warning) => warning.text).join("; ")}`);
      const inputs = Object.entries(metafile.inputs).map(([path, input]) => ({ path: path.replaceAll("\\", "/"), imports: input.imports }));
      const outside = inputs.find(({ path }) => path.startsWith("../") || path.startsWith("/") || path.includes(":"));
      if (outside !== undefined) throw new Error(`the generator imports ${outside.path}, outside the repository`);
      const packages = new Set(inputs.flatMap(({ imports }) => imports
        .filter((imported) => imported.external === true && !isBuiltin(imported.path) && !imported.path.startsWith("."))
        .map((imported) => packageOf(imported.path))));
      return { files: inputs.map(({ path }) => path), packages: [...packages].sort(byText) };
    });
    graphs.set(root, graph);
  }
  return graph;
}

/** The key the answers of `<pack>/<tool>` are stored under for the working tree at `root`, and what went into it. */
export async function generatorKey(root: string, pack: string, tool: string): Promise<GeneratorKey> {
  const manifest = readGatesManifest(root, pack, tool);
  const version = manifest.upstream.version;
  for (const [what, segment] of [["pack", pack], ["tool", tool], ["upstream.version", version]] as const) {
    if (!KEY_SEGMENT.test(segment) || segment === "." || segment === "..") throw new Error(`packs/${pack}/upstream/${tool}: the ${what} ${JSON.stringify(segment)} cannot name a key segment (${KEY_SEGMENT.source})`);
  }

  const graph = await generatorGraph(root);
  const directory = `packs/${pack}/upstream/${tool}`;
  const installed = recordedRuntimes(manifest);
  const runtimes = installed.map((runtime) => `${RUNTIMES_DIRECTORY}/${runtime}/`);
  const tracked = execFileSync("git", ["ls-files", "-z", "--",
    GENERATOR_DIRECTORY,
    `${directory}/${GATES_MANIFEST}`,
    `${directory}/${GOLDENS_FILE}`,
    `${directory}/fixtures`,
    ...conformancePaths(pack),
    `packs/${pack}/licenses/*/UPSTREAM.md`,
    CORPUS_TASKS_FILE,
    ...(ADAPTER_REPOSITORY_FILES[manifest.adapter] ?? []),
    GENERATOR_ACTIONS,
    ...(installed.includes("php") ? [PHP_ACTION] : []),
  ], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter((path) => path !== "");
  // Under the generator's directory: the drivers and other data (its TypeScript is the import graph's), and only the
  // runtimes this tool-pack installs.
  const data = tracked.filter((path) => !path.startsWith(`${GENERATOR_DIRECTORY}/`)
    || (path.startsWith(`${RUNTIMES_DIRECTORY}/`) ? runtimes.some((runtime) => path.startsWith(runtime)) : !path.endsWith(".ts")));
  const files = new Set([...graph.files, ...data]);
  const lines = new Map<string, string>();
  for (const path of files) {
    const absolute = join(root, path);
    lines.set(path, existsSync(absolute) ? sha256(readFileSync(absolute)) : sha256("deleted"));
  }
  const lock: unknown = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  for (const [location, value] of lockedClosure(lock, graph.packages)) lines.set(`npm:${location}`, sha256(value));
  const { type } = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { type?: unknown };
  lines.set("package.json#type", sha256(JSON.stringify(type ?? null)));

  const inputs = [...lines.keys()].sort(byText);
  const digest = sha256(inputs.map((input) => `${lines.get(input)!}  ${input}\n`).join(""));
  return { pack, tool, version, digest, key: `${pack}/${tool}/${version}/${digest}`, inputs };
}
