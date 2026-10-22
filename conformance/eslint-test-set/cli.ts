/**
 * Runs the javascript and typescript packs over the ESLint test set and checks Rerail reports every finding ESLint
 * reported, and nothing where ESLint reports nothing. The set is real files from open-source history, each at the commit
 * that introduced a violation of one rule, beside a copy with every finding of that rule fixed by hand
 * (s3://nera-test-fixtures/eslint-test-set/; built by benchmark-repos' `.lint/build-test-set.mjs`). Each file is
 * evaluated for its own rule, through the same workspace path the repository audit takes.
 *
 *   npm run eslint-test-set -- [--profile <aws profile>] [--zip <file>] [--rule <id>] [--json <file>]
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { z } from "zod";
import { compareDeterministicStrings, languageFor, loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson, INFORMATIONAL_FAILURE_CODES } from "../engine.js";
import { isCandidate, loadPack, PACKS_ROOT, RERAIL_ROOT, type PackRule } from "../languages/pack.js";
import { evaluateWorkspace, pointOf, type SourceFile } from "../languages/runner.js";

/** The set this checkout is checked against. A new set is a new key and digest here. */
const FIXTURE = {
  bucket: "nera-test-fixtures",
  key: "eslint-test-set/2026-10-21.2/eslint-test-set.zip",
  sha256: "dc8f2955ffd78ed9853b19a0aa10e976bf711be221195c171fbf62aceff90d41",
};
const PACKS = ["javascript", "typescript"];
const CACHE = join(RERAIL_ROOT, ".rerail", "cache", "eslint-test-set");

const findingSchema = z.object({ line: z.number().int().positive(), column: z.number().int().positive(), message: z.string() });
const entrySchema = z.object({
  file: z.string(),
  /** The same file with every finding of the rule fixed by hand: ESLint reports nothing for the rule in it. */
  fixed: z.string().nullable(),
  rule: z.string(),
  rerailRule: z.string(),
  pack: z.string(),
  source: z.object({ repo: z.string(), url: z.string() }),
  introduced: z.object({ startLine: z.number().int().positive() }),
  eslint: z.object({ findings: z.array(findingSchema) }),
});
type Entry = z.infer<typeof entrySchema>;
const manifestSchema = z.object({ schema: z.literal("nera.eslint-test-set/v1"), entries: z.array(z.object({ file: z.string() })) });

const { values } = parseArgs({
  options: { profile: { type: "string" }, zip: { type: "string" }, rule: { type: "string", multiple: true }, json: { type: "string" } },
});

/** Downloads the pinned zip once, checks its digest, and unpacks it; returns the set's directory. */
function fixture(): string {
  const directory = join(CACHE, FIXTURE.sha256);
  const root = join(directory, "eslint-test-set");
  if (existsSync(join(root, "manifest.json"))) return root;
  mkdirSync(directory, { recursive: true });
  const zip = values.zip ?? join(directory, "eslint-test-set.zip");
  if (values.zip === undefined) {
    const profile = values.profile === undefined ? [] : ["--profile", values.profile];
    execFileSync("aws", [...profile, "s3", "cp", "--only-show-errors", `s3://${FIXTURE.bucket}/${FIXTURE.key}`, zip], { stdio: "inherit" });
  }
  const digest = createHash("sha256").update(readFileSync(zip)).digest("hex");
  if (digest !== FIXTURE.sha256) throw new Error(`${zip} has sha256 ${digest}; this checkout pins ${FIXTURE.sha256}`);
  execFileSync("unzip", ["-q", "-o", zip, "-d", directory]);
  return root;
}

interface Compiled {
  rule: PackRule;
  candidate: boolean;
  policyId: string;
  compiled: NativeCompiledCodeRuleDsl;
}

