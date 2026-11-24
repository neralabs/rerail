import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";
import { NEXT_PROBES_ROOT, renderNextProbes } from "../../../scripts/generate/packs/next-probes.mjs";

/** Every file under `folder`, by its `/`-separated path relative to `root`. */
function filesUnder(root: string, folder = root): string[] {
  return readdirSync(folder, { withFileTypes: true }).flatMap((entry) => {
    const path = join(folder, entry.name);
    return entry.isDirectory() ? filesUnder(root, path) : [relative(root, path).split(sep).join("/")];
  });
}

// The probe pack's fixtures are a fake Next.js checkout whose package.json files pin next on purpose. A Dependabot
// security update, or any hand edit, that changes one leaves the generator and its output apart.
describe("the Next probe pack (packs/next/candidates/probes)", () => {
  it("is what its generator writes (node scripts/generate/packs/next-probes.mjs)", () => {
    const rendered = renderNextProbes();
    expect(filesUnder(NEXT_PROBES_ROOT).sort()).toEqual([...rendered.keys()].sort());
    for (const [path, text] of rendered) expect(readFileSync(join(NEXT_PROBES_ROOT, path), "utf8"), path).toBe(text);
  });
});
