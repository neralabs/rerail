/**
 * Temporary: re-file each tool-pack's locked recording under RECORDING_FORMAT 2 without asking upstream. The old
 * archive is read from `oldCache` (a `recordings/` directory), its config id checked against the formula of the head
 * before the goldens and corpus tier were removed, and its tree re-filed under the new config id without `answers/`,
 * `tool/<id>/corpus.json` and the verdicts no case source or probe names.
 */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { parse } from "yaml";

import { archiveSizeBound, decompressArchive, readArchive, sha256 } from "../../../scripts/s3/test-data-archive.mjs";
import { readGatesManifest, toolPackDirectory, type GatesManifest } from "../../port-readiness/manifest.js";
import { decodeSource } from "../../port-readiness/recorded-adapter.js";
import { ADAPTER_SOURCES, answerConfig, configId, configPath, memoryTree, packTree, parsersId, RECORDING_ARCHIVE, RECORDING_FORMAT } from "../../port-readiness/recording/archive.js";
import { canonicalText, type RecordedRule } from "../../port-readiness/recording/schema.js";
import { readRecordingLock } from "../../port-readiness/recording/store.js";
import { loadRecording } from "../../port-readiness/recording/tree.js";
import { verdictPath } from "../../port-readiness/recording/verdicts.js";
import type { Recorded } from "./record.js";

/** The head before the goldens and corpus tier were removed: the formula the locked config ids were made with. */
const OLD_COMMIT = "6e74da657";
const OLD_RECORDING_FORMAT = 1;

const git = (root: string, args: string[]): Buffer => execFileSync("git", args, { cwd: root, maxBuffer: 1 << 28 });

/** cache.ts's trackedDigest of `pathspec`, over OLD_COMMIT's tree rather than the working tree. */
function oldTrackedDigest(root: string, pathspec: readonly string[]): string {
  const listed = git(root, ["ls-tree", "-r", "-z", "--full-tree", OLD_COMMIT, "--", ...pathspec]).toString("utf8").split("\0").filter((line) => line !== "")
    .map((line) => { const tab = line.indexOf("\t"); return { path: line.slice(tab + 1), oid: line.slice(0, tab).split(" ")[2]! }; })
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return sha256(listed.map(({ path, oid }) => `${path}\0${sha256(git(root, ["cat-file", "blob", oid]))}`).join("\n"));
}

