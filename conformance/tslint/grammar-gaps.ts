import ts from "typescript";
import { loadNativeBinding } from "../../src/native.js";
import { applyToWorkspace, capabilitiesJson, PARSE_FAILURE_CODES } from "./runner.js";

/**
 * Constructs that TSLint fixtures (or ports) suggested tree-sitter-typescript cannot handle. Each is checked two
 * ways: tsc (is it valid TypeScript? syntax and grammar diagnostics only, codes 1xxx and 17xxx-18xxx) and the kernel
 * (does the file parse, or does `misparse` match, showing a tree shape TypeScript would not build).
 * A `grammar-gap` is valid TypeScript that the kernel rejects or misparses; anything tsc rejects is a fixture artifact.
 */
export interface GrammarProbe {
  id: string;
  code: string;
  /** Where the construct shows up. */
  seenIn: string;
  /** An ast rule that matches only if tree-sitter built the wrong tree; omitted = check for a parse failure. */
  misparse?: Record<string, unknown>;
}

export interface GrammarProbeResult extends GrammarProbe {
  tscErrors: string[];
  kernel: "parses" | "parse error" | "misparse";
  verdict: "grammar-gap" | "fixture-artifact" | "not a gap";
}

export const GRAMMAR_PROBES: GrammarProbe[] = [
  { id: "export-default-abstract-class", code: "export default abstract class { }\n", seenIn: "no-default-export (whole fixture lost)" },
  { id: "paren-target-in-array-pattern", code: "let foo, bar;\n[foo, , (bar)] = [];\n", seenIn: "no-sparse-arrays (whole fixture lost)" },
  { id: "paren-target-nested", code: "let d1, d2;\n[(d1), ([d2])] = [];\n", seenIn: "prefer-const default/spread.ts" },
  { id: "paren-target-object-rest", code: "let d5;\n({ ...(d5) } = {});\n", seenIn: "prefer-const default/spread.ts" },
  {
    id: "default-function-then-arrow",
    code: "export default function() {}\n() => {};\n",
    seenIn: "space-before-function-paren always/never/mixed (whole fixtures lost)",
  },
  {
    id: "await-as-precedence",
    code: "async function f(x: unknown) {\n  return (await x as number);\n}\n",
    seenIn: "no-return-await (port exempts as-expressions to compensate)",
    misparse: { kind: "await_expression", has: { stopBy: "neighbor", kind: "as_expression" } },
  },
  {
    id: "top-level-namespace-in-expression-statement",
    code: "namespace N { export const a = 1; }\n",
    seenIn: "batch 1 note (tree shape only; ports adapted)",
    misparse: { kind: "internal_module", inside: { stopBy: "neighbor", kind: "expression_statement" } },
  },
  { id: "nel-whitespace", code: "let \u0085 foo;\n", seenIn: "no-irregular-whitespace (regex port unaffected)" },
  { id: "bare-arrow-statement", code: "() => {};\n", seenIn: "claimed by batch 7" },
  { id: "empty-extends-list", code: "interface M extends {}\n", seenIn: "no-empty-interface (claimed as a gap)" },
  { id: "anonymous-function-statement", code: "function() {}\n", seenIn: "space-before-function-paren (claimed by batch 7)" },
  { id: "if-without-then", code: "declare const x: boolean;\nif (x) else if (x) {}\n", seenIn: "prefer-switch min-cases-2, batch 7" },
  { id: "not-triple-equals", code: "declare const a: boolean;\na !=== a;\n", seenIn: "space-within-parens no-space, batch 7" },
  { id: "decorated-abstract-member", code: "declare const d: any;\nabstract class A { @d abstract m() {} }\n", seenIn: "member-access default" },
  { id: "readonly-static-order", code: "class A { readonly static P = 1; }\n", seenIn: "member-access default" },
  { id: "unquoted-module-specifier", code: "import {Observable} from rxjs;\n", seenIn: "import-blacklist" },
  { id: "duplicate-modifier", code: "class A { private private h(): boolean { return true; } }\n", seenIn: "no-unnecessary-class" },
  { id: "abstract-async", code: "abstract class A { abstract async f(): Promise<void>; }\n", seenIn: "no-async-without-await" },
  { id: "decorator-on-function", code: "declare const d: any;\n@d\nfunction f() {}\n", seenIn: "no-shadowed-variable (no port)" },
];

function tscErrors(code: string): string[] {
  const host = ts.createCompilerHost({});
  const readSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (name, version) => (name === "/probe.ts" ? ts.createSourceFile(name, code, version, true) : readSourceFile(name, version));
  const program = ts.createProgram(["/probe.ts"], { noEmit: true, noLib: true, types: [], target: ts.ScriptTarget.ESNext }, host);
  return [...program.getSyntacticDiagnostics(), ...program.getSemanticDiagnostics()]
    .filter(diagnostic => diagnostic.file?.fileName === "/probe.ts" && (diagnostic.code < 2000 || (diagnostic.code >= 17000 && diagnostic.code < 19000)))
    .map(diagnostic => `TS${diagnostic.code} ${ts.flattenDiagnosticMessageText(diagnostic.messageText, " ")}`);
}

async function kernelVerdict(probe: GrammarProbe): Promise<GrammarProbeResult["kernel"]> {
  const binding = loadNativeBinding();
  const rule = probe.misparse ?? { kind: "program" };
  const document = JSON.stringify({
    format: "rerail.policy/v1",
    id: `grammar:${probe.id}`,
    revision: 1,
    mode: "steer",
    description: probe.id,
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{ id: "probe", forbid: { ast: { language: "typescript", rule } }, violation: { anchor: "witness.span", code: "PROBE", message: "probe" } }],
  });
  const workspace = new binding.PolicyDslWorkspace();
  try {
    workspace.addPolicy(binding.PolicyDslRuntime.compile(document, "json", capabilitiesJson()));
    const payload = JSON.stringify({ revision: 1, files: [{ path: "probe.ts", language: "typescript", source: probe.code }], removed: [] });
    const { evaluation } = JSON.parse(await applyToWorkspace(workspace, payload)) as { evaluation: { violations: unknown[]; failures: Array<{ code: string }> } };
    // `source-partial` (clauses ran around the errors) and `source-incomplete` (a clause skipped the file) both mean
    // the grammar could not parse the whole probe.
    if (evaluation.failures.some(failure => PARSE_FAILURE_CODES.has(failure.code))) return "parse error";
    if (probe.misparse !== undefined && evaluation.violations.length > 0) return "misparse";
    return "parses";
  } finally {
    workspace.close();
  }
}

export async function runGrammarProbes(): Promise<GrammarProbeResult[]> {
  const results: GrammarProbeResult[] = [];
  for (const probe of GRAMMAR_PROBES) {
    const errors = tscErrors(probe.code);
    const kernel = await kernelVerdict(probe);
    const verdict = errors.length > 0 ? (kernel === "parses" ? "not a gap" : "fixture-artifact") : kernel === "parses" ? "not a gap" : "grammar-gap";
    results.push({ ...probe, tscErrors: errors, kernel, verdict });
  }
  return results;
}

if (process.argv[1]?.endsWith("grammar-gaps.ts")) {
  for (const result of await runGrammarProbes()) {
    console.log(`${result.verdict.padEnd(16)} ${result.id}: kernel ${result.kernel}; tsc ${result.tscErrors.length === 0 ? "valid" : result.tscErrors.join(" | ")}`);
  }
}
