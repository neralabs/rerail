// The release build of the tray icon program (tray/, the `rerail-tray` crate) for one engine target: package.yml's
// native job builds it beside that target's engine, and the engine package carries both (scripts/release/engine/
// packages.mjs). With no target it builds this machine's (npm run build:tray).
//
//   node scripts/build/release-tray.mjs [<engine target>]
//
// A Linux engine target builds the program for musl instead of glibc: a static binary, which runs on every Linux
// whatever its C library, and needs none of the engine's glibc 2.17 sysroot. Its crates are pure Rust, so nothing links
// C and no musl C toolchain is needed. Windows links the C runtime statically through .cargo/config.toml, as the engine
// does. The binary is copied to native/ under the name the engine package and the runtime give it.
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ENGINE_PACKAGES } from "../release/engine/packages.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** The Rust target the tray program is built for, for an engine's target: the engine's, but musl for a Linux one. */
const trayTarget = (engineTarget) => engineTarget.replace(/-unknown-linux-gnu$/u, "-unknown-linux-musl");

const [target, ...rest] = process.argv.slice(2);
const row = target === undefined ? ENGINE_PACKAGES.find(({ os, cpu }) => os === process.platform && cpu === process.arch) : ENGINE_PACKAGES.find((enginePackage) => enginePackage.target === target);
if (row === undefined || rest.length > 0) {
  // Without a target on a machine no engine is built for, as with a target no engine is built for.
  console.error(`Usage: node scripts/build/release-tray.mjs [<engine target>], the target one of ${ENGINE_PACKAGES.map((enginePackage) => enginePackage.target).join(", ")}`);
  process.exit(2);
}

const rustTarget = trayTarget(row.target);
const cargo = spawnSync("cargo", ["build", "--release", "--locked", "-p", "rerail-tray", "--target", rustTarget], { cwd: ROOT, stdio: "inherit" });
if (cargo.error !== undefined) throw cargo.error;
if (cargo.status !== 0) process.exit(cargo.status ?? 1);
// Cargo's target folder, wherever the environment or a config puts it.
const { target_directory: targetDirectory } = JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }));
const built = join(targetDirectory, rustTarget, "release", `rerail-tray${row.os === "win32" ? ".exe" : ""}`);
const destination = join(ROOT, "native", row.tray);
copyFileSync(built, destination);
console.log(`${destination}: the tray program for ${row.suffix}, built for ${rustTarget}`);
