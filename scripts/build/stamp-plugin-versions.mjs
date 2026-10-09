#!/usr/bin/env node
// Stamps package.json's version into the Claude, Codex, and Cursor plugin manifests, or restores their
// versionless source form. npm runs `stamp` as prepack and `restore` as postpack, so packed tarballs carry
// the release version while the checkout never does. A pack that fails after prepack skips postpack and
// leaves the manifests stamped; `node scripts/stamp-plugin-versions.mjs restore` (idempotent) cleans them up.
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const MANIFESTS = [".claude-plugin", ".codex-plugin", ".cursor-plugin"].map((directory) => join(directory, "plugin.json"));

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function withVersion(manifest, version) {
  const { name, version: _previous, ...rest } = manifest;
  return version === undefined ? { name, ...rest } : { name, version, ...rest };
}

function stampPluginVersions(root) {
  const { version } = readJson(join(root, "package.json"));
  if (typeof version !== "string" || version.length === 0) throw new Error(`${join(root, "package.json")} has no version`);
  for (const manifest of MANIFESTS) writeJson(join(root, manifest), withVersion(readJson(join(root, manifest)), version));
}

function restorePluginVersions(root) {
  for (const manifest of MANIFESTS) writeJson(join(root, manifest), withVersion(readJson(join(root, manifest)), undefined));
}

const [mode, root = process.cwd()] = process.argv.slice(2);
if (mode === "stamp") stampPluginVersions(resolve(root));
else if (mode === "restore") restorePluginVersions(resolve(root));
else {
  process.stderr.write("usage: stamp-plugin-versions.mjs stamp|restore [package-root]\n");
  process.exit(2);
}
