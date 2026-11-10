import { readFile } from "node:fs/promises";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";
import { z } from "zod";

const manifestSchema = z.object({ scripts: z.record(z.string(), z.string()), bin: z.object({ rerail: z.string() }) });

/**
 * Built by tsc, `rerail version` resolved and read 667 modules before printing one line: 1.5 s and 262 MB on a Windows
 * laptop, and `rerail status` and `rerail session` about 1.7 s and 290 MB each. The command is one esbuild bundle.
 */
describe("the rerail command's bundle", () => {
  it("is built as one bundle at the path the package's bin names", async () => {
    const { scripts, bin } = manifestSchema.parse(JSON.parse(await readFile("package.json", "utf8")));
    expect(bin.rerail).toBe("dist/cli/cli.js");
    expect(scripts["build:ts"]).toMatch(/esbuild src\/cli\/cli\.ts --bundle [^&]*--outfile=dist\/cli\/cli\.js/u);
  });

  it("holds no module that runs itself when it is the launched script", async () => {
    // Each of these starts when its own file is argv[1]; inside the bundle the bundle is argv[1], so each would run in
    // every `rerail` command.
    const selfStarting = [
      "src/hosts/claude/claude-hook.ts", "src/hosts/claude/claude-status.ts", "src/hosts/codex/codex-hook.ts",
      "src/hosts/cursor/cursor-hook.ts", "src/harness/event-stream.ts", "src/harness/hook-launcher.ts", "src/cli/daemon-entry.ts",
    ];
    const { metafile } = await build({
      entryPoints: ["src/cli/cli.ts"], bundle: true, platform: "node", format: "esm", external: ["better-sqlite3"],
      write: false, metafile: true, logLevel: "silent",
    });
    expect(Object.keys(metafile.inputs).filter(path => selfStarting.includes(path))).toEqual([]);
  }, 60_000);
});
