import { policyPath } from "./policies.js";
/** Validate recovered candidates through the selected engine's own public schema and compiler. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { loadNativeBinding, resolveNativeBindingPath } from "../../src/engine/index.js";

const { values } = parseArgs({ options: {
  source: { type: "string" }, output: { type: "string" }, through: { type: "string", default: "7" },
} });
if (values.source === undefined || values.output === undefined) throw new Error("Usage: compile-integrated.ts --source <engine checkout> --output <receipt.json> [--through 7]");
const through = Number(values.through);
if (!Number.isInteger(through) || through < 1 || through > 15) throw new Error("--through must select a batch between 1 and 15");
const source = resolve(values.source);
const frontend = await import(pathToFileURL(join(source, "src/engine/code-rule/code-rule-schema.ts")).href);
if (typeof frontend.codeRuleSchema?.parse !== "function") throw new Error("The selected engine has no public code-rule schema");
const capabilities = readFileSync(join(source, "native/src/code_rule_dsl/capabilities.json"), "utf8");
const manifest = JSON.parse(readFileSync("fixtures/host/data-layer-port/batches.json", "utf8")) as { batches: { id: number; checks: string[] }[] };
const checks = manifest.batches.filter(batch => batch.id <= through).flatMap(batch => batch.checks);
const native = loadNativeBinding();
const hash = (value: string | Buffer) => createHash("sha256").update(value).digest("hex");
const rows = checks.map(check => {
  const authored = readFileSync(policyPath("packs/data-layer/candidates/rules", `data-layer-${check.replaceAll("_", "-")}`), "utf8");
  const policySha256 = hash(authored);
  try {
    const policy: unknown = frontend.codeRuleSchema.parse(JSON.parse(authored));
    native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilities);
    const authoredDigest = native.CodeRuleDslRuntime.compile(authored, "json", capabilities).digest();
    return { check, policySha256, authoredDigest, passed: true };
  } catch (error) { return { check, policySha256, passed: false, error: String(error) }; }
});
const report = { format: "rerail.integrated-schema-compatibility/v1", through, checks: checks.length,
  nativeSha256: hash(readFileSync(resolveNativeBindingPath())), capabilitiesSha256: hash(capabilities),
  schemaSha256: hash(readFileSync(join(source, "src/engine/code-rule/code-rule-schema.ts"))),
  failures: rows.filter(row => !row.passed).length, rows };
writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify({ ...report, rows: undefined }, null, 2));
if (report.failures !== 0) process.exitCode = 1;
