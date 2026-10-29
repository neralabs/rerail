/**
 * Three Python ports (B406 and B610 on by default, F722 off) must not steer an agent into rewriting idiomatic code. Each reports what
 * its upstream reports, but on these idioms the finding is wrong and the steer harmful:
 *
 * - bandit-b406-import-xml-sax: Bandit reports any import under xml.sax, including the xml.sax.saxutils writing and
 *   escaping helpers, and steers toward defusedxml, which has no saxutils.
 * - bandit-b610-django-extra-used: Bandit reports a call of any function or method named `extra`; the port now asks
 *   for a receiver that looks like a Django QuerySet or Manager.
 * - ruff-f722-forward-annotation-syntax-error: Ruff reads a jaxtyping shape (`Float[Array, "b n"]`) as a forward
 *   annotation; jaxtyping parses it with its own grammar.
 *
 * Each case is one file; the test asserts exactly which lines the rule reports. The divergences are declared in
 * each rule's description and, where the pinned corpus has one, in packs/python/divergences.json.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { loadPackRuleDocuments } from "../../../../src/engine/catalog/rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(ruleId: string): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = loadPackRuleDocuments([repositoryPath("packs", "python")]).find(candidate => candidate.rule.id === ruleId);
  if (document === undefined) throw new Error(`${ruleId} is not in packs/python`);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/** The sorted 1-based lines `rule` reports in `source`, one entry per finding. */
function reportedLines(rule: NativeCompiledCodeRuleDsl, source: string): number[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "python", path: "src/module.py", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.span.startLine + 1).sort((a, b) => a - b);
}

const py = (...lines: string[]) => [...lines, ""].join("\n");

type Case = readonly [name: string, source: string, lines: number[]];

const B406: readonly Case[] = [
  ["escape imported from xml.sax.saxutils", py("from xml.sax.saxutils import escape"), []],
  ["XMLGenerator and quoteattr, parenthesised", py("from xml.sax.saxutils import (", "    XMLGenerator,", "    quoteattr,", ")"), []],
  ["the saxutils module from xml.sax", py("from xml.sax import saxutils"), []],
  ["the saxutils module aliased", py("from xml.sax import saxutils as su"), []],
  ["the saxutils module parenthesised", py("from xml.sax import (saxutils)"), []],
  ["import xml.sax, which binds the parsers", py("import xml.sax"), [1]],
  ["import xml.sax.saxutils, which also binds xml.sax", py("import xml.sax.saxutils"), [1]],
  ["make_parser imported beside saxutils", py("from xml.sax import make_parser, saxutils"), [1]],
  ["saxutils imported beside handler", py("from xml.sax import (saxutils, handler)"), [1]],
  ["the expat reader", py("from xml.sax.expatreader import ExpatParser"), [1]],
  ["xml.sax from xml", py("from xml import sax"), [1]],
];

const B610: readonly Case[] = [
  ["a non-QuerySet receiver", py("parser.extra(data)"), []],
  ["a bare function named extra", py("extra(where=w)"), []],
  ["self in a class that is not a QuerySet", py("class Options(Base):", "    def f(self, w):", "        return self.extra(where=w)"), []],
  ["a receiver whose arguments merely mention objects", py("build(objects).extra(where=w)"), []],
  ["a QuerySet given plain string lists", py("User.objects.extra(where=['a'])"), []],
  ["a model manager", py("User.objects.extra(where=[condition])"), [1]],
  ["a filtered QuerySet", py("User.objects.filter(username='admin').extra(select=dict(a=1))"), [1]],
  ["a variable named queryset", py("queryset = queryset.extra(select={'v': '%s' % col})"), [1]],
  ["a variable named qs", py("qs.extra(where=w)"), [1]],
  ["a prefixed variable", py("base_qs.extra(where=w)"), [1]],
  ["a queryset attribute", py("self.queryset.extra(where=w)"), [1]],
  ["get_queryset() chained", py("self.get_queryset().filter(a=1).extra(where=w)"), [1]],
  ["_default_manager", py("Model._default_manager.extra(where=w)"), [1]],
  [
    "self and super() in a QuerySet subclass",
    py("class Q(models.QuerySet):", "    def f(self, w):", "        return self.extra(where=w)", "    def g(self, w):", "        return super().extra(where=w)"),
    [3, 5],
  ],
  ["self in a Manager subclass", py("class M(BaseUserManager):", "    def f(self, w):", "        return self.extra(where=w)"), [3]],
  ["self in a custom QuerySet subclass", py("class A(SoftDeleteQuerySet):", "    def f(self, w):", "        return self.extra(where=w)"), [3]],
  ["self in a dotted third-party QuerySet subclass", py("class T(mptt.querysets.TreeQuerySet):", "    def f(self, w):", "        return super().extra(where=w)"), [3]],
  ["self in a class whose base only ends in Manager", py("class S(contextlib.AbstractContextManager):", "    def f(self, w):", "        return self.extra(where=w)"), []],
  ["self in a class whose bare base only ends in Manager", py("class P(Base, PluginManager):", "    def f(self, w):", "        return self.extra(where=w)"), []],
];

const F722: readonly Case[] = [
  ["a jaxtyping shape", py("from jaxtyping import Array, Float", "def f(x: Float[Array, 'b n']): ..."), []],
  ["a jaxtyping shape through the module", py("import jaxtyping", "def f(x: jaxtyping.Float[Array, 'batch channels']): ..."), []],
  ["a jaxtyping shape through an alias", py("import jaxtyping as jt", "def f(x: jt.Shaped[Array, '*batch n']): ..."), []],
  ["a jaxtyping shape on a variable and in a return", py("from jaxtyping import Array, Float, Int", "y: Float[Array, 'b n'] = 1", "def f() -> Int[Array, 'b n']: ..."), []],
  ["a jaxtyping shape nested in a generic", py("from jaxtyping import Array, Float", "z: dict[str, Float[Array, 'b n']] = {}"), []],
  ["the same string in a typing generic", py("from typing import List", "def f(x: List['b n']): ..."), [2]],
  ["a non-jaxtyping generic of the same shape", py("class Float: ...", "def f(x: Float[Array, 'b n']): ..."), [2]],
  ["a string as a jaxtyping array type, the first element", py("from jaxtyping import Float", "def f(x: Float['a b', 'b n']): ..."), [2]],
  ["a jaxtyping shape inside a string annotation", py("from jaxtyping import Array, Float", "def f(x: \"Float[Array, 'b n']\"): ..."), [2]],
];

describe.each([
  ["bandit-b406-import-xml-sax", B406],
  ["bandit-b610-django-extra-used", B610],
  ["ruff-f722-forward-annotation-syntax-error", F722],
] as const)("%s", (ruleId, cases) => {
  const rule = compile(ruleId);
  it.each(cases)("%s", (_name, source, lines) => {
    expect(reportedLines(rule, source)).toEqual(lines);
  });
});
