import { loadNativeBinding } from "../../src/native.js";
import { applyToWorkspace, capabilitiesJson } from "./runner.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/**
 * Minimal repros for kernel behaviours found while porting TSLint rules. Each probe runs one ast (or regex) check
 * over a tiny source and records what the kernel matched. `expected` is what upstream ast-grep would match (its relational
 * rules look at siblings of the node only; `stopBy: end` means "all siblings in that direction", `neighbor` means "the
 * adjacent sibling"; a contextual pattern's `context` only disambiguates parsing and the `selector` node alone is
 * matched), or what the check plainly asks for when no upstream reference applies. `ast-grep` expectations were checked
 * against the ast-grep 0.45.3 CLI; test/policy-dsl-upstream-differential.test.ts keeps that comparison running.
 */
export interface EngineProbe {
  id: string;
  claim: string;
  source: string;
  path?: string;
  /** The `forbid` object of the check (`{ ast: {...} }` or `{ regex: {...} }`). */
  forbid: Record<string, unknown>;
  /** Matched source texts expected under upstream semantics; `null` = the check should fail to compile. */
  expected: string[] | null;
  /** How sure the expectation is. */
  basis: "ast-grep" | "ast-grep (unverified)" | "plain reading";
}

export interface EngineProbeResult extends EngineProbe {
  observed: string[];
  compileError: string | undefined;
  failures: string[];
  /** The kernel's behaviour differs from `expected`. */
  deviates: boolean;
}

const ast = (rule: Record<string, unknown>): Record<string, unknown> => ({ ast: { language: "typescript", rule } });

export const ENGINE_PROBES: EngineProbe[] = [
  {
    id: "follows-end-leaves-parent",
    claim: "follows with stopBy:end searches everything earlier in the file, including siblings of ancestors",
    source: "function a() { x(); }\nfunction b() { y(); }\n",
    forbid: ast({ kind: "expression_statement", has: { pattern: "y()" }, follows: { stopBy: "end", kind: "expression_statement", has: { pattern: "x()" } } }),
    expected: [],
    basis: "ast-grep",
  },
  {
    id: "precedes-end-leaves-parent",
    claim: "precedes with stopBy:end searches everything later in the file, including siblings of ancestors",
    source: "function a() { x(); }\nfunction b() { y(); }\n",
    forbid: ast({ kind: "expression_statement", has: { pattern: "x()" }, precedes: { stopBy: "end", kind: "expression_statement", has: { pattern: "y()" } } }),
    expected: [],
    basis: "ast-grep",
  },
  {
    id: "neighbor-searches-sibling-subtree",
    claim: "precedes with stopBy:neighbor matches anywhere inside the next sibling's subtree, not only the sibling itself",
    source: "a();\nif (c) { b(); }\n",
    forbid: ast({ kind: "expression_statement", has: { pattern: "a()" }, precedes: { stopBy: "neighbor", kind: "expression_statement" } }),
    expected: [],
    basis: "ast-grep",
  },
  {
    id: "neighbor-climbs-to-parent",
    claim: "precedes with stopBy:neighbor climbs to the parent's next sibling when the node is the last child (anonymous tokens such as `}` count as siblings, so a block's last statement does not climb)",
    source: "if (c) a();\nb();\n",
    forbid: ast({ kind: "expression_statement", has: { pattern: "a()" }, precedes: { stopBy: "neighbor", kind: "expression_statement" } }),
    expected: [],
    basis: "ast-grep",
  },
  {
    id: "follows-neighbor-climbs-to-parent",
    claim: "follows with stopBy:neighbor escapes the parent when the node is the first child",
    source: "x = 1;\na() || b;\n",
    forbid: ast({ pattern: "a()", follows: { stopBy: "neighbor", kind: "expression_statement" } }),
    expected: [],
    basis: "ast-grep",
  },
  {
    id: "end-relation-under-not",
    claim: "a precedes/follows stopBy:end relation inside `not` removes every match (control: the excluded node comes first)",
    source: "function f() { return 1; }\nfunction g() { if (a) {} return 2; }\n",
    forbid: ast({ kind: "return_statement", not: { follows: { stopBy: "end", kind: "if_statement" } } }),
    expected: ["return 1;"],
    basis: "ast-grep",
  },
  {
    id: "end-relation-under-not-wipes",
    claim: "same rule, functions swapped: the `if` in an earlier function now counts as 'before' `return 1`",
    source: "function g() { if (a) {} return 2; }\nfunction f() { return 1; }\n",
    forbid: ast({ kind: "return_statement", not: { follows: { stopBy: "end", kind: "if_statement" } } }),
    expected: ["return 1;"],
    basis: "ast-grep",
  },
  {
    id: "anon-token-kind-rejected",
    claim: "anonymous tokens cannot be named by kind",
    source: "if (a == b) {}\n",
    forbid: ast({ kind: "==" }),
    expected: null,
    basis: "ast-grep",
  },
  {
    id: "anon-token-regex-matches",
    claim: "anonymous tokens are matchable by regex (and are siblings/children for relations)",
    source: "if (a == b) {}\n",
    forbid: ast({ regex: "^==$" }),
    expected: ["=="],
    basis: "ast-grep (unverified)",
  },
  {
    id: "anon-token-is-neighbor",
    claim: "the `,` token is the neighbor sibling of an array element",
    source: "[1, 2];\n",
    forbid: ast({ kind: "number", follows: { stopBy: "neighbor", regex: "^,$" } }),
    expected: ["2"],
    basis: "ast-grep",
  },
  {
    id: "contextual-selector-ignores-context",
    claim: "a contextual pattern's selector matches nodes whose surroundings do not match the context",
    source: "var x = 1;\n",
    forbid: ast({ pattern: { context: "var $A = $B, $C = $D", selector: "variable_declarator" } }),
    expected: ["x = 1"],
    basis: "ast-grep",
  },
  {
    id: "contextual-selector-drops-metavar-identity",
    claim: "`var $A = $B, $B` with selector variable_declarator matches although the two $B differ",
    source: "var a = c, b;\n",
    forbid: ast({ pattern: { context: "var $A = $B, $B", selector: "variable_declarator" } }),
    expected: ["a = c"],
    basis: "ast-grep",
  },
  {
    id: "contextual-selector-identity-positive",
    claim: "control: the same contextual pattern where the two $B are equal",
    source: "var a = b, b;\n",
    forbid: ast({ pattern: { context: "var $A = $B, $B", selector: "variable_declarator" } }),
    expected: ["a = b"],
    basis: "ast-grep",
  },
  {
    id: "contextual-switch-duplicate-case",
    claim: "`switch ($X) { $$$A case $V: case $V: $$$B }` with a switch_case selector matches nothing",
    source: "switch (x) { case 1: case 1: break; }\n",
    forbid: ast({ pattern: { context: "switch ($X) { $$$A case $V: case $V: $$$B }", selector: "switch_case" } }),
    expected: ["case 1:", "case 1: break;"],
    basis: "ast-grep",
  },
  {
    id: "contextual-switch-distinct-cases",
    claim: "control: the same pattern over two DIFFERENT case values",
    source: "switch (x) { case 1: case 2: break; }\n",
    forbid: ast({ pattern: { context: "switch ($X) { $$$A case $V: case $V: $$$B }", selector: "switch_case" } }),
    expected: ["case 1:", "case 2: break;"],
    basis: "ast-grep",
  },
  {
    id: "path-regex-span-as-content",
    claim: "a file.path regex finding carries the path match offsets, which are then read as content offsets",
    source: "const a = 1;\nconst b = 2;\n",
    path: "dir/sub/x.ts",
    forbid: { regex: { source: "file.path", pattern: "x\\.ts$" } },
    expected: [""],
    basis: "plain reading",
  },
  {
    id: "kind-function-rejected",
    claim: "kind \"function\" is rejected",
    source: "const f = function () {};\n",
    forbid: ast({ kind: "function" }),
    expected: null,
    basis: "ast-grep",
  },
];

