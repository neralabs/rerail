/**
 * What the pack's conformance report says of one rule, as checks: the fixtures (upstream's tests and Rerail's probes),
 * their rename variants, the real-code samples, stays-silent and the pack's declared divergences. Every rule is held
 * to what `reportFailures` fails a shipped rule on, so a candidate is never "ready to promote" while the check would
 * fail it once shipped, and must show it stays silent somewhere it should.
 */
import type { LoadedPack } from "../pack.js";
import { renameVacuity, type RuleRow, type T4Report } from "../report.js";
import { SAMPLES_SOURCE } from "../runner.js";
import type { Check } from "./check.js";

/** The pack's conformance report, per rule: one fixtures row and at most one samples row per rule id. */
export interface PackConformance {
  rows: Map<string, RuleRow>;
  /** One samples row per rule id some sample is labelled for (none for a rule no sample judges). */
  sampleRows: Map<string, RuleRow>;
  /** Divergences of a rule that explain nothing, as `path:line (kind)`. */
  stale: Map<string, string[]>;
  /** The report's errors that name a rule (a document that does not compile, test data that does not cover it). */
  ruleErrors: Map<string, string[]>;
  /** The report's errors that name no rule but one tool (`fixtures <tool>: …`, `samples <tool>: …`), per tool. */
  toolErrors: Map<string, string[]>;
  /** The report's errors that name neither a rule nor a tool: they concern the whole pack. */
  errors: string[];
}

const add = (map: Map<string, string[]>, key: string, value: string) => map.set(key, [...(map.get(key) ?? []), value]);

/** What the pack's conformance report says per rule. */
export function conformanceOf(loaded: LoadedPack, report: T4Report): PackConformance {
  const ids = loaded.pack.rules.map((rule) => rule.id);
  const rowsOf = (source: string) => new Map(report.rules.filter((row) => row.source === source).map((row) => [row.rule, row]));
  const stale = new Map<string, string[]>();
  for (const divergence of report.staleDivergences) add(stale, divergence.rule, `${divergence.path}:${divergence.line} (${divergence.kind})`);
  const ruleErrors = new Map<string, string[]>();
  const toolErrors = new Map<string, string[]>();
  const errors: string[] = [];
  const tools = loaded.pack.tools.map((tool) => tool.name);
  for (const error of report.errors) {
    const id = ids.find((candidate) => error.startsWith(`${candidate} `) || error.startsWith(`${candidate}:`));
    const tool = tools.find((name) => ["fixtures", SAMPLES_SOURCE].some((source) => error.startsWith(`${source} ${name}:`)));
    if (id !== undefined) add(ruleErrors, id, error);
    else if (tool !== undefined) add(toolErrors, tool, error);
    else errors.push(error);
  }
  return { rows: rowsOf("fixtures"), sampleRows: rowsOf(SAMPLES_SOURCE), stale, ruleErrors, toolErrors, errors };
}

const point = (value: { path: string; line: number; column?: number | undefined } | undefined) => (value === undefined ? "" : `, first ${value.path}:${value.line}${value.column === undefined ? "" : `:${value.column}`}`);

const FIX = "fix the rule, or declare the divergence in the pack's divergences.json";

/**
 * holds-under-rename: the rule scored on its tool's rename variants, apart from the originals. An unexplained FP or
 * FN, or an engine failure, on a variant fails it; parse-induced `source-incomplete` reads and variants the renaming
 * took every finding from are only reported, as long as one finding survives. When the renaming took every finding, the
 * check fails: no finding survived to show the rule matches the construct rather than the cases' names. The renamer
 * keeps literals and every two names' order (scripts/pack-data/rename/ecmascript.ts), so a rule that reads a literal
 * or an order keeps its findings on the variants. With no variant for the rule it is n/a, naming the held-out evidence
 * left.
 */
function holdsUnderRename(row: RuleRow, samples: RuleRow | undefined): Check {
  const id = "holds-under-rename";
  const renamed = row.renamed;
  if (renamed === null) {
    const judged = samples?.judgedSamples ?? 0;
    return {
      id,
      ok: true,
      detail: judged > 0 ? `n/a: no rename variants; samples are the held-out evidence (${judged} judged samples)` : "n/a: no rename variants and no samples: no held-out evidence",
    };
  }
  // n/a only when the variants hold nothing against the rule: `reportFailures` fails a shipped row on any FP, FN or
  // engine failure on them, whether or not a variant unit covers the rule, and the gate must agree.
  if (renamed.units === 0 && renamed.fp + renamed.fn + renamed.engineFailures === 0) return { id, ok: true, detail: "n/a: no rename variant covers this rule" };
  const notes = [
    ...(renamed.incomplete > 0 ? [`${renamed.incomplete} parse-induced source-incomplete read(s) on variants, not failing`] : []),
    ...(renamed.renamedAway > 0 ? [`${renamed.renamedAway} variant(s) lost every finding of the rule (renamed away)`] : []),
  ].map(note => `; ${note}`).join("");
  const disagrees = renamed.fp > 0 || renamed.fn > 0;
  const problems = [
    ...(disagrees ? [`${renamed.fp} unexplained FP${point(renamed.falsePositives[0])} and ${renamed.fn} unexplained FN${point(renamed.falseNegatives[0])} on rename variants: the rule matches the test cases' names or literals, not the construct`] : []),
    ...(renamed.engineFailures > 0 ? [`${renamed.engineFailures} engine failure(s) on rename variants`] : []),
  ];
  const vacuity = renameVacuity(renamed);
  if (problems.length === 0 && vacuity !== false) {
    if (vacuity === "exempt") return { id, ok: true, detail: `n/a: exempt in gates.yaml (${renamed.exempt}); no finding survives on its ${renamed.units} rename variant unit(s)${notes}` };
    return {
      id,
      ok: false,
      detail: `vacuous: no finding of the rule survives on its ${renamed.units} rename variant unit(s)${notes}: the rule matches the test cases' names, not the construct; fix the rule`,
    };
  }
  return {
    id,
    ok: problems.length === 0,
    detail: problems.length > 0
      ? `${problems.join("; ")}${notes}; ${disagrees ? FIX : "fix the rule"}`
      : `agrees with upstream on its ${renamed.units} rename variant unit(s) (${renamed.tp} true positive(s))${notes}`,
  };
}

