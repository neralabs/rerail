import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { fromCurrentDrive, namesOneFile, programOnPath } from "../../src/engine/system-programs.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const drives: string[] = [];
afterEach(async () => {
  // A drive goes before its folder: Windows keeps a folder a drive maps.
  for (const drive of drives.splice(0)) execFileSync("subst", [drive, "/d"], { stdio: "ignore" });
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

/**
 * A folder of this test reached as the root of a free drive, holding `tools\node.exe` and an empty `src`, as a folder
 * cloned to a drive's root or mapped to a drive letter is reached. Vitest runs test files at once and every file shares
 * the user's drive letters, so this file takes only R to T (the start state's earlier-spelling tests take U to W, the
 * status line's X to Z, and the registry's tests count on Q being free), and the next one where `subst` refuses a letter.
 */
async function driveWithTools(): Promise<{ root: string; drive: string }> {
  const root = await binDirectory();
  await mkdir(join(root, "tools"));
  await writeFile(join(root, "tools", "node.exe"), "");
  await mkdir(join(root, "src"));
  for (const name of "RST") {
    const drive = `${name}:`;
    if (existsSync(`${drive}\\`)) continue;
    try {
      execFileSync("subst", [drive, root], { stdio: "ignore" });
      drives.push(drive);
      return { root, drive };
    } catch {
      // Taken since the check above: the next one.
    }
  }
  throw new Error("no free drive letter from R to T");
}

/** What the Windows PATH search finds for `name` (node by default) on `path` while this process's current folder is `folder`. */
function searchFrom(folder: string, path: string, name = "node"): string | undefined {
  const before = process.cwd();
  process.chdir(folder);
  try {
    return programOnPath(name, { env: { PATH: path, PATHEXT: ".exe" }, platform: "win32" });
  } finally {
    process.chdir(before);
  }
}

describe("programOnPath", () => {
  it.skipIf(process.platform === "win32")("returns an absolute name as it is and finds a bare one through PATH", async () => {
    const bin = await binDirectory("node");
    expect(programOnPath("/opt/x/bin/node", { env: { PATH: "" } })).toBe("/opt/x/bin/node");
    expect(programOnPath("node", { env: { PATH: `/nowhere${delimiter}${bin}` } })).toBe(join(bin, "node"));
  });

  it.skipIf(process.platform === "win32")("passes over a file this process may not execute, as a shell does", async () => {
    const unrunnable = await binDirectory("node");
    await chmod(join(unrunnable, "node"), 0o644);
    const bin = await binDirectory("node");
    expect(programOnPath("node", { env: { PATH: `${unrunnable}${delimiter}${bin}` } })).toBe(join(bin, "node"));
  });

  // A shell reads an empty PATH entry (a PATH that ends in a separator) as the current folder; the search skips it.
  it.skipIf(process.platform === "win32")("does not read an empty PATH entry as the current folder", async () => {
    const here = await binDirectory("node");
    const before = process.cwd();
    process.chdir(here);
    try {
      expect(programOnPath("node", { env: { PATH: "" } })).toBeUndefined();
      expect(programOnPath("node", { env: { PATH: `/nowhere${delimiter}` } })).toBeUndefined();
    } finally {
      process.chdir(before);
    }
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

  // On a Windows machine only: elsewhere a path from the root has no drive to change with. Windows reads such an entry
  // on the drive of the current folder, and so does the search, which then names the program by its drive: a program
  // start reads a path without one on the drive of the child's folder, which can be another.
  it.runIf(process.platform === "win32")("on Windows reads a PATH entry from the root of the current drive on the drive of the current folder, and names the program found with that drive", async () => {
    const { drive } = await driveWithTools();
    expect(searchFrom(`${drive}\\src`, String.raw`\tools`)).toBe(`${drive}\\tools\\node.exe`);
    expect(searchFrom(`${drive}\\src`, "/tools")).toBe(`${drive}\\tools\\node.exe`);
  });

  // On Linux and macOS a backslash is part of a file name, so there `\tools` names a folder in the current folder.
  it.skipIf(process.platform === "win32")("on Linux and macOS, playing Windows, skips an entry such as \\tools, which names a folder in the current folder there", async () => {
    const here = await binDirectory();
    await mkdir(join(here, String.raw`\tools`));
    await writeFile(join(here, String.raw`\tools`, "node.exe"), "");
    expect(searchFrom(here, String.raw`\tools`)).toBeUndefined();
  });

  // Git's own search skips a PATH folder inside the repository Rerail runs in, whose files are the repository's.
  it.runIf(process.platform === "win32")("on Windows skips a PATH entry from the root of the current drive that names a folder in the repository nearest the current folder, and reads one outside it", async () => {
    const { root, drive } = await driveWithTools();
    const binaries = await binDirectory("node.exe");
    // A repository elsewhere on the drive, the PATH folder's own clone among them, leaves the folder to the person.
    await mkdir(join(root, "src", ".git"));
    await mkdir(join(root, "tools", ".git"));
    expect(searchFrom(`${drive}\\src`, String.raw`\tools`)).toBe(`${drive}\\tools\\node.exe`);
    // A repository at the drive's root, as one cloned to or mapped at a drive's root, holds it.
    await mkdir(join(root, "docs"));
    await mkdir(join(root, ".git"));
    expect(searchFrom(`${drive}\\docs`, `\\tools;${binaries}`)).toBe(join(binaries, "node.exe"));
    expect(searchFrom(`${drive}\\docs`, String.raw`\tools`)).toBeUndefined();
    // Only the nearest repository counts, as in Git's search: from a nested clone the outer one's folder is read as before.
    expect(searchFrom(`${drive}\\src`, String.raw`\tools`)).toBe(`${drive}\\tools\\node.exe`);
  });

  // A caller can hand the search a program a person named so (`\nodejs\node.exe`), which the program-path check refuses
  // as written: it is read as such a PATH entry is.
  it.runIf(process.platform === "win32")("on Windows names a program written from the root of the current drive with the current folder's drive, and skips one in the repository nearest that folder", async () => {
    const { root, drive } = await driveWithTools();
    expect(searchFrom(`${drive}\\src`, "", String.raw`\tools\node.exe`)).toBe(`${drive}\\tools\\node.exe`);
    expect(searchFrom(`${drive}\\src`, "", "/tools/node.exe")).toBe(`${drive}\\tools\\node.exe`);
    await mkdir(join(root, ".git"));
    expect(searchFrom(`${drive}\\src`, "", String.raw`\tools\node.exe`)).toBeUndefined();
  });
});

describe("namesOneFile and fromCurrentDrive", () => {
  it("read a path by the platform they are given, on any machine", () => {
    expect(namesOneFile(String.raw`C:\tools\git.exe`, "win32")).toBe(true);
    expect(namesOneFile(String.raw`\\server\share\git.exe`, "win32")).toBe(true);
    expect(namesOneFile(String.raw`\tools\git.exe`, "win32")).toBe(false);
    expect(namesOneFile("/usr/bin/git", "win32")).toBe(false);
    expect(namesOneFile("/usr/bin/git", "linux")).toBe(true);
    expect(namesOneFile(String.raw`C:\tools\git.exe`, "darwin")).toBe(false);
    expect(fromCurrentDrive(String.raw`\tools`, "linux")).toBeUndefined();
    expect(fromCurrentDrive("/usr/bin", "darwin")).toBeUndefined();
  });
});
