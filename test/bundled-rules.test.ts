/**
 * Every bundled rule ships with a flag/pass fixture pair in test/fixtures/bundled-rules/<rule-file>.json
 * ({ flag: { path, content }, pass: { path, content } }). Fixtures are stored as data, not as source
 * files at their scope paths, so the repository's language-wide scanners never see a deliberately
 * non-compliant snippet. For each rule: the flag content, evaluated at its path, must be inside the
 * rule's scope (the engine's own inScope) and produce one of the rule's violation codes; the pass content
 * must be inside the scope and produce none. A rule without a fixture file fails: a default that nothing
 * proves can fire is exactly what this guards against.
 *
 * The TSLint pack (packs/tslint/rules/, shipped in core and seeded disabled) is proven against TSLint's own tests by the
 * pack validation (test/packs.test.ts, check 4) instead. Here each doc must compile and scope TypeScript sources only,
 * and no rule TSLint cannot run without configuration (a candidate whose verdict.json records `requiresConfiguration`)
 * ships.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";
import { bundledRulesDirectory, loadBundledRules } from "../src/rule-bootstrap.js";
import { evaluateExample } from "../conformance/pack-validation.js";

const TSLINT_CANDIDATES = fileURLToPath(new URL("../packs/tslint/candidates/", import.meta.url));
const verdictSchema = z.object({ audit: z.object({ class: z.string() }).optional(), requiresConfiguration: z.string().min(1).optional() });

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../code-rule-dsl/", import.meta.url));
const FIXTURES = fileURLToPath(new URL("./fixtures/bundled-rules/", import.meta.url));
const capabilities = readFileSync(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const ruleFileSchema = z.object({ id: z.string() });
const fixtureSideSchema = z.object({ path: z.string().min(1), content: z.string() });
const fixtureSchema = z.object({ flag: fixtureSideSchema, pass: fixtureSideSchema });

function violations(compiled: NativeCompiledCodeRuleDsl, path: string, source: string): string[] {
  const { codes, failures } = evaluateExample(compiled, path, source);
  expect(failures).toEqual([]);
  return codes;
}

const bundled = loadBundledRules(bundledRulesDirectory());
const documents = bundled.filter(document => document.enabledByDefault !== false);
const optIn = bundled.filter(document => document.enabledByDefault === false && document.rule.id.startsWith("tslint:"));
/** Rule id -> rule file stem; the fixture file shares the stem. Core ships no rules of its own today, so the directory may be absent. */
const stemByRuleId: Record<string, string> = Object.fromEntries(
  (existsSync(bundledRulesDirectory()) ? readdirSync(bundledRulesDirectory()) : [])
    .filter(name => name.endsWith(".json"))
    .map(name => [ruleFileSchema.parse(JSON.parse(readFileSync(join(bundledRulesDirectory(), name), "utf8"))).id, name.slice(0, -5)]),
);

describe("bundled rules", () => {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");

  it.each(documents.map(document => [document.rule.id] as const))("%s fires on its flag fixture and stays silent on its pass fixture", id => {
    const document = documents.find(candidate => candidate.rule.id === id)!;
    expect(document.kind).toBe("file");
    if (document.kind !== "file") return;
    const fixture = fixtureSchema.parse(JSON.parse(readFileSync(join(FIXTURES, `${stemByRuleId[id]!}.json`), "utf8")));
    const codes: Record<string, true> = Object.fromEntries(document.rule.clauses.map(clause => [clause.violation?.code, true]));
    const compiled = runtime.compile(JSON.stringify(document.rule), "json", capabilities);

    expect(compiled.inScope(fixture.flag.path), `${id} flag fixture ${fixture.flag.path} is outside the rule's scope`).toBe(true);
    expect(compiled.inScope(fixture.pass.path), `${id} pass fixture ${fixture.pass.path} is outside the rule's scope`).toBe(true);
    const flagHits = violations(compiled, fixture.flag.path, fixture.flag.content).filter(code => codes[code] === true);
    expect(flagHits, `${id} flag fixture produced no ${Object.keys(codes).join("/")} finding`).not.toEqual([]);
    const passHits = violations(compiled, fixture.pass.path, fixture.pass.content).filter(code => codes[code] === true);
    expect(passHits, `${id} pass fixture produced ${passHits.join(", ")}`).toEqual([]);
  });

  it("has a fixture file for every rule and none for rules that do not exist", () => {
    const stems = Object.values(stemByRuleId);
    // The package ships no enabled-by-default rules today, so the fixture directory may not exist yet.
    const fixtureStems = existsSync(FIXTURES)
      ? readdirSync(FIXTURES).filter(name => name.endsWith(".json")).map(name => name.slice(0, -5))
      : [];
    expect(fixtureStems.filter(stem => !stems.includes(stem))).toEqual([]);
    expect(stems.filter(stem => !fixtureStems.includes(stem))).toEqual([]);
  });
});

describe("TSLint pack", () => {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  it("leaves out every rule that requires configuration, each audited requires-config", () => {
    const verdicts = readdirSync(TSLINT_CANDIDATES)
      .filter(rule => existsSync(join(TSLINT_CANDIDATES, rule, "verdict.json")))
      .map(rule => ({ rule, verdict: verdictSchema.parse(JSON.parse(readFileSync(join(TSLINT_CANDIDATES, rule, "verdict.json"), "utf8"))) }));
    const requiresConfig = verdicts.filter(({ verdict }) => verdict.requiresConfiguration !== undefined);
    expect(requiresConfig.map(({ rule }) => rule)).toEqual(expect.arrayContaining(["file-header", "typedef"]));
    for (const { rule, verdict } of requiresConfig) expect(verdict.audit?.class, rule).toBe("requires-config");
    const ids = new Set(optIn.map(document => document.rule.id));
    expect(requiresConfig.filter(({ rule }) => ids.has(`tslint:${rule}`)).map(({ rule }) => rule)).toEqual([]);
  });

  it.each(optIn.map(document => [document.rule.id] as const))("%s compiles and scopes TypeScript sources outside build output", id => {
    const document = optIn.find(candidate => candidate.rule.id === id)!;
    const compiled = runtime.compile(JSON.stringify(document.rule), "json", capabilities);
    for (const path of ["src/a.ts", "a.ts", "web/src/view.tsx", "lib/m.mts"]) expect(compiled.inScope(path), path).toBe(true);
    for (const path of ["src/a.js", "node_modules/p/a.ts", "pkg/dist/a.ts", "src/generated/api.ts", "README.md"]) {
      expect(compiled.inScope(path), path).toBe(false);
    }
  });
});
