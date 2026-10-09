import { describe, expect, it } from "vitest";
import { nativeHealth } from "../src/index.js";
import { nativePlatformSuffix } from "../src/native.js";

describe("native binding", () => {
  it("loads in the JavaScript runtime and reports its kernel version", () => {
    expect(nativeHealth()).toEqual({
      ok: true,
      kernelVersion: "1",
    });
  });

  it("maps Windows architectures to napi-rs MSVC artifact names", () => {
    expect(nativePlatformSuffix("win32", "x64")).toBe("win32-x64-msvc");
    expect(nativePlatformSuffix("win32", "arm64")).toBe("win32-arm64-msvc");
  });
});
