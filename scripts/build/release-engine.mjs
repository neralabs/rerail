// The release build of the native engine for one target: package.yml's native job, release-cache.yml (the compile cache
// store it starts from, built by the same command) and ci.yml's build job (the Linux x64 engine every test job runs).
//
//   node scripts/build/release-engine.mjs <target>
//
// Only the library napi packs (`native/*.node`): the crate's `rerail-engine` binary is a tool for the engine's
// developers, and nothing ships it. A Linux engine links against the glibc 2.17 sysroot of napi-rs's cross toolchain
// (`--use-napi-cross`), so it loads on old distributions, and drops its symbol tables (`--strip`), which nothing loads
// at runtime. The toolchain's GCC is 4.8.5, too old for what the engine's C and C++ dependencies need (C++17 in
// simdjson, `-Wno-error=date-time` in mimalloc's build), so the runner's compiler compiles them, against that sysroot's
// headers. napi 3.10.9 left the runner's compiler alone for a target that is the runner's own (cc-rs reads its
// `TARGET_CC` only for another target); from 3.10.12 napi sets `CC_<target>`, `CXX_<target>`, `AR_<target>` and
// `RANLIB_<target>` to the toolchain's, unless they are set. They are set here, to the runner's, so the release does not
// change compiler with napi's version. Compiled against the runner's headers, SQLite took `fcntl64` (glibc 2.28) and left
// it for the machine loading the engine. simdjson's C++ is compiled for the sysroot by its own build script
// (native/crates/simdjson_native/build.rs), which reads `TARGET_SYSROOT`.
import { spawnSync } from "node:child_process";
import { accessSync, constants } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

/** `name`'s path on this PATH, before napi puts the toolchain's `bin` first. */
function onPath(name) {
  for (const dir of (process.env.PATH ?? "").split(delimiter).filter(Boolean)) {
    try {
      accessSync(join(dir, name), constants.X_OK);
      return join(dir, name);
    } catch {
      // not in this directory
    }
  }
  throw new Error(`release-engine: no \`${name}\` on PATH for the runner's C toolchain`);
}

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
  const key = target.replaceAll("-", "_");
  env[`CFLAGS_${key}`] = [`--sysroot=${sysroot}`, process.env[`CFLAGS_${key}`]].filter(Boolean).join(" ");
  for (const [tool, name] of [["CC", "cc"], ["CXX", "c++"], ["AR", "ar"], ["RANLIB", "ranlib"]]) {
    env[`${tool}_${key}`] ||= onPath(name);
  }
}
// After `--`, cargo's: the library alone.
const napi = spawnSync(process.execPath, [join(napiCli, "dist", "cli.js"), ...args, "--", "--lib"], { stdio: "inherit", env });
if (napi.error !== undefined) throw napi.error;
process.exitCode = napi.status ?? 1;
