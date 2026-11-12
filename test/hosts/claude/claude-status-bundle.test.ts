import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import { REQUIRED_NODE_MAJOR } from "../../../src/engine/index.js";
import { bundleTarget, OLDER_NODE_VERSION, olderNodeArguments } from "../../support/platform/older-node.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

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

  it("says which Node it needs when an older Node starts it", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-status-bundle-"));
    try {
      await writeFile(join(root, "package.json"), "{}\n");
      const entry = "src/hosts/claude/claude-status.ts";
      // The same options as `npm run build:claude`, its target included.
      await build({
        entryPoints: [entry], bundle: true, platform: "node", format: "esm", external: ["better-sqlite3"],
        ...await bundleTarget("build:claude", entry), outdir: join(root, "dist", "hosts", "claude"), logLevel: "silent",
      });
      const status = spawnSync(process.execPath, [...olderNodeArguments(), join(root, "dist", "hosts", "claude", "claude-status.js")], {
        input: JSON.stringify({ session_id: "session", cwd: root }), encoding: "utf8",
      });
      expect(status.stdout, status.stderr).toContain(`needs Node ${REQUIRED_NODE_MAJOR} or newer (this is ${OLDER_NODE_VERSION})`);
    } finally {
      await removeFolder(root);
    }
  }, 60_000);
});
