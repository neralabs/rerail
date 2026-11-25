import * as childProcess from "node:child_process";
import { mkdir, mkdtemp, readdir, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { deviceSecretStore, FileDeviceSecretStore, SystemDeviceSecretStore } from "../../src/activation/device-keystore.js";
import { POSIX_MODES } from "../support/platform/file-mode.js";
import { cmdletsWithoutModule, commandScript, MODULE_QUALIFIED_ONLY, withCommandPrefix } from "../support/platform/powershell-commands.js";
import { removeFolder } from "../support/platform/remove-folder.js";

// A spy that calls through, so a case can change the command the Credential Manager store starts.
vi.mock("node:child_process", { spy: true });

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => removeFolder(path)));
});

describe("device secret store selection", () => {
  it("runs this suite against the file store", () => {
    expect(process.env.RERAIL_DEVICE_SECRET_STORE).toBe("file");
    expect(deviceSecretStore()).toBeInstanceOf(FileDeviceSecretStore);
  });

  it("refuses the system store inside a test run instead of reaching the keychain", () => {
    expect(() => deviceSecretStore({ VITEST: "true" })).toThrow("Tests must not use the system keychain");
    expect(() => deviceSecretStore({ VITEST: "true", RERAIL_DEVICE_SECRET_STORE: "system" })).toThrow("system keychain");
    expect(deviceSecretStore({})).toBeInstanceOf(SystemDeviceSecretStore);
    expect(() => deviceSecretStore({ RERAIL_DEVICE_SECRET_STORE: "keychain" })).toThrow("must be system or file");
  });

  it("keeps file secrets owner-only under the Rerail home and removes them", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-device-secrets-"));
    directories.push(home);
    const store = deviceSecretStore({ VITEST: "true", RERAIL_DEVICE_SECRET_STORE: "file", RERAIL_HOME: home });
    expect(await store.read("profile-a")).toBeUndefined();
    await store.write("profile-a", "secret");
    expect(await store.read("profile-a")).toBe("secret");
    if (POSIX_MODES) expect((await stat(join(home, "device-secrets", "profile-a"))).mode & 0o777).toBe(0o600);
    await store.remove("profile-a");
    expect(await store.read("profile-a")).toBeUndefined();
    await expect(store.write("../escape", "x")).rejects.toThrow("Invalid Rerail profile name");
  });

  it("lets concurrent writes to one profile both land and leaves no staging files behind", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-device-secrets-"));
    directories.push(home);
    const store = new FileDeviceSecretStore(join(home, "device-secrets"));
    await Promise.all(["first", "second", "third"].map(value => store.write("profile-a", value)));
    expect(["first", "second", "third"]).toContain(await store.read("profile-a"));
    await mkdir(join(home, "device-secrets", "profile-b"));
    await expect(store.write("profile-b", "secret")).rejects.toThrow();
    expect((await readdir(join(home, "device-secrets"))).sort()).toEqual(["profile-a", "profile-b"]);
  });
});

