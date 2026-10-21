/**
 * `npm run generate`: a tool-pack's expected answers (conformance/port-readiness/recording/expected.ts), under the
 * generator key of this tree (recording/generator-key.ts), in the local cache the gates read: taken from the cache, else
 * from the answers main stores (recording/expected-store.ts), else from the pull request's own, else made by running the
 * live adapter on the pinned upstream, which also copies into the tool-pack the case sources it lacks. Nothing it makes
 * but those case sources is written into the tree.
 *
 * Modes, beside that one:
 * - `--upload pr:<n>|main` makes sure the store holds the key's answers: the pull request's container (unless main's
 *   holds them) or main's.
 * - `--promote` (main only) puts the key's answers in main's container, from the pull request's (`--pr`), the local
 *   cache, or a generation, and points main's `latest/<pack>/<tool>.json` at them.
 * - `--drift` (the nightly, after a credential-less `--force`) compares the local cache's answers with the ones main
 *   stores under the same key: a difference is a generator that is not deterministic, or an input the key misses.
 * - `--key` prints the key and its inputs, as one line of JSON, and does nothing else.
 *
 * `--changes <file>` writes the per-rule summary (recording/diff.ts) against main's latest answers, or, with `--drift`,
 * against the stored ones. Exit 0 when done, 1 when `--drift` finds a difference, 2 on a usage error or any failure, 3
 * when `--no-generate` stops a generation that was needed.
 */
import { execFileSync } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { parseArgs } from "node:util";

import { DEFAULT_CACHE_ROOT } from "../../port-readiness/corpus.js";
import {
  blobExists, downloadExpected, expectedStore, MAIN_CONTAINER, mainBlobName, PR_CONTAINER, prBlobName, promoteExpected, readLatest, uploadExpected, writeLatest,
  type ExpectedStore, type Provenance,
} from "../../port-readiness/recording/expected-store.js";
import { generatorKey, type GeneratorKey } from "../../port-readiness/recording/generator-key.js";
import { toolPackDirectory } from "../../port-readiness/manifest.js";
import { ProcessCommandRunner } from "../../port-readiness/process.js";
import { diffRecordings, renderRecordingChanges, type RecordedPair, type RecordingChanges, type ResolvedRuntime, type UnreadRecording } from "../../port-readiness/recording/diff.js";
import { encodeExpected, expectedCachePath, joinExpected, readCachedExpected, writeCachedExpected } from "../../port-readiness/recording/expected.js";
import { REPOSITORY_ROOT } from "../../port-readiness/upstream-files.js";
import { caseSourceDiff, generatedFiles, recordToolPack, writeCaseSources } from "./record.js";

const USAGE = `usage:
  npm run generate -- --pack <id> --tool <tool> [--upload pr:<n>|main] [--force | --no-generate] [--changes file] [--cache dir]
  npm run generate -- --promote --pack <id> --tool <tool> [--pr <n>] [--no-generate] [--cache dir]
  npm run generate -- --drift --pack <id> --tool <tool> [--changes file] [--cache dir]
  npm run generate -- --key --pack <id> --tool <tool>
`;

/** Where the answers came from: the local cache, main's container, the pull request's, or a generation. */
type Source = "cache" | "main" | "pr" | "generated";

class UsageError extends Error {}

/** Exit 3: answers that `--no-generate` forbids generating were needed. */
class GenerationNeeded extends Error {}

/** The store, or the error a mode that needs one fails with. */
function requiredStore(store: ExpectedStore | { unavailable: string }, mode: string): ExpectedStore {
  if ("unavailable" in store) throw new Error(`${mode} needs the answer store: ${store.unavailable}`);
  return store;
}

/** Where the answers being stored were made: this commit, and this GitHub run, or `local`. */
function provenance(root: string): Provenance {
  const { GITHUB_SHA, GITHUB_SERVER_URL, GITHUB_REPOSITORY, GITHUB_RUN_ID } = process.env;
  const commit = GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  const run = GITHUB_RUN_ID === undefined || GITHUB_REPOSITORY === undefined ? "local" : `${GITHUB_SERVER_URL ?? "https://github.com"}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}`;
  return { commit, run };
}

interface Generated {
  key: GeneratorKey;
  files: Map<string, string>;
  resolved: ResolvedRuntime[];
}

/**
 * Run the live adapter and keep its answers in the local cache under the key. The answers are split and checked to
 * join back into the recording before anything is written (expectedFiles). A case source the tool-pack lacks is
 * copied into it, which moves the key, so the answers are kept under the key of the tree as it is afterwards; when the
 * answers are to be stored (`committed`), a missing case source fails instead, since what is stored must be keyed by
 * the commit.
 */
