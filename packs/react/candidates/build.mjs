// Writes the React pack's policies from specs.mjs: packs/react/rules/<id>.json (ported rules) and
// packs/react/candidates/rules/<id>.json (candidates). The two conformance.json files are maintained by hand and must
// list exactly the ported / candidate specs (id, upstream, policy, capabilities, match), which this script checks.
// Candidates are scored as their own pack: node --import tsx conformance/languages/cli.ts
//   --pack-dir packs/react/candidates --corpus-root <checkout>/.rerail/corpora/react
// Usage: node packs/react/candidates/build.mjs
// Run offline by a maintainer, never in CI.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { SPECS } from "./specs.mjs";

const CANDIDATES = dirname(fileURLToPath(import.meta.url));
const PACK_DIRECTORY = dirname(CANDIDATES);
const REPOSITORY = dirname(dirname(PACK_DIRECTORY));
const EXTENSIONS = ["ts", "mts", "cts", "tsx", "js", "jsx", "mjs", "cjs"];

/** A check's own utilities: those the document's shared define does not already hold. */
function local(spec, check) {
  if (!check.utils) return undefined;
  const own = Object.fromEntries(Object.entries(check.utils).filter(([name]) => !(spec.shared && name in spec.shared)));
  return Object.keys(own).length > 0 ? own : undefined;
}

function document(spec) {
  return {
    format: "rerail.policy/v1",
    id: spec.id,
    revision: 1,
    mode: "steer",
    description: spec.description,
    scope: { include: EXTENSIONS.map(extension => `**/*.${extension}`), exclude: [] },
    clauses: [
      ...(spec.shared ? [{ id: "shared", define: { language: "typescript", utils: spec.shared } }] : []),
      ...spec.checks.map(check => ({
        id: check.id,
        forbid: {
          ast: {
            language: "typescript",
            ...(check.languageOptions ? { languageOptions: check.languageOptions } : {}),
            rule: check.rule,
            ...(local(spec, check) ? { utils: local(spec, check) } : {}),
            ...(check.constraints ? { constraints: check.constraints } : {}),
            ...(check.file ? { file: check.file } : {}),
          },
        },
        violation: {
          anchor: check.anchor ?? "witness.start",
          code: spec.upstream,
          message: check.message,
          ...(check.each ? { each: check.each } : {}),
          suppress: { eslint: { rules: [spec.upstream] } },
        },
      })),
    ],
  };
}

for (const spec of SPECS) {
  const directory = join(spec.candidate ? CANDIDATES : PACK_DIRECTORY, "rules");
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, `${spec.id}.json`), `${JSON.stringify(document(spec), null, 2)}\n`);
}

const manifest = spec => ({ id: spec.id, upstream: { tool: spec.tool, rule: spec.upstream }, policy: `rules/${spec.id}.json`, capabilities: spec.capabilities, match: spec.match });
for (const [directory, specs] of [
  [PACK_DIRECTORY, SPECS.filter(spec => !spec.candidate)],
  [CANDIDATES, SPECS.filter(spec => spec.candidate)],
]) {
  const path = join(directory, "conformance.json");
  const listed = JSON.stringify(JSON.parse(readFileSync(path, "utf8")).rules);
  if (listed !== JSON.stringify(specs.map(manifest))) throw new Error(`${relative(REPOSITORY, path)} rules differ from the specs`);
}
console.log(`wrote ${SPECS.length} policies (${SPECS.filter(spec => !spec.candidate).length} ported)`);