/** Every rule of the packs, by its id (the set's directory names), compiled. */
function compileRules(): Map<string, Compiled> {
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const rules = new Map<string, Compiled>();
  for (const pack of PACKS) {
    const loaded = loadPack(join(PACKS_ROOT, pack));
    for (const rule of loaded.pack.rules) {
      const text = readFileSync(join(loaded.directory, rule.policy), "utf8");
      const policyId = z.object({ id: z.string() }).parse(JSON.parse(text)).id;
      rules.set(rule.id, { rule, candidate: isCandidate(rule), policyId, compiled: runtime.compile(text, "json", capabilitiesJson()) });
    }
  }
  return rules;
}

interface Miss {
  file: string;
  line: number;
  column: number;
  message: string;
  url: string;
  /** The engine reported syntax errors in the file for this rule, so the miss may be the grammar's, not the rule's. */
  syntaxErrors: boolean;
}
interface RuleResult {
  rule: string;
  eslintRule: string;
  candidate: boolean;
  match: PackRule["match"];
  files: number;
  expected: number;
  caught: number;
  missed: Miss[];
  /** Rerail findings ESLint does not report on the originals: reported, not failed. */
  extra: number;
  /** Fixed copies checked, and Rerail's findings of the rule on them: each fails, since ESLint reports none. */
  fixedFiles: number;
  falseAlarms: Miss[];
  /** Engine failures other than `source-partial`, per file. */
  failures: string[];
}

const root = fixture();
const manifest = manifestSchema.parse(JSON.parse(readFileSync(join(root, "manifest.json"), "utf8")));
const entries: Entry[] = manifest.entries.map(({ file }) => entrySchema.parse(JSON.parse(readFileSync(join(root, `${file}.eslint.json`), "utf8"))));
const byRule = new Map<string, Entry[]>();
for (const entry of entries) {
  if (values.rule !== undefined && !values.rule.includes(entry.rerailRule) && !values.rule.includes(entry.rule)) continue;
  byRule.set(entry.rerailRule, [...(byRule.get(entry.rerailRule) ?? []), entry]);
}

const compiled = compileRules();
const results: RuleResult[] = [];
const errors: string[] = [];
for (const [ruleId, ruleEntries] of [...byRule].sort(([left], [right]) => compareDeterministicStrings(left, right))) {
  const rule = compiled.get(ruleId);
  if (rule === undefined) {
    errors.push(`${ruleId}: no such rule in the ${PACKS.join(" or ")} pack`);
    continue;
  }
  const files: SourceFile[] = ruleEntries.flatMap(entry => (entry.fixed === null ? [entry.file] : [entry.file, entry.fixed])).map(path => {
    const language = languageFor(path);
    if (language === undefined) throw new Error(`${path}: no language routes it`);
    return { path, language, source: readFileSync(join(root, path), "utf8") };
  });
  // One workspace per rule: a file's AST work budget is shared by the rules beside it.
  const evaluation = await evaluateWorkspace([rule.compiled], files);
  const sources = new Map(files.map(file => [file.path, Buffer.from(file.source, "utf8")]));
  const found = evaluation.violations.filter(violation => violation.ruleId === rule.policyId).map(violation => pointOf(violation, sources));
  const result: RuleResult = {
    rule: ruleId, eslintRule: rule.rule.upstream.rule, candidate: rule.candidate, match: rule.rule.match,
    files: ruleEntries.length, expected: 0, caught: 0, missed: [], extra: 0, fixedFiles: 0, falseAlarms: [],
    failures: evaluation.failures.filter(failure => !INFORMATIONAL_FAILURE_CODES.has(failure.code)).map(failure => `${failure.path ?? "?"}: ${failure.code}: ${failure.message}`),
  };
  const syntaxErrorFiles = new Set(evaluation.failures.flatMap(failure => (failure.code.startsWith("source-") && failure.path !== undefined ? [failure.path] : [])));
  for (const entry of ruleEntries) {
    // Each Rerail finding answers at most one ESLint finding.
    const unmatched = found.filter(point => point.path === entry.file);
    for (const finding of entry.eslint.findings) {
      result.expected += 1;
      const index = unmatched.findIndex(point => point.line === finding.line && (rule.rule.match === "line" || point.column === finding.column));
      if (index === -1) result.missed.push({ file: entry.file, line: finding.line, column: finding.column, message: finding.message, url: entry.source.url, syntaxErrors: syntaxErrorFiles.has(entry.file) });
      else {
        unmatched.splice(index, 1);
        result.caught += 1;
      }
    }
    result.extra += unmatched.length;
    if (entry.fixed !== null) {
      result.fixedFiles += 1;
      for (const point of found.filter(point => point.path === entry.fixed))
        result.falseAlarms.push({ file: entry.fixed, line: point.line, column: point.column ?? 0, message: `reported on the fixed copy of ${entry.file}`, url: entry.source.url, syntaxErrors: syntaxErrorFiles.has(entry.fixed) });
    }
  }
  results.push(result);
}

