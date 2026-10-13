// Writes the React pack from specs.mjs: each spec's policy, packs/react/rules/<id>.json (ported) or
// packs/react/candidates/rules/<id>.json (`candidate: true`), and packs/react/conformance.json, the manifest that lists
// every port: the specs and the hand-written HAND_WRITTEN ports. The conformance runner gates the ports under rules/
// and scores and reports the candidates without gating them; the recorders read its corpora.
// Score: node --import tsx conformance/languages/cli.ts --pack react [--fixtures]
// Usage: node packs/react/candidates/build.mjs
// Run offline by a maintainer, never in CI.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PINS } from "../upstream/common.mjs";
import { HAND_WRITTEN, SPECS } from "./specs.mjs";

const CANDIDATES = dirname(fileURLToPath(import.meta.url));
const PACK_DIRECTORY = dirname(CANDIDATES);
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

/** Every capability the pack's ports exercise, in the manifest's order. */
const CAPABILITIES = ["syntax", "scope", "value", "compare", "arguments", "suppression"];
const CORPORA = [
  {
    name: "excalidraw",
    repository: "https://github.com/excalidraw/excalidraw",
    commit: "ed10ac7dca7e40f3f4a31269b4bfba980d0db41e",
    paths: ["packages/excalidraw", "excalidraw-app"],
    exclude: [],
  },
];
const TOOLS = ["eslint-plugin-react", "eslint-plugin-react-hooks"];

const port = spec => ({
  id: spec.id,
  upstream: { tool: spec.tool, rule: spec.upstream },
  policy: `${spec.candidate ? "candidates/" : ""}rules/${spec.id}.json`,
  capabilities: spec.capabilities,
  match: spec.match,
});
const ports = [...SPECS.map(port), ...HAND_WRITTEN];
for (const entry of ports) {
  const undeclared = entry.capabilities.filter(capability => !CAPABILITIES.includes(capability));
  if (undeclared.length > 0) throw new Error(`${entry.id}: capabilities ${undeclared.join(", ")} are not in CAPABILITIES`);
}
const manifest = {
  pack: "react",
  family: "ecma",
  capabilities: CAPABILITIES,
  corpora: CORPORA,
  tools: TOOLS.map(name => ({ name, version: PINS[name] })),
  rules: ports,
};
writeFileSync(join(PACK_DIRECTORY, "conformance.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${SPECS.length} policies (${SPECS.filter(spec => !spec.candidate).length} ported) and conformance.json (${ports.length} ports)`);
