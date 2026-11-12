import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";
import { z } from "zod";
import { REQUIRED_NODE_MAJOR } from "../../src/engine/index.js";
import { bundleTarget, OLDER_NODE_VERSION, olderNodeArguments } from "../support/platform/older-node.js";
import { removeFolder } from "../support/platform/remove-folder.js";

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

  it("refuses an older Node with set-up's message", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cli-bundle-"));
    try {
      await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.0.0" }));
      // The bundle leaves the SQLite binding out, and refusing a Node never opens a database: an empty module stands in.
      const binding = join(root, "node_modules", "better-sqlite3");
      await mkdir(binding, { recursive: true });
      await writeFile(join(binding, "package.json"), JSON.stringify({ name: "better-sqlite3", main: "index.js" }));
      await writeFile(join(binding, "index.js"), "module.exports = class Database {};\n");
      const entry = "src/cli/cli.ts";
      // The same options as `npm run build:ts` bundles the command with, its target included.
      await build({
        entryPoints: [entry], bundle: true, platform: "node", format: "esm", external: ["better-sqlite3"],
        ...await bundleTarget("build:ts", entry), outfile: join(root, "dist", "cli", "cli.js"), logLevel: "silent",
      });
      const version = spawnSync(process.execPath, [...olderNodeArguments(), join(root, "dist", "cli", "cli.js"), "version"], { encoding: "utf8" });
      expect(version.stderr).toContain(`Rerail needs Node ${REQUIRED_NODE_MAJOR} or newer. You have Node ${OLDER_NODE_VERSION}.`);
      expect(version.status).toBe(1);
    } finally {
      await removeFolder(root);
    }
  }, 60_000);
});
