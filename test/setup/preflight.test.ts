import { describe, expect, it } from "vitest";
import { gitInstallCommand, MIN_LINUX_GLIBC, setupPreflight, unsupportedPlatform, wslNote } from "../../src/setup/preflight.js";

describe("unsupportedPlatform", () => {
  it("passes every system and processor Rerail ships an engine for", () => {
    for (const platform of ["darwin", "win32"] as const) {
      for (const arch of ["arm64", "x64"]) expect(unsupportedPlatform({ platform, arch })).toBeUndefined();
    }
    for (const arch of ["arm64", "x64"]) expect(unsupportedPlatform({ platform: "linux", arch, glibc: MIN_LINUX_GLIBC })).toBeUndefined();
    expect(unsupportedPlatform({ platform: "linux", arch: "x64", glibc: "2.41" })).toBeUndefined();
  });

  it("stops on musl, an older glibc, and a system or processor with no engine", () => {
    expect(unsupportedPlatform({ platform: "linux", arch: "arm64" })).toEqual({ kind: "musl" });
    expect(unsupportedPlatform({ platform: "linux", arch: "x64", glibc: "2.36" })).toEqual({ kind: "glibc", found: "2.36" });
    expect(unsupportedPlatform({ platform: "linux", arch: "arm", glibc: "2.41" })).toEqual({ kind: "system", platform: "Linux", arch: "arm" });
    expect(unsupportedPlatform({ platform: "freebsd", arch: "x64" })).toEqual({ kind: "system", platform: "freebsd", arch: "x64" });
  });
});

describe("setupPreflight", () => {
  it("says what runs Rerail and that nothing changed, instead of failing the install on the engine", () => {
    const text = setupPreflight("agent", { platform: "linux", arch: "arm64" });
    expect(text).toContain("✕ Rerail can’t run on this computer: this Linux uses musl (Alpine, for example), and Rerail needs glibc.");
    expect(text).toContain("Nothing was changed on this machine.");
    expect(text).toContain(`Rerail runs on macOS and Windows, and on Linux with glibc ${MIN_LINUX_GLIBC} or newer, each on arm64 or x64.`);
    expect(setupPreflight("agent", { platform: "linux", arch: "x64", glibc: "2.36" }))
      .toContain(`this Linux has glibc 2.36, and Rerail needs glibc ${MIN_LINUX_GLIBC} or newer.`);
  });
});

describe("Git", () => {
  it("names the install command for the system's package manager", () => {
    expect(gitInstallCommand("linux", 'PRETTY_NAME="Debian GNU/Linux 13 (trixie)"\nID=debian\n', true)).toBe("apt-get install git");
    expect(gitInstallCommand("linux", "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n", false)).toBe("sudo apt-get install git");
    expect(gitInstallCommand("linux", 'ID="rocky"\nID_LIKE="rhel centos fedora"\n', false)).toBe("sudo dnf install git");
    expect(gitInstallCommand("linux", "ID=void\n", false)).toBeUndefined();
    expect(gitInstallCommand("win32", undefined, false)).toBe("winget install --id Git.Git -e");
  });
});

describe("wslNote", () => {
  it("notes a repo on the Windows drive inside WSL, and nothing anywhere else", () => {
    expect(wslNote("/mnt/c/Users/me/app", "agent", true)).toContain("△ This is WSL, and /mnt/c/Users/me/app is on the Windows drive.");
    expect(wslNote("/home/me/app", "agent", true)).toBeUndefined();
    expect(wslNote("/mnt/data/app", "agent", true)).toBeUndefined();
    expect(wslNote("/mnt/c/Users/me/app", "agent", false)).toBeUndefined();
  });
});
