#!/usr/bin/env node
// Audit normal candidate lowering without submitting candidate SQL or selecting another engine.
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { loadNativeBinding } from "../../src/engine/index.ts";
const [source, output] = process.argv.slice(2);
if (!source || !output) throw new Error("Usage: node --import tsx scripts/profiling/frontier-audit.mjs POLICY.json OUTPUT.json");
if (resolve(source) === resolve(output)) throw new Error("Output must not overwrite the policy");
const caps = readFileSync(new URL("../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const rule = loadNativeBinding().CodeRuleDslRuntime.compile(readFileSync(source, "utf8"), "json", caps);
if (typeof rule.candidateFrontierAudit !== "function") throw new Error("This addon does not expose the candidate frontier audit; rebuild the addon");
const report = JSON.parse(rule.candidateFrontierAudit());
writeFileSync(output, JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify({ rule: report.rule, deterministic: report.deterministic, variants: report.languages.map(l => l.variants.map(v => ({ order: v.order, stages: v.stages, broadened: v.admissions.filter(a => a[2]).length }))) }));
if (!report.deterministic) process.exitCode = 1;