const missedRules = results.filter(result => result.missed.length > 0);
const alarmRules = results.filter(result => result.falseAlarms.length > 0);
const total = (key: "expected" | "caught" | "extra"): number => results.reduce((sum, result) => sum + result[key], 0);
const missed = results.flatMap(result => result.missed.map(miss => ({ ...miss, candidate: result.candidate })));
const lines = [
  `ESLint test set ${FIXTURE.key}: ${entries.length} files; ${results.length} rules checked.`,
  `Rerail caught ${total("caught")} of ${total("expected")} ESLint findings; ${total("extra")} more findings ESLint did not report.`,
  `Fixed copies: ${results.reduce((sum, result) => sum + result.fixedFiles, 0)}; Rerail reported its rule on ${new Set(results.flatMap(result => result.falseAlarms.map(alarm => alarm.file))).size} of them (${results.reduce((sum, result) => sum + result.falseAlarms.length, 0)} findings).`,
  `Missed ${missed.length}: ${missed.filter(miss => !miss.candidate).length} by shipped rules, ${missed.filter(miss => miss.candidate).length} by candidates; ${missed.filter(miss => miss.syntaxErrors).length} in files the engine read with syntax errors.`,
  "",
];
for (const result of missedRules) {
  lines.push(`${result.rule}${result.candidate ? " (candidate)" : ""}: missed ${result.missed.length} of ${result.expected} (${result.eslintRule}, matched by ${result.match})`);
  for (const miss of result.missed.slice(0, 5)) lines.push(`  ${miss.file}:${miss.line}:${miss.column} ${miss.message}${miss.syntaxErrors ? " [syntax errors]" : ""}`);
  if (result.missed.length > 5) lines.push(`  … ${result.missed.length - 5} more`);
}
for (const result of alarmRules) {
  lines.push(`${result.rule}${result.candidate ? " (candidate)" : ""}: ${result.falseAlarms.length} findings on fixed copies, where ESLint reports none`);
  for (const alarm of result.falseAlarms.slice(0, 5)) lines.push(`  ${alarm.file}:${alarm.line}:${alarm.column}${alarm.syntaxErrors ? " [syntax errors]" : ""}`);
  if (result.falseAlarms.length > 5) lines.push(`  … ${result.falseAlarms.length - 5} more`);
}
for (const result of results.filter(entry => entry.failures.length > 0)) {
  lines.push(`${result.rule}: ${result.failures.length} engine failures`);
  for (const failure of result.failures.slice(0, 3)) lines.push(`  ${failure}`);
}
for (const error of errors) lines.push(`error: ${error}`);
const ok = missedRules.length === 0 && alarmRules.length === 0 && errors.length === 0;
lines.push("", ok ? "Every ESLint finding caught, and nothing reported on the fixed copies." : `${missedRules.length} rules missed findings; ${alarmRules.length} rules reported on fixed copies.`);
console.log(lines.join("\n"));
if (values.json !== undefined) writeFileSync(values.json, JSON.stringify({ fixture: FIXTURE, results, errors }, null, 2) + "\n");
process.exitCode = ok ? 0 : 1;
