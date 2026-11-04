import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../../src/engine/index.js";

it("distinguishes declaration names, parameters and references with canonical traversal order", () => {
  const source = "@decorate()\ndef f(x):\n    return {first(): value(), second(): other()}\n";
  const request = (text: string, kind: string) => ({ start: source.indexOf(text), end: source.indexOf(text) + text.length, kind,
    fields: ["ast_kind", "ast_name", "ast_preorder", "ast_breadth_order", "ast_parent_kind", "ast_field"] });
  const requests = [request("f", "identifier"), request("x", "identifier"),
    ...["decorate()", "first()", "value()", "second()", "other()"].map(text => request(text, "call"))];
  // Locate the declaration token rather than the f in def.
  requests[0]!.start = source.indexOf("f("); requests[0]!.end = requests[0]!.start + 1;
  const response = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{ source, requests }])));
  const [declaration, parameter, decorator, first, value, second, other] = response.documents[0];
  expect(declaration.ast_kind).toBe("");
  expect(parameter).toMatchObject({ ast_kind: "arg", ast_name: "x", ast_field: "args", ast_parent_kind: "arguments" });
  expect(Number(first.ast_preorder)).toBeLessThan(Number(second.ast_preorder));
  expect(Number(second.ast_preorder)).toBeLessThan(Number(value.ast_preorder));
  expect(Number(value.ast_preorder)).toBeLessThan(Number(other.ast_preorder));
  expect(Number(other.ast_preorder)).toBeLessThan(Number(decorator.ast_preorder));
  expect(Number(second.ast_breadth_order)).toBeLessThan(Number(value.ast_breadth_order));
  expect(response.parses).toBe(1); expect(response.module_walks).toBe(1);
});

it("expands a complete typed AST and forward annotations from a module demand", () => {
  const source = 'def f(x: "Sequence[Widget]", *args: int, **kwargs: str):\n    return {first(): value(), second(): other()}\n';
  const response = JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify([{ source, requests: [
    { start: 0, end: Buffer.byteLength(source), kind: "module", fields: ["ast_nodes"] },
    { start: 0, end: Buffer.byteLength(source), kind: "module", fields: ["ast_annotations"] },
  ] }])));
  const columns = JSON.parse(response.documents[0][0].ast_nodes) as Record<string, string[]>;
  const nodes = columns.ast_kind!.map((_, i) => Object.fromEntries(Object.entries(columns).map(([name, values]) => [name, values[i]!])));
  expect(nodes.filter(n => n.ast_kind === "arg").map(n => [n.ast_name, n.ast_field])).toEqual([
    ["x", "args"], ["args", "vararg"], ["kwargs", "kwarg"],
  ]);
  expect(nodes.filter(n => n.ast_kind === "Name").map(n => n.ast_name)).toEqual(["int", "str", "first", "second", "value", "other"]);
  expect(nodes.every(n => Number(n.ast_preorder) <= Number(n.ast_end))).toBe(true);
  const names = JSON.parse(response.documents[0][1].ast_annotations);
  expect(names.name).toEqual(["Sequence", "Widget", "int", "str"]);
  expect(response.parses).toBe(1); expect(response.module_walks).toBe(1);
});

it("anchors canonical variadic parameters whose spans differ from CST parameter spans", async () => {
  const { readFileSync } = await import("node:fs");
  const capabilities = readFileSync(new URL("../../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
  const source = "def f(*args: int, **kwargs: str):\n    pass\n";
  const authored = { format: "rerail.policy/v1", id: "canonical-parameter-spans", revision: 1, mode: "log", clauses: [{ id: "args", relational: {
    inputs: { gate: { evidence: { as: "f", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
    syntax: { name: "nodes", gate: "gate", columns: ["__path", "id", "kind", "__start", "__end"] },
    stages: [{ name: "canonical", pythonAst: { syntax: "nodes", roots: "SELECT __path,id FROM nodes WHERE kind='module'", fields: ["ast_kind", "ast_start", "ast_stop", "ast_name"], expand: "nodes" } }],
    query: "SELECT __path path,CAST(ast_start AS INTEGER) start_byte,CAST(ast_stop AS INTEGER) end_byte,ast_name identity FROM canonical WHERE ast_kind='arg'", key: ["identity"],
  }, violation: { code: "PARAM", message: "Canonical parameter", anchor: "witness.span" } }] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "a.py", source, language: "python" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((v: { span: { startByte: number; endByte: number } }) => source.slice(v.span.startByte, v.span.endByte))).toEqual(["args: int", "kwargs: str"]);
  authored.clauses[0]!.relational.query = authored.clauses[0]!.relational.query.replace("CAST(ast_start AS INTEGER) start_byte", "CAST(ast_start AS INTEGER)+1 start_byte");
  const fabricated = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilities);
  const rejected = JSON.parse(fabricated.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: "a.py", source, language: "python" }] }), "report"));
  expect(rejected.failures).toEqual([expect.objectContaining({ message: "relational results must anchor to an input witness span" })]);
});
