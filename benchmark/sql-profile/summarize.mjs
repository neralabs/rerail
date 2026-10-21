// Rebuild priorities from a recorded report. The engine and timeline are not rerun.
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { writeReport } from "./report.mjs";
import { summaryMarkdown } from "./summary.mjs";

if (process.argv.length !== 3) {
  throw new Error("Usage: node benchmark/sql-profile/summarize.mjs /existing/profile/directory");
}
const directory = resolve(process.argv[2]);
const report = JSON.parse(readFileSync(join(directory, "profile.json"), "utf8"));
if (report.version !== 1) throw new Error(`Unsupported SQL profile version: ${report.version}`);
writeReport(report, directory);
console.log(summaryMarkdown(report));
console.log(`Reports: ${directory}/summary.md and ${directory}/profile.html`);
