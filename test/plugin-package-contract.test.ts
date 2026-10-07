import { spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { z } from "zod";

const HARNESSES = ["claude", "codex", "cursor"] as const;
const STAMPER = resolve("scripts/stamp-plugin-versions.mjs");

// Source manifests carry no version; `npm pack` stamps package.json's version in (prepack) and removes it again (postpack).
const PluginManifestSchema = z.object({
  name: z.literal("rerail"),
  description: z.string().min(1),
  license: z.literal("MIT"),
  hooks: z.string().optional(),
}).strict();

const PackageSchema = z.object({
  version: z.string(),
  files: z.array(z.string()),
  scripts: z.record(z.string(), z.string()),
  bin: z.object({ rerail: z.literal("dist/cli.js") }).strict(),
  exports: z.object({
    ".": z.object({
      types: z.literal("./dist/index.d.ts"),
      import: z.literal("./dist/index.js"),
    }),
  }),
  engines: z.object({ node: z.string().regex(/^>=24$/u) }),
  omp: z.object({ extensions: z.array(z.string()) }),
  pi: z.object({ extensions: z.array(z.string()) }),
}).passthrough();

async function json(path: string): Promise<unknown> {
  return JSON.parse(await readFile(resolve(path), "utf8"));
}

describe("supported harness plugin manifests", () => {
  it("publishes native OMP and Pi extension manifests", async () => {
    const manifest = PackageSchema.parse(await json("package.json"));

    expect(manifest.omp.extensions).toEqual(["./dist/default-extension.js"]);
    expect(manifest.pi.extensions).toEqual(["./dist/pi-extension.js"]);
  });

  it("publishes a Node 24 CLI and library entrypoint from the packed dist directory", async () => {
    const manifest = PackageSchema.parse(await json("package.json"));

    expect(manifest.files).toContain("dist");
    expect(manifest.bin.rerail).toBe("dist/cli.js");
    expect(manifest.exports["."]).toEqual({
      types: "./dist/index.d.ts",
      import: "./dist/index.js",
    });
    expect(manifest.engines.node).toBe(">=24");
  });

  it("publishes transport-neutral Claude, Codex, and Cursor plugin manifests", async () => {
    const packageManifest = PackageSchema.parse(await json("package.json"));

    for (const harness of HARNESSES) {
      const directory = `.${harness}-plugin`;
      const manifest = PluginManifestSchema.parse(await json(`${directory}/plugin.json`));

      expect(manifest).not.toHaveProperty("version");
      expect(packageManifest.files).toContain(directory);
      expect(manifest).not.toHaveProperty("mcpServers");
      expect(manifest.hooks).toBe(`./hooks/${harness}.json`);
      expect(packageManifest.scripts[`build:${harness}`]).toContain(`--outfile=dist/${harness}-hook.js`);
    }
    expect(packageManifest.files).toContain("hooks");
  });

  it("stamps the package version into the plugin manifests around npm pack", async () => {
    const packageManifest = PackageSchema.parse(await json("package.json"));

    expect(packageManifest.scripts.prepack).toBe("node scripts/stamp-plugin-versions.mjs stamp");
    expect(packageManifest.scripts.postpack).toBe("node scripts/stamp-plugin-versions.mjs restore");
  });

  it("writes the package version right after the name and restores the source form", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-stamp-"));
    try {
      await writeFile(join(root, "package.json"), `${JSON.stringify({ name: "@neralabs/rerail", version: "9.8.7-rc.6" }, null, 2)}\n`);
      const sources = new Map<string, string>();
      for (const harness of HARNESSES) {
        const directory = `.${harness}-plugin`;
        await mkdir(join(root, directory));
        await copyFile(resolve(directory, "plugin.json"), join(root, directory, "plugin.json"));
        sources.set(harness, await readFile(join(root, directory, "plugin.json"), "utf8"));
      }

      for (let pass = 0; pass < 2; pass += 1) {
        const stamp = spawnSync(process.execPath, [STAMPER, "stamp", root], { encoding: "utf8" });
        expect(stamp.status, stamp.stderr).toBe(0);
        expect(stamp.stdout).toBe("");
      }
      for (const harness of HARNESSES) {
        const text = await readFile(join(root, `.${harness}-plugin`, "plugin.json"), "utf8");
        const source = JSON.parse(sources.get(harness)!) as Record<string, unknown>;
        const stamped = JSON.parse(text) as Record<string, unknown>;
        expect(Object.keys(stamped)).toEqual(["name", "version", ...Object.keys(source).filter((key) => key !== "name")]);
        expect(stamped).toEqual({ ...source, version: "9.8.7-rc.6" });
        expect(text).toBe(`${JSON.stringify(stamped, null, 2)}\n`);
      }

      for (let pass = 0; pass < 2; pass += 1) {
        const restore = spawnSync(process.execPath, [STAMPER, "restore", root], { encoding: "utf8" });
        expect(restore.status, restore.stderr).toBe(0);
      }
      for (const harness of HARNESSES) {
        expect(await readFile(join(root, `.${harness}-plugin`, "plugin.json"), "utf8")).toBe(sources.get(harness));
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it("rejects an unknown stamping mode", () => {
    const result = spawnSync(process.execPath, [STAMPER, "bump"], { encoding: "utf8" });

    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("stamp|restore");
  });

  it("routes every Claude and Codex plugin hook event through the bundled stdin executable", async () => {
    const HooksSchema = z.object({
      hooks: z.record(z.string(), z.array(z.object({
        matcher: z.string().optional(),
        hooks: z.array(z.object({ type: z.literal("command"), command: z.string() })),
      }))),
    }).passthrough();

    const claude = HooksSchema.parse(await json("hooks/claude.json"));
    expect(Object.keys(claude.hooks).sort()).toEqual([
      "MessageDisplay",
      "PostToolUse",
      "PostToolUseFailure",
      "PreToolUse",
      "SessionEnd",
      "SessionStart",
      "Stop",
      "StopFailure",
      "UserPromptSubmit",
    ]);
    for (const [event, groups] of Object.entries(claude.hooks)) {
      expect(groups).toHaveLength(1);
      expect(groups[0]!.matcher).toBe(/^(Pre|Post)Tool/.test(event) ? "" : undefined);
      expect(groups[0]!.hooks).toEqual([{ type: "command", command: "node ${CLAUDE_PLUGIN_ROOT}/dist/claude-hook.js" }]);
    }

    const codex = HooksSchema.parse(await json("hooks/codex.json"));
    expect(Object.keys(codex.hooks).sort()).toEqual(["PostToolUse", "PreToolUse", "SessionStart", "Stop", "UserPromptSubmit"]);
    for (const [event, groups] of Object.entries(codex.hooks)) {
      expect(groups).toEqual([{
        ...(event === "PostToolUse" ? { matcher: "" } : {}),
        hooks: [{ type: "command", command: "node \"$PLUGIN_ROOT/dist/codex-hook.js\"" }],
      }]);
    }
  });
});
