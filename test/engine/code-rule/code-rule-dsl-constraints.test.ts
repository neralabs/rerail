import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { fixturesPath } from "../../support/oracles.js";

// The constraint examples in fixtures/host/constraint-code-rules/: rules that name and filter captures with
// `constraints` and `utils`, evaluated on TypeScript and TSX.
const binding = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const example = (name: string) => readFileSync(fixturesPath("host", "constraint-code-rules", `${name}.json`), "utf8");

/** The source text of every violation, in order. */
function violations(document: string, source: string, path = "case.ts"): string[] {
  const compiled = binding.CodeRuleDslRuntime.compile(document, "json", capabilities);
  const result = JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ path, language: path.endsWith(".tsx") ? "tsx" : "typescript", source }],
  }), "report")) as { failures: unknown[]; violations: Array<{ span: { startByte: number; endByte: number } }> };
  expect(result.failures).toEqual([]);
  const bytes = Buffer.from(source);
  return result.violations.map(({ span }) => bytes.subarray(span.startByte, span.endByte).toString());
}

describe("constraints examples", () => {
  it("names hooks by capture, including calls with type arguments that a text regex misses", () => {
    const story = [
      "// useState(0) in a comment is not a call",
      "export const S = () => { const [a] = useState<number>(0); const b = React.useMemo(() => 1, []); user(1); return a + b; };",
      "",
    ].join("\n");
    expect(violations(example("story-hook-call"), story, "x.stories.tsx")).toEqual([
      "useState<number>(0)",
      "React.useMemo(() => 1, [])",
    ]);
  });

  it("flags hooks reached conditionally within their own function only", () => {
    const component = [
      "function C({ ok }) {",
      "  const [s] = useState(0);",
      "  if (ok) { useEffect(() => {}); }",
      "  const v = ok ? useMemo(() => 1, []) : 0;",
      "  ok && useCallback(() => {}, []);",
      "  for (const x of xs) { useRef(x); }",
      "  if (ok) { const handler = () => { const t = useLocal(); }; }",
      "  return s;",
      "}",
      "",
    ].join("\n");
    expect(violations(example("react-hook-in-conditional"), component, "c.tsx")).toEqual([
      "useEffect(() => {})",
      "useMemo(() => 1, [])",
      "useCallback(() => {}, [])",
      "useRef(x)",
    ]);
  });
});
