import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { cleanCommandError, environmentForNode, resolveExecutable, runNpm } from "../src/npm.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(async directory => await rm(directory, { recursive: true, force: true })));
});

async function binDirectory(...files: string[]): Promise<string> {
  const bin = await mkdtemp(join(tmpdir(), "rerail-npm-"));
  temporaryDirectories.push(bin);
  for (const file of files) {
    await writeFile(join(bin, file), "#!/bin/sh\n");
    await chmod(join(bin, file), 0o755);
  }
  return bin;
}

async function fakeNpm(body: string): Promise<string> {
  const bin = await binDirectory();
  await writeFile(join(bin, "npm"), `#!/bin/sh\n${body}\n`);
  await chmod(join(bin, "npm"), 0o755);
  return bin;
}

describe("resolveExecutable", () => {
  it("returns an absolute command unchanged and finds a bare name through PATH", async () => {
    const bin = await binDirectory("node");
    expect(resolveExecutable("/opt/x/bin/node", { PATH: "" })).toBe("/opt/x/bin/node");
    expect(resolveExecutable("node", { PATH: `/nowhere${delimiter}${bin}` })).toBe(join(bin, "node"));
  });

  it("returns undefined when nothing on PATH carries the name", async () => {
    const bin = await binDirectory();
    expect(resolveExecutable("node", { PATH: bin })).toBeUndefined();
    expect(resolveExecutable("node", {})).toBeUndefined();
  });

  it("tries PATHEXT extensions on Windows", async () => {
    const bin = await binDirectory("node.exe");
    expect(resolveExecutable("node", { Path: bin, PATHEXT: ".com;.exe" }, "win32")).toBe(join(bin, "node.exe"));
    expect(resolveExecutable("node", { Path: bin, PATHEXT: ".CMD" }, "win32")).toBeUndefined();
  });
});

describe("environmentForNode", () => {
  it("puts the chosen Node's directory first on PATH so npm's env shebang resolves to it", () => {
    const env = environmentForNode("/opt/other/bin/node", { PATH: "/usr/bin", HOME: "/home/x" });
    expect(env).toEqual({ PATH: `/opt/other/bin${delimiter}/usr/bin`, HOME: "/home/x" });
  });

  it("keeps the platform's own PATH key and handles an empty PATH", () => {
    expect(environmentForNode("/opt/other/bin/node", { Path: "" })).toEqual({ Path: "/opt/other/bin" });
    expect(environmentForNode("/opt/other/bin/node", {})).toEqual({ PATH: "/opt/other/bin" });
  });

  it("leaves the environment alone for a name PATH cannot resolve, and resolves one it can", async () => {
    const empty = await binDirectory();
    const env = { PATH: empty };
    expect(environmentForNode("node", env)).toBe(env);
    const bin = await binDirectory("node");
    const searched = `${empty}${delimiter}${bin}`;
    expect(environmentForNode("node", { PATH: searched })).toEqual({ PATH: `${bin}${delimiter}${searched}` });
  });

  it("prefixes the running Node's directory by default", () => {
    expect(environmentForNode(process.execPath).PATH?.startsWith(`${dirname(process.execPath)}${delimiter}`)).toBe(true);
  });
});

describe("cleanCommandError", () => {
  it("redacts _authToken and _password values from npm output", () => {
    const text = [
      "//pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/registry/:_authToken=secret-token",
      "//pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/:_password=c2VjcmV0",
    ].join("\n");
    const cleaned = cleanCommandError(text);
    expect(cleaned).not.toContain("secret-token");
    expect(cleaned).not.toContain("c2VjcmV0");
    expect(cleaned).toContain(":_authToken=[REDACTED]");
    expect(cleaned).toContain(":_password=[REDACTED]");
  });
});

describe.skipIf(process.platform === "win32")("runNpm", () => {
  it("passes arguments through without adding a registry, userconfig or registry environment", async () => {
    const bin = await fakeNpm('printf "%s\\n" "$@"; printf "registry=%s\\n" "$npm_config_registry"');
    const result = await runNpm(["view", "@neralabs/rerail@latest", "version"], { env: { PATH: bin } });
    expect(result.stdout).toBe("view\n@neralabs/rerail@latest\nversion\nregistry=\n");
  });

  it.each([
    "npm error code E401",
    "npm error 401 Unauthorized - GET https://pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/registry/@neralabs%2frerail",
  ])("tells the user to refresh credentials when npm reports %s", async (line) => {
    const bin = await fakeNpm(`echo "${line}" >&2; echo "//x/npm/:_password=c2VjcmV0" >&2; exit 1`);
    const failure = runNpm(["view", "@neralabs/rerail"], { env: { PATH: bin } });
    await expect(failure).rejects.toThrow("npm registry credentials are missing or expired. Run `just rerail` to refresh them.");
    await expect(failure).rejects.not.toThrow("c2VjcmV0");
  });

  it("leaves other npm failures unchanged", async () => {
    const bin = await fakeNpm('echo "npm error code E404" >&2; exit 1');
    await expect(runNpm(["view", "@neralabs/missing"], { env: { PATH: bin } })).rejects.toThrow(/^npm failed \(1\): npm error code E404$/u);
  });
});
