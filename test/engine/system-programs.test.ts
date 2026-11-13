import { chmod, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { programOnPath } from "../../src/engine/system-programs.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(async directory => await removeFolder(directory)));
});

async function binDirectory(...files: string[]): Promise<string> {
  const bin = await mkdtemp(join(tmpdir(), "rerail-programs-"));
  temporaryDirectories.push(bin);
  for (const file of files) {
    await writeFile(join(bin, file), "#!/bin/sh\n");
    await chmod(join(bin, file), 0o755);
  }
  return bin;
}

describe("programOnPath", () => {
  it.skipIf(process.platform === "win32")("returns an absolute name as it is and finds a bare one through PATH", async () => {
    const bin = await binDirectory("node");
    expect(programOnPath("/opt/x/bin/node", { env: { PATH: "" } })).toBe("/opt/x/bin/node");
    expect(programOnPath("node", { env: { PATH: `/nowhere${delimiter}${bin}` } })).toBe(join(bin, "node"));
  });

  it("returns undefined when nothing on PATH carries the name", async () => {
    const bin = await binDirectory();
    expect(programOnPath("node", { env: { PATH: bin } })).toBeUndefined();
    expect(programOnPath("node", { env: {} })).toBeUndefined();
  });

  it("on Windows takes only a .exe or .com, in PATHEXT's order, past a node.cmd shim earlier on PATH", async () => {
    // A spawn without a shell cannot start a `.cmd` or `.bat` (Node refuses it with EINVAL), and every caller spawns one.
    // PATHEXT in lower case, as the files are named: the extension found is PATHEXT's, and Linux matches names by case.
    const shims = await binDirectory("node.cmd");
    const binaries = await binDirectory("node.com", "node.exe");
    const env = { Path: `${shims};${binaries}`, PATHEXT: ".cmd;.exe;.bat;.com" };
    expect(programOnPath("node", { env, platform: "win32" })).toBe(join(binaries, "node.exe"));
    expect(programOnPath("node", { env: { Path: shims, PATHEXT: ".cmd" }, platform: "win32" })).toBeUndefined();
  });

  it("on Windows skips a relative PATH entry, which names the current folder, and reads a quoted one", async () => {
    const binaries = await binDirectory("node.exe");
    const search = (path: string) => programOnPath("node", { env: { PATH: path, PATHEXT: ".exe" }, platform: "win32" });
    expect(search(`.;bin;${binaries}`)).toBe(join(binaries, "node.exe"));
    expect(search(".")).toBeUndefined();
    // Windows allows a quoted entry, which cmd.exe and CreateProcess unquote.
    expect(search(`"${binaries}"`)).toBe(join(binaries, "node.exe"));
  });
});
