/**
 * `npm run gates:record`: run a tool-pack's live adapter and write the recording the gates read
 * (packs/<pack>/upstream/<tool>/gates.json, gates-corpus.json and missing case sources). `--check` writes nothing and
 * exits 1 when a fresh recording differs from the committed one.
 */
import { resolve } from "node:path";
import { parseArgs } from "node:util";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { DEFAULT_CACHE_ROOT } from "../../gates/corpus.js";
import { toolPackDirectory } from "../../gates/manifest.js";
import { ProcessCommandRunner } from "../../gates/process.js";
import { diffRecording, recordToolPack, writeRecording } from "./record.js";

const USAGE = `usage:
  npm run gates:record -- --pack <id> --tool <tool> [--cache dir] [--check]
`;

async function main(): Promise<number> {
  const { values } = parseArgs({
    options: { pack: { type: "string" }, tool: { type: "string" }, cache: { type: "string" }, check: { type: "boolean", default: false }, help: { type: "boolean", default: false } },
    strict: true,
  });
  if (values.help === true || values.pack === undefined || values.tool === undefined) {
    process.stderr.write(USAGE);
    return values.help === true ? 0 : 2;
  }
  const root = PACKAGE_ROOT;
  const started = Date.now();
  const recorded = await recordToolPack({
    root,
    pack: values.pack,
    tool: values.tool,
    cacheRoot: resolve(values.cache ?? DEFAULT_CACHE_ROOT),
    runner: new ProcessCommandRunner(),
  });
  const label = `packs/${values.pack}/upstream/${values.tool}`;
  const cases = Object.values(recorded.recording.rules).flatMap((rule) => rule.cases);
  const summary = `${Object.keys(recorded.recording.rules).length} rules (${recorded.recording.absent.length} absent upstream), ${cases.length} cases `
    + `(${cases.filter((testCase) => testCase.skipped === "not-copied").length} not copied), ${recorded.sources.length} case sources, `
    + `${recorded.corpus.files.length} corpus files, ${((Date.now() - started) / 1000).toFixed(0)} s`;
  if (values.check) {
    const diff = diffRecording(toolPackDirectory(root, values.pack, values.tool), recorded);
    const problems = [
      ...diff.changed.map((file) => `${label}/${file} differs from a fresh recording`),
      ...diff.missing.slice(0, 10).map((copy) => `${label}/${copy.file} is missing`),
      ...(diff.missing.length > 10 ? [`... and ${diff.missing.length - 10} more missing case sources`] : []),
      ...diff.conflicting.map((file) => `${label}/${file} differs from upstream's copy`),
    ];
    if (problems.length === 0) {
      process.stdout.write(`${label}: the recording is current (${summary})\n`);
      return 0;
    }
    process.stdout.write(`${label}: the recording is stale; run npm run gates:record -- --pack ${values.pack} --tool ${values.tool}\n${problems.map((problem) => `  ${problem}`).join("\n")}\n`);
    return 1;
  }
  const diff = await writeRecording(root, values.pack, values.tool, recorded);
  process.stdout.write(`${label}: recorded ${summary}; copied ${diff.missing.length} case sources\n`);
  return 0;
}

main().then((code) => {
  process.exitCode = code;
}, (error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exitCode = 2;
});
