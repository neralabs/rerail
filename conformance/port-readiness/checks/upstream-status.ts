import type { Check, PackAdapter } from "../types.js";

/**
 * upstream-status: the rule exists upstream with an implementation and tests, is stable (upstream does not deprecate
 * it), and is recommended (an approved configuration turns it on). Informational: it never stops a rule, and only
 * decides whether a shipped rule starts on (`defaultOn`).
 */
export function upstreamStatusChecks(name: string, adapter: PackAdapter): Check[] {
  const upstream = adapter.rule(name);
  if (upstream === undefined) {
    return [{ id: "upstream-rule", ok: false, detail: `${name} is not a rule of the pinned upstream: no implementation and no rule of that name in the runtime` }];
  }
  const recommended = upstream.recommended;
  return [
    {
      id: "upstream-rule",
      ok: upstream.implementation.length > 0 && upstream.tests.length > 0,
      detail: `implementation: ${upstream.implementation.join(", ") || "(none)"}; ${upstream.tests.length} upstream test file(s)`,
    },
    {
      id: "stable",
      ok: upstream.deprecation === null,
      detail: upstream.deprecation === null ? "upstream metadata carries no deprecation" : `upstream deprecates the rule: ${upstream.deprecation}`,
    },
    {
      id: "recommended",
      ok: recommended !== null,
      detail: recommended === null
        ? "not on in any approved upstream configuration"
        : recommended.options.length === 0
          ? `on in ${recommended.config} with default options`
          : `on in ${recommended.config} with options ${JSON.stringify(recommended.options)}; matches-upstream-tests and matches-real-code check the rule with these options`,
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
