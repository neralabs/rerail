// Writes the React pack from specs.mjs: each spec's policy, packs/react/rules/<id>.json (ported) or
// packs/react/candidates/rules/<id>.json (`candidate: true`), and packs/react/conformance.json, the manifest that lists
// every port: the specs and the hand-written HAND_WRITTEN ports. The conformance runner gates the ports under rules/
// and scores and reports the candidates without gating them.
// Each violation carries the suppression comments its rule honours: `eslint-disable` for the upstream rule, Oxlint
// 1.88.0's for a rule its `react` plugin implements (OXLINT_REACT), and Biome's for the rules Biome marks the same
// (conformance/biome-same-sources.generated.ts, which scripts/generate/packs/biome-suppression.mjs writes). A document
// keeps its committed revision when nothing else changes; a shipped one (rules/) that changes gets the next revision.
// Score: node --import tsx conformance/languages/cli.ts --pack react [--fixtures]
// Usage: node packs/react/candidates/build.mjs
// Run offline by a maintainer, never in CI.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { BIOME_SAME_SOURCES } from "../../../conformance/biome-same-sources.generated.ts";
import { PINS } from "../upstream/common.mjs";
import { HAND_WRITTEN, SPECS } from "./specs.mjs";

const CANDIDATES = dirname(fileURLToPath(import.meta.url));
const PACK_DIRECTORY = dirname(CANDIDATES);
const EXTENSIONS = ["ts", "mts", "cts", "tsx", "js", "jsx", "mjs", "cjs"];
/**
 * The JSX runtimes a React rule runs under: React, or a runtime the file does not settle. A file whose JSX belongs to
 * another library (a `@jsx h` / `@jsxImportSource preact` pragma, `h` imported from Stencil, Preact or Vue, a manifest
 * depending on Vue and not React) is not React code, so no React rule reads it.
 */
const JSX_RUNTIMES = ["react", "unknown"];

/** The rules Oxlint 1.88.0's `react` plugin implements (`oxlint --rules`), by their name without a plugin prefix. */
const OXLINT_REACT = new Set([
  "button-has-type", "capitalized-calls", "checked-requires-onchange-or-readonly", "display-name", "error-boundaries",
  "exhaustive-deps", "exhaustive-effect-dependencies", "forbid-component-props", "forbid-dom-props", "forbid-elements",
  "forward-ref-uses-ref", "function-component-definition", "globals", "hook-use-state", "hooks", "iframe-missing-sandbox",
  "immutability", "incompatible-library", "invariant", "jsx-boolean-value", "jsx-curly-brace-presence",
  "jsx-filename-extension", "jsx-fragments", "jsx-handler-names", "jsx-key", "jsx-max-depth", "jsx-no-comment-textnodes",
  "jsx-no-constructed-context-values", "jsx-no-duplicate-props", "jsx-no-literals", "jsx-no-script-url",
  "jsx-no-target-blank", "jsx-no-undef", "jsx-no-useless-fragment", "jsx-pascal-case", "jsx-props-no-spread-multi",
  "jsx-props-no-spreading", "memo-dependencies", "no-array-index-key", "no-children-prop", "no-clone-element", "no-danger",
  "no-danger-with-children", "no-deriving-state-in-effects", "no-did-mount-set-state", "no-did-update-set-state",
  "no-direct-mutation-state", "no-find-dom-node", "no-is-mounted", "no-multi-comp", "no-namespace",
  "no-object-type-as-default-prop", "no-react-children", "no-redundant-should-component-update", "no-render-return-value",
  "no-set-state", "no-string-refs", "no-this-in-sfc", "no-unescaped-entities", "no-unknown-property", "no-unsafe",
  "no-unstable-nested-components", "no-will-update-set-state", "only-export-components", "prefer-es6-class",
  "prefer-function-component", "preserve-manual-memoization", "purity", "react-in-jsx-scope", "refs",
  "require-render-return", "rule-suppression", "rules-of-hooks", "self-closing-comp", "set-state-in-effect",
  "set-state-in-render", "state-in-constructor", "static-components", "style-prop-object", "syntax", "todo",
  "unsupported-syntax", "use-memo", "void-dom-elements-no-children", "void-use-memo",
]);

/** The suppression comments a violation of the upstream rule `upstream` honours, as the tools that implement it read them. */
function suppress(upstream) {
  const name = upstream.slice(upstream.indexOf("/") + 1);
  const biome = BIOME_SAME_SOURCES[upstream];
  return {
    eslint: { rules: [upstream] },
    ...(OXLINT_REACT.has(name) ? { oxlint: { rules: [name] } } : {}),
    ...(biome === undefined ? {} : { biome: { rules: [...biome] } }),
  };
}

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
            file: { ...check.file, jsxRuntime: JSX_RUNTIMES },
          },
        },
        violation: {
          anchor: check.anchor ?? "witness.start",
          code: spec.upstream,
          message: check.message,
          ...(check.each ? { each: check.each } : {}),
          suppress: suppress(spec.upstream),
        },
      })),
    ],
  };
}

/** `next` with the revision `committed` holds when the rest is unchanged, else one past it for a shipped document. */
function revised(next, committed, shipped) {
  if (committed === undefined) return next;
  const same = JSON.stringify({ ...committed, revision: 0 }) === JSON.stringify({ ...next, revision: 0 });
  return { ...next, revision: same || !shipped ? committed.revision : committed.revision + 1 };
}

for (const spec of SPECS) {
  const directory = join(spec.candidate ? CANDIDATES : PACK_DIRECTORY, "rules");
  const other = join(spec.candidate ? PACK_DIRECTORY : CANDIDATES, "rules", `${spec.id}.json`);
  const path = join(directory, `${spec.id}.json`);
  const committed = [path, other].filter(existsSync).map(file => JSON.parse(readFileSync(file, "utf8")))[0];
  mkdirSync(directory, { recursive: true });
  writeFileSync(path, `${JSON.stringify(revised(document(spec), committed, !spec.candidate), null, 2)}\n`);
  // A spec that moves between candidates/rules/ and rules/ leaves no copy behind.
  rmSync(other, { force: true });
}

/** Every capability the pack's ports exercise, in the manifest's order. */
const CAPABILITIES = ["syntax", "scope", "value", "compare", "arguments", "suppression"];
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
  tools: TOOLS.map(name => ({ name, version: PINS[name] })),
  rules: ports,
};
writeFileSync(join(PACK_DIRECTORY, "conformance.json"), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`wrote ${SPECS.length} policies (${SPECS.filter(spec => !spec.candidate).length} ported) and conformance.json (${ports.length} ports)`);
