import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { NpmOptions, NpmRunner } from "../src/npm.js";
import { ensureSqlite, sqliteInstallPrefix } from "../src/sqlite-repair.js";

const WORKING_BINDING = "module.exports = function Database() { this.close = () => {}; };";
const ABI_MISMATCH =
  "The module '/nowhere/build/Release/better_sqlite3.node' was compiled against a different Node.js version using " +
  "NODE_MODULE_VERSION 137. This version of Node.js requires NODE_MODULE_VERSION 141. " +
  "Please try re-compiling or re-installing the module (for instance, using `npm rebuild` or `npm install`).";
const BROKEN_BINDING = `throw new Error(${JSON.stringify(ABI_MISMATCH)});`;

interface NpmCall {
  args: readonly string[];
  cwd: string | undefined;
  node: string | undefined;
}

/** A managed-install layout: the binding is hoisted to the prefix, the package sits two directories below it. */
async function managedInstall(root: string, binding: string): Promise<{ prefix: string; packageRoot: string }> {
  const prefix = join(root, "versions", "0.1.0");
  const packageRoot = join(prefix, "node_modules", "@neralabs", "rerail");
  const bindingRoot = join(prefix, "node_modules", "better-sqlite3");
  await mkdir(packageRoot, { recursive: true });
  await mkdir(bindingRoot, { recursive: true });
  await writeFile(join(prefix, "package.json"), JSON.stringify({ dependencies: { "@neralabs/rerail": "^0.1.0" } }));
  await writeFile(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.1.0" }));
  await writeFile(join(bindingRoot, "package.json"), JSON.stringify({ name: "better-sqlite3", main: "index.js" }));
  await writeFile(join(bindingRoot, "index.js"), binding);
  return { prefix, packageRoot };
}

function recordingNpm(calls: NpmCall[], effect: (cwd: string | undefined) => Promise<void>): NpmRunner {
  return async (args: readonly string[], options: NpmOptions = {}) => {
    calls.push({ args, cwd: options.cwd, node: options.node });
    await effect(options.cwd);
    return { stdout: "", stderr: "" };
  };
}

describe("SQLite binding repair", () => {
  let root: string;

  beforeEach(async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "rerail-sqlite-repair-")));
  });

  afterEach(async () => {
    await rm(root, { recursive: true, force: true });
  });

  it("resolves the npm prefix that hoists the binding, for a checkout and for a managed install", async () => {
    const checkout = resolve(dirname(fileURLToPath(import.meta.url)), "..");
    expect(sqliteInstallPrefix(checkout)).toBe(checkout);
    const { prefix, packageRoot } = await managedInstall(root, WORKING_BINDING);
    expect(sqliteInstallPrefix(packageRoot)).toBe(prefix);
    expect(sqliteInstallPrefix(join(root, "nowhere"))).toBeUndefined();
  });

  it("resolves through a symlinked package root whose own parents hold no node_modules", async () => {
    // ~/.rerail/current -> ~/.rerail/versions/<v>/node_modules/@neralabs/rerail: resolution must start from the target.
    const { prefix, packageRoot } = await managedInstall(root, WORKING_BINDING);
    const current = join(root, "current");
    await symlink(packageRoot, current, "dir");
    expect(sqliteInstallPrefix(current)).toBe(prefix);
    await expect(ensureSqlite(current, { npm: recordingNpm([], async () => {}) })).resolves.toEqual({ prefix, rebuilt: false });
  });

  it("leaves a binding that loads alone", async () => {
    const { prefix, packageRoot } = await managedInstall(root, WORKING_BINDING);
    const calls: NpmCall[] = [];
    const lines: string[] = [];
    const result = await ensureSqlite(packageRoot, { npm: recordingNpm(calls, async () => {}), log: line => lines.push(line) });
    expect(result).toEqual({ prefix, rebuilt: false });
    expect(calls).toEqual([]);
    expect(lines).toEqual([]);
  });

  it("rebuilds in the prefix when the binding was built for another Node ABI, then passes the re-probe", async () => {
    const { prefix, packageRoot } = await managedInstall(root, BROKEN_BINDING);
    const calls: NpmCall[] = [];
    const lines: string[] = [];
    const npm = recordingNpm(calls, async cwd => {
      if (cwd === undefined) throw new Error("rebuild ran without a prefix");
      await writeFile(join(cwd, "node_modules", "better-sqlite3", "index.js"), WORKING_BINDING);
    });
    const result = await ensureSqlite(packageRoot, { npm, log: line => lines.push(line) });
    expect(result).toEqual({ prefix, rebuilt: true });
    expect(calls).toEqual([{ args: ["rebuild", "better-sqlite3"], cwd: prefix, node: process.execPath }]);
    expect(lines).toEqual([
      `Rebuilding better-sqlite3 in ${prefix} for Node ${process.versions.node} (ABI ${process.versions.modules})`,
    ]);
  });

  it("hands npm the Node the daemon will run under, so the rebuild targets that ABI", async () => {
    const { packageRoot } = await managedInstall(root, BROKEN_BINDING);
    const calls: NpmCall[] = [];
    const executable = join(root, "other-node", "bin", "node");
    await expect(ensureSqlite(packageRoot, { executable, npm: recordingNpm(calls, async () => {}) })).rejects.toThrow(
      "npm rebuild did not fix it",
    );
    expect(calls.map(call => call.node)).toEqual([executable]);
  });

  it("fails with an actionable message when the rebuild does not fix the binding", async () => {
    const { prefix, packageRoot } = await managedInstall(root, BROKEN_BINDING);
    const calls: NpmCall[] = [];
    await expect(ensureSqlite(packageRoot, { npm: recordingNpm(calls, async () => {}) })).rejects.toThrow(
      new RegExp(
        `^better-sqlite3 in ${prefix} cannot load under Node ${process.versions.node} \\(ABI ${process.versions.modules}\\) ` +
        "and npm rebuild did not fix it\\.\\nProbe still fails: [\\s\\S]*NODE_MODULE_VERSION 141[\\s\\S]*run rerail start again[\\s\\S]*" +
        "npx @neralabs/rerail setup",
        "u",
      ),
    );
    expect(calls).toHaveLength(1);
  });

  it("fails with an actionable message when npm rebuild itself fails", async () => {
    const { prefix, packageRoot } = await managedInstall(root, BROKEN_BINDING);
    const npm: NpmRunner = async () => {
      throw new Error("npm failed (1): gyp ERR! build error");
    };
    await expect(ensureSqlite(packageRoot, { npm })).rejects.toThrow(
      new RegExp(`better-sqlite3 in ${prefix} cannot load[\\s\\S]*Rebuild failed: npm failed \\(1\\): gyp ERR! build error`, "u"),
    );
  });

  it("names the package root when no binding is installed at all", async () => {
    const packageRoot = join(root, "empty");
    await mkdir(packageRoot, { recursive: true });
    await expect(ensureSqlite(packageRoot)).rejects.toThrow(`better-sqlite3 is not installed below ${packageRoot}`);
  });
});
