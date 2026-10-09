// Records the Python pack's source facts, upstream/<tool>/source.json, the facts checks 7 to 9 read about Ruff and
// Bandit (`upstreamSourceSchema`, rerail/conformance/languages/pack.ts). Run offline by a maintainer, never in CI:
//
//   node --import tsx conformance/recorders/python-source.ts ruff <Ruff checkout> <ruff executable>
//   node --import tsx conformance/recorders/python-source.ts bandit <Bandit checkout> <python of Bandit's environment>
//
// The checkout is upstream's repository at the commit the tool's fixtures recording was copied from (its `origin`),
// with an unmodified working tree and the release tag fetched; for Ruff, at least `RUFF_SPARSE_PATHS`. The runtime is
// the version conformance.json pins. `python3.14` on the PATH reads Python (python_toolkit.py). The facts are recorded
// under conformance.json's `tools[].recommended`.
import { resolve } from "node:path";
import { fixturesRecordingPath, loadPack, loadRecording, packDirectory, upstreamSourcePath } from "../languages/pack.js";
import { banditSource } from "./bandit-source.js";
import { createPythonToolkit } from "./python-toolkit.js";
import { ruffSource } from "./ruff-source.js";
import { assertPinnedCheckout, parseOrigin, writeUpstreamSource } from "./upstream-source.js";

/** The source tree each tool's runtime is published from; both tag releases with the bare version. */
const RELEASE_PATHS: Record<string, string> = { ruff: "crates", bandit: "bandit" };

function main(): void {
  const [tool, checkoutArgument, runtime] = process.argv.slice(2);
  if (tool === undefined || !(tool in RELEASE_PATHS) || checkoutArgument === undefined || runtime === undefined) {
    throw new Error("usage: python-source.ts ruff|bandit <checkout> <runtime>");
  }
  const checkout = resolve(checkoutArgument);
  const loaded = loadPack(packDirectory("python"));
  const declared = loaded.pack.tools.find(entry => entry.name === tool);
  if (declared?.recommended === undefined) throw new Error(`packs/python/conformance.json declares no recommended configurations for ${tool}`);
  const recording = loadRecording(fixturesRecordingPath(loaded, tool));
  if (recording.source.kind !== "fixtures") throw new Error(`${tool}'s fixtures recording has no origin`);
  const pin = { ...parseOrigin(recording.source.origin), tag: declared.version, releasePath: RELEASE_PATHS[tool]!, version: declared.version };
  assertPinnedCheckout(checkout, pin);
  const ported = loaded.pack.rules.filter(rule => rule.upstream.tool === tool).map(rule => rule.upstream.rule);
  const toolkit = createPythonToolkit("python3.14");
  // A bare command name is looked up on the PATH; a path is made absolute, since the tools run from other directories.
  const executable = runtime.includes("/") ? resolve(runtime) : runtime;
  const recorded = recording.source.files.flat();
  const source = tool === "ruff"
    ? ruffSource(checkout, executable, toolkit, ported, pin, declared.recommended, recorded)
    : banditSource(checkout, executable, toolkit, ported, pin, declared.recommended, recorded);
  writeUpstreamSource(upstreamSourcePath(loaded, tool), source);
}

main();
