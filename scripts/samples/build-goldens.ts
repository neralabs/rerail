/**
 * The port-readiness goldens as samples: every golden file of a tool-pack (`goldens.tsv`), fetched as the gates fetch
 * it (the pinned task's reference solution, or the pinned corpus's file) and checked against its sha256, labelled for
 * every rule its recording answered: findings give line labels, silence `[]`, an excluded or missing answer no key.
 */
import { join, matchesGlob } from "node:path";

import { buildCorpus, buildRerailCorpora, readCorpusTasks, rerailCorpusId, wantedKey, wantedPlaces, type CorpusFile } from "../../conformance/port-readiness/corpus.js";
import { readGoldens, type Golden } from "../../conformance/port-readiness/goldens.js";
import type { CommandRunner } from "../../conformance/port-readiness/process.js";
import { recordedInCorpus } from "../../conformance/port-readiness/recorded-adapter.js";
import { cachedTree, directoryTree, treeConfigs } from "../../conformance/port-readiness/recording/archive.js";
import { CONTEXT_FILES, answerKey, answerPath, corpusCommits, fetchedContext, parseAnswer, type Answer } from "../../conformance/port-readiness/recording/corpus-answers.js";
import { readRecordingLock } from "../../conformance/port-readiness/recording/store.js";
import { loadRecording } from "../../conformance/port-readiness/recording/tree.js";
import { checkoutReader, contextFiles, corpusRepo, packRuleIds, type PackConformance } from "./corpora.js";
import { copyleftReason } from "./licences.js";
import type { SampleLabel } from "./format.js";
import { byText, sha256, type Sample, type ToolPack } from "./samples.js";

/** What a recording answered for one rule on one golden file. */
export type AnswerState = "findings" | "silent" | "excluded by upstream" | "unanswered";

/** One `goldens.tsv` row, and what became of its file. */
export interface GoldenRow {
  toolPack: ToolPack;
  /** The pack rule id the row is listed under. */
  rule: string;
  task: string;
  path: string;
  sha256: string;
  repo: string;
  /**
   * Why no sample was made of the row's file: a copyleft source, a file its pinned corpus leaves out (an `exclude`
   * glob, so the gates never fetched or judged it either), or a file that could not be fetched as pinned.
   */
  dropped?: { reason: "licence" | "left out by the corpus pin" | "unfetched"; detail: string };
  /** The recording's answer for the row's rule on its file, when a sample was made. */
  answer?: AnswerState | "rule not recorded";
}

export interface GoldenSamples {
  samples: Sample[];
  rows: GoldenRow[];
  /** The recording's config id and tool version, for the report. */
  recording: { config: string; version: string; rules: number };
}

const RECORDING_ARCHIVE = "recording.tar.zst";

/**
 * What a recorded answer gives a sample for its rule: findings give line labels (0-based recorded lines to 1-based,
 * the native column kept as provenance), silence `[]`, an excluded file or a missing answer no labels (no key).
 */
export function answerLabels(answer: Answer | undefined): { state: AnswerState; labels?: SampleLabel[] } {
  if (answer === undefined) return { state: "unanswered" };
  if ("excluded" in answer) return { state: "excluded by upstream" };
  const labels = answer.findings.map(([line, column]) => (column === null ? { line: line + 1 } : { line: line + 1, column: column + 1 }));
  return { state: labels.length > 0 ? "findings" : "silent", labels };
}

