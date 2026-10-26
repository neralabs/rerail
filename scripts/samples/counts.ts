/**
 * The counts report: what became of every input of the sample sets. Each golden row and each corpus-recording finding
 * ends as a kept positive label, a kept clean label (with the reason it was kept), a dropped clean label (not relevant,
 * or over the cap), a label not judged (no answer), or an exclusion (licence, or no file to label).
 */
import type { GoldenRow } from "./build-goldens.js";
import type { CorpusFinding, CorpusSummary } from "./convert-corpora.js";
import type { CleanOutcome } from "./relevance.js";
import { byText, hasPositive, sampleKey, type LabelConflict, type Sample } from "./samples.js";

export interface ToolPackCounts {
  toolPack: string;
  goldenRows: GoldenRow[];
  corpusFindings: CorpusFinding[];
  corpora: CorpusSummary[];
  testSet: { originals: number; fixed: number };
  replacements: number;
  /** Samples before deduplication, after it, and kept. */
  candidates: number;
  deduplicated: Sample[];
  kept: Sample[];
  merged: number;
  conflicts: LabelConflict[];
  clean: Map<string, Map<string, CleanOutcome>>;
  probes: Map<string, { probe: string; cleanOnly: number; nearMisses: number }>;
  archive: { sha256: string; files: number; bytes: number; compressed: number };
}

const tally = (values: Iterable<string>): Map<string, number> => {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  return new Map([...counts].sort(([a], [b]) => byText(a, b)));
};

const table = (counts: Map<string, number>, total: number, heading: string): string[] => [
  `| ${heading} | count |`,
  "|---|---:|",
  ...[...counts].map(([outcome, count]) => `| ${outcome} | ${count} |`),
  `| **total** | **${total}** |`,
  "",
];

/** What became of one label of a kept-or-dropped sample: its final state for `rule`. */
function labelOutcome(counts: ToolPackCounts, byKey: Map<string, Sample>, key: string, rule: string, unjudged: string): string {
  const sample = byKey.get(key);
  const outcome = counts.clean.get(key)?.get(rule);
  const labels = sample?.expected.get(rule);
  if (labels !== undefined && labels.length > 0) return "kept positive";
  if (outcome !== undefined) return outcome.kept ? `kept clean: ${outcome.reason}` : `dropped clean: ${outcome.reason}`;
  return `not judged: ${unjudged}`;
}

export function goldenOutcome(counts: ToolPackCounts, byKey: Map<string, Sample>, row: GoldenRow): string {
  if (row.dropped !== undefined) return row.dropped.reason === "licence" ? "excluded: licence (copyleft source)" : `excluded: ${row.dropped.reason}`;
  return labelOutcome(counts, byKey, `${row.repo}\0${row.path}\0${row.sha256}`, row.rule, row.answer === "excluded by upstream" ? "upstream excluded the file" : row.answer === "rule not recorded" ? "the rule was not recorded" : "no recorded answer");
}

export function corpusOutcome(counts: ToolPackCounts, byPath: Map<string, Sample>, finding: CorpusFinding, ids: readonly string[]): string {
  if (finding.dropped !== undefined) return finding.dropped.reason === "licence" ? "excluded: licence (copyleft source)" : `excluded: ${finding.dropped.reason}`;
  const sample = byPath.get(`${finding.repo}\0${finding.path}`);
  const lines = ids.flatMap((id) => sample?.expected.get(id) ?? []).map((label) => label.line);
  return lines.includes(finding.line) ? "kept positive" : "superseded: the golden's recorded labels for the file differ";
}

