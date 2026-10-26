/**
 * The conformance check's corpus recordings as samples (D3): each `corpus-<name>.expected.json` of a tool-pack speaks
 * for every file its corpus tier scored (corpora.ts `corpusFiles`) at the pinned commit. A file gets, for every rule the
 * recording ran, its findings as line labels or `[]`; a file the tool skipped gets none, and on a file with lines the
 * tool did not analyse (`unanalysed`) a rule without findings there gets no key, since its silence says nothing.
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { recordingSchema, type CorpusRecording } from "../../conformance/languages/pack.js";
import type { CommandRunner } from "../../conformance/port-readiness/process.js";
import { checkoutReader, contextFiles, corpusCheckout, corpusFiles, corpusRepo, packRuleIds, type PackConformance } from "./corpora.js";
import type { SampleLabel } from "./format.js";
import { copyleftReason } from "./licences.js";
import { byText, sha256, type Sample, type ToolPack } from "./samples.js";

const RECORDING_FILE = /^corpus-(.+)\.expected\.json$/u;

/** One finding of a corpus recording, and why it made no label when it made none. */
export interface CorpusFinding {
  toolPack: ToolPack;
  corpus: string;
  repo: string;
  /** The upstream rule. */
  rule: string;
  path: string;
  line: number;
  dropped?: { reason: "licence" | "a file the tool skipped" | "a trait in a using class's context" | "not a scored file" | "not a pack rule"; detail: string };
}

export interface CorpusSummary {
  toolPack: ToolPack;
  corpus: string;
  repo: string;
  commit: string;
  version: string;
  rules: number;
  files: number;
  excluded?: string;
}

export interface CorpusSamples {
  samples: Sample[];
  findings: CorpusFinding[];
  corpora: CorpusSummary[];
}

/** The corpus recordings a tool-pack's directory holds, by corpus name. */
export function corpusRecordings(directory: string): Map<string, CorpusRecording> {
  const recordings = new Map<string, CorpusRecording>();
  if (!existsSync(directory)) return recordings;
  for (const name of readdirSync(directory).sort(byText)) {
    const match = RECORDING_FILE.exec(name);
    if (match === null) continue;
    const recording = recordingSchema.parse(JSON.parse(readFileSync(join(directory, name), "utf8")));
    if (recording.source.kind !== "corpus" || recording.source.name !== match[1]) throw new Error(`${name} is not a recording of corpus ${match[1]}`);
    recordings.set(match[1]!, recording as CorpusRecording);
  }
  return recordings;
}

export async function corpusSamples(options: {
  root: string;
  cacheRoot: string;
  toolPack: ToolPack;
  conformance: PackConformance;
  runner: CommandRunner;
  licence: (repo: string, commit: string) => string;
}): Promise<CorpusSamples> {
  const { root, cacheRoot, toolPack, conformance, runner } = options;
  const result: CorpusSamples = { samples: [], findings: [], corpora: [] };
  for (const [name, recording] of corpusRecordings(join(root, "packs", toolPack.pack, "upstream", toolPack.tool))) {
    const corpus = conformance.corpora.find((entry) => entry.name === name);
    if (corpus === undefined) throw new Error(`${toolPack.pack}/${toolPack.tool}: corpus-${name}.expected.json names a corpus the pack does not pin`);
    if (recording.source.commit !== corpus.commit) throw new Error(`corpus-${name}.expected.json records ${name} at ${recording.source.commit}; the pack pins ${corpus.commit}`);
    const repo = corpusRepo(corpus);
    const summary: CorpusSummary = { toolPack, corpus: name, repo, commit: corpus.commit, version: recording.version, rules: recording.source.rules.length, files: 0 };
    result.corpora.push(summary);
    const finding = (entry: CorpusRecording["findings"][number]): CorpusFinding => ({ toolPack, corpus: name, repo, rule: entry.rule, path: entry.path, line: entry.line });
    const copyleft = copyleftReason(repo);
    if (copyleft !== undefined) {
      summary.excluded = copyleft;
      for (const entry of recording.findings) result.findings.push({ ...finding(entry), dropped: { reason: "licence", detail: `${repo}: ${copyleft}` } });
      continue;
    }
    const checkout = await corpusCheckout(cacheRoot, corpus, runner);
    const files = corpusFiles(checkout, corpus, conformance.family, new Set(recording.source.skipped));
    summary.files = files.length;
    const scored = new Set(files);
    const byFile = new Map<string, Map<string, SampleLabel[]>>();
    for (const entry of recording.findings) {
      if (!scored.has(entry.path)) {
        // PHPStan reports a trait's lines once per class using it, as `<path> (in context of class <name>)` (an
        // anonymous class's name holds its own path): a finding in that class's context, which no file holds alone.
        const reason = recording.source.skipped.includes(entry.path) ? "a file the tool skipped" : / \(in context of class/u.test(entry.path) ? "a trait in a using class's context" : "not a scored file";
        result.findings.push({ ...finding(entry), dropped: { reason, detail: `${entry.path} is not a file the corpus tier scored` } });
        continue;
      }
      if (packRuleIds(conformance, toolPack.tool, entry.rule).length === 0) {
        result.findings.push({ ...finding(entry), dropped: { reason: "not a pack rule", detail: `${entry.rule} is ported by no rule of the pack` } });
        continue;
      }
      result.findings.push(finding(entry));
      const rules = byFile.get(entry.path) ?? new Map<string, SampleLabel[]>();
      rules.set(entry.rule, [...(rules.get(entry.rule) ?? []), entry.column === undefined ? { line: entry.line } : { line: entry.line, column: entry.column }]);
      byFile.set(entry.path, rules);
    }
    const unanalysed = new Set(recording.source.unanalysed.map((region) => region.path));
    const reader = checkoutReader(checkout);
    for (const path of files) {
      const expected = corpusFileLabels(recording.source.rules, byFile.get(path), unanalysed.has(path), (rule) => packRuleIds(conformance, toolPack.tool, rule));
      if (expected.size === 0) continue;
      const bytes = reader.read(path)!;
      result.samples.push({
        toolPack,
        short: repo.split("/")[1]!,
        source: { repo, commit: corpus.commit, path, license: options.licence(repo, corpus.commit), sha256: sha256(bytes) },
        bytes,
        labelledBy: { tool: recording.tool, version: recording.version, how: "corpus recording" },
        expected,
        context: contextFiles(toolPack, path, reader),
        pinned: false,
        origins: ["corpus"],
      });
    }
  }
  return result;
}

/**
 * A scored file's labels from its recording: each rule the recording ran gives its findings on the file, or `[]`,
 * under every pack rule id porting it; on a file with unanalysed lines a rule without findings gives no key.
 */
export function corpusFileLabels(
  rules: readonly string[],
  found: ReadonlyMap<string, SampleLabel[]> | undefined,
  partlyUnanalysed: boolean,
  idsOf: (rule: string) => string[],
): Map<string, SampleLabel[]> {
  const expected = new Map<string, SampleLabel[]>();
  for (const rule of rules) {
    const labels = found?.get(rule);
    if (labels === undefined && partlyUnanalysed) continue;
    for (const id of idsOf(rule)) expected.set(id, labels ?? []);
  }
  return expected;
}
