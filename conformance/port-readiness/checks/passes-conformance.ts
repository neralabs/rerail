/**
 * passes-conformance: the pack contract's own check (`test:conformance`, conformance/languages/) as a gate, so the rule
 * gates are a superset of it and a candidate is never "ready to promote" while the pack's check would fail it once
 * shipped. The pack's check runs once per tool-pack run, over every rule, shipped or candidate (each in its own
 * workspace, as the check runs them): upstream's fixtures and Rerail's probes, and the pack's real-code samples, after
 * the pack's declared divergences. Every rule is held to what the check fails a shipped rule on (`reportFailures`),
 * and must show it stays silent somewhere it should.
 */
import { join } from "node:path";

import { loadPack, type LoadedPack } from "../../languages/pack.js";
import type { RuleRow, T4Report } from "../../languages/report.js";
import { runPack, SAMPLES_SOURCE } from "../../languages/runner.js";
import type { Check } from "../types.js";

/** The pack's conformance check, as the gate reads it: one fixtures row per rule id, and what it says about the rest. */
export interface PackConformance {
  rows: Map<string, RuleRow>;
  /** One samples row per rule id some sample is labelled for (none for a rule no sample judges). */
  sampleRows: Map<string, RuleRow>;
  /** Divergences of a rule that explain nothing, as `path:line (kind)`. */
  stale: Map<string, string[]>;
  /**
   * The check's errors that name a rule (a document that does not compile, a recording that does not cover it, its
   * tool-pack's samples tree missing: a leg restores only its own tool-pack's data, so that fails no other tool's rule).
   */
  ruleErrors: Map<string, string[]>;
  /** The check's errors that name no rule: they fail the whole run. */
  errors: string[];
}

const add = (map: Map<string, string[]>, key: string, value: string) => map.set(key, [...(map.get(key) ?? []), value]);

/** Run the pack's conformance check on its fixtures and samples, as `test:conformance` does. */
export async function packConformance(root: string, pack: string): Promise<PackConformance> {
  const loaded = loadPack(join(root, "packs", pack));
  return conformanceOf(loaded, await runPack(loaded, { fixtures: true, samples: true, date: "port-readiness" }));
}

/** What the pack's conformance report says per rule. */
export function conformanceOf(loaded: LoadedPack, report: T4Report): PackConformance {
  const ids = loaded.pack.rules.map((rule) => rule.id);
  const rowsOf = (source: string) => new Map(report.rules.filter((row) => row.source === source).map((row) => [row.rule, row]));
  const stale = new Map<string, string[]>();
  for (const divergence of report.staleDivergences) add(stale, divergence.rule, `${divergence.path}:${divergence.line} (${divergence.kind})`);
  const ruleErrors = new Map<string, string[]>();
  const errors: string[] = [];
  for (const error of report.errors) {
    const id = ids.find((candidate) => error.startsWith(`${candidate} `) || error.startsWith(`${candidate}:`));
    if (id === undefined) errors.push(error);
    else add(ruleErrors, id, error);
  }
  return { rows: rowsOf("fixtures"), sampleRows: rowsOf(SAMPLES_SOURCE), stale, ruleErrors, errors };
}

const point = (value: { path: string; line: number; column?: number | undefined } | undefined) => (value === undefined ? "" : `, first ${value.path}:${value.line}${value.column === undefined ? "" : `:${value.column}`}`);

/**
 * The checks the pack's conformance check makes of one rule, as it makes them of a shipped rule. `ownSilentCases` is
 * how many of the rule's own upstream test cases upstream reports nothing on and the rule agrees on (matches-upstream-
 * tests' cases for its upstream rule): with the clean samples the rule is silent on, the evidence stays-silent reads.
 */
export function passesConformanceChecks(conformance: PackConformance, id: string, ownSilentCases: number): Check[] {
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
  const samples = conformance.sampleRows.get(id);
  if (samples !== undefined) {
    // A sample the grammar reads in part (T1) fails nothing by itself; a label the partial read hides is a missed label.
    const readInPart = samples.syntaxErrorFiles.count + samples.partialFiles;
    checks.push(
      { id: "samples-catch-labelled", ok: samples.fn === 0, detail: samples.fn === 0 ? `catches every labelled violation of its real-code samples (${samples.tp})` : `${samples.fn} labelled violation(s) of its real-code samples missed${point(samples.falseNegatives[0])}: ${fix}` },
      { id: "samples-silent-on-clean", ok: samples.fp === 0, detail: samples.fp === 0 ? `reports nothing on its ${samples.cleanSamples} clean sample(s)${samples.reviewed > 0 ? `; ${samples.reviewed} report(s) on labelled samples' unlabelled lines to review` : ""}` : `${samples.fp} finding(s) on clean samples${point(samples.falsePositives[0])}: ${fix}` },
      {
        id: "engine-reads-samples",
        ok: samples.engineFailures === 0,
        detail: samples.engineFailures + readInPart === 0
          ? "the engine evaluates it on every sample judged for it"
          : `${samples.engineFailures} engine failure(s); ${readInPart} file(s) read in part${point(samples.syntaxErrorFiles.files[0])}`,
      },
    );
  }
  // The counterpart of catches-something: somewhere the rule should report nothing, it reports nothing. The evidence
  // must be relevant to the rule: its own upstream test cases, or clean samples (kept as near misses for the rule).
  // Other rules' fixture units are not chosen to hold its construct, so silence there proves nothing, and Rerail's
  // probes are no upstream case.
  const silentSamples = samples?.silentCleanSamples ?? 0;
  checks.push({
    id: "stays-silent",
    ok: ownSilentCases + silentSamples > 0,
    detail: ownSilentCases + silentSamples > 0
      ? `silent where it should be: ${ownSilentCases} own upstream case(s), ${silentSamples} clean sample(s)`
      : "no own upstream case upstream reports nothing on and no clean sample where the rule is silent: add a clean real-code sample for it",
  });
  const stale = conformance.stale.get(id) ?? [];
  checks.push({ id: "divergences-current", ok: stale.length === 0, detail: stale.length === 0 ? "every declared divergence explains a disagreement" : `stale divergence(s) explain nothing: ${stale.slice(0, 3).join(", ")}` });
  return checks;
}
