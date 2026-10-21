/** Compare production gates against a baseline policy, separately from upstream oracle parity. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../../conformance/engine.js";

const fileSchema = z.object({ path: z.string(), source: z.string() });
const recordingSchema = z.object({ commit: z.string(), source_mismatches: z.array(z.string()).max(0),
  events: z.array(z.object({ test: z.string(), check: z.string(), files: z.array(fileSchema), findings: z.array(z.object({ code: z.string() })) })) });
const resultSchema = z.object({ failures: z.array(z.unknown()), violations: z.array(z.object({
  path: z.string(), code: z.string(), message: z.string(), span: z.unknown(),
})) });
const pageSchema = resultSchema.extend({ done: z.boolean(), cursor: z.number() });
type Result = z.infer<typeof resultSchema>;
type File = z.infer<typeof fileSchema>;

function normalized(result: Result): string[] {
  return result.violations.map(row => JSON.stringify([row.path, row.code, row.message, row.span])).sort();
}

async function workspace(rule: NativeCompiledCodeRuleDsl, files: File[], transitions?: { initial: Result; clean: Result; removed: Result }): Promise<{ initial: Result; transitionDifferences: number }> {
  const value = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    value.addCodeRule(rule);
    let revision = 0;
    const apply = async (current: File[], removed: string[] = []): Promise<Result> => {
      await value.applyAsync(JSON.stringify({ revision: ++revision, files: current.map(file => ({ ...file, language: "python" })), removed, quiet: true }));
      const result: Result = { failures: [], violations: [] };
      for (let cursor = 0; ;) {
        const page = pageSchema.parse(JSON.parse(await value.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
        result.failures.push(...page.failures); result.violations.push(...page.violations);
        if (page.done) return result;
        if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
        cursor = page.cursor;
      }
    };
    const initial = await apply(files);
    let transitionDifferences = 0;
    if (transitions) {
      for (const [actual, expected] of [
        [await apply(files.map(file => ({ ...file, source: "pass\n" }))), transitions.clean],
        [await apply(files), transitions.initial],
        [await apply([], files.map(file => file.path)), transitions.removed],
      ]) {
        if (actual!.failures.length || JSON.stringify(normalized(actual!)) !== JSON.stringify(normalized(expected!))) transitionDifferences++;
      }
    }
    return { initial, transitionDifferences };
  } finally { value.close(); }
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { baseline: { type: "string" }, candidate: { type: "string" },
    recording: { type: "string", multiple: true }, output: { type: "string" }, offset: { type: "string", default: "0" }, limit: { type: "string", default: "40" }, transitions: { type: "boolean", default: false },
    "baseline-input": { type: "string" }, "candidate-input": { type: "string" },
    "baseline-completed": { type: "string" }, "candidate-completed": { type: "string" } } });
  if (!values.baseline || !values.candidate || !values.recording || !values.output) throw new Error("Required: --baseline --candidate --recording --output [--offset --limit]");
  const recordings = values.recording.map(path => recordingSchema.parse(JSON.parse(readFileSync(path, "utf8"))));
  if (new Set(recordings.map(row => row.commit)).size !== 1) throw new Error("Source commits differ");
  const native = loadNativeBinding();
  const originalInput = process.env.RERAIL_SQL_INPUT_VTAB;
  const originalCompleted = process.env.RERAIL_SQL_COMPLETED_OUTPUT;
  for (const value of [values["baseline-completed"], values["candidate-completed"]]) {
    if (value !== undefined && !["table", "virtual"].includes(value)) throw new Error("Invalid completed output backend");
  }
  const selectCompleted = (mode: string | undefined) => {
    const value = mode ?? originalCompleted;
    if (value === undefined) delete process.env.RERAIL_SQL_COMPLETED_OUTPUT;
    else process.env.RERAIL_SQL_COMPLETED_OUTPUT = value;
  };
  for (const input of [values["baseline-input"], values["candidate-input"]]) {
    if (input !== undefined && !["table", "rows", "captures"].includes(input)) throw new Error("Invalid input backend");
  }
  const selectInput = (input: string | undefined) => {
    const value = input === undefined ? originalInput : input === "table" ? undefined : input;
    if (value === undefined) delete process.env.RERAIL_SQL_INPUT_VTAB;
    else process.env.RERAIL_SQL_INPUT_VTAB = value;
  };
  const compile = (path: string) => native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(JSON.parse(readFileSync(path, "utf8")))), "json", capabilitiesJson());
  const baseline = compile(values.baseline), candidate = compile(values.candidate);
  const events = recordings.flatMap(row => row.events).filter(row => row.check === "serverplan_quality");
  const cases = events.map((event, index) => ({ name: `${index}:${event.test}`, files: event.files }));
  // Relocate real positive sources across scope boundaries and source-trigger groups.
  // These are baseline-relative probes, not new upstream oracle expectations.
  for (const code of ["SPQ-011", "SPQ-051", "SPQ-052", "SPQ-053"]) {
    const event = events.find(row => row.files.length === 1 && row.findings.some(finding => finding.code === code));
    if (!event) throw new Error(`Missing positive source for ${code}`);
    for (const path of ["impl/common/l2070_server_plan/steps/gate_probe.py", "impl/common/l9999_misc/gate_probe.py", "tests/gate_probe.py", "outside/gate_probe.py"]) {
      cases.push({ name: `relocated:${code}:${path}`, files: [{ path, source: event.files[0]!.source }] });
    }
  }
  for (const path of ["impl/common/l2070_server_plan/steps/clean_probe.py", "impl/common/l9999_misc/clean_probe.py", "outside/clean_probe.py"]) {
    cases.push({ name: `clean:${path}`, files: [{ path, source: "class Ordinary:\n    def read(self): return 1\n" }] });
  }
  const offset = Number(values.offset), limit = Number(values.limit);
  if (!Number.isSafeInteger(offset) || offset < 0 || offset >= cases.length || !Number.isSafeInteger(limit) || limit < 1) throw new Error("Invalid offset or limit");
  const rows = [];
  for (const item of cases.slice(offset, offset + limit)) {
    const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: item.files.map(file => ({ ...file, language: "python" })) });
    selectInput(values["baseline-input"]);
    selectCompleted(values["baseline-completed"]);
    const expected = resultSchema.parse(JSON.parse(baseline.evaluate(input, "report")));
    selectInput(values["candidate-input"]);
    selectCompleted(values["candidate-completed"]);
    const actual = resultSchema.parse(JSON.parse(candidate.evaluate(input, "report")));
    const baselineSnapshot = (files: File[]): Result => {
      selectInput(values["baseline-input"]);
      selectCompleted(values["baseline-completed"]);
      return resultSchema.parse(JSON.parse(baseline.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: files.map(file => ({ ...file, language: "python" })) }), "report")));
    };
    const transitionExpected = values.transitions ? { initial: expected, clean: baselineSnapshot(item.files.map(file => ({ ...file, source: "pass\n" }))), removed: baselineSnapshot([]) } : undefined;
    selectInput(values["candidate-input"]);
    selectCompleted(values["candidate-completed"]);
    const incremental = await workspace(candidate, item.files, transitionExpected);
    rows.push({ name: item.name, positive: expected.violations.length > 0, codes: expected.violations.map(row => row.code),
      expected: expected.violations.length, actual: actual.violations.length,
      parity: JSON.stringify(normalized(expected)) === JSON.stringify(normalized(actual)),
      differences: JSON.stringify(normalized(expected)) === JSON.stringify(normalized(actual)) ? [] : [normalized(expected), normalized(actual)],
      workspaceParity: JSON.stringify(normalized(actual)) === JSON.stringify(normalized(incremental.initial)),
      transitionDifferences: incremental.transitionDifferences,
      failures: [...expected.failures, ...actual.failures, ...incremental.initial.failures, ...transitionExpected?.clean.failures ?? [], ...transitionExpected?.removed.failures ?? []] });
  }
  const result = { provenance: "Baseline-relative production gates; these are not upstream oracle findings", commit: recordings[0]!.commit,
    nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"),
    policyDigests: Object.fromEntries([values.baseline, values.candidate].map(path => [path, createHash("sha256").update(readFileSync(path)).digest("hex")])),
    baselineInput: values["baseline-input"] ?? originalInput ?? "table", candidateInput: values["candidate-input"] ?? originalInput ?? "table",
    baselineCompleted: values["baseline-completed"] ?? originalCompleted ?? "table", candidateCompleted: values["candidate-completed"] ?? originalCompleted ?? "table",
    totalCases: cases.length, offset, transitions: values.transitions ? rows.length * 3 : 0, cases: rows.length, positive: rows.filter(row => row.positive).length,
    clean: rows.filter(row => !row.positive).length, rows };
  writeFileSync(values.output, JSON.stringify(result, null, 2) + "\n");
  console.log(JSON.stringify({ cases: result.cases, positive: result.positive, clean: result.clean, totalCases: result.totalCases }));
  if (rows.some(row => !row.parity || !row.workspaceParity || row.transitionDifferences || row.failures.length)) process.exitCode = 1;
}

await main();