/** OLD_COMMIT's answerConfig: format 1, the old adapter sources, the old UPSTREAM.md "Not copied" prefixes, and the ESLint settings the corpus read (eslint.vue's script_parsers, directive_rules). */
function oldConfig(root: string, pack: string, tool: string, manifest: GatesManifest, current: Record<string, unknown>): Record<string, unknown> {
  const oldYaml = parse(git(root, ["show", `${OLD_COMMIT}:packs/${pack}/upstream/${tool}/gates.yaml`]).toString("utf8")) as { eslint?: { vue?: Record<string, unknown> } };
  const eslint = manifest.eslint === undefined || manifest.eslint.vue === undefined ? manifest.eslint : {
    ...manifest.eslint,
    vue: { ...manifest.eslint.vue, script_parsers: oldYaml.eslint?.vue?.script_parsers, directive_rules: oldYaml.eslint?.vue?.directive_rules },
  };
  const notCopied = git(root, ["ls-tree", "-r", "-z", "--name-only", OLD_COMMIT, "--", `packs/${pack}/licenses/`]).toString("utf8").split("\0")
    .filter((path) => /^packs\/[^/]+\/licenses\/[^/]+\/UPSTREAM\.md$/u.test(path))
    .flatMap((path) => [...git(root, ["show", `${OLD_COMMIT}:${path}`]).toString("utf8").matchAll(/^- Not copied: `([^`]+)`/gmu)].map((match) => match[1]!))
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return {
    ...current,
    recording: OLD_RECORDING_FORMAT,
    sources: oldTrackedDigest(root, ADAPTER_SOURCES[manifest.adapter]),
    gates: { ...(current.gates as Record<string, unknown>), eslint },
    notCopied,
  };
}

function readPath(testCase: { path: string; parsedAs?: string | undefined }): string {
  return testCase.parsedAs === undefined ? testCase.path : testCase.path.replace(/\.[^./]*$/u, testCase.parsedAs);
}

/** record.ts's verdict inputs a record keeps: every committed case source and every known gap's probe. */
function keptVerdicts(directory: string, manifest: GatesManifest, rules: Readonly<Record<string, RecordedRule>>, parsers: string, label: string): Set<string> {
  const kept = new Set<string>();
  for (const rule of Object.values(rules)) {
    for (const testCase of rule.cases) {
      if (testCase.file === null) continue;
      const file = join(directory, testCase.file);
      if (!existsSync(file)) throw new Error(`${label}: the case source ${testCase.file} is not in the checkout (npm run fixtures:pull)`);
      kept.add(verdictPath(parsers, { path: readPath(testCase), sha256: sha256(decodeSource(readFileSync(file))), script: testCase.script === true }));
    }
  }
  const caseByFixture = new Map(Object.values(rules).flatMap((rule) => rule.cases).map((testCase) => [testCase.fixture, testCase]));
  for (const gap of manifest.known_gaps) {
    const extensions = [...new Set(gap.fixtures.map((fixture) => caseByFixture.get(fixture)).filter((testCase) => testCase !== undefined).map((testCase) => extname(readPath(testCase))))];
    if (extensions.length !== 1 || extensions[0] === "") continue;
    const script = gap.fixtures.every((fixture) => caseByFixture.get(fixture)?.script === true);
    kept.add(verdictPath(parsers, { path: `probe${extensions[0]}`, sha256: sha256(gap.probe), script }));
  }
  return kept;
}

/** The locked recording re-filed at this branch's config, as a Recorded writeRecording writes; throws unless the locked config id is the old formula's. */
export function rekeyToolPack(root: string, pack: string, tool: string, oldCache: string): Recorded {
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  const manifest = readGatesManifest(root, pack, tool);
  const lock = readRecordingLock(root, pack, tool);
  const pin = lock.files[RECORDING_ARCHIVE];
  const tar = decompressArchive(readFileSync(join(oldCache, `${pin.sha256}.tar.zst`)), archiveSizeBound(pin));
  if (sha256(tar) !== pin.sha256) throw new Error(`${label}: the old archive's tar is not the locked ${pin.sha256}`);
  const old = new Map(readArchive(tar).map((entry) => [entry.path, entry.bytes]));

  const current = answerConfig(root, manifest, lock.runtimes);
  const before = oldConfig(root, pack, tool, manifest, current);
  const oldId = configId(before);
  const stored = [...old.keys()].filter((path) => path.startsWith("config/"));
  if (stored.length !== 1 || stored[0] !== configPath(oldId)) throw new Error(`${label}: the archive's config is ${stored.join(", ")}, not the old formula's ${configPath(oldId)}: refusing to re-key`);
  if (old.get(configPath(oldId))!.toString("utf8") !== canonicalText(before)) throw new Error(`${label}: config/${oldId}.json is not the old formula's config`);
  const parsers = JSON.parse(old.get(`tool/${oldId}/parsers.json`)!.toString("utf8")) as unknown;
  const oldParsers = sha256(canonicalText({ parsers, sources: before.sources, recording: OLD_RECORDING_FORMAT }));

  if (current.recording !== RECORDING_FORMAT) throw new Error("answerConfig is not at RECORDING_FORMAT");
  const id = configId(current);
  const newParsers = parsersId(manifest.adapter, parsers as Parameters<typeof parsersId>[1]);
  const tree = new Map<string, Buffer>([[configPath(id), Buffer.from(canonicalText(current), "utf8")]]);
  for (const [path, bytes] of old) {
    if (path.startsWith("config/") || path.startsWith("answers/") || path === `tool/${oldId}/corpus.json`) continue;
    if (path.startsWith(`tool/${oldId}/`)) tree.set(`tool/${id}/${path.slice(`tool/${oldId}/`.length)}`, bytes);
    else if (path.startsWith(`rules/${oldId}/`)) tree.set(`rules/${id}/${path.slice(`rules/${oldId}/`.length)}`, bytes);
    else if (path.startsWith(`verdicts/${oldParsers}/`)) tree.set(`verdicts/${newParsers}/${path.slice(`verdicts/${oldParsers}/`.length)}`, bytes);
    else if (path.startsWith("texts/")) tree.set(path, bytes);
    else throw new Error(`${label}: ${path} is under neither the old config ${oldId.slice(0, 12)} nor the old parsers ${oldParsers.slice(0, 12)}`);
  }
  const recording = loadRecording(memoryTree(tree), id, label);
  const wanted = keptVerdicts(toolPackDirectory(root, pack, tool), manifest, recording.rules, newParsers, label);
  const missing = [...wanted].filter((path) => !tree.has(path));
  if (missing.length > 0) throw new Error(`${label}: the old archive lacks ${missing.length} verdict(s) a record keeps, e.g. ${missing[0]}`);
  const rekeyed = new Map([...tree].filter(([path]) => !path.startsWith("verdicts/") || wanted.has(path)));
  return {
    tree: rekeyed,
    packed: packTree(rekeyed),
    recording: loadRecording(memoryTree(rekeyed), id, label),
    configId: id,
    asked: { rules: [], verdicts: 0 },
    written: new Map(),
    deduplicated: 0,
    sources: [],
    resolved: [],
  };
}
