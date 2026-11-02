// Test data archives' provenance.json and LICENSES.md, as the publish workflow's prep (packFrom) requires them.

import { PROVENANCE_FORMAT } from "../../../scripts/s3/pack-fixtures.mjs";

const PROVENANCE = "provenance.json";
const LICENSES = "LICENSES.md";
const PUBLISH = "publish/";
const SAMPLES = "publish/samples/";

const byBytes = (a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));

/** A LICENSES.md for an archive whose files are all Rerail's. */
export const LICENSES_TEXT = "# Licences\n\nEvery file in it is Rerail's.\n";

/** A provenance.json recording each of `paths` (relative to the tool directory, the two outputs left out) as Rerail's. */
export function provenanceText(paths) {
  const files = [...new Set(paths)].filter((path) => path !== PROVENANCE && path !== LICENSES).sort(byBytes);
  return `${JSON.stringify({ format: PROVENANCE_FORMAT, files: Object.fromEntries(files.map((path) => [path, { source: "rerail", how: "authored" }])) }, null, 2)}\n`;
}

/** The two outputs describing an archive of `paths`, by their paths relative to the tool directory. */
export function provenanceFiles(paths) {
  return { [PROVENANCE]: provenanceText(paths), [LICENSES]: LICENSES_TEXT };
}

/**
 * A branch's `edits` (repository path → text) with `directory`'s publish/provenance.json and publish/LICENSES.md
 * restaged for the archive publishing them makes: the locked archive's `base` paths (relative to the tool directory;
 * [] for a replace) less `dropped`, plus the test data `edits` puts under `<directory>/publish/`.
 */
export function restaged(directory, base, edits, dropped = []) {
  const prefix = `${directory}/${PUBLISH}`;
  const published = Object.keys(edits)
    .filter((path) => path.startsWith(prefix) && !path.startsWith(`${directory}/${SAMPLES}`))
    .map((path) => path.slice(prefix.length));
  const gone = new Set(dropped);
  const paths = [...base.filter((path) => !gone.has(path)), ...published];
  return { ...edits, ...Object.fromEntries(Object.entries(provenanceFiles(paths)).map(([path, text]) => [`${prefix}${path}`, text])) };
}
