import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { defaultRerailHome } from "./rerail-home.js";

export interface DeviceSecretStore {
  read(profileName: string): Promise<string | undefined>;
  write(profileName: string, value: string): Promise<void>;
  remove(profileName: string): Promise<void>;
}

interface ProcessResult {
  code: number;
  stdout: string;
  stderr: string;
}
const WINDOWS_CREDENTIAL_SCRIPT = `
$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class RerailCredentialManager {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  private struct Credential {
    public uint Flags;
    public uint Type;
    public string TargetName;
    public string Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public uint CredentialBlobSize;
    public IntPtr CredentialBlob;
    public uint Persist;
    public uint AttributeCount;
    public IntPtr Attributes;
    public string TargetAlias;
    public string UserName;
  }

  [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
  private static extern bool CredRead(string target, uint type, uint flags, out IntPtr credential);

  [DllImport("advapi32.dll", EntryPoint = "CredWriteW", CharSet = CharSet.Unicode, SetLastError = true)]
  private static extern bool CredWrite(ref Credential credential, uint flags);

  [DllImport("advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)]
  private static extern bool CredDelete(string target, uint type, uint flags);

  [DllImport("advapi32.dll")]
  private static extern void CredFree(IntPtr credential);

  public static string Read(string target) {
    IntPtr pointer;
    if (!CredRead(target, 1, 0, out pointer)) {
      int error = Marshal.GetLastWin32Error();
      if (error == 1168) return null;
      throw new Win32Exception(error);
    }
    try {
      Credential credential = Marshal.PtrToStructure<Credential>(pointer);
      byte[] bytes = new byte[credential.CredentialBlobSize];
      if (bytes.Length > 0) Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
      return Encoding.UTF8.GetString(bytes);
    } finally {
      CredFree(pointer);
    }
  }

  public static void Write(string target, string value) {
    byte[] bytes = Encoding.UTF8.GetBytes(value);
    IntPtr blob = Marshal.AllocHGlobal(bytes.Length);
    try {
      if (bytes.Length > 0) Marshal.Copy(bytes, 0, blob, bytes.Length);
      Credential credential = new Credential {
        Type = 1,
        TargetName = target,
        CredentialBlobSize = (uint)bytes.Length,
        CredentialBlob = blob,
        Persist = 2,
        UserName = "Rerail"
      };
      if (!CredWrite(ref credential, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
    } finally {
      Marshal.FreeHGlobal(blob);
    }
  }

  public static void Delete(string target) {
    if (CredDelete(target, 1, 0)) return;
    int error = Marshal.GetLastWin32Error();
    if (error != 1168) throw new Win32Exception(error);
  }
}
"@
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
switch ($request.operation) {
  "read" {
    $value = [RerailCredentialManager]::Read($request.target)
    if ($null -eq $value) {
      [Console]::Out.Write('{"found":false}')
    } else {
      $encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($value))
      [Console]::Out.Write((@{ found = $true; value = $encoded } | ConvertTo-Json -Compress))
    }
  }
  "write" {
    $value = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($request.value))
    [RerailCredentialManager]::Write($request.target, $value)
  }
  "remove" { [RerailCredentialManager]::Delete($request.target) }
  default { throw "Unknown Rerail credential operation" }
}
`;

async function executeWindowsCredential(
  operation: "read" | "write" | "remove",
  profileName: string,
  value?: string,
): Promise<string | undefined> {
  const target = `com.neralabs.rerail.device:${profileName}`;
  const input = JSON.stringify({
    operation,
    target,
    ...(value === undefined ? {} : { value: Buffer.from(value, "utf8").toString("base64") }),
  });
  const result = await execute(
    "powershell.exe",
    ["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", WINDOWS_CREDENTIAL_SCRIPT],
    input,
  );
  if (result.code !== 0) throw new Error(`Windows Credential Manager ${operation} failed: ${result.stderr.trim()}`);
  if (operation !== "read") return undefined;
  const response = JSON.parse(result.stdout) as { found?: unknown; value?: unknown };
  if (response.found === false) return undefined;
  if (response.found !== true || typeof response.value !== "string") {
    throw new Error("Windows Credential Manager returned an invalid response");
  }
  return Buffer.from(response.value, "base64").toString("utf8");
}