export function countsMarkdown(all: readonly ToolPackCounts[], ruleIds: (toolPack: string, rule: string) => string[]): string {
  const header: string[] = [
    "# Real-code samples: counts",
    "",
    "Every input of the sample sets and what became of it, per tool-pack, from `scripts/samples/build.ts`. A golden row is one `goldens.tsv` line (rule, task or corpus, file); a corpus finding is one finding of a `corpus-*.expected.json`. Outcomes:",
    "",
    "- **kept positive**: the sample carries the labelled violation.",
    "- **kept clean**: the sample carries `[]` for the rule, because the file has a labelled violation for some rule (all its labels are kept), it is a near miss (the rule's relaxed query selects a node in it; up to 5 per rule, smallest first), or it is the rule's smallest clean file when the rule has no near miss.",
    "- **dropped clean**: a clean label on a clean-only file the relevance rule did not keep (not relevant: no candidate node; over the cap: a near miss past the 5 smallest).",
    "- **not judged**: the recording has no answer for the rule on the file (upstream excluded it, or never answered), so the sample has no key for it.",
    "- **excluded**: a copyleft source (D1), or a finding with no scored file or pack rule to label.",
    "",
  ];
  const lines: string[] = [];
  const grand = new Map<string, number>();
  const grandFindings = new Map<string, number>();
  for (const counts of all) {
    const byKey = new Map(counts.deduplicated.map((sample) => [sampleKey(sample), sample]));
    const keptByKey = new Map(counts.kept.map((sample) => [sampleKey(sample), sample]));
    const merged = new Map([...byKey].map(([key, sample]) => [key, keptByKey.get(key) ?? sample]));
    const byPath = new Map(counts.deduplicated.map((sample) => [`${sample.source.repo}\0${sample.source.path}`, sample]));
    const golden = tally(counts.goldenRows.map((row) => goldenOutcome(counts, merged, row)));
    const findings = tally(counts.corpusFindings.map((finding) => corpusOutcome(counts, byPath, finding, ruleIds(counts.toolPack, finding.rule))));
    for (const [outcome, count] of golden) grand.set(outcome, (grand.get(outcome) ?? 0) + count);
    for (const [outcome, count] of findings) grandFindings.set(outcome, (grandFindings.get(outcome) ?? 0) + count);
    const goldenFiles = new Set(counts.goldenRows.map((row) => `${row.task}\0${row.path}\0${row.sha256}`)).size;
    const positive = counts.kept.filter(hasPositive).length;
    lines.push(
      `## ${counts.toolPack}`,
      "",
      `Archive: sha256 \`${counts.archive.sha256}\`, ${counts.archive.files} files, ${counts.archive.bytes} bytes (${counts.archive.compressed} compressed). ${counts.kept.length} samples kept (${positive} with a labelled violation or chosen, ${counts.kept.length - positive} clean-only), of ${counts.deduplicated.length} distinct files read from ${counts.candidates} source files (${counts.merged} merged as the same repository, path and sha256; ${counts.conflicts.length} rule label(s) where two sources disagreed, the earlier source kept).`,
      "",
    );
    if (counts.goldenRows.length > 0) lines.push(`### Golden rows (${counts.goldenRows.length} rows, ${goldenFiles} distinct files)`, "", ...table(golden, counts.goldenRows.length, "outcome"));
    if (counts.corpora.length > 0) {
      lines.push("### Corpus recordings", "", "| corpus | repository | commit | tool version | rules | scored files | excluded |", "|---|---|---|---|---:|---:|---|");
      for (const corpus of counts.corpora) lines.push(`| ${corpus.corpus} | ${corpus.repo} | ${corpus.commit.slice(0, 12)} | ${corpus.version} | ${corpus.rules} | ${corpus.files} | ${corpus.excluded ?? ""} |`);
      lines.push("", `Findings (${counts.corpusFindings.length}):`, "", ...table(findings, counts.corpusFindings.length, "outcome"));
    }
    const exclusions = tally([
      ...counts.goldenRows.flatMap((row) => (row.dropped === undefined ? [] : [`golden row, ${row.dropped.reason}: ${row.dropped.detail}`])),
      ...counts.corpusFindings.flatMap((finding) => (finding.dropped === undefined ? [] : [`corpus finding, ${finding.dropped.reason}: ${finding.dropped.reason === "licence" ? finding.dropped.detail : `${finding.corpus} ${finding.rule} ${finding.path}`}`])),
    ]);
    if (exclusions.size > 0) lines.push("### Exclusions", "", ...table(exclusions, [...exclusions.values()].reduce((sum, count) => sum + count, 0), "input, reason: what"));
    if (counts.testSet.originals + counts.testSet.fixed > 0) {
      const testSetKept = counts.kept.filter((sample) => sample.origins.includes("eslint test set")).length;
      lines.push("### ESLint test set", "", `${counts.testSet.originals} originals (labelled for their rule) and ${counts.testSet.fixed} fixed copies (clean for their rule), all kept: ${testSetKept} samples, since a file the set holds for several rules (the same repository, path and sha256) is one sample labelled for each.`, "");
    }
    if (counts.replacements > 0) lines.push("### Licence replacements", "", `${counts.replacements} hand-labelled file(s) (scripts/samples/replacements.ts).`, "");
    const sources = tally(counts.kept.map((sample) => `${sample.source.repo} (${sample.source.license})`));
    lines.push("### Samples by repository", "", ...table(sources, counts.kept.length, "repository (licence)"));
    if (counts.probes.size > 0) {
      lines.push("### Clean-only files per rule (D2)", "", "| rule | candidate probe | clean-only files | near misses | kept |", "|---|---|---:|---:|---:|");
      for (const [rule, probe] of [...counts.probes].sort(([a], [b]) => byText(a, b))) {
        const kept = counts.kept.filter((sample) => !hasPositive(sample) && !sample.pinned && sample.expected.has(rule)).length;
        lines.push(`| ${rule} | ${probe.probe} | ${probe.cleanOnly} | ${probe.nearMisses} | ${kept} |`);
      }
      lines.push("");
    }
    if (counts.conflicts.length > 0) {
      lines.push("### Label disagreements (earlier source kept)", "", ...table(tally(counts.conflicts.map((conflict) => `${conflict.rule}: ${conflict.kept} over ${conflict.dropped}`)), counts.conflicts.length, "rule: kept over dropped"));
    }
  }
  const totalRows = [...grand.values()].reduce((sum, count) => sum + count, 0);
  const totalFindings = [...grandFindings.values()].reduce((sum, count) => sum + count, 0);
  const sum = (pick: (counts: ToolPackCounts) => number): number => all.reduce((total, counts) => total + pick(counts), 0);
  const sorted = (counts: Map<string, number>): Map<string, number> => new Map([...counts].sort(([a], [b]) => byText(a, b)));
  const summary = [
    "## All tool-packs",
    "",
    `Golden rows (${totalRows}):`,
    "",
    ...table(sorted(grand), totalRows, "outcome"),
    `Corpus findings (${totalFindings}):`,
    "",
    ...table(sorted(grandFindings), totalFindings, "outcome"),
    "| tool-pack | golden rows | corpus findings | test-set originals | test-set fixed | samples kept | archive files | archive bytes |",
    "|---|---:|---:|---:|---:|---:|---:|---:|",
    ...all.map((counts) => `| ${counts.toolPack} | ${counts.goldenRows.length} | ${counts.corpusFindings.length} | ${counts.testSet.originals} | ${counts.testSet.fixed} | ${counts.kept.length} | ${counts.archive.files} | ${counts.archive.bytes} |`),
    `| **total** | **${sum((counts) => counts.goldenRows.length)}** | **${sum((counts) => counts.corpusFindings.length)}** | **${sum((counts) => counts.testSet.originals)}** | **${sum((counts) => counts.testSet.fixed)}** | **${sum((counts) => counts.kept.length)}** | **${sum((counts) => counts.archive.files)}** | **${sum((counts) => counts.archive.bytes)}** |`,
    "",
  ];
  return `${[...header, ...summary, ...lines].join("\n").trimEnd()}\n`;
}