function document(probe: EngineProbe): string {
  return JSON.stringify({
    format: "rerail.policy/v1",
    id: `probe:${probe.id}`,
    revision: 1,
    description: probe.claim,
    scope: { include: ["**/*"], exclude: [] },
    checks: [{ id: "probe", forbid: probe.forbid, finding: { anchor: "witness.span", code: "PROBE", severity: "error", message: "probe" } }],
  });
}

export async function runEngineProbes(): Promise<EngineProbeResult[]> {
  const binding = loadNativeBinding();
  const results: EngineProbeResult[] = [];
  for (const probe of ENGINE_PROBES) {
    let compiled;
    try {
      compiled = binding.PolicyDslRuntime.compile(document(probe), "json", capabilitiesJson());
    } catch (error) {
      const compileError = error instanceof Error ? error.message : String(error);
      results.push({ ...probe, observed: [], compileError, failures: [], deviates: probe.expected !== null });
      continue;
    }
    const workspace = new binding.PolicyDslWorkspace();
    try {
      workspace.addPolicy(compiled);
      const path = probe.path ?? "probe.ts";
      const payload = JSON.stringify({ revision: 1, files: [{ path, language: "typescript", source: probe.source }], removed: [] });
      const { evaluation } = JSON.parse(await applyToWorkspace(workspace, payload)) as {
        evaluation: { findings: Array<{ span?: { startByte: number; endByte: number } }>; failures: Array<{ code: string; message: string }> };
      };
      const bytes = Buffer.from(probe.source, "utf8");
      const observed = evaluation.findings.map(finding =>
        finding.span === undefined ? "<no span>" : `${bytes.subarray(finding.span.startByte, finding.span.endByte).toString("utf8")}@${finding.span.startByte}`,
      );
      const texts = observed.map(entry => entry.replace(/@\d+$/, ""));
      const expected = probe.expected ?? [];
      const deviates =
        probe.expected === null ||
        (probe.id === "path-regex-span-as-content"
          ? evaluation.findings.some(finding => finding.span !== undefined && finding.span.startByte !== 0)
          : [...texts].sort(compareDeterministicStrings).join("\u0000") !== [...expected].sort(compareDeterministicStrings).join("\u0000"));
      results.push({ ...probe, observed, compileError: undefined, failures: evaluation.failures.map(failure => `${failure.code}: ${failure.message}`), deviates });
    } finally {
      workspace.close();
    }
  }
  return results;
}

if (process.argv[1]?.endsWith("engine-probes.ts")) {
  for (const result of await runEngineProbes()) {
    console.log(`${result.deviates ? "DEVIATES" : "as expected"}  ${result.id}`);
    console.log(`  expected ${result.expected === null ? "compile error" : JSON.stringify(result.expected)} (${result.basis})`);
    console.log(`  observed ${result.compileError !== undefined ? `compile error: ${result.compileError}` : JSON.stringify(result.observed)}${result.failures.length > 0 ? `  failures ${result.failures.join("; ")}` : ""}`);
  }
}
