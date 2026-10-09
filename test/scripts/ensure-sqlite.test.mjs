import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";
import { ensureSqlite, parseEnsureSqliteArguments } from "./ensure-sqlite.mjs";

const execute = promisify(execFile);
const script = join(dirname(fileURLToPath(import.meta.url)), "ensure-sqlite.mjs");
const checkoutRoot = resolve(dirname(script), "..");

const ABI_MISMATCH =
  "The module 'better_sqlite3.node' was compiled against a different Node.js version using NODE_MODULE_VERSION 137. " +
  "This version of Node.js requires NODE_MODULE_VERSION 141. Please try re-compiling or re-installing the module.";

async function stubPrefix(source) {
  const prefix = await mkdtemp(join(tmpdir(), "ensure-sqlite-"));
  const packageDirectory = join(prefix, "node_modules", "better-sqlite3");
  await mkdir(packageDirectory, { recursive: true });
  await writeFile(join(prefix, "package.json"), "{}");
  await writeFile(join(packageDirectory, "package.json"), JSON.stringify({ name: "better-sqlite3", main: "index.js" }));
  await writeFile(join(packageDirectory, "index.js"), source);
  return prefix;
}

test("defaults to the checkout and accepts --prefix in both spellings", () => {
  assert.deepEqual(parseEnsureSqliteArguments([]), { prefix: checkoutRoot });
  assert.deepEqual(parseEnsureSqliteArguments(["--prefix", "/opt/x"]), { prefix: resolve("/opt/x") });
  assert.deepEqual(parseEnsureSqliteArguments(["--prefix=/opt/y"]), { prefix: resolve("/opt/y") });
  assert.throws(() => parseEnsureSqliteArguments(["--prefix"]), /requires a directory/u);
  assert.throws(() => parseEnsureSqliteArguments(["--bogus"]), /Unknown ensure-sqlite option/u);
});

test("does not rebuild a binding that loads", async () => {
  const prefix = await stubPrefix("module.exports = function Database() { this.close = () => {}; };");
  try {
    const env = { ...process.env, npm_execpath: undefined };
    const { stdout } = await execute(process.execPath, [script, "--prefix", prefix], { env });
    assert.equal(stdout, "");
    assert.deepEqual(ensureSqlite(prefix, { log: () => {} }), { prefix, rebuilt: false });
  } finally {
    await rm(prefix, { recursive: true, force: true });
  }
});

test("rebuilds through the npm npm_execpath points at when the binding was built for another ABI", async () => {
  const prefix = await stubPrefix(`throw new Error(${JSON.stringify(ABI_MISMATCH)});`);
  const fakeNpm = join(prefix, "npm-cli.js");
  await writeFile(fakeNpm, `
    const { writeFileSync } = require("node:fs");
    const { join } = require("node:path");
    if (process.argv.slice(2).join(" ") !== "rebuild better-sqlite3") process.exit(2);
    writeFileSync(join(process.cwd(), "node_modules", "better-sqlite3", "index.js"),
      "module.exports = function Database() { this.close = () => {}; };");
  `);
  try {
    const env = { ...process.env, npm_execpath: fakeNpm };
    const { stdout } = await execute(process.execPath, [script, "--prefix", prefix], { env });
    const { node, modules } = process.versions;
    assert.match(stdout, new RegExp(String.raw`Rebuilding better-sqlite3 in .* for Node ${node} \(ABI ${modules}\)`, "u"));
    assert.deepEqual(ensureSqlite(prefix, { log: () => {} }), { prefix, rebuilt: false });
  } finally {
    await rm(prefix, { recursive: true, force: true });
  }
});

test("fails with the prefix when the rebuild leaves the binding unusable", async () => {
  const prefix = await stubPrefix(`throw new Error(${JSON.stringify(ABI_MISMATCH)});`);
  const fakeNpm = join(prefix, "npm-cli.js");
  await writeFile(fakeNpm, "process.exit(0);");
  try {
    await assert.rejects(
      execute(process.execPath, [script, "--prefix", prefix], { env: { ...process.env, npm_execpath: fakeNpm } }),
      error =>
        error.stderr.includes(`SQLite dependency in ${prefix} is still unusable`) && error.stderr.includes("NODE_MODULE_VERSION 141"),
    );
  } finally {
    await rm(prefix, { recursive: true, force: true });
  }
});
