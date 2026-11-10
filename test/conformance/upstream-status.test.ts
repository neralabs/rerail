import assert from "node:assert/strict";
import { test } from "vitest";

import { defaultOn, upstreamStatusChecks } from "../../conformance/languages/checks/upstream-status.js";
import type { UpstreamRuleFacts } from "../../conformance/languages/manifest.js";

function failed(checks: { id: string; ok: boolean }[]): string[] {
  return checks.filter((check) => !check.ok).map((check) => check.id);
}

test("upstream-status reads gates.yaml's declared facts and the test data's evidence for the rule", () => {
  const facts = (overrides: Partial<UpstreamRuleFacts> = {}): UpstreamRuleFacts => ({ deprecated: null, recommended: { config: "default", options: [] }, ...overrides });
  const held = { ownUnits: 2, upstreamFindings: 0 };
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts(), held)), []);
  // Findings alone hold it too: a unit with no origin can still carry upstream's findings of the rule.
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts(), { ownUnits: 0, upstreamFindings: 3 })), []);
  assert.deepEqual(upstreamStatusChecks("nera-only", undefined, held), [{ id: "upstream-rule", ok: false, detail: "nera-only is not a rule of the pinned upstream: gates.yaml's rules does not list it" }]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts(), { ownUnits: 0, upstreamFindings: 0 })), ["upstream-rule"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts(), undefined)), ["upstream-rule"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts({ deprecated: "redundant" }), held)), ["stable"]);
  assert.deepEqual(failed(upstreamStatusChecks("b101", facts({ recommended: null }), held)), ["recommended"]);

  const withOptions = upstreamStatusChecks("b101", facts({ recommended: { config: "default", options: [{ skips: [] }] } }), held);
  assert.equal(withOptions.find((check) => check.id === "recommended")!.detail, 'on in default with options [{"skips":[]}]');
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