async function generate(root: string, pack: string, tool: string, cacheRoot: string, committed: boolean): Promise<Generated> {
  const started = Date.now();
  const recorded = await recordToolPack({ root, pack, tool, cacheRoot, runner: new ProcessCommandRunner() });
  const files = generatedFiles(root, pack, tool, recorded);
  if (committed) {
    const { missing, conflicting } = caseSourceDiff(toolPackDirectory(root, pack, tool), recorded);
    if (missing.length + conflicting.length > 0) {
      throw new Error(`packs/${pack}/upstream/${tool}: ${missing.length} case source(s) are missing and ${conflicting.length} differ from upstream's (${[...missing.map((copy) => copy.file), ...conflicting].slice(0, 5).join(", ")}): run npm run generate -- --pack ${pack} --tool ${tool} and commit them`);
    }
  }
  const copied = committed ? [] : await writeCaseSources(root, pack, tool, recorded);
  const key = await generatorKey(root, pack, tool);
  await writeCachedExpected(cacheRoot, key.key, encodeExpected(pack, tool, key.key, files).bytes);
  const bytes = [...files.values()].reduce((total, text) => total + Buffer.byteLength(text), 0);
  process.stdout.write(`generated ${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream), `
    + `${[...files.keys()].filter((path) => path.startsWith("cases/")).length} test files, ${recorded.sources.length} case sources, `
    + `${recorded.corpus.files.length} corpus files; ${files.size} files, ${(bytes / 1024 / 1024).toFixed(2)} MB; ${((Date.now() - started) / 1000).toFixed(0)} s\n`);
  if (copied.length > 0) process.stdout.write(`copied ${copied.length} case source(s) into packs/${pack}/upstream/${tool}, listed in UPSTREAM.md: commit them; the key is now ${key.key}\n`);
  return { key, files, resolved: recorded.resolved };
}

/** The answers as the gates read them, for the summary. */
function joined(files: ReadonlyMap<string, string>, key: string): RecordedPair {
  const { recording, corpus } = joinExpected(files, key);
  return { recording, corpus };
}

/** Write the per-rule summary, or remove an earlier one when there is nothing to compare (`changes` null). */
async function writeChanges(file: string | undefined, changes: RecordingChanges | null): Promise<void> {
  if (file === undefined) return;
  if (changes === null) {
    await rm(resolve(file), { force: true });
    return;
  }
  await mkdir(dirname(resolve(file)), { recursive: true });
  await writeFile(resolve(file), renderRecordingChanges([changes]));
}

/** What the summary says of a comparison. */
function describe(changes: RecordingChanges): string {
  if (changes.first) return "nothing to compare with";
  if (changes.unavailable !== null) return `not compared (${changes.unavailable})`;
  return `${changes.rules.length} upstream rule(s) answer differently${changes.pinsChanged.length === 0 ? "" : `, pins moved (${changes.pinsChanged.join("; ")})`}`;
}

/**
 * The summary against the answers main last promoted: null when they are these (the same key), a first generation
 * when main promoted none, and not compared when there is no store or main's answers cannot be read.
 */
async function changesAgainstMain(store: ExpectedStore | { unavailable: string }, key: GeneratorKey, files: ReadonlyMap<string, string>, resolved: ResolvedRuntime[]): Promise<RecordingChanges | null> {
  const after = joined(files, key.key);
  if ("unavailable" in store) return diffRecordings(key.pack, key.tool, { unavailable: store.unavailable }, after, resolved);
  const latest = await readLatest(store, key.pack, key.tool);
  if (latest?.key === key.key) return null;
  let before: RecordedPair | UnreadRecording | null = null;
  if (latest !== null) {
    try {
      const stored = await downloadExpected(store, MAIN_CONTAINER, mainBlobName(latest.key), latest.key);
      before = stored === null ? { unavailable: `main's latest answers, ${latest.key}, are not in its container` } : joined(stored.files, latest.key);
    } catch (error) {
      before = { unavailable: error instanceof Error ? error.message : String(error) };
    }
  }
  return diffRecordings(key.pack, key.tool, before, after, resolved);
}

