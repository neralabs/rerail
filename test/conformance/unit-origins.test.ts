import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { recordingSchema } from "../../conformance/languages/pack.js";
// @ts-expect-error: a JavaScript module without declarations, run by the generators.
import { withOrigins, writeRenameVariants } from "../../conformance/recorders/eslint-record.mjs";

const directories: string[] = [];
afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function fixtures(files: Record<string, string>): string {
  const directory = mkdtempSync(join(tmpdir(), "rerail-unit-origins-"));
  directories.push(directory);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    writeFileSync(join(directory, path), text);
  }
  return directory;
}

const recording = (files: unknown[], units: unknown) => ({
  tool: "t",
  version: "1",
  source: { kind: "fixtures", origin: "https://example.test@0", rules: ["r1", "r2"], files, units },
  findings: [],
});

describe("rename variants and unit origins", () => {
  it("labels each upstream unit with its rules and each kept rename variant with its original", () => {
    const directory = fixtures({
      "r1/001.js": "const foo = 1;\n",
      "r1/002.js": "0;\n",
      "r2/001.js": "let bar = 'baz';\n",
      "r2/package.json": "{}\n",
      "rerail-probe.js": "const mine = 1;\n",
    });
    const units = ["r1/001.js", "r1/002.js", ["r2/001.js", "r2/package.json"], "rerail-probe.js"];
    const origins = new Map([
      ["r1/001.js", ["r1"]],
      ["r1/002.js", ["r1"]],
      ["r2/001.js", ["r2"]],
    ]);
    const { variants, skipped } = writeRenameVariants(directory, units, origins);
    expect(skipped).toEqual([{ of: "r1/002.js", problem: "nothing to rename" }]);
    expect(variants.map((variant: { unit: unknown }) => variant.unit)).toEqual(["renamed/r1/001.js", ["renamed/r2/001.js", "renamed/r2/package.json"]]);
    expect(readFileSync(join(directory, "renamed/r2/package.json"), "utf8")).toBe("{}\n");
    expect(readFileSync(join(directory, "renamed/r1/001.js"), "utf8")).not.toContain("foo");

    const findings = [{ rule: "r1", path: "renamed/r1/001.js", line: 1, column: 7 }];
    const unparsed = new Set(["renamed/r1/001.js"]);
    const recorded = withOrigins(directory, units, origins, variants, unparsed, findings);
    expect(recorded.files).toEqual([...units, ["renamed/r2/001.js", "renamed/r2/package.json"]]);
    expect(recorded.units).toEqual({
      "r1/001.js": { rules: ["r1"] },
      "r1/002.js": { rules: ["r1"] },
      "r2/001.js": { rules: ["r2"] },
      "renamed/r2/001.js": { rules: ["r2"], renamedFrom: "r2/001.js" },
    });
    expect(recorded.findings).toEqual([]);
    expect(existsSync(join(directory, "renamed/r1/001.js"))).toBe(false);
    expect(recordingSchema.safeParse(recording(recorded.files, recorded.units)).success).toBe(true);
  });

  it("refuses an origin for a unit the recording does not list, and a variant of no original", () => {
    expect(recordingSchema.safeParse(recording(["a.js"], { "b.js": { rules: ["r1"] } })).success).toBe(false);
    expect(recordingSchema.safeParse(recording(["a.js", "renamed/a.js"], { "renamed/a.js": { rules: ["r1"], renamedFrom: "a.js" } })).success).toBe(false);
  });
});
