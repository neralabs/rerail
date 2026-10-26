/**
 * The ESLint test set as samples: each original file is labelled for its rule with ESLint's findings, used as they
 * are; each fixed copy is a clean sample (`[]`) for that rule. A rule belongs to the tool-pack whose pack rule ports
 * it (`conformance.json`), and every pack rule of that tool porting the same upstream rule gets the labels (D4).
 */
import { readFileSync } from "node:fs";
import { join, posix } from "node:path";

import { z } from "zod";

import { packRuleIds, type PackConformance } from "./corpora.js";
import type { SampleLabel } from "./format.js";
import { sha256, toolPackName, type Sample, type ToolPack } from "./samples.js";

const manifestSchema = z.looseObject({
  tools: z.record(z.string(), z.string()),
  entries: z.array(z.looseObject({ file: z.string(), fixed: z.string().nullable(), rule: z.string(), rerailRule: z.string(), pack: z.string() })),
});

const originalSchema = z.looseObject({
  rule: z.string(),
  rerailRule: z.string(),
  source: z.looseObject({ repo: z.string(), sha: z.string(), path: z.string(), license: z.string() }),
  eslint: z.looseObject({ findings: z.array(z.looseObject({ line: z.number().int().positive(), column: z.number().int().positive() })) }),
});

const fixedSchema = z.looseObject({ fixedOf: z.string(), rule: z.string(), eslint: z.looseObject({ findings: z.array(z.unknown()) }) });

/** The package whose version labelled a tool-pack's rules, by tool: the core rules are ESLint's, the rest a plugin's. */
const LABELLING_PACKAGE: Readonly<Record<string, string>> = {
  eslint: "eslint",
  "eslint-plugin-unicorn": "eslint-plugin-unicorn",
  "typescript-eslint": "@typescript-eslint/eslint-plugin",
  "eslint-plugin-vitest": "@vitest/eslint-plugin",
};

export interface TestSetSamples {
  samples: Sample[];
  originals: Map<string, number>;
  fixed: Map<string, number>;
}

/** The samples of the test set extracted at `directory` (its `manifest.json` and per-rule directories), by tool-pack. */
export function eslintTestSetSamples(directory: string, conformances: ReadonlyMap<string, PackConformance>): TestSetSamples {
  const manifest = manifestSchema.parse(JSON.parse(readFileSync(join(directory, "manifest.json"), "utf8")));
  const result: TestSetSamples = { samples: [], originals: new Map(), fixed: new Map() };
  const count = (counts: Map<string, number>, toolPack: ToolPack): void => void counts.set(toolPackName(toolPack), (counts.get(toolPackName(toolPack)) ?? 0) + 1);
  for (const entry of manifest.entries) {
    const conformance = conformances.get(entry.pack);
    const ported = conformance?.rules.find((rule) => rule.id === entry.rerailRule);
    if (conformance === undefined || ported === undefined || ported.upstream.rule !== entry.rule) {
      throw new Error(`${entry.file}: ${entry.rerailRule} is not a rule of pack ${entry.pack} porting ${entry.rule}`);
    }
    const toolPack = { pack: entry.pack, tool: ported.upstream.tool };
    const ids = packRuleIds(conformance, toolPack.tool, entry.rule);
    const original = originalSchema.parse(JSON.parse(readFileSync(join(directory, `${entry.file}.eslint.json`), "utf8")));
    const packageName = LABELLING_PACKAGE[toolPack.tool];
    const version = packageName === undefined ? undefined : manifest.tools[packageName];
    if (version === undefined) throw new Error(`${entry.file}: no labelling tool version for ${toolPackName(toolPack)}`);
    const labelledBy = { tool: toolPack.tool, version, how: "eslint test set" as const };
    const short = original.source.repo.split("/")[1]!;
    const source = { repo: original.source.repo, commit: original.source.sha, path: original.source.path, license: original.source.license };
    const bytes = readFileSync(join(directory, entry.file));
    const labels: SampleLabel[] = original.eslint.findings.map((finding) => ({ line: finding.line, column: finding.column }));
    result.samples.push({
      toolPack,
      short,
      source: { ...source, sha256: sha256(bytes) },
      bytes,
      labelledBy,
      expected: new Map(ids.map((id) => [id, labels])),
      context: [],
      pinned: true,
      origins: ["eslint test set"],
    });
    count(result.originals, toolPack);
    if (entry.fixed === null) continue;
    const fixed = fixedSchema.parse(JSON.parse(readFileSync(join(directory, `${entry.fixed}.eslint.json`), "utf8")));
    if (fixed.fixedOf !== entry.file || fixed.rule !== entry.rule || fixed.eslint.findings.length !== 0) throw new Error(`${entry.fixed} is not a clean fixed copy of ${entry.file}`);
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
      origins: ["eslint test set"],
    });
    count(result.fixed, toolPack);
  }
  return result;
}

/**
 * Where a fixed copy sits in its repository: the original's path, unless the fix renamed the file (filename-case's
 * fix is the rename). The set names an original `<repo>-<name>-<sha12><extension>` and its fixed copy
 * `<repo>-<new name>-<sha12>-fixed<extension>`; where the names differ, the part that differs is replaced in the
 * original's file name.
 */
export function fixedCopyPath(path: string, originalFile: string, fixedFile: string): string {
  const original = posix.basename(originalFile);
  const fixed = posix.basename(fixedFile).replace(/(-[0-9a-f]{12})-fixed/u, "$1");
  if (fixed === original) return path;
  let prefix = 0;
  while (prefix < Math.min(original.length, fixed.length) && original[prefix] === fixed[prefix]) prefix += 1;
  let suffix = 0;
  while (suffix < Math.min(original.length, fixed.length) - prefix && original.at(-1 - suffix) === fixed.at(-1 - suffix)) suffix += 1;
  const before = original.slice(prefix, original.length - suffix);
  const after = fixed.slice(prefix, fixed.length - suffix);
  const name = posix.basename(path);
  if (before === "" || name.split(before).length !== 2) throw new Error(`${fixedFile}: cannot tell its file name from ${path}`);
  return posix.join(posix.dirname(path), name.replace(before, after));
}
