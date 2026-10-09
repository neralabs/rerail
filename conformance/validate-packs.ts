/**
 * `npm run validate:packs -- [--pack <id>]... [--write]`: every pack under packs/ (or the named ones) through checks 1
 * to 9, one summary per pack on stdout. `--write` also writes each pack's `rerail.pack-validation/v1` report to
 * `.rerail/reports/<id>-validation.{json,md}` (gitignored), deterministic for one checkout: no date, no timing. Exits 1
 * when a pack fails. CI does not write reports; scrub.json is what CI enforces (test/packs.test.ts,
 * scripts/check-scrub.mjs).
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { compareDeterministicStrings } from "../src/stable-json.js";
import { PACK_CHECKS } from "./checks.js";
import { PACKS_ROOT, RERAIL_ROOT, REPORTS_DIRECTORY } from "./languages/pack.js";
import { validatePack } from "./pack-validation.js";
import { renderValidationReport, validationReport, type ValidationReport } from "./validation-report.js";

const { values } = parseArgs({ options: { pack: { type: "string", multiple: true }, write: { type: "boolean", default: false } }, strict: true });

// A written report names the checkout whose engine ran; an engine loaded from elsewhere is not that checkout's.
const foreign = ["RERAIL_ENGINE_PATH", "RERAIL_CODE_RULE_PROFILE"].filter(name => process.env[name] !== undefined);
if (values.write && foreign.length > 0) {
  console.error(`validate:packs --write: unset ${foreign.join(" and ")}; a written report is of this checkout's own engine`);
  process.exit(2);
}

const all = readdirSync(PACKS_ROOT, { withFileTypes: true })
  .filter(entry => entry.isDirectory())
  .map(entry => entry.name)
  .sort(compareDeterministicStrings);
const unknown = (values.pack ?? []).filter(id => !all.includes(id));
if (unknown.length > 0) {
  console.error(`validate:packs: no pack ${unknown.join(", ")} under packs/`);
  process.exit(2);
}
const packs = values.pack === undefined ? all : all.filter(id => values.pack!.includes(id));

const git = (...args: string[]): string => execFileSync("git", ["-C", RERAIL_ROOT, ...args], { encoding: "utf8" });
const engine: ValidationReport["engine"] = { commit: git("rev-parse", "HEAD").trim(), dirty: git("status", "--porcelain").trim().length > 0 };

let failed = false;
for (const id of packs) {
  const report = validationReport(await validatePack(join(PACKS_ROOT, id)), engine);
  const { summary } = report;
  failed ||= !report.pass;
  const stops = PACK_CHECKS.filter(check => summary.stoppedAt[check.name] > 0).map(check => `${check.number} ${check.name}: ${summary.stoppedAt[check.name]}`);
  console.log(
    `${report.pass ? "PASS" : "FAIL"} ${id}: ${summary.rules} rules, ${summary.ok} ok, ${summary.scrubbed} scrubbed, ${summary.failed} failed, ${summary.notSupportedYet} not supported yet; ${summary.notJudged} with a check not judged; ${report.packFailures.length} pack failure(s)${stops.length === 0 ? "" : `; stopped at ${stops.join(", ")}`}`,
  );
  if (values.write) {
    if (!existsSync(REPORTS_DIRECTORY)) mkdirSync(REPORTS_DIRECTORY, { recursive: true });
    const base = join(REPORTS_DIRECTORY, `${id}-validation`);
    writeFileSync(`${base}.json`, `${JSON.stringify(report, null, 2)}\n`);
    writeFileSync(`${base}.md`, renderValidationReport(report));
    console.log(`  wrote ${base}.json and .md`);
  }
}
process.exit(failed ? 1 : 0);
