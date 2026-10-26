/**
 * Builds every tool-pack's real-code sample set (format.ts) from what the port-readiness and conformance tiers pinned:
 * the goldens and their recorded answers (build-goldens.ts), the corpus recordings (convert-corpora.ts), the ESLint
 * test set (convert-eslint-test-set.ts) and the licence replacements (replacements.ts). Copyleft sources are left out
 * (licences.ts), files are deduplicated by repository, path and sha256, clean-only files are kept only where relevant
 * (relevance.ts), and each set is packed into its canonical archive with its samples.lock and SAMPLES.md (pack.ts).
 * It uploads nothing: `node scripts/s3/pack-fixtures.mjs samples push` does, from the cache this writes.
 *
 *   npm run samples:build -- --eslint-test-set <extracted test set directory> --report <counts.md>
 *     [--cache-root <port-readiness cache>] [--pack <pack>]
 *
 * Every input is pinned (goldens.tsv, recording.lock, corpus pins, corpus-*.expected.json, the test set's zip, the
 * replacements' sha256), and fetched once into the caches, so a second run writes byte-identical archives.
 */
import { existsSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

import { DEFAULT_CACHE_ROOT } from "../../conformance/port-readiness/corpus.js";
import { ProcessCommandRunner } from "../../conformance/port-readiness/process.js";
import { PACKAGE_ROOT } from "../../src/engine/index.js";
import { goldenSamples, type GoldenSamples } from "./build-goldens.js";
import { corpusSamples } from "./convert-corpora.js";
import { eslintTestSetSamples } from "./convert-eslint-test-set.js";
import { packRuleIds, readConformance, type PackConformance } from "./corpora.js";
import { countsMarkdown, type ToolPackCounts } from "./counts.js";
import { licenceTexts, licensesMarkdown, samplesMarkdown, type LicenceText } from "./licences.js";
import { packSamples, writeSampleSet } from "./pack.js";
import { nearMissCandidates, selectRelevant } from "./relevance.js";
import { replacementSamples } from "./replacements.js";
import { archiveEntries, byText, dedupe, hasPositive, toolPackName, type Sample, type ToolPack } from "./samples.js";

const USAGE = "usage: npm run samples:build -- --eslint-test-set <dir> --report <file> [--cache-root <dir>] [--pack <pack>]";

/** Every tool-pack that has samples from any source, sorted. */
function toolPacksOf(root: string, packs: readonly string[], extra: readonly Sample[]): ToolPack[] {
  const found = new Map<string, ToolPack>();
  for (const pack of packs) {
    const upstream = join(root, "packs", pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const entry of readdirSync(upstream, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const files = readdirSync(join(upstream, entry.name));
      if (files.includes("goldens.tsv") || files.some((name) => /^corpus-.+\.expected\.json$/u.test(name))) found.set(`${pack}/${entry.name}`, { pack, tool: entry.name });
    }
  }
  for (const sample of extra) if (packs.includes(sample.toolPack.pack)) found.set(toolPackName(sample.toolPack), sample.toolPack);
  return [...found.values()].sort((a, b) => byText(toolPackName(a), toolPackName(b)));
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { "eslint-test-set": { type: "string" }, report: { type: "string" }, "cache-root": { type: "string" }, pack: { type: "string" } } });
  if (values["eslint-test-set"] === undefined || values.report === undefined) throw new Error(USAGE);
  const root = PACKAGE_ROOT;
  const cacheRoot = resolve(values["cache-root"] ?? DEFAULT_CACHE_ROOT);
  const samplesCache = join(root, ".rerail", "samples");
  const runner = new ProcessCommandRunner();

  const allPacks = readdirSync(join(root, "packs"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(root, "packs", entry.name, "conformance.json")))
    .map((entry) => entry.name)
    .sort(byText);
  const conformances = new Map<string, PackConformance>();
  for (const pack of allPacks) conformances.set(pack, await readConformance(root, pack));
  const packs = values.pack === undefined ? allPacks : [values.pack];

  const licences = new Map<string, { spdx: string; texts: LicenceText[] }>();
  const licence = (repo: string, commit: string): string => {
    const key = `${repo}@${commit}`;
    if (!licences.has(key)) licences.set(key, licenceTexts(samplesCache, repo, commit));
    return licences.get(key)!.spdx;
  };

  const testSet = eslintTestSetSamples(resolve(values["eslint-test-set"]), conformances);
  const replacements = replacementSamples({ cacheRoot: samplesCache, conformances, licence });
  const counts: ToolPackCounts[] = [];
  for (const toolPack of toolPacksOf(root, packs, [...testSet.samples, ...replacements])) {
    const name = toolPackName(toolPack);
    const conformance = conformances.get(toolPack.pack)!;
    process.stderr.write(`${name}: reading\n`);
    // The corpus recordings first: their checkouts widen the pinned corpora the goldens' context files are read from.
    const corpus = await corpusSamples({ root, cacheRoot, toolPack, conformance, runner, licence });
    const goldens: GoldenSamples | undefined = existsSync(join(root, "packs", toolPack.pack, "upstream", toolPack.tool, "goldens.tsv"))
      ? await goldenSamples({ root, cacheRoot, toolPack, conformance, runner, licence })
      : undefined;
    const mine = (samples: readonly Sample[]) => samples.filter((sample) => toolPackName(sample.toolPack) === name);
    // The order decides a disagreement: a golden's recorded answer, then a hand label, then a corpus recording.
    const candidates = [...(goldens?.samples ?? []), ...mine(replacements), ...corpus.samples, ...mine(testSet.samples)];
    const deduplicated = dedupe(candidates);
    const cleanOnly = deduplicated.samples.filter((sample) => !sample.pinned && !hasPositive(sample));
    const nearMisses = new Map<string, Set<string>>();
    const probes = new Map<string, { probe: string; cleanOnly: number; nearMisses: number }>();
    for (const rule of [...new Set(cleanOnly.flatMap((sample) => [...sample.expected.keys()]))].sort(byText)) {
      const policy = conformance.rules.find((entry) => entry.id === rule)!.policy;
      const judged = cleanOnly.filter((sample) => sample.expected.has(rule));
      const { selected, probe } = await nearMissCandidates(join(root, "packs", toolPack.pack, policy), judged);
      nearMisses.set(rule, selected);
      probes.set(rule, { probe, cleanOnly: judged.length, nearMisses: selected.size });
    }
    const selection = selectRelevant(deduplicated.samples, nearMisses);
    const licenses = licensesMarkdown(selection.samples, (repo, commit) => licences.get(`${repo}@${commit}`)?.texts);
    const packed = packSamples(archiveEntries(selection.samples, licenses));
    writeSampleSet(root, toolPack, packed, samplesMarkdown(name, packed, selection.samples));
    process.stderr.write(`${name}: ${selection.samples.length} samples, archive ${packed.sha256} (${packed.files} files, ${packed.bytes} bytes)\n`);
    counts.push({
      toolPack: name,
      goldenRows: goldens?.rows ?? [],
      corpusFindings: corpus.findings,
      corpora: corpus.corpora,
      testSet: { originals: testSet.originals.get(name) ?? 0, fixed: testSet.fixed.get(name) ?? 0 },
      replacements: mine(replacements).length,
      candidates: candidates.length,
      deduplicated: deduplicated.samples,
      kept: selection.samples,
      merged: deduplicated.merged,
      conflicts: deduplicated.conflicts,
      clean: selection.clean,
      probes,
      archive: { sha256: packed.sha256, files: packed.files, bytes: packed.bytes, compressed: packed.compressed.length },
    });
  }
  writeFileSync(resolve(values.report), countsMarkdown(counts, (name, rule) => {
    const [pack, tool] = name.split("/") as [string, string];
    return packRuleIds(conformances.get(pack)!, tool, rule);
  }));
}

main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? (error.stack ?? error.message) : String(error)}\n`);
  process.exitCode = 1;
});
