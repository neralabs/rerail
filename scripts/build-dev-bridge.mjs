#!/usr/bin/env node
import { cp, lstat, mkdir, realpath, rm, writeFile, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const buildRoot = [resolve(root, "dist"), "/var/lib/rerail/build"].find(candidate =>
  process.argv[2] === undefined ? candidate === resolve(root, "dist") : process.argv[2] === resolve(candidate, "host-bridge"));
if (buildRoot === undefined) throw new Error("Bridge output must be the host-bridge directory in a supported private build root");
const output = resolve(buildRoot, "host-bridge");
await mkdir(buildRoot, { recursive: true });
if (!(await lstat(buildRoot)).isDirectory()) throw new Error(`Bridge build root is not a directory: ${buildRoot}`);
if (await realpath(buildRoot) !== buildRoot) throw new Error(`Refusing a symlinked ancestor of bridge build root: ${buildRoot}`);
try {
  if ((await lstat(output)).isSymbolicLink()) throw new Error(`Refusing symlinked bridge output: ${output}`);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
await rm(output, { recursive: true, force: true });
await mkdir(resolve(output, "dist"), { recursive: true });
await cp(resolve(root, "skills", "rerail"), resolve(output, "skills", "rerail"), { recursive: true });
await build({
  absWorkingDir: root,
  entryPoints: {
    "bundled-omp": "src/default-extension.ts",
    "bundled-pi": "src/dev-pi-extension.ts",
    "bundled-cursor": "src/dev-cursor-hook.ts",
    "bundled-claude": "src/dev-claude-hook.ts",
    "bundled-codex": "src/dev-codex-hook.ts",
    "claude-status": "src/claude-status.ts",
    registration: "src/harness-registration.ts",
    routing: "src/dev-routing.ts",
    mcp: "src/dev-bridge.ts",
  },
  outdir: resolve(output, "dist"),
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node24",
  external: ["better-sqlite3"],
  banner: { js: 'import { createRequire as bridgeCreateRequire } from "node:module"; const require = bridgeCreateRequire(import.meta.url);' },
});
const manifest = JSON.parse(await readFile(resolve(root, "package.json"), "utf8"));
await writeFile(resolve(output, "package.json"), JSON.stringify({
  name: "@neralabs/rerail-dev-bridge", version: manifest.version, type: "module",
  omp: { extensions: ["./dist/default-extension.js"] },
  pi: { extensions: ["./dist/pi-extension.js"] },
}, null, 2) + "\n");
const common = `import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { resolveRerailWorkspaceRoute } from "./routing.js";
const managed = resolve(process.env.RERAIL_HOME ?? resolve(homedir(), ".rerail"), "current", "dist");
`;
for (const [host, filename] of [["omp", "default-extension.js"], ["pi", "pi-extension.js"]]) {
  await writeFile(resolve(output, "dist", filename), common + `
export default async function bridge(api) {
  const route = await resolveRerailWorkspaceRoute(process.cwd());
  if (route.kind === "workspace") {
    const module = await import("./bundled-${host}.js");
    return module.default(api);
  }
  if (route.kind === "host") {
    const installed = resolve(managed, "${filename}");
    if (existsSync(installed)) return (await import(pathToFileURL(installed).href)).default(api);
  }
}
`);
}
const streams = "{ stdin: [input], stdout: process.stdout, stderr: process.stderr }";
for (const [host, serve] of [
  ["cursor", "serveCursorHook(input)"],
  ["claude", `serveHook(module.runClaudeHook, ${streams})`],
  ["codex", `serveHook(module.runCodexHook, ${streams})`],
]) {
  await writeFile(resolve(output, "dist", `${host}-hook.js`), common + String.raw`
import { spawnSync } from "node:child_process";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = Buffer.concat(chunks).toString("utf8");
const payload = JSON.parse(input);
const route = await resolveRerailWorkspaceRoute(payload.cwd ?? process.cwd());
if (route.kind === "workspace") {
  // The bundled entry records a steer receipt only once the host confirms the write.
  const module = await import("./bundled-${host}.js");
  await module.${serve};
} else if (route.kind === "host") {
  const installed = resolve(managed, "${host}-hook.js");
  if (existsSync(installed)) {
    const result = spawnSync(process.execPath, [installed], { input, encoding: "utf8" });
    if (result.error) throw result.error;
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    process.exitCode = result.status ?? 1;
  } else process.stdout.write("{}\n");
} else process.stdout.write("{}\n");
`);
}
await writeFile(resolve(output, "dist", "cli.js"), common + `
import { spawnSync } from "node:child_process";
import { rerailDevLauncher } from "./routing.js";
const args = process.argv.slice(2);
if (args[0] === "mcp-bridge") {
  await (await import("./mcp.js")).runRerailMcpBridge();
} else {
  const route = await resolveRerailWorkspaceRoute(process.cwd());
  const workspace = route.kind === "workspace" ? route.instance
    : route.kind === "disabled" ? route : undefined;
  const launcher = workspace === undefined
    ? { command: process.execPath, args: [resolve(managed, "cli.js"), ...args], cwd: process.cwd() }
    : { ...rerailDevLauncher(workspace, args), cwd: workspace.workspaceRoot };
  const result = spawnSync(launcher.command, launcher.args, { cwd: launcher.cwd, stdio: "inherit" });
  if (result.error) {
    process.stderr.write(\`rerail: \${result.error.message}\\n\`);
    process.exitCode = 1;
  } else process.exitCode = result.status ?? 1;
}
`);