/** Answers in the cache, main's container or the pull request's, in that order, copied into the cache; null on a miss. */
async function stored(store: ExpectedStore | { unavailable: string }, key: string, cacheRoot: string, pr: number | undefined): Promise<{ files: Map<string, string>; source: Source } | null> {
  const cached = readCachedExpected(cacheRoot, key);
  if (cached !== undefined) return { files: cached, source: "cache" };
  if ("unavailable" in store) return null;
  const places: [Source, string, string][] = [["main", MAIN_CONTAINER, mainBlobName(key)]];
  if (pr !== undefined) places.push(["pr", PR_CONTAINER, prBlobName(pr, key)]);
  for (const [source, container, name] of places) {
    const found = await downloadExpected(store, container, name, key);
    if (found === null) continue;
    await writeCachedExpected(cacheRoot, key, found.encoded.bytes);
    return { files: found.files, source };
  }
  return null;
}

function parsePositive(value: string, what: string): number {
  if (!/^[1-9]\d*$/u.test(value)) throw new UsageError(`${what} must be a pull request number, not ${JSON.stringify(value)}`);
  return Number(value);
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    options: {
      pack: { type: "string" },
      tool: { type: "string" },
      cache: { type: "string" },
      changes: { type: "string" },
      upload: { type: "string" },
      force: { type: "boolean", default: false },
      "no-generate": { type: "boolean", default: false },
      promote: { type: "boolean", default: false },
      pr: { type: "string" },
      drift: { type: "boolean", default: false },
      key: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    allowPositionals: true,
    strict: true,
  });
  if (values.help) {
    process.stdout.write(USAGE);
    return 0;
  }
  if (positionals.length > 0 || values.pack === undefined || values.tool === undefined) throw new UsageError("--pack and --tool are required");
  const modes = [values.promote && "--promote", values.drift && "--drift", values.key && "--key"].filter((mode) => mode !== false);
  if (modes.length > 1) throw new UsageError(`${modes.join(" and ")} exclude each other`);
  const mode = modes[0] ?? "generate";
  const allowed: Record<string, string[]> = {
    generate: ["--upload", "--force", "--no-generate", "--changes", "--cache"],
    "--promote": ["--pr", "--no-generate", "--cache"],
    "--drift": ["--changes", "--cache"],
    "--key": ["--cache"],
  };
  const set: Record<string, boolean> = {
    "--upload": values.upload !== undefined, "--force": values.force, "--no-generate": values["no-generate"], "--changes": values.changes !== undefined,
    "--cache": values.cache !== undefined, "--pr": values.pr !== undefined,
  };
  const extra = Object.keys(set).filter((flag) => set[flag] && !allowed[mode]!.includes(flag));
  if (extra.length > 0) throw new UsageError(`${extra.join(", ")} cannot be given ${mode === "generate" ? "without --promote" : `with ${mode}`}`);
  if (values.force && values["no-generate"]) throw new UsageError("--force and --no-generate exclude each other");

  const root = REPOSITORY_ROOT;
  const { pack, tool } = values;
  const cacheRoot = resolve(values.cache ?? DEFAULT_CACHE_ROOT);
  const key = await generatorKey(root, pack, tool);
  if (mode === "--key") {
    process.stdout.write(`${JSON.stringify(key)}\n`);
    return 0;
  }
  process.stdout.write(`key: ${key.key}\n`);
  const store = expectedStore(new ProcessCommandRunner());

  if (mode === "--drift") {
    const reached = requiredStore(store, "--drift");
    const local = readCachedExpected(cacheRoot, key.key);
    if (local === undefined) throw new Error(`the local cache holds no answers for ${key.key} (${expectedCachePath(cacheRoot, key.key)}): run npm run generate -- --pack ${pack} --tool ${tool} --force first`);
    const main = await downloadExpected(reached, MAIN_CONTAINER, mainBlobName(key.key), key.key);
    if (main === null) {
      await writeChanges(values.changes, diffRecordings(pack, tool, null, joined(local, key.key)));
      process.stdout.write(`drift: main stores no answers for ${key.key}\n`);
      return 1;
    }
    const differing = [...new Set([...local.keys(), ...main.files.keys()])].filter((path) => local.get(path) !== main.files.get(path)).sort();
    const changes = diffRecordings(pack, tool, joined(main.files, key.key), joined(local, key.key));
    await writeChanges(values.changes, changes);
    if (differing.length === 0) {
      process.stdout.write(`no drift: the regenerated answers are the ones main stores (made at ${main.provenance.commit}, ${main.provenance.run})\n`);
      return 0;
    }
    process.stdout.write(`drift: ${differing.length} file(s) of the regenerated answers differ from the ones main stores for the same key (made at ${main.provenance.commit}, ${main.provenance.run}); ${describe(changes)}\n`);
    for (const path of differing.slice(0, 50)) process.stdout.write(`  ${path}: ${!main.files.has(path) ? "only regenerated" : !local.has(path) ? "only stored" : "differs"}\n`);
    if (differing.length > 50) process.stdout.write(`  ... and ${differing.length - 50} more\n`);
    return 1;
  }

  if (mode === "--promote") {
    const reached = requiredStore(store, "--promote");
    const pr = values.pr === undefined ? undefined : parsePositive(values.pr, "--pr");
    const made = provenance(root);
    let source: Source;
    if (await blobExists(reached, MAIN_CONTAINER, mainBlobName(key.key))) {
      source = "main";
    } else if (pr !== undefined && await promoteExpected(reached, pr, key.key)) {
      source = "pr";
    } else {
      const cached = readCachedExpected(cacheRoot, key.key);
      if (cached === undefined && values["no-generate"]) throw new GenerationNeeded(`neither main's container${pr === undefined ? "" : `, pull request ${pr}'s`} nor the local cache holds answers for ${key.key}`);
      const files = cached ?? (await generate(root, pack, tool, cacheRoot, true)).files;
      source = cached === undefined ? "generated" : "cache";
      await uploadExpected(reached, MAIN_CONTAINER, mainBlobName(key.key), encodeExpected(pack, tool, key.key, files), made);
    }
    await writeLatest(reached, pack, tool, { key: key.key, commit: made.commit });
    process.stdout.write(`source: ${source}\npromoted: ${MAIN_CONTAINER}/${mainBlobName(key.key)}, latest/${pack}/${tool}.json at ${made.commit}\n`);
    return 0;
  }

  if (values.upload !== undefined && values.upload !== "main" && !values.upload.startsWith("pr:")) throw new UsageError(`--upload takes pr:<n> or main, not ${JSON.stringify(values.upload)}`);
  const upload = values.upload === undefined ? undefined
    : values.upload === "main" ? { container: MAIN_CONTAINER, pr: undefined }
    : { container: PR_CONTAINER, pr: parsePositive(values.upload.slice("pr:".length), "--upload pr:<n>") };
  const reached = upload === undefined ? undefined : requiredStore(store, "--upload");
  let found = values.force ? null : await stored(store, key.key, cacheRoot, upload?.pr);
  let resolved: ResolvedRuntime[] = [];
  let current = key;
  if (found === null) {
    if (values["no-generate"]) throw new GenerationNeeded(`no answers for ${key.key} are in the local cache${"unavailable" in store ? "" : `, main's container${upload?.pr === undefined ? "" : ` or pull request ${upload.pr}'s`}`}`);
    const generated = await generate(root, pack, tool, cacheRoot, upload !== undefined);
    found = { files: generated.files, source: "generated" };
    resolved = generated.resolved;
    current = generated.key;
  }
  process.stdout.write(`source: ${found.source}\n`);

  if (reached !== undefined && upload !== undefined) {
    const name = upload.pr === undefined ? mainBlobName(current.key) : prBlobName(upload.pr, current.key);
    // Answers taken from a container are held there; answers from the local cache are uploaded unless a container
    // already holds them; generated ones always are.
    const held = found.source === "main" || found.source === "pr" || (found.source === "cache"
      && (await blobExists(reached, MAIN_CONTAINER, mainBlobName(current.key)) || (upload.pr !== undefined && await blobExists(reached, PR_CONTAINER, name))));
    if (held) {
      process.stdout.write(`uploaded: no, the store already holds ${current.key}\n`);
    } else {
      await uploadExpected(reached, upload.container, name, encodeExpected(pack, tool, current.key, found.files), provenance(root));
      process.stdout.write(`uploaded: ${upload.container}/${name}\n`);
    }
  }

  if (values.changes !== undefined) {
    const changes = await changesAgainstMain(store, current, found.files, resolved);
    await writeChanges(values.changes, changes);
    process.stdout.write(changes === null ? "changes: none, these are main's latest answers\n" : `changes: ${describe(changes)}\n`);
  }
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  if (error instanceof GenerationNeeded) {
    process.stdout.write(`generation needed: ${error.message}\n`);
    process.exitCode = 3;
    return;
  }
  // parseArgs throws a TypeError whose code starts ERR_PARSE_ARGS_ for an unknown or malformed option.
  const usage = error instanceof UsageError || (error instanceof TypeError && "code" in error && String(error.code).startsWith("ERR_PARSE_ARGS"));
  process.stderr.write(usage ? `${error.message}\n${USAGE}` : `${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
