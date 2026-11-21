// native/wasm/env.sh, the C and C++ toolchain of the engine's browser build (wasm32-wasip1), and CI's check of that
// build, a job of its own in ci.yml. The check runs `cargo check`, which links nothing, so a wrong link flag passes it:
// these read env.sh's link line, and the job around the check.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";

const ENV_SH = fileURLToPath(new URL("../../../native/wasm/env.sh", import.meta.url));
const CI = fileURLToPath(new URL("../../../.github/workflows/ci.yml", import.meta.url));

/** The flags env.sh gives rustc for wasm32-wasip1 with the wasi-sdk at `sdk`, split at spaces. */
function rustflags(sdk) {
  const flags = execFileSync("bash", ["-c", '. "$1" && printf %s "$CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS"', "env.sh", ENV_SH], {
    env: { ...process.env, WASI_SDK_PATH: sdk },
    encoding: "utf8",
  });
  return flags.split(" ");
}

// Its other path takes Homebrew's toolchain. On Windows `bash` may be WSL's, which gets no WASI_SDK_PATH.
describe.skipIf(process.platform === "win32")("env.sh with wasi-sdk", () => {
  it("links wasi-libc, then libc++ and its unwinder from the folder built for wasm exceptions, searched first", () => {
    const lib = "/sdk/share/wasi-sysroot/lib/wasm32-wasip1";
    expect(rustflags("/sdk")).toEqual([
      "-C", "link-self-contained=no",
      "-C", `link-arg=${lib}/crt1-command.o`,
      "-L", `native=${lib}/eh`, "-l", "static=unwind",
      "-L", `native=${lib}`,
      "-l", "static=c++", "-l", "static=c++abi",
    ]);
  });
});

describe("CI's check of the browser build", () => {
  const checks = (step) => step.run?.includes("--target wasm32-wasip1");
  const job = Object.values(parse(readFileSync(CI, "utf8")).jobs).find((each) => each.steps?.some(checks));
  const check = job.steps.find(checks);

  it("runs in a job with no compile cache, when a change reaches the engine, so it never changes what another job saves", () => {
    expect(job.steps.filter((step) => step.uses?.startsWith("./.github/actions/rust-compile-cache"))).toEqual([]);
    expect(job.if).toContain("needs.changes.outputs.rust-engine != 'false'");
  });

  it("installs the wasm32-wasip1 standard library only in the check, which runs when a change reaches the engine", () => {
    const toolchain = job.steps.find((step) => step.uses?.startsWith("dtolnay/rust-toolchain@"));
    expect(toolchain.with.targets).toBeUndefined();
    expect(check.run).toContain("rustup target add wasm32-wasip1");
  });
});
