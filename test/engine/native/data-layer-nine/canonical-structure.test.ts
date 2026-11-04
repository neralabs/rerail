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
