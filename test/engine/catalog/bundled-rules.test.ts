/**
 * The TSLint pack (packs/tslint/rules/, shipped in core and seeded disabled) is proven against TSLint's own tests by the
 * pack validation (test/engine/catalog/packs.test.ts, check 4) instead. Here each doc must compile and scope TypeScript sources only,
 * and no rule TSLint cannot run without configuration (a candidate whose verdict.json records `requiresConfiguration`)
 * ships.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { bundledCodeRuleDslDirectory } from "../../../src/engine/code-rule/code-rule-compile.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { loadBundledRules } from "../../../src/engine/catalog/rule-bootstrap.js";

const TSLINT_CANDIDATES = fileURLToPath(new URL("../../../packs/tslint/candidates/", import.meta.url));
const verdictSchema = z.object({ audit: z.object({ class: z.string() }).optional(), requiresConfiguration: z.string().min(1).optional() });

const capabilities = readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const optIn = loadBundledRules().filter(document => document.enabledByDefault === false && document.rule.id.startsWith("tslint:"));

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
