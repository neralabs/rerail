/**
 * Check 7 (provenance): a port is of a rule its upstream tool has, implements and tests at the pinned commit, does
 * not deprecate, and turns on in an approved configuration; and the tool's recorded source facts are of the pinned
 * tool. The rule-level checks are the rule validation ladder's V1 Gate 1 rule checks (nera-rule-validation
 * `src/validation/gates/provenance.ts`), read from `upstream/<tool>/source.json`. V1's pack-level licence and notice
 * checks are not here: check 6 is the licence check.
 */
import { existsSync } from "node:fs";
import { relative, sep } from "node:path";
import {
  corpusRecordingPath,
  fixturesRecordingPath,
  loadRecording,
  loadUpstreamSource,
  upstreamSourcePath,
  type LoadedPack,
  type Recording,
  type UpstreamSource,
} from "../languages/pack.js";
import type { GateCheck } from "./check.js";

/** A conformance.json tool that declares its approved configurations, and so its source facts. */
export interface DeclaredTool {
  name: string;
  version: string;
  recommended: readonly string[];
}

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** `provenance:upstream-rule`, `provenance:stable` and `provenance:recommended` for a port of upstream rule `rule`. */
export function ruleProvenanceChecks(rule: string, source: UpstreamSource): GateCheck[] {
  const facts = source.rules[rule];
  if (facts === undefined) {
    const missing = `${rule} is not a rule source.json records for ${source.tool} ${source.version}`;
    return [
      { id: "provenance:upstream-rule", ok: false, detail: missing },
      { id: "provenance:stable", ok: false, detail: `not run: ${missing}` },
      { id: "provenance:recommended", ok: false, detail: `not run: ${missing}` },
    ];
  }
  const enabled = facts.enabledBy;
  return [
    {
      id: "provenance:upstream-rule",
      ok: facts.implementation.length > 0 && facts.tests.length > 0,
      detail: `implementation: ${facts.implementation.join(", ") || "(none)"}; ${facts.tests.length} upstream test file(s)`,
    },
    {
      id: "provenance:stable",
      ok: facts.deprecated === null,
      detail: facts.deprecated === null ? "upstream metadata carries no deprecation" : `upstream deprecates the rule: ${facts.deprecated}`,
    },
    {
      id: "provenance:recommended",
      ok: enabled !== null,
      detail:
        enabled === null
          ? "not on in any approved upstream configuration"
          : enabled.options.length === 0
            ? `on in ${enabled.configuration} with default options`
            : `on in ${enabled.configuration} with options ${JSON.stringify(enabled.options)}`,
    },
  ];
}

/**
 * `provenance:pins` for one tool that declares source facts: its `source.json` parses and is of the tool conformance.json
 * pins (its directory, version and approved configurations), and of what its recordings were recorded from (each
 * recording's version, and the fixtures recording's `<repository>@<commit>` origin). `source` is the facts whenever
 * source.json is valid, even when a pin differs; the failures are never scrubbed.
 */
export function toolPins(loaded: LoadedPack, tool: DeclaredTool): { source: UpstreamSource | undefined; failures: string[] } {
  const at = (path: string): string => relative(loaded.directory, path).split(sep).join("/");
  const sourcePath = upstreamSourcePath(loaded, tool.name);
  if (!existsSync(sourcePath)) return { source: undefined, failures: [`${tool.name}: ${at(sourcePath)} is missing; the tool declares recommended configurations`] };
  let source: UpstreamSource;
  try {
    source = loadUpstreamSource(sourcePath);
  } catch (error) {
    return { source: undefined, failures: [`${tool.name}: ${message(error).replace(sourcePath, at(sourcePath))}`] };
  }
  const failures: string[] = [];
  const fail = (text: string): void => {
    failures.push(`${tool.name}: ${text}`);
  };
  if (source.tool !== tool.name) fail(`${at(sourcePath)} records tool ${source.tool}, not the tool its directory names`);
  if (source.version !== tool.version) fail(`${at(sourcePath)} records version ${source.version}; conformance.json pins ${tool.version}`);
  if (source.configurations.join("\0") !== tool.recommended.join("\0"))
    fail(`${at(sourcePath)} records configurations ${JSON.stringify(source.configurations)}; conformance.json recommends ${JSON.stringify(tool.recommended)}`);

  const origin = `${source.repository}@${source.commit}`;
  const fixtures = fixturesRecordingPath(loaded, tool.name);
  const recordings = [fixtures, ...loaded.pack.corpora.map(corpus => corpusRecordingPath(loaded, tool.name, corpus.name))];
  if (!existsSync(fixtures)) fail(`${at(fixtures)} is missing, so nothing pins source.json's commit`);
  for (const path of recordings.filter(existsSync)) {
    let recording: Recording;
    try {
      recording = loadRecording(path);
    } catch (error) {
      fail(`not run: ${message(error).replace(path, at(path))}`);
      continue;
    }
    if (recording.version !== source.version) fail(`${at(path)} was recorded with version ${recording.version}; source.json records ${source.version}`);
    if (recording.source.kind === "fixtures" && recording.source.origin !== origin) fail(`${at(path)} was copied from ${recording.source.origin}; source.json records ${origin}`);
  }
  return { source, failures };
}