/** The counterpart of catches-something: somewhere the rule should report nothing, it reports nothing. */
function staysSilent(row: RuleRow, samples: RuleRow | undefined): Check {
  // The evidence must be relevant to the rule: its own upstream units, or clean samples (kept as near misses for the
  // rule). Other rules' units are not chosen to hold its construct, so silence there proves nothing, and Rerail's probes
  // and units of unknown origin are no upstream case of the rule.
  const silentSamples = samples?.silentCleanSamples ?? 0;
  const ok = row.silentOwnUnits + silentSamples > 0;
  return {
    id: "stays-silent",
    ok,
    detail: ok
      ? `silent where it should be: ${row.silentOwnUnits} of ${row.ownUnits} own upstream unit(s), ${silentSamples} clean sample(s)`
      : `no own upstream unit upstream reports nothing in (${row.ownUnits} own unit(s)) and no clean sample where the rule is silent (${samples?.cleanSamples ?? 0} clean sample(s)): add a clean real-code sample for it`,
  };
}

/** The checks the pack's conformance report makes of one rule, as it makes them of a shipped rule. */
export function conformanceChecks(conformance: PackConformance, id: string): Check[] {
  const errors = conformance.ruleErrors.get(id) ?? [];
  const checks: Check[] = [{ id: "scored", ok: errors.length === 0, detail: errors.length === 0 ? "the pack's conformance check compiles and scores it" : errors.slice(0, 2).join("; ") }];
  const row = conformance.rows.get(id);
  if (row === undefined) {
    checks.push({ id: "fixtures-row", ok: false, detail: "the pack's conformance check scored no fixtures for it: add its conformance.json entry and its test data" });
    return checks;
  }
  checks.push(
    { id: "no-unexplained-false-positives", ok: row.fp === 0, detail: row.fp === 0 ? `no unexplained finding on the pack's fixtures (upstream's cases and Rerail's probes), with ${row.tp} true positive(s)` : `${row.fp} unexplained FP on the pack's fixtures${point(row.falsePositives[0])}: ${FIX}` },
    { id: "no-unexplained-misses", ok: row.fn === 0, detail: row.fn === 0 ? "misses no finding upstream reports on the pack's fixtures" : `${row.fn} unexplained FN on the pack's fixtures${point(row.falseNegatives[0])}: ${FIX}` },
    { id: "catches-something", ok: row.tp > 0 || row.examples, detail: row.tp > 0 ? `${row.tp} finding(s) agree with upstream on the fixtures` : row.examples ? "pack.json gives it examples" : "0 true positives on the fixtures and no examples in pack.json: nothing shows it catches anything" },
    { id: "engine-reads-fixtures", ok: row.engineFailures === 0, detail: row.engineFailures === 0 ? "the engine evaluates it on every fixture" : `${row.engineFailures} engine failure(s) on the fixtures` },
    holdsUnderRename(row, conformance.sampleRows.get(id)),
  );
  const samples = conformance.sampleRows.get(id);
  if (samples !== undefined) {
    // A sample the grammar reads in part (T1, `source-partial`, a parse-induced `source-incomplete`) fails nothing by
    // itself; a label the partial read hides is a missed label. A context file is never judged: its engine failures are
    // only listed.
    const readInPart = samples.syntaxErrorFiles.count + samples.partialFiles;
    const context = samples.contextFailures > 0 ? `; ${samples.contextFailures} on context files, not judged` : "";
    checks.push(
      { id: "samples-catch-labelled", ok: samples.fn === 0, detail: samples.fn === 0 ? `catches every labelled violation of its real-code samples (${samples.tp})` : `${samples.fn} labelled violation(s) of its real-code samples missed${point(samples.falseNegatives[0])}: ${FIX}` },
      { id: "samples-silent-on-clean", ok: samples.fp === 0, detail: samples.fp === 0 ? `reports nothing on its ${samples.cleanSamples} clean sample(s)${samples.reviewed > 0 ? `; ${samples.reviewed} report(s) on labelled samples' unlabelled lines to review` : ""}` : `${samples.fp} finding(s) on clean samples${point(samples.falsePositives[0])}: ${FIX}` },
      {
        id: "engine-reads-samples",
        ok: samples.engineFailures === 0,
        detail: samples.engineFailures + readInPart === 0
          ? `the engine evaluates it on every sample judged for it${context}`
          : `${samples.engineFailures} engine failure(s); ${readInPart} file(s) read in part${point(samples.syntaxErrorFiles.files[0])}${context}`,
      },
    );
  }
  checks.push(staysSilent(row, samples));
  const stale = conformance.stale.get(id) ?? [];
  checks.push({ id: "divergences-current", ok: stale.length === 0, detail: stale.length === 0 ? "every declared divergence explains a disagreement" : `stale divergence(s) explain nothing: ${stale.slice(0, 3).join(", ")}` });
  return checks;
}
