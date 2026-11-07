import assert from "node:assert/strict";
import { test } from "vitest";

import { defaultOn, upstreamStatusChecks } from "../../../conformance/port-readiness/checks/upstream-status.js";
import { verdict, type PackAdapter, type UpstreamRule } from "../../../conformance/port-readiness/types.js";

// NRV's pack-precondition tests (spdxIds, licenseDifference, packPreconditionChecks) are not ported: the gates no
// longer make those checks; rerail's pack validator (conformance/pack-validation.ts) checks the pack's licenses.

const LICENSE = "Apache License\nVersion 2.0\n\n4. Redistribution. You must meet the following conditions:\n\n(a) You must give any other recipients a copy;\n\n(b) You must cause modified files to carry notices.\n";

function adapter(rules: Record<string, UpstreamRule> = {}): PackAdapter {
  return {
    identity: {},
    packChecks: [{ id: "upstream-commit", ok: true, detail: "pinned" }],
    licenseText: LICENSE,
    rule: (name) => rules[name],
    implementationNames: () => new Set(),
    sharedImplementationNames: () => new Set(),
    implementationRegexes: () => [],
    fixtureWords: () => new Set(),
    fixtureTexts: () => [],
    keywords: new Set(),
    commentVocabulary: new Set(),
    inScopeSamples: [],
    cases: () => [],
    toSpan: () => ({ startLine: 0, startCol: 0, endLine: 0, endCol: 0 }),
    inCorpus: () => true,
    lintCorpus: async () => [],
    parserVerdicts: async () => [],
  };
}

function failed(checks: { id: string; ok: boolean }[]): string[] {
  return checks.filter((check) => !check.ok).map((check) => check.id);
}

test("upstream-status checks that the rule exists upstream, is stable and is on in an approved configuration", () => {
  const upstream = (overrides: Partial<UpstreamRule>): UpstreamRule => ({
    name: "b101",
    implementation: ["bandit/plugins/asserts.py"],
    tests: ["examples/assert.py"],
    deprecation: null,
    recommended: { config: "default", options: [] },
    notOnRealCode: null,
    ...overrides,
  });
  assert.deepEqual(failed(upstreamStatusChecks("b101", adapter({ b101: upstream({}) }))), []);
  assert.deepEqual(failed(upstreamStatusChecks("nera-only", adapter())), ["upstream-rule"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", adapter({ b101: upstream({ tests: [] }) }))), ["upstream-rule"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", adapter({ b101: upstream({ deprecation: "redundant" }) }))), ["stable"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", adapter({ b101: upstream({ recommended: null }) }))), ["recommended"]);
  // The licence is never a rule's check.
  assert.equal(upstreamStatusChecks("b101", adapter({ b101: upstream({}) })).some((check) => check.id === "pack"), false);

  const withOptions = upstreamStatusChecks("b101", adapter({ b101: upstream({ recommended: { config: "default", options: [{ skips: [] }] } }) }));
  assert.equal(withOptions.find((check) => check.id === "recommended")!.detail, 'on in default with options [{"skips":[]}]; matches-upstream-tests checks the rule with these options');
});

test("a rule starts on exactly when it exists upstream, is stable and is recommended", () => {
  const check = (id: string, ok: boolean) => ({ id, ok, detail: `${id} detail` });
  for (const exists of [true, false]) {
    for (const stable of [true, false]) {
      for (const recommended of [true, false]) {
        const result = defaultOn([check("upstream-rule", exists), check("stable", stable), check("recommended", recommended)]);
        assert.equal(result.on, exists && stable && recommended);
        assert.equal(result.reason.startsWith("experimental: "), !result.on);
      }
    }
  }
  assert.equal(defaultOn([check("upstream-rule", false)]).on, false);
  assert.match(defaultOn([check("upstream-rule", true), check("stable", true), check("recommended", false)]).reason, /^experimental: recommended \(recommended detail\)$/u);
});

test("upstream-status never decides the verdict: a rule ships when it passes the blocking gates", () => {
  const pass = { pass: true };
  const fail = { pass: false };
  assert.deepEqual(verdict({ loads: pass, "not-overfit": pass, compact: pass, "matches-upstream-tests": pass, "passes-conformance": pass }), { pass: true, stoppedAt: null });
  assert.deepEqual(verdict({ loads: pass, "not-overfit": fail, compact: pass, "matches-upstream-tests": fail, "passes-conformance": pass }), { pass: false, stoppedAt: "not-overfit" });
  // upstream-status is not among the gates a verdict reads, whatever it says.
  const gates = { "upstream-status": fail, loads: pass, "not-overfit": pass, compact: pass, "matches-upstream-tests": pass, "passes-conformance": pass };
  assert.deepEqual(verdict(gates), { pass: true, stoppedAt: null });
  assert.deepEqual(verdict({ ...gates, "matches-upstream-tests": fail }), { pass: false, stoppedAt: "matches-upstream-tests" });
});
