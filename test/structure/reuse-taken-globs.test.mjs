// REUSE.toml: every pack-path glob of a "Taken from it" table (packs/<pack>/licenses/<project>/UPSTREAM.md) is
// annotated, and the annotation REUSE applies to a file under it, at its pulled path and its publish/ twin, carries the
// project's licence: `<licence>` for a verbatim glob, `<licence> AND Apache-2.0` for one Rerail adapted. The files Rerail
// wrote among them are carved out by later annotations with names no glob's own file takes.

import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseReuseToml } from "../../scripts/check/check-structure/metrics/07-pack-licence-annotations.mjs";
import { licenceAlternatives, PUBLISH_DIRECTORY, readUpstreamSources } from "../../scripts/s3/pack-fixtures.mjs";

const ROOT = new URL("../../", import.meta.url);
const REUSE = readFileSync(new URL("REUSE.toml", ROOT), "utf8");
/** A tool-pack path's parts: the pack and tool, then the path within the tool directory. */
const DATA = /^(packs\/[^/]+\/upstream\/[^/]+\/)(.+)$/u;

/** Every licenses/<project>/UPSTREAM.md as `{ record, licence, globs }`, the globs as repository paths. */
function upstreamRecords() {
  return readdirSync(new URL("packs/", ROOT), { withFileTypes: true })
    .filter((pack) => pack.isDirectory())
    .flatMap((pack) => {
      const licenses = new URL(`packs/${pack.name}/licenses/`, ROOT);
      let projects;
      try {
        projects = readdirSync(licenses, { withFileTypes: true }).filter((entry) => entry.isDirectory());
      } catch {
        return [];
      }
      return projects.flatMap((project) => {
        const record = `packs/${pack.name}/licenses/${project.name}/UPSTREAM.md`;
        let text;
        try {
          text = readFileSync(new URL(record, ROOT), "utf8");
        } catch {
          return [];
        }
        const { licence, source, taken } = readUpstreamSources(text);
        return [{ record, licence: licence ?? source?.licence, globs: taken.map(({ glob }) => `packs/${pack.name}/${glob}`) }];
      });
    });
}

/** Whether every way to satisfy `expression` satisfies `required` too: it names all of some choice of `required`'s terms. */
function includesLicence(expression, required) {
  const choices = licenceAlternatives(required) ?? [];
  const alternatives = licenceAlternatives(expression ?? "");
  return alternatives !== undefined && choices.length > 0 && alternatives.every((terms) => choices.some((choice) => choice.every((term) => terms.includes(term))));
}

/** The licence of the last annotation (REUSE.toml's last match wins) matching `path`, or null when none does. */
function resolvedLicence(annotations, path) {
  return annotations.findLast((annotation) => annotation.patterns.some((pattern) => pattern.test(path)))?.license ?? null;
}

describe("REUSE.toml", () => {
  it("annotates every Taken from it glob, and its publish/ twin, with its project's licence", () => {
    const annotations = parseReuseToml(REUSE);
    const records = upstreamRecords();
    expect(records.flatMap(({ globs }) => globs).length).toBeGreaterThan(0);
    const wrong = records.flatMap(({ record, licence, globs }) =>
      globs.flatMap((glob) => {
        // A file the glob names: each run of `*` read as one ordinary name.
        const file = glob.replace(/\*+/gu, "x");
        return [file, file.replace(DATA, `$1${PUBLISH_DIRECTORY}/$2`)]
          .map((path) => ({ path, resolved: resolvedLicence(annotations, path) }))
          .filter(({ resolved }) => !includesLicence(resolved, licence))
          .map(({ path, resolved }) => `${glob} (${record}, ${licence}): ${path} resolves ${resolved}`);
      }),
    );
    expect(wrong).toEqual([]);
  });
});
