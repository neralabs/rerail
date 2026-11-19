import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";
import { nativeHealth } from "../../../src/index.js";
import { nativePlatformSuffix, resolveNativeBindingPath } from "../../../src/engine/native/native.js";

describe("native binding", () => {
  it("loads in the JavaScript runtime and reports its kernel version", () => {
    expect(nativeHealth()).toEqual({
      ok: true,
      kernelVersion: "1",
    });
  });

  // A fresh process loads the addon on its main thread, as the host does: making the engine's thread pool while the
  // library loads (inside dlopen, under glibc's loader lock) hung the load for good on Linux.
  it("loads in a fresh Node process and returns", () => {
    const addon = resolveNativeBindingPath();
    const script = `process.stdout.write(JSON.stringify(require(${JSON.stringify(addon)}).health()))`;
    const run = spawnSync(process.execPath, ["-e", script], { encoding: "utf8", timeout: 60_000 });
    expect(run.error).toBeUndefined();
    expect(run.status).toBe(0);
    expect(JSON.parse(run.stdout)).toEqual({ ok: true, kernelVersion: "1" });
  });

  it("maps Windows architectures to napi-rs MSVC artifact names", () => {
    expect(nativePlatformSuffix("win32", "x64")).toBe("win32-x64-msvc");
    expect(nativePlatformSuffix("win32", "arm64")).toBe("win32-arm64-msvc");
  });
});