async function execute(command: string, args: readonly string[], input?: string): Promise<ProcessResult> {
  const { promise, resolve, reject } = Promise.withResolvers<ProcessResult>();
  const child = spawn(command, [...args], { stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.once("error", reject);
  child.once("exit", (code) => resolve({ code: code ?? 1, stdout, stderr }));
  child.stdin.end(input);
  return await promise;
}

function assertProfileName(profileName: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(profileName)) {
    throw new Error(`Invalid Rerail profile name: ${profileName}`);
  }
}

export class SystemDeviceSecretStore implements DeviceSecretStore {
  async read(profileName: string): Promise<string | undefined> {
    assertProfileName(profileName);
    if (process.platform === "win32") return await executeWindowsCredential("read", profileName);
    if (process.platform === "darwin") {
      const result = await execute("security", [
        "find-generic-password",
        "-a",
        profileName,
        "-s",
        "com.neralabs.rerail.device",
        "-w",
      ]);
      if (result.code === 44) return undefined;
      if (result.code !== 0) throw new Error(`macOS Keychain lookup failed: ${result.stderr.trim()}`);
      return result.stdout.replace(/\r?\n$/u, "");
    }
    if (process.platform === "linux") {
      const result = await execute("secret-tool", ["lookup", "application", "rerail", "profile", profileName]);
      if (result.code === 1) return undefined;
      if (result.code !== 0) throw new Error(`Secret Service lookup failed: ${result.stderr.trim()}`);
      const value = result.stdout.replace(/\r?\n$/u, "");
      return value.length === 0 ? undefined : value;
    }
    throw new Error(`Secure Rerail device storage is unsupported on ${process.platform}`);
  }

  async write(profileName: string, value: string): Promise<void> {
    assertProfileName(profileName);
    if (process.platform === "win32") {
      await executeWindowsCredential("write", profileName, value);
      return;
    }
    if (process.platform === "darwin") {
      const result = await execute("security", [
        "add-generic-password",
        "-U",
        "-a",
        profileName,
        "-s",
        "com.neralabs.rerail.device",
        "-w",
        value,
      ]);
      if (result.code !== 0) throw new Error(`macOS Keychain write failed: ${result.stderr.trim()}`);
      return;
    }
    if (process.platform === "linux") {
      const result = await execute(
        "secret-tool",
        ["store", "--label", `Rerail device identity (${profileName})`, "application", "rerail", "profile", profileName],
        value,
      );
      if (result.code !== 0) throw new Error(`Secret Service write failed: ${result.stderr.trim()}`);
      return;
    }
    throw new Error(`Secure Rerail device storage is unsupported on ${process.platform}`);
  }

  async remove(profileName: string): Promise<void> {
    assertProfileName(profileName);
    if (process.platform === "win32") {
      await executeWindowsCredential("remove", profileName);
      return;
    }
    if (process.platform === "darwin") {
      const result = await execute("security", [
        "delete-generic-password",
        "-a",
        profileName,
        "-s",
        "com.neralabs.rerail.device",
      ]);
      if (result.code !== 0 && result.code !== 44) {
        throw new Error(`macOS Keychain removal failed: ${result.stderr.trim()}`);
      }
      return;
    }
    if (process.platform === "linux") {
      const result = await execute("secret-tool", ["clear", "application", "rerail", "profile", profileName]);
      if (result.code !== 0) throw new Error(`Secret Service removal failed: ${result.stderr.trim()}`);
      return;
    }
    throw new Error(`Secure Rerail device storage is unsupported on ${process.platform}`);
  }
}

/**
 * Device secrets as owner-only files under `<RERAIL_HOME>/device-secrets/`, unencrypted. Selected only
 * by `RERAIL_DEVICE_SECRET_STORE=file`; test runs use it so they never touch the system keychain.
 */
export class FileDeviceSecretStore implements DeviceSecretStore {
  constructor(private readonly directory: string) {}

  #path(profileName: string): string {
    assertProfileName(profileName);
    return join(this.directory, profileName);
  }

  async read(profileName: string): Promise<string | undefined> {
    try {
      return await readFile(this.#path(profileName), "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  async write(profileName: string, value: string): Promise<void> {
    const path = this.#path(profileName);
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const staging = `${path}.${randomUUID()}.tmp`;
    try {
      await writeFile(staging, value, { mode: 0o600, flag: "wx" });
      await rename(staging, path);
    } catch (error) {
      await rm(staging, { force: true });
      throw error;
    }
  }

  async remove(profileName: string): Promise<void> {
    await rm(this.#path(profileName), { force: true });
  }
}

/**
 * The device secret store for this process: the system store (Keychain, Secret Service, Credential
 * Manager) unless `RERAIL_DEVICE_SECRET_STORE=file`. A test run (`VITEST` set, inherited by every CLI it
 * spawns) must choose the file store; falling back to the system store there is an error, not a dialog.
 */
export function deviceSecretStore(environment: NodeJS.ProcessEnv = process.env): DeviceSecretStore {
  const selected = environment.RERAIL_DEVICE_SECRET_STORE;
  if (selected === "file") {
    return new FileDeviceSecretStore(join(defaultRerailHome(environment), "device-secrets"));
  }
  if (selected !== undefined && selected !== "system") {
    throw new Error(`RERAIL_DEVICE_SECRET_STORE must be system or file, not ${selected}`);
  }
  if (environment.VITEST !== undefined) {
    throw new Error("Tests must not use the system keychain; set RERAIL_DEVICE_SECRET_STORE=file");
  }
  return new SystemDeviceSecretStore();
}
