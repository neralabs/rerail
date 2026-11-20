// native/wasm/env.sh, the C and C++ toolchain of the engine's browser build (wasm32-wasip1). CI checks that build with
// `cargo check`, which links nothing, so a wrong link flag passes it: this reads env.sh's link line.

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ENV_SH = fileURLToPath(new URL("../../../native/wasm/env.sh", import.meta.url));

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
