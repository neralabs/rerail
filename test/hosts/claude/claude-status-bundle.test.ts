import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";
import { z } from "zod";

const manifestSchema = z.object({ scripts: z.record(z.string(), z.string()) });

/**
 * Claude Code runs the status line every five seconds in each session. Run from tsc's output it resolved and read
 * 117 modules on every run, and through the daemon's own response contract it linked Drizzle, the storage schema and
 * every code-rule schema: about 300 ms of CPU and 86 MB per run on a Windows laptop, against 80 ms for Node alone.
 */
describe("the Claude Code status line's bundle", () => {
  it("is built as one bundle", async () => {
    const { scripts } = manifestSchema.parse(JSON.parse(await readFile("package.json", "utf8")));
    expect(scripts["build:claude"]).toContain("src/hosts/claude/claude-status.ts");
    expect(scripts["build:claude"]).toContain("--bundle");
  });

  it("links neither Drizzle, the storage schema, the code-rule schemas nor the SQLite reader", async () => {
    // The same options as `npm run build:claude`.
    const { metafile } = await build({
      entryPoints: ["src/hosts/claude/claude-status.ts"], bundle: true, platform: "node", format: "esm",
      external: ["better-sqlite3"], write: false, metafile: true, logLevel: "silent",
    });
    const inputs = Object.keys(metafile.inputs);
    const imported = Object.values(metafile.inputs).flatMap(input => input.imports.map(entry => entry.path));
    expect(inputs.filter(path => /node_modules\/drizzle-(orm|zod)\//u.test(path))).toEqual([]);
    expect(imported.filter(path => /^(drizzle-orm|better-sqlite3)(\/|$)/u.test(path))).toEqual([]);
    expect(inputs).not.toContain("src/engine/storage/schema.ts");
    expect(inputs).not.toContain("src/engine/code-rule/code-rule-schema.ts");
    // Read only when no daemon answers, from the built package at run time.
    expect(inputs).not.toContain("src/runtime/repository-start.ts");
  }, 60_000);
});
