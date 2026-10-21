/**
 * passes-conformance: the pack contract's own check (`test:conformance`, conformance/languages/) as a gate, so the rule
 * gates are a superset of it and a candidate is never "ready to promote" while the pack's check would fail it once
 * shipped. The pack's fixtures tier runs once per tool-pack run, over every rule, shipped or candidate (each in its own
 * workspace, as the check runs them): upstream's fixtures and Rerail's probes, after the pack's declared divergences.
 * Every rule is held to what the check fails a shipped rule on (`reportFailures`). When the pack pins corpora, the
 * tool's corpus recordings must also cover the rule, as the check requires of every shipped rule.
 */
import { existsSync } from "node:fs";
import { join } from "node:path";

import { corpusRecordingPath, loadPack, loadRecording } from "../../languages/pack.js";
import type { RuleRow } from "../../languages/report.js";
import { runPack } from "../../languages/runner.js";
import type { Check } from "../types.js";

/** The pack's conformance check, as the gate reads it: one fixtures row per rule id, and what it says about the rest. */
export interface PackConformance {
  rows: Map<string, RuleRow>;
  /** The pack pins corpora, so its corpus recordings must cover every rule that ships. */
  pinsCorpora: boolean;
  /** Divergences of a rule that explain nothing, as `path:line (kind)`. */
  stale: Map<string, string[]>;
  /** Why the pack's corpus recordings do not cover a rule. */
  uncovered: Map<string, string[]>;
  /** The check's errors that name a rule (a document that does not compile, a recording that does not cover it). */
  ruleErrors: Map<string, string[]>;
  /** The check's errors that name no rule: they fail the whole run. */
  errors: string[];
}

const add = (map: Map<string, string[]>, key: string, value: string) => map.set(key, [...(map.get(key) ?? []), value]);

/** Run the pack's conformance check on its fixtures, as `test:conformance` does. */
export async function packConformance(root: string, pack: string): Promise<PackConformance> {
  const loaded = loadPack(join(root, "packs", pack));
  const report = await runPack(loaded, { fixtures: true, corpora: [], date: "port-readiness" });
  const ids = loaded.pack.rules.map((rule) => rule.id);
  const rows = new Map(report.rules.filter((row) => row.source === "fixtures").map((row) => [row.rule, row]));
  const stale = new Map<string, string[]>();
  for (const divergence of report.staleDivergences) add(stale, divergence.rule, `${divergence.path}:${divergence.line} (${divergence.kind})`);
  const ruleErrors = new Map<string, string[]>();
  const errors: string[] = [];
  for (const error of report.errors) {
    const id = ids.find((candidate) => error.startsWith(`${candidate} `) || error.startsWith(`${candidate}:`));
    if (id === undefined) errors.push(error);
    else add(ruleErrors, id, error);
  }
  const uncovered = new Map<string, string[]>();
  for (const corpus of loaded.pack.corpora) {
    for (const rule of loaded.pack.rules) {
      const path = corpusRecordingPath(loaded, rule.upstream.tool, corpus.name);
      if (!existsSync(path)) {
        add(uncovered, rule.id, `the pack pins corpus ${corpus.name}, and ${rule.upstream.tool} has no recording of it`);
        continue;
      }
      if (!loadRecording(path).source.rules.includes(rule.upstream.rule)) add(uncovered, rule.id, `the ${rule.upstream.tool} recording of corpus ${corpus.name} does not cover ${rule.upstream.rule}`);
    }
  }
  return { rows, pinsCorpora: loaded.pack.corpora.length > 0, stale, uncovered, ruleErrors, errors };
}

const point = (value: { path: string; line: number; column?: number | undefined } | undefined) => (value === undefined ? "" : `, first ${value.path}:${value.line}${value.column === undefined ? "" : `:${value.column}`}`);

/** The checks the pack's conformance check makes of one rule, as it makes them of a shipped rule. */
export function passesConformanceChecks(conformance: PackConformance, id: string): Check[] {
  const errors = conformance.ruleErrors.get(id) ?? [];
  const checks: Check[] = [{ id: "scored", ok: errors.length === 0, detail: errors.length === 0 ? "the pack's conformance check compiles and scores it" : errors.slice(0, 2).join("; ") }];
  const row = conformance.rows.get(id);
  if (row === undefined) {
    checks.push({ id: "fixtures-row", ok: false, detail: "the pack's conformance check scored no fixtures for it: add its conformance.json entry and fixtures recording" });
    return checks;
  }
  const fix = "fix the rule, or declare the divergence in the pack's divergences.json";
  checks.push(
    { id: "no-unexplained-false-positives", ok: row.fp === 0, detail: row.fp === 0 ? `no unexplained finding on the pack's fixtures (upstream's cases and Rerail's probes), with ${row.tp} true positive(s)` : `${row.fp} unexplained FP on the pack's fixtures${point(row.falsePositives[0])}: ${fix}` },
    { id: "no-unexplained-misses", ok: row.fn === 0, detail: row.fn === 0 ? "misses no finding upstream reports on the pack's fixtures" : `${row.fn} unexplained FN on the pack's fixtures${point(row.falseNegatives[0])}: ${fix}` },
    { id: "catches-something", ok: row.tp > 0 || row.examples, detail: row.tp > 0 ? `${row.tp} finding(s) agree with upstream on the fixtures` : row.examples ? "pack.json gives it examples" : "0 true positives on the fixtures and no examples in pack.json: nothing shows it catches anything" },
    { id: "engine-reads-fixtures", ok: row.engineFailures === 0, detail: row.engineFailures === 0 ? "the engine evaluates it on every fixture" : `${row.engineFailures} engine failure(s) on the fixtures` },
  );
  const stale = conformance.stale.get(id) ?? [];
  checks.push({ id: "divergences-current", ok: stale.length === 0, detail: stale.length === 0 ? "every declared divergence explains a disagreement" : `stale divergence(s) explain nothing: ${stale.slice(0, 3).join(", ")}` });
  if (conformance.pinsCorpora) {
    const uncovered = conformance.uncovered.get(id) ?? [];
    checks.push({ id: "corpus-recorded", ok: uncovered.length === 0, detail: uncovered.length === 0 ? "the pack's corpus recordings cover it" : `${uncovered[0]}: record the corpus again with it` });
  }
  return checks;
}
