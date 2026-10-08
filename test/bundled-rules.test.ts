/**
 * Every bundled rule ships with a flag/pass fixture pair in test/fixtures/bundled-rules/<rule-file>.json
 * ({ flag: { path, content }, pass: { path, content } }). Fixtures are stored as data, not as source
 * files at their scope paths, so the repository's language-wide scanners never see a deliberately
 * non-compliant snippet. For each rule: the flag content, evaluated at its path, must be inside the
 * rule's scope (the engine's own inScope) and produce one of the rule's violation codes; the pass content
 * must be inside the scope and produce none. A rule without a fixture file fails: a default that nothing
 * proves can fire is exactly what this guards against.
 *
 * The TSLint pack (packs/tslint/rules/, shipped in core and seeded disabled) is the golden set: the conformance harness proves it
 * against TSLint's own fixtures and gates it with conformance.json instead. Here each doc must compile and scope TypeScript
 * sources only, the pack must agree with conformance.json and the audit (`conformance/tslint/promote.ts --check`), and the
 * harness must score golden rules with the pack's docs. Rules TSLint cannot run without configuration
 * (`REQUIRES_CONFIGURATION`, audited `requires-config`) stay out of it.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";
import { bundledRulesDirectory, corePackDirectories, loadBundledRules } from "../src/rule-bootstrap.js";
import { evaluateExample } from "../conformance/pack-validation.js";
import { PASS_AUDIT } from "../conformance/tslint/audit.js";
import { emptyGoldenManifest, goldenRules, loadGoldenManifest, PACK_DIRECTORY } from "../conformance/tslint/golden.js";
import { checkGolden, REQUIRES_CONFIGURATION } from "../conformance/tslint/promote.js";
import { COMMITTED_PORTS_DIRECTORY, findPort, listPortedRules } from "../conformance/tslint/runner.js";

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
const optIn = bundled.filter(document => document.enabledByDefault === false);
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
  it("is exactly the golden set, consistent with conformance.json and the PASS audit", () => {
    expect(corePackDirectories().map(directory => join(directory, "rules"))).toEqual([PACK_DIRECTORY]);
    expect(optIn.map(document => document.rule.id).sort()).toEqual(goldenRules().map(rule => `tslint:${rule}`));
    expect(Object.keys(loadGoldenManifest().rules).sort()).toEqual(goldenRules());
    expect(checkGolden()).toEqual([]);
    for (const rule of goldenRules()) expect(PASS_AUDIT[rule]?.class, rule).toBe("genuine");
  });

  it("flags a conformance.json that disagrees with the pack folder", () => {
    const [first] = goldenRules();
    const manifest = loadGoldenManifest();
    const missing = { ...emptyGoldenManifest(), rules: Object.fromEntries(Object.entries(manifest.rules).filter(([rule]) => rule !== first)) };
    expect(checkGolden(missing)).toEqual([`${first}: in the pack but has no conformance.json entry`]);
    const extra = { ...manifest, rules: { ...manifest.rules, "not-a-pack-rule": { verdict: "PASS" as const, truePositives: 1 } } };
    expect(checkGolden(extra)).toEqual(["not-a-pack-rule: conformance.json entry without a pack doc"]);
  });

  it("is where the harness takes golden default docs from; non-golden ports stay in packs/tslint/candidates", () => {
    const golden = goldenRules();
    for (const rule of golden) {
      const port = findPort(rule);
      expect(port.golden, rule).toBe(true);
      expect(port.document, rule).toBe(join(PACK_DIRECTORY, `${rule}.json`));
      expect(port.sidecar, rule).toBeUndefined();
    }
    const port = findPort("curly");
    expect(golden).not.toContain("curly");
    expect(port.golden).toBe(false);
    expect(port.document).toBe(join(COMMITTED_PORTS_DIRECTORY, "curly.json"));
    expect(listPortedRules()).toEqual(expect.arrayContaining([...golden, "curly"]));
  });

  it("leaves out every rule that requires configuration, each audited requires-config", () => {
    const requiresConfig = Object.entries(PASS_AUDIT).filter(([, audit]) => audit.class === "requires-config").map(([rule]) => rule).sort();
    expect([...REQUIRES_CONFIGURATION.keys()].sort()).toEqual(requiresConfig);
    expect(requiresConfig).toEqual(expect.arrayContaining(["file-header", "typedef"]));
    const ids = new Set(optIn.map(document => document.rule.id));
    expect(requiresConfig.filter(rule => ids.has(`tslint:${rule}`))).toEqual([]);
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
