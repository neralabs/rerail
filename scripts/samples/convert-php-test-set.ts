/**
 * The PHP test set as samples: real `.php` files from open-source history, each at the commit that introduced a
 * violation of one PHP_CodeSniffer sniff, PHPStan rule or Psalm issue the php pack ports, labelled with that tool's
 * findings for the rule at the pack's pins (PHP_CodeSniffer 4.0.4 with the pack's `upstream/phpcs/ruleset.xml`,
 * PHPStan 2.2.16 at level max filtered as the pack's generator filters, Psalm 6.19.1 as its generator configures it,
 * all on PHP 8.5.11); each hand-fixed copy is a clean sample (`[]`) for that rule; a file whose own suppression
 * comment silences the rule carries the tool's findings as they are (none, or the unsuppressed ones). As the ESLint
 * test set's converter does (`convert-eslint-test-set.ts`), a rule belongs to the tool-pack whose pack rule ports it
 * (`conformance.json`), and every pack rule of that tool porting the same upstream rule gets the labels (D4).
 *
 * The set's layout is the ESLint test set's with the labelling tool in the sidecar's name: `manifest.json` (`tools`:
 * Composer package → version; `entries`), one `<file>.<tool>.json` beside each original (`source`, the tool's
 * `findings`) and each fixed copy (`fixedOf`, no findings).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { z } from "zod";

import { packRuleIds, type PackConformance } from "./conformance.js";
import { fixedCopyPath, type TestSetSamples } from "./convert-eslint-test-set.js";
import type { SampleLabel } from "./format.js";
import { sha256, toolPackName, type Sample } from "./samples.js";

const manifestSchema = z.looseObject({
  tools: z.record(z.string(), z.string()),
  entries: z.array(z.looseObject({ file: z.string(), fixed: z.string().nullable(), rule: z.string(), rerailRule: z.string(), pack: z.string(), tool: z.string() })),
});

const findingSchema = z.looseObject({ line: z.number().int().positive(), column: z.number().int().positive().optional() });

const originalSchema = z.looseObject({
  rule: z.string(),
  rerailRule: z.string(),
  source: z.looseObject({ repo: z.string(), sha: z.string(), path: z.string(), license: z.string() }),
  findings: z.array(findingSchema),
});

const fixedSchema = z.looseObject({ fixedOf: z.string(), rule: z.string(), findings: z.array(z.unknown()) });

/** The Composer package whose version labelled a tool-pack's rules, by tool. */
export const LABELLING_PACKAGE: Readonly<Record<string, string>> = {
  phpcs: "squizlabs/php_codesniffer",
  phpstan: "phpstan/phpstan",
  psalm: "vimeo/psalm",
};

/** The samples of the PHP test set extracted at `directory` (its `manifest.json` and per-rule directories), by tool-pack. */
export function phpTestSetSamples(directory: string, conformances: ReadonlyMap<string, PackConformance>): TestSetSamples {
  const manifest = manifestSchema.parse(JSON.parse(readFileSync(join(directory, "manifest.json"), "utf8")));
  const result: TestSetSamples = { samples: [], originals: new Map(), fixed: new Map() };
  const count = (counts: Map<string, number>, toolPack: { pack: string; tool: string }): void =>
    void counts.set(toolPackName(toolPack), (counts.get(toolPackName(toolPack)) ?? 0) + 1);
  for (const entry of manifest.entries) {
    const conformance = conformances.get(entry.pack);
    const ported = conformance?.rules.find((rule) => rule.id === entry.rerailRule);
    if (conformance === undefined || ported === undefined || ported.upstream.rule !== entry.rule || ported.upstream.tool !== entry.tool) {
      throw new Error(`${entry.file}: ${entry.rerailRule} is not a rule of pack ${entry.pack} porting ${entry.tool} ${entry.rule}`);
    }
    const toolPack = { pack: entry.pack, tool: entry.tool };
    const ids = packRuleIds(conformance, toolPack.tool, entry.rule);
    const original = originalSchema.parse(JSON.parse(readFileSync(join(directory, `${entry.file}.${entry.tool}.json`), "utf8")));
    const packageName = LABELLING_PACKAGE[entry.tool];
    const version = packageName === undefined ? undefined : manifest.tools[packageName];
    if (version === undefined) throw new Error(`${entry.file}: no labelling tool version for ${toolPackName(toolPack)}`);
    const labelledBy = { tool: entry.tool, version, how: "php test set" as const };
    const short = original.source.repo.split("/")[1]!;
    const source = { repo: original.source.repo, commit: original.source.sha, path: original.source.path, license: original.source.license };
    const bytes = readFileSync(join(directory, entry.file));
    const labels: SampleLabel[] = original.findings.map((finding) => (finding.column === undefined ? { line: finding.line } : { line: finding.line, column: finding.column }));
    result.samples.push({
      toolPack,
      short,
      source: { ...source, sha256: sha256(bytes) },
      bytes,
      labelledBy,
      expected: new Map(ids.map((id) => [id, labels])),
      context: [],
      pinned: true,
      origins: ["php test set"],
    });
    count(result.originals, toolPack);
    if (entry.fixed === null) continue;
    const fixed = fixedSchema.parse(JSON.parse(readFileSync(join(directory, `${entry.fixed}.${entry.tool}.json`), "utf8")));
    if (fixed.fixedOf !== entry.file || fixed.rule !== entry.rule || fixed.findings.length !== 0) throw new Error(`${entry.fixed} is not a clean fixed copy of ${entry.file}`);
    const fixedBytes = readFileSync(join(directory, entry.fixed));
    result.samples.push({
      toolPack,
      short: `${short}-fixed`,
      source: { ...source, path: fixedCopyPath(source.path, entry.file, entry.fixed), sha256: sha256(fixedBytes) },
      bytes: fixedBytes,
      labelledBy,
      expected: new Map(ids.map((id) => [id, []])),
      context: [],
      pinned: true,
      origins: ["php test set"],
    });
    count(result.fixed, toolPack);
  }
  return result;
}

/** The samples of one tool-pack, in the order given. */
export function samplesOf(samples: readonly Sample[], toolPack: { pack: string; tool: string }): Sample[] {
  return samples.filter((sample) => toolPackName(sample.toolPack) === toolPackName(toolPack));
}
