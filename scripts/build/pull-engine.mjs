#!/usr/bin/env node
// Puts a released native engine in native/, for a checkout built without Rust: `npm pack @neralabs/rerail@<version>`
// (package.json's version unless --version names another; on main it is the last release), then copies the tarball's
// native/rerail_engine.<platform>.node over this checkout's. The engine is that release's. When what it is built from
// (native/, Cargo.toml, Cargo.lock, languages/) changed since the release's tag, this warns and lists the changes:
// tests of them then run the older engine, and `npm run build:native` builds the checkout's own. RERAIL_ENGINE_PATH
// points the runtime at an engine kept elsewhere instead (src/engine/native/native.ts).
//
//   node scripts/build/pull-engine.mjs [--version <version>]   (npm run engine:pull)
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { npmCommand } from "./ensure-sqlite.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** What the engine is built from: a change to any of them since a release makes that release's engine stale. */
export const ENGINE_SOURCES = ["native/", "Cargo.toml", "Cargo.lock", "languages/"];

/** The engine file for a platform, named as src/engine/native/native.ts (nativePlatformSuffix) loads it. */
export function engineFileName(platform = process.platform, arch = process.arch) {
  const suffix = { darwin: arch, linux: `${arch}-gnu`, win32: `${arch}-msvc` }[platform] ?? arch;
  return `rerail_engine.${platform}-${suffix}.node`;
}

/** The warning for an engine from `version` when `diffStat` (git diff --stat v<version> -- ENGINE_SOURCES) lists changes. */
export function staleWarning(version, diffStat) {
  if (diffStat.trim() === "") return undefined;
  return `warning: the engine's sources changed since v${version}, so its engine is older than this checkout's code; tests of these changes run the old engine. npm run build:native builds this checkout's own (it needs Rust).\n${diffStat.trimEnd()}`;
}

function parseVersion(argv) {
  if (argv.length === 0) return JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;
  if (argv.length === 2 && argv[0] === "--version") return argv[1];
  throw new Error("usage: pull-engine.mjs [--version <version>]");
}

/** Copies `version`'s engine for this platform into native/, through a scratch folder it removes. */
function pullEngine(version) {
  const file = engineFileName();
  const scratch = mkdtempSync(join(tmpdir(), "rerail-engine-"));
  try {
    const [command, ...leading] = npmCommand();
    const pack = [...leading, "pack", `@neralabs/rerail@${version}`, "--json", "--pack-destination", scratch];
    const [{ filename }] = JSON.parse(execFileSync(command, pack, { cwd: scratch, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }));
    // A relative path: Git for Windows' GNU tar reads the `C:` of an absolute one as a remote host.
    execFileSync("tar", ["-xzf", filename, `package/native/${file}`], { cwd: scratch, stdio: "inherit" });
    copyFileSync(join(scratch, "package", "native", file), join(ROOT, "native", file));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  return file;
}

/** The stale-engine warning for `version`, or one saying the clone lacks its tag. */
function staleness(version) {
  try {
    const stat = execFileSync("git", ["diff", "--stat", `v${version}`, "--", ...ENGINE_SOURCES], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return staleWarning(version, stat);
  } catch {
    return `warning: this clone has no tag v${version} (git fetch --tags), so whether the engine's sources changed since it is unknown`;
  }
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const version = parseVersion(process.argv.slice(2));
  const file = pullEngine(version);
  console.log(`native/${file} is now @neralabs/rerail@${version}'s engine.`);
  const warning = staleness(version);
  if (warning !== undefined) console.warn(warning);
}
