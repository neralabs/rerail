/**
 * `node --import tsx e2e/support/seed-scenario.ts <scenario> <folder> [<now ms>]` with `RERAIL_HOME=<folder>/home`:
 * seeds the scenario as of `now` (default `SEED_NOW`) and writes its manifest to `<folder>/scenario.json`. A process of
 * its own, because seeding gives the stores the seed clock and loads the native engine; `scenario-daemon.ts` runs it.
 */
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { isScenarioName, SCENARIO_NAMES, SEED_NOW } from "./scenario-manifest.js";
import { seedScenario } from "./scenarios.js";

const [scenario, folder, nowText] = process.argv.slice(2);
const now = nowText === undefined ? SEED_NOW : Number(nowText);
if (scenario === undefined || folder === undefined || !isScenarioName(scenario) || !Number.isSafeInteger(now)) {
  process.stderr.write(`usage: seed-scenario <${SCENARIO_NAMES.join("|")}> <folder> [<now ms>]\n`);
  process.exit(2);
}
const manifest = seedScenario(scenario, folder, now);
writeFileSync(join(manifest.root, "scenario.json"), `${JSON.stringify(manifest, null, 2)}\n`);
