import { spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { build } from "esbuild";
import { describe, expect, it } from "vitest";
import { z } from "zod";

const HOSTS = ["claude", "codex", "cursor"] as const;
const STAMPER = resolve("scripts/build/stamp-plugin-versions.mjs");

// Source manifests carry no version; `npm pack` stamps package.json's version in (prepack) and removes it again (postpack).
const PluginManifestSchema = z.object({
  name: z.literal("rerail"),
  description: z.string().min(1),
  license: z.literal("Apache-2.0"),
  hooks: z.string().optional(),
}).strict();

const PackageSchema = z.object({
  version: z.string(),
  files: z.array(z.string()),
  scripts: z.record(z.string(), z.string()),
  bin: z.object({ rerail: z.literal("dist/cli/cli.js") }).strict(),
  exports: z.object({
    ".": z.object({
      types: z.literal("./dist/index.d.ts"),
      import: z.literal("./dist/index.js"),
    }),
    "./schemas": z.object({
      types: z.literal("./dist/engine/schemas.d.ts"),
      import: z.literal("./dist/engine/schemas.js"),
    }),
    "./pi": z.object({
      types: z.literal("./dist/hosts/pi/index.d.ts"),
      import: z.literal("./dist/hosts/pi/index.js"),
    }),
  }).catchall(z.object({ types: z.string(), import: z.string() })),
  engines: z.object({ node: z.string().regex(/^>=24$/u) }),
  omp: z.object({ extensions: z.array(z.string()) }),
  pi: z.object({ extensions: z.array(z.string()) }),
}).passthrough();

async function json(path: string): Promise<unknown> {
  return JSON.parse(await readFile(resolve(path), "utf8"));
}

/** A plugin root under a Windows profile folder with a space, as Claude Code hands it to Git Bash. */
const SPACED_CLAUDE_ROOT = "C:/Users/Jane Doe/.claude/plugins/cache/neralabs/rerail/0.1.3";
/** A plugin root under a Windows profile folder with a space, as Codex writes it into a hook command. */
const SPACED_CODEX_ROOT = String.raw`C:\Users\Jane Doe\.codex\plugins\cache\rerail`;

/**
 * The words POSIX sh makes of `command` once each `${NAME}` in it expands from `env`: double quotes group, and an
 * unquoted expansion splits at blanks. Enough of sh for a hook command line.
 */
function shWords(command: string, env: Readonly<Record<string, string>>): string[] {
  const words: string[] = [];
  let word: string | undefined;
  let quoted = false;
  const end = (): void => {
    if (word !== undefined) words.push(word);
    word = undefined;
  };
  for (let index = 0; index < command.length; index++) {
    const expansion = /^\$\{(\w+)\}/u.exec(command.slice(index));
    if (expansion !== null) {
      index += expansion[0].length - 1;
      const value = env[expansion[1]!] ?? "";
      if (quoted) word = (word ?? "") + value;
      else {
        value.split(/[ \t\n]/u).forEach((field, at) => {
          if (at > 0) end();
          if (field !== "") word = (word ?? "") + field;
        });
      }
      continue;
    }
    const character = command[index]!;
    if (character === '"') {
      quoted = !quoted;
      word ??= "";
    } else if (!quoted && /\s/u.test(character)) end();
    else word = (word ?? "") + character;
  }
  end();
  return words;
}

/** Every `command` string anywhere in a hooks manifest. */
function commandsIn(value: unknown): string[] {
  if (Array.isArray(value)) return value.flatMap(commandsIn);
  if (value === null || typeof value !== "object") return [];
  return Object.entries(value).flatMap(([key, item]) => key === "command" && typeof item === "string" ? [item] : commandsIn(item));
}

/** The files an npm script's esbuild runs write: each `--outfile`, or each entry point's own name in `--outdir`. */
function esbuildOutputs(script: string): string[] {
  return script.split("&&").flatMap(command => {
    const [program, ...words] = command.trim().split(/\s+/u);
    if (program !== "esbuild") return [];
    const option = (name: string): string | undefined => words.find(word => word.startsWith(`${name}=`))?.slice(name.length + 1);
    const outfile = option("--outfile");
    if (outfile !== undefined) return [outfile];
    const outdir = option("--outdir");
    if (outdir === undefined) return [];
    return words.filter(word => !word.startsWith("-")).map(entry => `${outdir}/${basename(entry, ".ts")}.js`);
  });
}

describe("supported host plugin manifests", () => {
  it("publishes native OMP and Pi extension manifests", async () => {
    const manifest = PackageSchema.parse(await json("package.json"));

    expect(manifest.omp.extensions).toEqual(["./dist/hosts/omp/default-extension.js"]);
    expect(manifest.pi.extensions).toEqual(["./dist/hosts/pi/pi-extension.js"]);
  });

  it("publishes a Node 24 CLI and library entrypoint from the packed dist directory", async () => {
    const manifest = PackageSchema.parse(await json("package.json"));

    expect(manifest.files).toContain("dist");
    expect(manifest.bin.rerail).toBe("dist/cli/cli.js");
    expect(manifest.exports["."]).toEqual({
      types: "./dist/index.d.ts",
      import: "./dist/index.js",
    });
    expect(manifest.engines.node).toBe(">=24");
  });

  it("loads typebox, Pi's optional peer, only from the Pi entry point", async () => {
    const manifest = PackageSchema.parse(await json("package.json"));
    expect(manifest).toMatchObject({ peerDependencies: { typebox: "*" }, peerDependenciesMeta: { typebox: { optional: true } } });
    expect(manifest).not.toHaveProperty("dependencies.typebox");
    // Without the optional peer installed (npm with --legacy-peer-deps, Yarn Berry, any host but Pi) these must still load.
    const entries = [
      ...Object.entries(manifest.exports).map(([subpath, target]) => ({ subpath, file: target.import })),
      { subpath: "bin", file: manifest.bin.rerail },
      ...manifest.omp.extensions.map(file => ({ subpath: "omp", file })),
    ];
    const reaches = async (file: string): Promise<boolean> => {
      const source = file.replace(/^\.?\/?dist\//u, "src/").replace(/\.js$/u, ".ts");
      const { metafile } = await build({ entryPoints: [source], bundle: true, platform: "node", format: "esm", packages: "external", write: false, metafile: true, logLevel: "silent" });
      return Object.values(metafile.inputs).some(input => input.imports.some(imported => imported.path === "typebox" || imported.path.startsWith("typebox/")));
    };
    const loading = [];
    for (const { subpath, file } of entries) if (await reaches(file)) loading.push(subpath);
    expect(loading).toEqual(["./pi"]);
  }, 60_000);

  it("publishes transport-neutral Claude, Codex, and Cursor plugin manifests", async () => {
    const packageManifest = PackageSchema.parse(await json("package.json"));

    for (const host of HOSTS) {
      const directory = `.${host}-plugin`;
      const manifest = PluginManifestSchema.parse(await json(`${directory}/plugin.json`));

      expect(manifest).not.toHaveProperty("version");
      expect(packageManifest.files).toContain(directory);
      expect(manifest).not.toHaveProperty("mcpServers");
      expect(manifest.hooks).toBe(`./hooks/${host}.json`);
      // Every hook the plugin registers runs the one bundle its host's build writes.
      const hook = `dist/hosts/${host}/${host}-hook.js`;
      const run = commandsIn(await json(`hooks/${host}.json`)).map(command => /dist\/[\w./-]+\.js/u.exec(command)?.[0]);
      expect(new Set(run)).toEqual(new Set([hook]));
      expect(esbuildOutputs(packageManifest.scripts[`build:${host}`] ?? "")).toContain(hook);
    }
    expect(packageManifest.files).toContain("hooks");
  });

  it("stamps the package version into the plugin manifests around npm pack", async () => {
    const packageManifest = PackageSchema.parse(await json("package.json"));

    expect(packageManifest.scripts.prepack).toContain("node scripts/build/stamp-plugin-versions.mjs stamp");
    expect(packageManifest.scripts.postpack).toContain("node scripts/build/stamp-plugin-versions.mjs restore");
  });

  it("writes the package version right after the name and restores the source form", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-stamp-"));
    try {
      await writeFile(join(root, "package.json"), `${JSON.stringify({ name: "@neralabs/rerail", version: "9.8.7-rc.6" }, null, 2)}\n`);
      const sources = new Map<string, string>();
      for (const host of HOSTS) {
        const directory = `.${host}-plugin`;
        await mkdir(join(root, directory));
        await copyFile(resolve(directory, "plugin.json"), join(root, directory, "plugin.json"));
        sources.set(host, await readFile(join(root, directory, "plugin.json"), "utf8"));
      }

      for (let pass = 0; pass < 2; pass += 1) {
        const stamp = spawnSync(process.execPath, [STAMPER, "stamp", root], { encoding: "utf8" });
        expect(stamp.status, stamp.stderr).toBe(0);
        expect(stamp.stdout).toBe("");
      }
      for (const host of HOSTS) {
        const text = await readFile(join(root, `.${host}-plugin`, "plugin.json"), "utf8");
        const source = JSON.parse(sources.get(host)!) as Record<string, unknown>;
        const stamped = JSON.parse(text) as Record<string, unknown>;
        expect(Object.keys(stamped)).toEqual(["name", "version", ...Object.keys(source).filter((key) => key !== "name")]);
        expect(stamped).toEqual({ ...source, version: "9.8.7-rc.6" });
        expect(text).toBe(`${JSON.stringify(stamped, null, 2)}\n`);
      }

      for (let pass = 0; pass < 2; pass += 1) {
        const restore = spawnSync(process.execPath, [STAMPER, "restore", root], { encoding: "utf8" });
        expect(restore.status, restore.stderr).toBe(0);
      }
      for (const host of HOSTS) {
        expect(await readFile(join(root, `.${host}-plugin`, "plugin.json"), "utf8")).toBe(sources.get(host));
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
      expect(groups[0]!.hooks).toEqual([{ type: "command", command: "node \"${CLAUDE_PLUGIN_ROOT}/dist/hosts/claude/claude-hook.js\"" }]);
      // Claude Code on Windows sets the root with forward slashes and lets Git Bash expand it, and a local account's
      // profile folder may hold a space: the path must stay one word.
      for (const hook of groups[0]!.hooks) {
        expect(shWords(hook.command, { CLAUDE_PLUGIN_ROOT: SPACED_CLAUDE_ROOT }))
          .toEqual(["node", `${SPACED_CLAUDE_ROOT}/dist/hosts/claude/claude-hook.js`]);
      }
    }

    const codex = HooksSchema.parse(await json("hooks/codex.json"));
    expect(Object.keys(codex.hooks).sort()).toEqual(["PostToolUse", "PreToolUse", "SessionStart", "Stop", "UserPromptSubmit"]);
    for (const [event, groups] of Object.entries(codex.hooks)) {
      expect(groups).toEqual([{
        ...(event === "PostToolUse" ? { matcher: "" } : {}),
        hooks: [{ type: "command", command: "node \"${PLUGIN_ROOT}/dist/hosts/codex/codex-hook.js\"" }],
      }]);
      // Codex replaces only `${PLUGIN_ROOT}` in the text; a bare `$PLUGIN_ROOT` would reach cmd.exe or PowerShell on
      // Windows, which never expand it.
      for (const hook of groups[0]!.hooks) {
        expect(hook.command.replaceAll("${PLUGIN_ROOT}", SPACED_CODEX_ROOT)).toBe(`node "${SPACED_CODEX_ROOT}/dist/hosts/codex/codex-hook.js"`);
      }
    }
  });
});
