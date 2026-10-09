/**
 * What every conformance harness (conformance/tslint, conformance/languages, the pack validator and the T1 census)
 * shares about the engine it scores: core's capabilities and which engine failure codes are not failures.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { bundledCodeRuleDslDirectory } from "../src/engine/index.js";

let capabilities: string | undefined;
/** Core's `code-rule-dsl/capabilities.json`, as the catalog compiles code rules against it. */
export function capabilitiesJson(): string {
  capabilities ??= readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  return capabilities;
}

/**
 * `source-partial` says the file has syntax errors and AST witnesses were reported only outside them. That is
 * the engine working as designed, so it is not an engine failure: missed violations inside the error region already
 * show up as FNs. `source-incomplete` (a rule unsound under errors skipped the file) stays an engine failure.
 */
export const INFORMATIONAL_FAILURE_CODES: ReadonlySet<string> = new Set(["source-partial"]);
