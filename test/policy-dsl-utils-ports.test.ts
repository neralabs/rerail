import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../src/native.js";
import { policyRuleSchema } from "../src/policy-schema.js";

// TSLint ports that recurse through `utils`, and constraint examples. Each deep case is one a port unrolled to a fixed
// depth would miss. A golden rule's default doc is its opt-in pack doc; option-variant overrides stay in
// conformance/tslint/rules/<rule>/.
const binding = loadNativeBinding();
const capabilities = readFileSync(new URL("../policies/executable-policy-dsl/capabilities.json", import.meta.url), "utf8");
const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const port = (name: string) =>
  read(name.includes("/") ? `../conformance/tslint/rules/${name}.json` : `../policies/executable-policy-dsl/rules/opt-in/tslint/${name}.json`);
const example = (name: string) => read(`./fixtures/constraint-policies/${name}.json`);

/** The source text of every finding, in order. */
function findings(document: string, source: string, path = "case.ts"): string[] {
  const compiled = binding.PolicyDslRuntime.compile(document, "json", capabilities);
  const result = JSON.parse(compiled.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ path, language: path.endsWith(".tsx") ? "tsx" : "typescript", source }],
  }), "diagnostic")) as { failures: unknown[]; violations: Array<{ span: { startByte: number; endByte: number } }> };
  expect(result.failures).toEqual([]);
  const bytes = Buffer.from(source);
  return result.violations.map(({ span }) => bytes.subarray(span.startByte, span.endByte).toString());
}

describe("TSLint ports that recurse through utils", () => {
  it.each([
    "no-return-await",
    "no-unused-expression",
    "no-unused-expression/allow-fast-null-checks",
    "no-unused-expression/allow-new",
    "no-unused-expression/allow-tagged-template",
    "prefer-template",
    "prefer-template/allow-single-concat",
  ])("%s is a valid policy document", name => {
    expect(policyRuleSchema.safeParse(JSON.parse(port(name))).success).toBe(true);
    expect(() => binding.PolicyDslRuntime.compile(port(name), "json", capabilities)).not.toThrow();
  });

  it("no-return-await follows tail positions to any depth and stops at function boundaries", () => {
    const shallow = "async function f() { return ((await x)); }\n";
    // Findings anchor at the keyword, as TSLint reports them.
    expect(findings(port("no-return-await"), shallow)).toEqual(["await"]);

    const deep = "async function f() { return c ? (a || ((((((await x)))))) ) : y; }\n";
    expect(findings(port("no-return-await"), deep)).toEqual(["await"]);

    // TSLint's isInsideTryBlock stops at the enclosing function: this await is not protected by the try.
    const nested = "async function f() { try { return g(async () => { return await x; }); } catch {} }\n";
    expect(findings(port("no-return-await"), nested)).toEqual(["await"]);
  });

  it("no-unused-expression follows TSLint's checking walk through any nesting", () => {
    const shallow = "a;\nf();\n";
    expect(findings(port("no-unused-expression"), shallow)).toEqual(["a;"]);

    const deep = "(((a, b)));\nc ? d : (e ? (f ? g : h()) : i);\n";
    expect(findings(port("no-unused-expression"), deep)).toEqual(["(((a, b)));", "d", "g", "i"]);
  });

  it("prefer-template reports only the outermost offending concatenation", () => {
    const cases = "const s = \"a\" + x + \"b\";\nf(\"a\" + g(y + \"b\" + z));\n";
    expect(findings(port("prefer-template"), cases)).toEqual([
      "\"a\" + x + \"b\"",
      "\"a\" + g(y + \"b\" + z)",
    ]);
  });
});

describe("constraints examples", () => {
  it("names hooks by capture, including calls with type arguments that a text regex misses", () => {
    const story = [
      "// useState(0) in a comment is not a call",
      "export const S = () => { const [a] = useState<number>(0); const b = React.useMemo(() => 1, []); user(1); return a + b; };",
      "",
    ].join("\n");
    expect(findings(example("story-hook-call"), story, "x.stories.tsx")).toEqual([
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
    expect(findings(example("react-hook-in-conditional"), component, "c.tsx")).toEqual([
      "useEffect(() => {})",
      "useMemo(() => 1, [])",
      "useCallback(() => {}, [])",
      "useRef(x)",
    ]);
  });
});
