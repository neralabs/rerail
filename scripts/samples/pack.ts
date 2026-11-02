/**
 * Stages a sample set for publishing: its archive entries (`archiveEntries`, scripts/samples/samples.ts) go into the
 * tool-pack's tracked `publish/samples/`, with a `REUSE.toml` licensing each sample under its source's licence. The
 * publish workflow packs them (`publishedSamples`, scripts/samples/publish.ts) and writes `samples.lock`; the
 * contributor edits `SAMPLES.md` in the same pull request.
 */
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { LABELS_SUFFIX, LICENSES_FILE, sampleLabelsSchema } from "./format.js";
import { PUBLISH_REUSE, publishedSamples } from "./publish.js";
import { byText, type ToolPack } from "./samples.js";

/** A tool-pack's staged samples, relative to the checkout. */
export const publishSamplesDirectory = ({ pack, tool }: ToolPack): string => join("packs", pack, "upstream", tool, "publish", "samples");

interface Entry {
  path: string;
  bytes: Buffer;
}

/**
 * The staged copy's REUSE.toml: each sample (`<sample-id>/**`) under its labels' `source.license`, copyright its
 * repository's contributors; LICENSES.md, the set's own compilation, as the repository's other files are.
 */
export function samplesReuseToml(entries: readonly Entry[]): string {
  const annotation = (path: string, copyright: string, licence: string): string[] =>
    ["", "[[annotations]]", `path = ${JSON.stringify(path)}`, `SPDX-FileCopyrightText = ${JSON.stringify(copyright)}`, `SPDX-License-Identifier = ${JSON.stringify(licence)}`];
  const lines = ["# Licensing of the staged samples only; never archived (scripts/samples/pack.ts).", "version = 1"];
  if (entries.some(entry => entry.path === LICENSES_FILE)) lines.push(...annotation(LICENSES_FILE, "2026 Nera Software Inc.", "Apache-2.0"));
  const labels = entries
    .filter(entry => entry.path.endsWith(LABELS_SUFFIX) && entry.path.includes("/"))
    .map(entry => ({ id: entry.path.slice(0, entry.path.indexOf("/")), source: sampleLabelsSchema.parse(JSON.parse(entry.bytes.toString("utf8"))).source }))
    .sort((a, b) => byText(a.id, b.id));
  for (const { id, source } of labels) lines.push(...annotation(`${id}/**`, `${source.repo} contributors`, source.license));
  return `${lines.join("\n")}\n`;
}

/**
 * Replaces the tool-pack's `publish/samples/` in the checkout at `root` with `entries` and their REUSE.toml: the
 * samples to add or replace in the published set (all of it for a `replace` publish). Checked as prep checks them first.
 */
export function writeSampleSet(root: string, toolPack: ToolPack, entries: readonly Entry[]): void {
  publishedSamples(entries);
  const directory = join(root, publishSamplesDirectory(toolPack));
  rmSync(directory, { recursive: true, force: true });
  for (const { path, bytes } of entries) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    writeFileSync(join(directory, path), bytes);
  }
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, PUBLISH_REUSE), samplesReuseToml(entries));
}
