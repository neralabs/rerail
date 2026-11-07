// `npm run ui:scenario -- <scenario> [--port N]` / `just ui-scenario <scenario>`: seed one of the dashboard's browser-test
// scenarios (e2e/support/scenarios.ts) into a fresh folder under the system temp directory, run the dashboard's daemon
// over it in the foreground (`--dev`: edits to ui/ show on reload) and print its URL, for design review. The history is
// dated back from now (the browser tests use the fixed seed clock instead), so a real browser shows it as recent.
// Ctrl-C stops it; the next run seeds the scenario again. Nothing is written to ~/.rerail or this checkout.
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { scenarioDashboardUrl, seedScenarioFolder, startScenarioDaemon } from "../../e2e/support/scenario-daemon.js";
import { isScenarioName, MINUTE, SCENARIO_NAMES } from "../../e2e/support/scenario-manifest.js";

const USAGE = `usage: ui-scenario <${SCENARIO_NAMES.join("|")}> [--port N]`;

const { positionals, values } = parseArgs({ allowPositionals: true, options: { port: { type: "string" } } });
const [scenario] = positionals;
const port = values.port === undefined ? 0 : Number(values.port);
if (scenario === undefined || !isScenarioName(scenario) || positionals.length > 1 || !Number.isInteger(port)) {
  process.stderr.write(`${USAGE}\n`);
  process.exit(2);
}

const folder = join(tmpdir(), "rerail-ui-scenario", scenario);
rmSync(folder, { recursive: true, force: true });
process.stdout.write(`Seeding ${scenario} into ${folder}\n`);
const now = Date.now() - (Date.now() % MINUTE);
const manifest = await seedScenarioFolder(scenario, folder, now);
const running = await startScenarioDaemon(manifest, { port, inheritOutput: true });

process.stdout.write([
  "",
  `Dashboard: ${scenarioDashboardUrl(running.origin, manifest.primary)}`,
  ...manifest.repositories.slice(1).map(repository => `  also ${repository.displayName}: ${scenarioDashboardUrl(running.origin, repository)}`),
  "Ctrl-C stops the daemon.",
  "",
].join("\n"));

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.once(signal, () => {
    void running.stop().then(() => process.exit(0));
  });
}
