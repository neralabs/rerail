// Records PHP_CodeSniffer 4.0.4's Generic.PHP.ForbiddenFunctions over the sample pack's fixtures and its `mini` corpus.
// Run by hand: PHPCS=<checkout>/.rerail/tools/php/vendor/bin/phpcs node generate.mjs
import { execFileSync } from "node:child_process";
import { readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "4.0.4";
const SNIFF = "Generic.PHP.ForbiddenFunctions";
const RULES = ["Generic.PHP.ForbiddenFunctions.Found", "Generic.PHP.ForbiddenFunctions.FoundWithAlternative"];
const here = dirname(fileURLToPath(import.meta.url));
const phpcs = process.env.PHPCS ?? "phpcs";

const version = execFileSync(phpcs, ["--version"], { encoding: "utf8" });
if (!version.includes(`version ${VERSION} `)) throw new Error(`expected PHP_CodeSniffer ${VERSION}, got ${version}`);

/** PHP_CodeSniffer's JSON report over `files` (absolute), as recording findings relative to `root`. */
function record(root, files) {
  let output;
  try {
    output = execFileSync(phpcs, ["--standard=Generic", `--sniffs=${SNIFF}`, "--report=json", "-q", ...files], { encoding: "utf8" });
  } catch (error) {
    // phpcs exits 1 or 2 when it reports anything.
    output = error.stdout;
  }
  const report = JSON.parse(output);
  return Object.entries(report.files)
    .flatMap(([path, file]) =>
      file.messages.map(message => ({ rule: message.source, path: relative(root, path), line: message.line, column: message.column })),
    )
    .sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.column - b.column);
}

const write = (name, recording) => writeFileSync(join(here, name), `${JSON.stringify(recording, null, 2)}\n`);

const fixtures = join(here, "fixtures");
const fixtureFiles = readdirSync(fixtures).filter(name => name.endsWith(".php")).sort();
write("fixtures.expected.json", {
  tool: "phpcs",
  version: VERSION,
  source: { kind: "fixtures", origin: "rerail (authored sample, not copied upstream)", rules: RULES, files: fixtureFiles },
  findings: record(fixtures, fixtureFiles.map(name => join(fixtures, name))),
});

const corpus = join(here, "..", "..", "corpora", "mini");
const corpusFiles = readdirSync(join(corpus, "src"))
  .filter(name => name.endsWith(".php") && !name.endsWith(".blade.php"))
  .map(name => join(corpus, "src", name));
write("corpus-mini.expected.json", {
  tool: "phpcs",
  version: VERSION,
  source: { kind: "corpus", name: "mini", repository: "fixtures/packs/sample-php/corpora/mini", commit: "fixture", rules: RULES, skipped: [] },
  findings: record(corpus, corpusFiles),
});
