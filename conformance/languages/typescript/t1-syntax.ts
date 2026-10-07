// T1 for the TypeScript pack, the part the Rust tape-parity test does not report: every corpus file the native
// grammar reads with syntax errors, located at its first error and set beside the reference parsers' verdict, and the
// sites where tree-sitter-typescript reads a postfix non-null assertion `!` over a whole binary (or other non-member)
// expression, which TypeScript never does (tree-sitter/tree-sitter-typescript#299). Run offline, from `rerail/`, with
// the addon built:
//   node --import tsx conformance/languages/typescript/t1-syntax.ts <corpus checkout> <path>[,<path>...] <out.json> [<unparsed.json>]
// <unparsed.json> is a corpus recording (`../rerail-fixtures/typescript/rules/eslint/corpus-<name>.expected.json`): its `source.skipped` are
// the files ESLint 10.11.0 (espree, or typescript-eslint 8.70.1's parser for TypeScript files) cannot parse.
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { capabilitiesJson } from "../../tslint/runner.js";
import type { PolicySourceLanguage } from "../../../src/executable-policy.js";
import { IGNORED_DIRECTORIES, languageFor } from "../../../src/executable-policy-enforcer.js";
import { encodePolicyWorkspaceRequest, loadNativeBinding } from "../../../src/native.js";

const [root, pathList, output, unparsedRecording] = process.argv.slice(2);
if (root === undefined || pathList === undefined || output === undefined) throw new Error("usage: classify.ts <corpus> <paths> <out.json> [<recording>]");
const ECMA = new Set(["typescript", "tsx", "javascript"]);
const unparsed = new Set<string>(unparsedRecording === undefined ? [] : JSON.parse(readFileSync(unparsedRecording, "utf8")).source.skipped);

type Source = { path: string; language: PolicySourceLanguage; source: string };
const files: Source[] = [];
const walk = (relative: string): void => {
  for (const entry of readdirSync(join(root, relative), { withFileTypes: true })) {
    const path = `${relative}/${entry.name}`;
    if (entry.isDirectory()) {
      if (!IGNORED_DIRECTORIES.has(entry.name)) walk(path);
    } else if (entry.isFile()) {
      const language = languageFor(path);
      if (language !== undefined && ECMA.has(language)) files.push({ path, language, source: readFileSync(join(root, path), "utf8") });
    }
  }
};
for (const path of pathList.split(",")) walk(path);
files.sort((left, right) => (left.path < right.path ? -1 : 1));

const policy = (id: string, rule: unknown): string =>
  JSON.stringify({
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "log",
    description: id,
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{ id: "check", forbid: { ast: { language: "typescript", rule } }, violation: { anchor: "witness.span", code: id, message: id } }],
  });
