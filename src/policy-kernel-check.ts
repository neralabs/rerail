import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { loadNativeBinding, type NativeBinding } from "./native.js";
import type { PolicyRule } from "./policy-schema.js";
import { stableJsonText } from "./stable-json.js";

/** The kernel rejected a policy document that the TypeScript schema accepted. */
export class PolicyCompileError extends Error {
  readonly ruleId: string;
  readonly reason: string;

  constructor(ruleId: string, reason: string) {
    super(`policy rule ${ruleId} does not compile: ${reason}`);
    this.name = "PolicyCompileError";
    this.ruleId = ruleId;
    this.reason = reason;
  }
}

/** The native kernel could not be loaded, so a policy write cannot be checked the way the live catalog will compile it. */
export class PolicyKernelUnavailableError extends Error {
  constructor(ruleId: string, reason: string) {
    super(`cannot validate policy rule ${ruleId}: native kernel unavailable: ${reason}`);
    this.name = "PolicyKernelUnavailableError";
  }
}

/** `<package>/policies/executable-policy-dsl`: `capabilities.json` and the bundled `rules/` documents live here. */
export function bundledPolicyDslDirectory(): string {
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  return resolve(moduleDirectory, "..", "policies", "executable-policy-dsl");
}

let bundledCapabilities: string | undefined;

function capabilitiesJson(): string {
  bundledCapabilities ??= readFileSync(resolve(bundledPolicyDslDirectory(), "capabilities.json"), "utf8");
  return bundledCapabilities;
}

/**
 * Compile a schema-valid policy exactly as the live catalog will (same source text, same bundled capabilities),
 * so a rule the kernel rejects never reaches the catalog and fails every later refresh. Policy evaluation has
 * no fallback without the kernel either, so an unavailable kernel refuses the write rather than storing an
 * unchecked rule.
 */
export function assertPolicyCompiles(rule: PolicyRule): void {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new PolicyCompileError(rule.id, "the document is not JSON representable");
  let binding: NativeBinding;
  let capabilities: string;
  try {
    binding = loadNativeBinding();
    capabilities = capabilitiesJson();
  } catch (error) {
    throw new PolicyKernelUnavailableError(rule.id, error instanceof Error ? error.message : String(error));
  }
  try {
    binding.PolicyDslRuntime.compile(sourceText, "json", capabilities);
  } catch (error) {
    throw new PolicyCompileError(rule.id, error instanceof Error ? error.message : String(error));
  }
}
