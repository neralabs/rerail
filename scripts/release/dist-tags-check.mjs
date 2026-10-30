#!/usr/bin/env node
// After a release run publishes or promotes, lists each package's npm dist-tags in the run summary and fails unless
// the dist-tag it was released under (`latest` unless named) points at the version, and, for a release under `latest`
// or `next`, its `next` is set and not older than its `latest` (distTagReport in dist-tags.mjs). It reads them with
// `npm dist-tag ls`, which sees a publish straight away; a package not there yet is still read again, for up to about
// two minutes, before the check fails.
//
//   node scripts/release/dist-tags-check.mjs <name>@<version>[:<dist-tag>] ...   (or $RELEASED, the same, space-separated)
import { appendFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { distTagReport, parseExpectation, readDistTags } from "./dist-tags.mjs";

const ATTEMPTS = 8;
const PAUSE_MS = 15_000;

const args = process.argv.slice(2);
const expected = (args.length > 0 ? args : (process.env.RELEASED ?? "").split(/\s+/u).filter((entry) => entry !== "")).map(parseExpectation);
if (expected.length === 0) throw new Error("no published package to check: name <name>@<version>[:<dist-tag>] or set RELEASED");

let report;
for (let attempt = 1; ; attempt += 1) {
  const distTags = await readDistTags(expected.map(({ name }) => name), { fresh: true });
  report = distTagReport(expected, (name) => distTags.get(name) ?? null);
  if (report.failures.length === 0 || attempt === ATTEMPTS) break;
  console.log(`attempt ${attempt}: ${report.failures.join("; ")}; reading again in ${PAUSE_MS / 1000}s`);
  await sleep(PAUSE_MS);
}

console.log(report.table);
if (process.env.GITHUB_STEP_SUMMARY !== undefined) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### Published\n\n${report.table}\n`);
for (const failure of report.failures) console.log(`::error title=Dist-tag::${failure}`);
if (report.failures.length > 0) process.exitCode = 1;
