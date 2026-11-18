// The release build of the native engine for one target: package.yml's native job, release-cache.yml (the cargo cache
// it restores, built by the same command) and ci.yml's build job (the Linux x64 engine every test job runs).
//
//   node scripts/build/release-engine.mjs <target>
//
// Only the library napi packs (`native/*.node`): the crate's `rerail-engine` binary is a tool for the engine's
// developers, and nothing ships it. A Linux engine links against the glibc 2.17 sysroot of napi-rs's cross toolchain
// (`--use-napi-cross`), so it loads on old distributions, and drops its symbol tables (`--strip`), which nothing loads
// at runtime. The toolchain's GCC is 4.8.5, too old for the C11 and C++17 the engine's dependencies are written in, so
// the runner's compiler compiles them, against that sysroot's headers: napi's `TARGET_CC` and `TARGET_SYSROOT` reach a
// build script's `cc` only when the target is not the runner's. Compiled against the runner's headers, SQLite took
// `fcntl64` (glibc 2.28) and left it for the machine loading the engine. simdjson's C++ is compiled for the sysroot by
// its own build script (native/crates/simdjson_native/build.rs), which reads `TARGET_SYSROOT`.
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

const target = process.argv[2];
if (target === undefined || process.argv.length > 3) {
  console.error("Usage: node scripts/build/release-engine.mjs <target>");
  process.exit(2);
}
const napiCli = join("node_modules", "@napi-rs", "cli");
const args = ["build", "--platform", "--release", "--target", target, "--manifest-path", "native/Cargo.toml", "--output-dir", "native"];
const env = { ...process.env };
if (target.endsWith("-linux-gnu")) {
  args.push("--use-napi-cross", "--strip");
  // Where napi extracts the toolchain (its `pickCrossToolchain`), the sysroot it gives cargo as `TARGET_SYSROOT`.
  const { version } = createRequire(resolve(napiCli, "package.json"))("@napi-rs/cross-toolchain");
  const sysroot = join(homedir(), ".napi-rs", "cross-toolchain", version, target, target, "sysroot");
  const cflags = `CFLAGS_${target.replaceAll("-", "_")}`;
  env[cflags] = [`--sysroot=${sysroot}`, process.env[cflags]].filter(Boolean).join(" ");
}
// After `--`, cargo's: the library alone.
const napi = spawnSync(process.execPath, [join(napiCli, "dist", "cli.js"), ...args, "--", "--lib"], { stdio: "inherit", env });
if (napi.error !== undefined) throw napi.error;
process.exitCode = napi.status ?? 1;