describe("Windows Credential Manager", () => {
  it("runs Windows PowerShell by its System32 path, never a powershell.exe the current folder holds", async () => {
    const commands: string[] = [];
    const store = new SystemDeviceSecretStore("win32", async (command) => {
      commands.push(command);
      return { code: 0, stdout: JSON.stringify({ found: false }), stderr: "" };
    });
    expect(await store.read("profile-a")).toBeUndefined();
    await store.write("profile-a", "secret");
    await store.remove("profile-a");
    expect(commands).toHaveLength(3);
    for (const command of commands) expect(command).toMatch(/^[A-Za-z]:\\.*\\System32\\WindowsPowerShell\\v1\.0\\powershell\.exe$/u);
  });

  it("names the module of each cmdlet its script runs, so PowerShell reads no other installed module first", async () => {
    const scripts: Array<string | undefined> = [];
    const store = new SystemDeviceSecretStore("win32", async (_command, args) => {
      scripts.push(commandScript(args));
      return { code: 0, stdout: JSON.stringify({ found: false }), stderr: "" };
    });
    expect(await store.read("profile-a")).toBeUndefined();
    expect(scripts).toHaveLength(1);
    expect(scripts[0]).toMatch(/\\Add-Type /u);
    expect(cmdletsWithoutModule(scripts[0] ?? "")).toEqual([]);
  });

  // The case above reads the script's text: a wrong module (Microsoft.PowerShell.Management\Add-Type) passes it. The real
  // PowerShell finds no cmdlet under a wrong module, and under this setting none named alone either. The read calls
  // CredRead only after Add-Type and ConvertFrom-Json have run; over SSH, whose logon session Windows gives no
  // credential store, CredRead then fails with ERROR_NO_SUCH_LOGON_SESSION, a Win32Exception. ConvertTo-Json writes a
  // secret that was found, so a read of an absent target never runs it.
  it.runIf(process.platform === "win32")("runs its script's read of an absent target while PowerShell loads only the modules a command names", async () => {
    const scripts: Array<string | undefined> = [];
    const { spawn: realSpawn } = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.spawn).mockImplementationOnce(((program: string, args: readonly string[], options: childProcess.SpawnOptions) => {
      scripts.push(commandScript(args));
      return realSpawn(program, withCommandPrefix(args, MODULE_QUALIFIED_ONLY), options);
    }) as unknown as typeof childProcess.spawn);
    const read = await new SystemDeviceSecretStore("win32").read(`absent-${process.pid}`)
      .then(value => `read ${String(value)}`, (error: unknown) => String(error));
    expect(scripts).toHaveLength(1);
    expect(read).toMatch(/^read undefined$|FullyQualifiedErrorId\s*:\s*Win32Exception/u);
    expect(read).not.toMatch(/CommandNotFoundException/u);
  }, 60_000);
});

describe("macOS Keychain writes", () => {
  interface Call { command: string; args: readonly string[]; input: string | undefined }

  function recordingStore(): { store: SystemDeviceSecretStore; calls: Call[] } {
    const calls: Call[] = [];
    const store = new SystemDeviceSecretStore("darwin", async (command, args, input) => {
      calls.push({ command, args, input });
      return { code: 0, stdout: "", stderr: "" };
    });
    return { store, calls };
  }

  /** The `-w` value `security -i` reads from a command line: inside double quotes, `\x` stands for `x`. */
  function passwordSecurityReads(line: string): string | undefined {
    const quoted = /-w "((?:[^"\\]|\\.)*)"\n$/u.exec(line)?.[1];
    return quoted?.replace(/\\(.)/gu, "$1");
  }

  it("hands security the secret on stdin, never in an argv another user can list", async () => {
    const { store, calls } = recordingStore();
    const secret = JSON.stringify({ schemaVersion: 1, privateKeyPem: "-----BEGIN PRIVATE KEY-----\nMC4C\"\\x\n-----END PRIVATE KEY-----\n" });
    await store.write("profile-a", secret);
    expect(calls).toHaveLength(1);
    const [call] = calls;
    expect(call!.command).toBe("security");
    expect(call!.args.some(arg => arg.includes("MC4C"))).toBe(false);
    expect(passwordSecurityReads(call!.input!)).toBe(secret);
  });

  it("refuses, without running security, a secret security -i would cut short", async () => {
    const { store, calls } = recordingStore();
    await expect(store.write("profile-a", "first line\nsecond line")).rejects.toThrow("macOS Keychain write failed");
    await expect(store.write("profile-a", "x".repeat(4_000))).rejects.toThrow("macOS Keychain write failed");
    expect(calls).toEqual([]);
    await store.write("profile-a", "x".repeat(3_800));
    expect(passwordSecurityReads(calls[0]!.input!)).toBe("x".repeat(3_800));
  });
});
