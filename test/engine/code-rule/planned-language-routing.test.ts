import { describe, expect, it, vi } from "vitest";
import { languageFor } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { generatedEditPaths } from "../../../src/engine/code-rule/code-rule-bridge.js";
import {
  CODE_RULE_DIALECTS,
  CODE_RULE_PLANNED_LANGUAGES,
  LANGUAGE_REGISTRY,
  codeRuleDialectForPath,
  isCodeRuleAstLanguage,
  isCodeRulePlannedLanguage,
} from "../../../src/engine/code-rule/code-rule-languages.js";
import { EXEMPLAR } from "../support/exemplar-registry.js";

// The host's planned-language behaviour is covered with the test-only exemplar, so it holds whether or not a shipped
// language is still planned (test/engine/support/exemplar-registry.ts).
vi.mock("../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../support/exemplar-registry.js")).exemplarRegistryModule());

describe("a planned language in the host", () => {
  it("is planned, not supported, and selects no dialect", () => {
    expect(CODE_RULE_PLANNED_LANGUAGES).toContain(EXEMPLAR);
    expect(isCodeRulePlannedLanguage(EXEMPLAR)).toBe(true);
    expect(isCodeRuleAstLanguage(EXEMPLAR)).toBe(false);
    expect(Object.keys(CODE_RULE_DIALECTS)).not.toContain(EXEMPLAR);
  });

  it("routes a planned language's files to no dialect, reading them as text", () => {
    const planned = LANGUAGE_REGISTRY.filter(entry => (entry.status as string) === "planned");
    expect(planned.map(entry => entry.id)).toContain(EXEMPLAR);
    for (const entry of planned) {
      for (const { label, suffixes } of entry.dialects) {
        expect(Object.keys(CODE_RULE_DIALECTS)).not.toContain(label);
        for (const suffix of suffixes) {
          const path = `src/file${suffix}`;
          expect(codeRuleDialectForPath(path)).toBeUndefined();
          expect(languageFor(path)).toBe("text");
          expect(generatedEditPaths("write", { path })).toEqual([path]);
        }
      }
    }
  });
});