export async function goldenSamples(options: {
  root: string;
  cacheRoot: string;
  toolPack: ToolPack;
  conformance: PackConformance;
  runner: CommandRunner;
  licence: (repo: string, commit: string) => string;
}): Promise<GoldenSamples> {
  const { root, cacheRoot, toolPack, conformance, runner } = options;
  const { pack, tool } = toolPack;
  const directory = join(root, "packs", pack, "upstream", tool);
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  const lock = readRecordingLock(root, pack, tool);
  const treeDirectory = cachedTree(cacheRoot, lock.files[RECORDING_ARCHIVE], label);
  if (treeDirectory === undefined) throw new Error(`${label} is not in the cache under ${cacheRoot}: fetch it with node scripts/s3/pack-fixtures.mjs recordings --pack ${pack} --tool ${tool}`);
  const tree = directoryTree(treeDirectory);
  const configs = treeConfigs(tree);
  if (configs.length !== 1) throw new Error(`${label} holds ${configs.length} configs, not one`);
  const config = configs[0]!;
  const recording = loadRecording(tree, config, label);
  const names = Object.keys(recording.rules).sort(byText);

  const goldens = await readGoldens(directory);
  const named: Golden[] = [...goldens.values()].flat();
  const contextOf = CONTEXT_FILES[recording.adapter];
  const wanted = wantedPlaces([...named, ...named.flatMap((golden) => contextOf(golden.path).map((path) => ({ task: golden.task, path })))]);
  const tasks = readCorpusTasks();
  const inCorpus = recordedInCorpus(recording);
  const catalog = await buildCorpus(cacheRoot, inCorpus, runner, wanted, tasks);
  const pinned = await buildRerailCorpora(conformance.corpora, cacheRoot, inCorpus, runner, wanted);
  const fetched: CorpusFile[] = [...catalog.files, ...pinned.files];
  const byPlace = new Map(fetched.map((file) => [wantedKey(file.task, file.path), file]));
  const keyContext = fetchedContext(fetched, corpusCommits(conformance.corpora, tasks), names);
  const corpusById = new Map(conformance.corpora.map((corpus) => [rerailCorpusId(corpus), corpus]));
  const taskById = new Map(tasks.map((task) => [task.task, task]));
  const failures = new Map([...catalog.tasks, ...pinned.tasks].filter((task) => task.status !== "applied").map((task) => [task.task, task.reason]));

  const samples: Sample[] = [];
  const answers = new Map<string, Map<string, AnswerState>>();
  const dropped = new Map<string, NonNullable<GoldenRow["dropped"]>>();
  const files = [...new Map(named.map((golden) => [`${golden.task}\0${golden.path}\0${golden.sha256}`, golden])).values()];
  for (const golden of files) {
    const place = `${golden.task}\0${golden.path}\0${golden.sha256}`;
    const corpus = corpusById.get(golden.task);
    const repo = corpus !== undefined ? corpusRepo(corpus) : taskById.get(golden.task)?.repository;
    const commit = corpus?.commit ?? taskById.get(golden.task)?.baseSha;
    if (repo === undefined || commit === undefined) throw new Error(`${pack}/${tool}: golden task ${golden.task} is neither a pinned task nor a corpus the pack pins`);
    const copyleft = copyleftReason(repo);
    if (copyleft !== undefined) {
      dropped.set(place, { reason: "licence", detail: `${repo}: ${copyleft}` });
      continue;
    }
    const file = byPlace.get(wantedKey(golden.task, golden.path));
    if (file === undefined || file.sha256 !== golden.sha256) {
      const leftOut = corpus?.exclude.find((glob) => matchesGlob(golden.path, glob));
      const why = leftOut !== undefined ? `the corpus pin excludes ${leftOut}` : file === undefined ? (failures.get(golden.task) ?? "not fetched") : `fetched sha256 ${file.sha256.slice(0, 12)}`;
      dropped.set(place, { reason: leftOut !== undefined ? "left out by the corpus pin" : "unfetched", detail: `${golden.task}/${golden.path}: ${why}` });
      continue;
    }
    const key = answerKey(recording.adapter, file, keyContext);
    const expected = new Map<string, SampleLabel[]>();
    const states = new Map<string, AnswerState>();
    for (const name of names) {
      const path = answerPath(config, name, file, key);
      const text = tree.text(path);
      const { state, labels } = answerLabels(text === undefined ? undefined : parseAnswer(text, `${label}: ${path}`));
      for (const id of packRuleIds(conformance, tool, name)) {
        states.set(id, state);
        if (labels !== undefined) expected.set(id, labels);
      }
    }
    answers.set(place, states);
    const bytes = Buffer.from(file.source, "utf8");
    if (sha256(bytes) !== golden.sha256) throw new Error(`${golden.task}/${golden.path}: its bytes are not its golden sha256`);
    const reader = corpus !== undefined
      ? checkoutReader(join(cacheRoot, "corpus", "rerail", corpus.name, corpus.commit))
      : { read: (path: string) => { const other = byPlace.get(wantedKey(golden.task, path)); return other === undefined ? undefined : Buffer.from(other.source, "utf8"); } };
    samples.push({
      toolPack,
      short: repo.split("/")[1]!,
      source: { repo, commit, path: golden.path, license: options.licence(repo, commit), sha256: golden.sha256 },
      bytes,
      labelledBy: { tool: recording.tool, version: recording.pins.version, how: "port-readiness recording" },
      expected,
      context: contextFiles(toolPack, golden.path, reader),
      pinned: false,
      origins: ["golden"],
    });
  }

  const rows: GoldenRow[] = [];
  for (const [rule, list] of [...goldens].sort(([a], [b]) => byText(a, b))) {
    for (const golden of list) {
      const place = `${golden.task}\0${golden.path}\0${golden.sha256}`;
      const corpus = corpusById.get(golden.task);
      const repo = corpus !== undefined ? corpusRepo(corpus) : taskById.get(golden.task)!.repository;
      const row: GoldenRow = { toolPack, rule, task: golden.task, path: golden.path, sha256: golden.sha256, repo };
      const drop = dropped.get(place);
      if (drop !== undefined) row.dropped = drop;
      else row.answer = answers.get(place)?.get(rule) ?? "rule not recorded";
      rows.push(row);
    }
  }
  return { samples, rows, recording: { config, version: recording.pins.version, rules: names.length } };
}
