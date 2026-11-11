import type { UpstreamRuleFacts } from "../manifest.js";
import type { RuleRow } from "../report.js";
import type { Check } from "./check.js";

/**
 * upstream-status: the rule is one of the pinned upstream's (gates.yaml `rules` lists it) and the test data holds it
 * (an original unit whose origin names it, or an upstream finding of it), is stable (upstream does not deprecate it),
 * and is recommended (an approved configuration turns it on). Informational: it never stops a rule, and only decides
 * whether a shipped rule starts on (`defaultOn`). `row` is the rule's fixtures row of the conformance report.
 */
export function upstreamStatusChecks(name: string, facts: UpstreamRuleFacts | undefined, row: Pick<RuleRow, "ownUnits" | "upstreamFindings"> | undefined): Check[] {
  if (facts === undefined) {
    return [{ id: "upstream-rule", ok: false, detail: `${name} is not a rule of the pinned upstream: gates.yaml's rules does not list it` }];
  }
  const units = row?.ownUnits ?? 0;
  const findings = row?.upstreamFindings ?? 0;
  const recommended = facts.recommended;
  return [
    {
      id: "upstream-rule",
      ok: units + findings > 0,
      detail: units + findings > 0
        ? `gates.yaml lists it; the test data holds ${units} unit(s) from its upstream tests and ${findings} upstream finding(s) of it`
        : "gates.yaml lists it, but the test data holds no unit from its upstream tests and no upstream finding of it",
    },
    {
      id: "stable",
      ok: facts.deprecated === null,
      detail: facts.deprecated === null ? "upstream does not deprecate it" : `upstream deprecates the rule: ${facts.deprecated}`,
    },
    {
      id: "recommended",
      ok: recommended !== null,
      detail: recommended === null
        ? "not on in any approved upstream configuration"
        : recommended.options.length === 0
          ? `on in ${recommended.config} with default options`
          : `on in ${recommended.config} with options ${JSON.stringify(recommended.options)}`,
    },
  ];
}

/**
 * Whether a rule that ships starts on: every upstream-status check holds (it exists upstream, is stable and is
 * recommended). Otherwise it ships off, as an experimental rule. Whether it ships at all is the blocking gates' call.
 */
export function defaultOn(checks: Check[]): { on: boolean; reason: string } {
  const failing = checks.filter((check) => !check.ok);
  if (failing.length === 0) {
    const recommended = checks.find((check) => check.id === "recommended")!;
    return { on: true, reason: `upstream has it, does not deprecate it, and turns it on: ${recommended.detail.split(";")[0]}` };
  }
  return { on: false, reason: `experimental: ${failing.map((check) => `${check.id} (${check.detail.split(";")[0]})`).join("; ")}` };
}