// A probe sound under syntax errors: ERROR nodes are never witnesses, so it reports nothing, and its `source-partial`
// failure locates the file's first syntax error. The second reports every `!` the grammar puts over an expression
// TypeScript's parser never asserts as a whole (it parses `!` inside the member chain: parseMemberExpressionRest).
// The third reports every call whose callee the grammar reads as a prefix-operator expression: TypeScript binds a
// prefix operator looser than a call, so `!f<T>(x)` is `!(f<T>(x))`, never `(!f)<T>(x)`.
const NOT_MEMBER = ["binary_expression", "ternary_expression", "assignment_expression", "augmented_assignment_expression", "unary_expression", "await_expression", "update_expression", "as_expression", "satisfies_expression", "yield_expression", "arrow_function", "new_expression"];
const binding = loadNativeBinding();
const workspace = new binding.PolicyDslWorkspace();
workspace.addPolicy(binding.PolicyDslRuntime.compile(policy("syntax-probe", { kind: "ERROR" }), "json", capabilitiesJson()));
workspace.addPolicy(binding.PolicyDslRuntime.compile(policy("non-null-over", { kind: "non_null_expression", has: { stopBy: "neighbor", any: NOT_MEMBER.map(kind => ({ kind })) } }), "json", capabilitiesJson()));
workspace.addPolicy(binding.PolicyDslRuntime.compile(policy("prefix-callee", { kind: "call_expression", has: { field: "function", any: ["unary_expression", "await_expression", "update_expression"].map(kind => ({ kind })) } }), "json", capabilitiesJson()));
const started = performance.now();
let revision = 0;
for (let start = 0; start < files.length; start += 32) {
  revision += 1;
  await workspace.applyBufferAsync(encodePolicyWorkspaceRequest({ revision, files: files.slice(start, start + 32), removed: [], deferGlobal: true, quiet: true }));
}
revision += 1;
await workspace.applyBufferAsync(encodePolicyWorkspaceRequest({ revision, files: [], removed: [], quiet: true }));
type Finding = { ruleId: string; path: string; span: { startLine: number; startColumn: number; startByte: number; endByte: number } };
const findings: Finding[] = [];
const failures: { ruleId?: string; code: string; message: string; path?: string }[] = [];
for (let cursor = 0; ; ) {
  const page = JSON.parse(await workspace.resultPageAsync(cursor, 8 << 20));
  findings.push(...page.violations);
  failures.push(...page.failures);
  if (page.done) break;
  cursor = page.cursor;
}
workspace.close();
const elapsedMs = Math.round(performance.now() - started);

const byPath = new Map(files.map(file => [file.path, file]));
const lineOf = (path: string, line: number): string => (byPath.get(path)?.source.split(/\r\n|[\r\n\u2028\u2029]/u)[line - 1] ?? "").trim().slice(0, 160);
const syntaxErrors = failures
  .filter(failure => failure.ruleId === "syntax-probe" && failure.code === "source-partial" && failure.path !== undefined)
  .map(failure => {
    const match = /first at line (\d+), column (\d+)$/u.exec(failure.message);
    const line = Number(match?.[1] ?? 0);
    return { path: failure.path!, line, column: Number(match?.[2] ?? 0), text: lineOf(failure.path!, line), referenceRejects: unparsed.has(failure.path!) };
  })
  .sort((left, right) => (left.path < right.path ? -1 : 1));
const sites = (policyId: string) =>
  findings
    .filter(finding => finding.ruleId === policyId)
    .map(finding => {
      const source = Buffer.from(byPath.get(finding.path)!.source, "utf8");
      const text = source.subarray(finding.span.startByte, finding.span.endByte).toString("utf8");
      return { path: finding.path, line: finding.span.startLine + 1, column: finding.span.startColumn + 1, text: text.slice(0, 120) };
    });
const nonNull = sites("non-null-over");
const prefixCallee = sites("prefix-callee");
// Every check is sound under syntax errors, so `source-partial` is expected on every syntax-error file.
const otherFailures = failures.filter(failure => failure.code !== "source-partial");
const report = {
  files: files.length,
  bytes: files.reduce((total, file) => total + Buffer.byteLength(file.source), 0),
  elapsedMs,
  filesWithSyntaxErrors: syntaxErrors.length,
  referenceRejects: syntaxErrors.filter(entry => entry.referenceRejects).length,
  syntaxErrors,
  nonNullOverExpression: { sites: nonNull.length, files: new Set(nonNull.map(site => site.path)).size, sitesList: nonNull },
  prefixOperatorCallee: { sites: prefixCallee.length, files: new Set(prefixCallee.map(site => site.path)).size, sitesList: prefixCallee },
  otherFailures,
};
writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
console.log(`${files.length} files, ${syntaxErrors.length} with syntax errors (${report.referenceRejects} rejected by the reference parser too), ${nonNull.length} non-null sites over a non-member expression in ${report.nonNullOverExpression.files} files, ${prefixCallee.length} calls of a prefix-operator callee in ${report.prefixOperatorCallee.files} files, ${otherFailures.length} other failures, ${elapsedMs} ms`);
