/**
 * One-shot (deleted in the same change): seed every gates.yaml's `upstream.lints` and `rules` from the tool-pack's
 * current recording, the archive its recording.lock names, read from a copy of the archive cache.
 *
 *   node --import tsx scripts/seed-gates-facts.ts <cache root holding recordings/<sha256>.tar.zst>
 */
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { fileTypeOf } from "../conformance/port-readiness/engine.js";
import { gatedToolPacks } from "../conformance/port-readiness/run.js";
import { parseGatesManifest } from "../conformance/port-readiness/manifest.js";
import { cachedTree, directoryTree, RECORDING_ARCHIVE, treeConfigs } from "../conformance/port-readiness/recording/archive.js";
import { readRecordingLock } from "../conformance/port-readiness/recording/store.js";
import { loadRecording } from "../conformance/port-readiness/recording/tree.js";

const root = process.cwd();
const cacheRoot = process.argv[2];
if (cacheRoot === undefined) throw new Error("usage: seed-gates-facts.ts <cache root>");

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);
const quote = (text: string): string => JSON.stringify(text);
for (const { pack, tool } of gatedToolPacks(root)) {
  if (/^ {2}lints:/mu.test(readFileSync(join(root, "packs", pack, "upstream", tool, "gates.yaml"), "utf8"))) continue;
  const label = `packs/${pack}/upstream/${tool}/${RECORDING_ARCHIVE}`;
  const lock = readRecordingLock(root, pack, tool);
  const directory = cachedTree(cacheRoot, lock.files[RECORDING_ARCHIVE], label);
  if (directory === undefined) throw new Error(`${label}: not in ${cacheRoot}`);
  const tree = directoryTree(directory);
  const configs = treeConfigs(tree);
  if (configs.length !== 1) throw new Error(`${label}: ${configs.length} configs`);
  const recording = loadRecording(tree, configs[0]!, label);

  const lints = [...new Set(recording.inScopeSamples.map(fileTypeOf))].sort(byText);
  const names = Object.keys(recording.rules).sort(byText);
  const rules = names.flatMap((name) => {
    const { deprecation, recommended } = recording.rules[name]!.facts;
    return [
      `  ${quote(name)}:`,
      `    deprecated: ${deprecation === null ? "null" : quote(deprecation)}`,
      `    recommended: ${recommended === null ? "null" : `{ config: ${quote(recommended.config)}, options: ${JSON.stringify(recommended.options)} }`}`,
    ];
  });

  const path = join(root, "packs", pack, "upstream", tool, "gates.yaml");
  const lines = readFileSync(path, "utf8").split("\n");
  const popularity = lines.findIndex((line) => line.startsWith("popularity:"));
  // Back over the top-level comments and blank lines before `popularity:`, so `lints` lands in the upstream block.
  let at = popularity;
  while (at > 0 && (lines[at - 1]!.startsWith("#") || lines[at - 1]!.trim() === "")) at -= 1;
  lines.splice(at, 0,
    "  # The file types upstream lints with its approved configuration at the pins (seeded from the recording's in-scope samples).",
    `  lints: [${lints.map(quote).join(", ")}]`);
  const gaps = lines.findIndex((line) => line.startsWith("known_gaps:"));
  lines.splice(gaps, 0,
    "# Every rule of the pinned upstream the pack ports or may port, with upstream's deprecation and the first approved",
    "# configuration that turns it on (seeded from the recording; reviewed at each bump).",
    ...(rules.length === 0 ? ["rules: {}"] : ["rules:", ...rules]));
  const text = lines.join("\n");
  const manifest = parseGatesManifest(text, path);
  writeFileSync(path, text);
  console.log(`${pack}/${tool}: lints ${manifest.upstream.lints.join(" ")}; ${names.length} rule(s); absent ${recording.absent.length}`);
}
