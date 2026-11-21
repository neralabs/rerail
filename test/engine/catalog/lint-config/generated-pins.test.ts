/**
 * Set-up's linter tables (src/engine/catalog/lint-config/generated/) are each generated at one release of every tool
 * they are read from, and say which in `UPSTREAM_VERSIONS`. A reader deciding a port by a table at another release than
 * the pack pins can call a rule on or off where the linter at the pin decides otherwise, so each release must be the
 * pinned one: a gated tool-pack's (`<pack>/<tool>`) is its gates.yaml `upstream.version`; oxlint's, the release Rerail's
 * `oxlint-disable` port follows (native/src/code_rule_dsl/suppress/oxlint.rs, at oxc tag `oxlint_v<version>`); Biome's,
 * the release its `biome-ignore` port and the packs' `suppress.biome` are generated from
 * (scripts/generate/packs/biome-suppression.mjs's `PIN`). A bump moves the pins and lists the tables for regeneration
 * (scripts/upstream/bump.ts); this test fails until they are.
 */
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "vitest";

import { gatedToolPacks, readGatesManifest } from "../../../../conformance/languages/manifest.js";
import { UPSTREAM_VERSIONS as BIOME } from "../../../../src/engine/catalog/lint-config/generated/biome-rules.generated.js";
import { UPSTREAM_VERSIONS as ESLINT } from "../../../../src/engine/catalog/lint-config/generated/eslint-presets.generated.js";
import { UPSTREAM_VERSIONS as OXLINT } from "../../../../src/engine/catalog/lint-config/generated/oxlint-rules.generated.js";
import { UPSTREAM_VERSIONS as PHPCS } from "../../../../src/engine/catalog/lint-config/generated/phpcs-standards.generated.js";
import { UPSTREAM_VERSIONS as RUFF } from "../../../../src/engine/catalog/lint-config/generated/ruff-rules.generated.js";

const ROOT = fileURLToPath(new URL("../../../../", import.meta.url));
const DIRECTORY = "src/engine/catalog/lint-config/generated";

/**
 * Each table's releases and what it is generated from: every gated tool-pack of an adapter whose ports its reader
 * decides by the table, or a tool no tool-pack gates. A new table must say which here.
 */
const TABLES: Readonly<Record<string, { readonly versions: Readonly<Record<string, string>>; readonly from: { readonly adapter: string } | { readonly tool: string } }>> = {
  "biome-rules.generated.ts": { versions: BIOME, from: { tool: "biome" } },
  "eslint-presets.generated.ts": { versions: ESLINT, from: { adapter: "eslint" } },
  "oxlint-rules.generated.ts": { versions: OXLINT, from: { tool: "oxlint" } },
  "phpcs-standards.generated.ts": { versions: PHPCS, from: { adapter: "phpcs" } },
  "ruff-rules.generated.ts": { versions: RUFF, from: { adapter: "ruff" } },
};

/** The release `pattern` (its group 1) names in the repository file `path`. */
function pinIn(path: string, pattern: RegExp): string {
  const version = pattern.exec(readFileSync(join(ROOT, path), "utf8"))?.[1];
  assert.ok(version !== undefined, `${path} names no release matching ${pattern}`);
  return version;
}

/** Every pinned release a table may be generated from, by gated tool-pack id or tool, with the adapter reading it. */
function pinnedReleases(): Map<string, { readonly version: string; readonly adapter?: string }> {
  const pins = new Map<string, { version: string; adapter?: string }>([
    ["oxlint", { version: pinIn("native/src/code_rule_dsl/suppress/oxlint.rs", /at oxc tag `oxlint_v([^`]+)`/u) }],
    ["biome", { version: pinIn("scripts/generate/packs/biome-suppression.mjs", /^const PIN = "([^"]+)";$/mu) }],
  ]);
  for (const { pack, tool } of gatedToolPacks(ROOT)) {
    const manifest = readGatesManifest(ROOT, pack, tool);
    pins.set(manifest.id, { version: manifest.upstream.version, adapter: manifest.adapter });
  }
  return pins;
}

const pins = pinnedReleases();

test("every table says what it is generated from", () => {
  assert.deepEqual(readdirSync(join(ROOT, DIRECTORY)).filter((name) => name.endsWith(".generated.ts")).sort(), Object.keys(TABLES).sort());
});

test.each(Object.entries(TABLES))("%s is generated at the releases the repository pins", (name, { versions, from }) => {
  // Every gated tool-pack its reader decides by the table, or the one tool no tool-pack gates; nothing else.
  const expected = "tool" in from ? [from.tool] : [...pins].filter(([, pin]) => pin.adapter === from.adapter).map(([id]) => id).sort();
  assert.ok(expected.length > 0, `${name}: no gated tool-pack has adapter ${JSON.stringify(from)}`);
  assert.deepEqual(Object.keys(versions).sort(), expected, `${DIRECTORY}/${name}: UPSTREAM_VERSIONS names other tools than it is read for`);
  for (const [key, version] of Object.entries(versions)) {
    assert.equal(version, pins.get(key)!.version, `${DIRECTORY}/${name} is generated at ${key} ${version}, but the repository pins ${pins.get(key)!.version}: regenerate it (scripts/generate/README.md)`);
  }
});
