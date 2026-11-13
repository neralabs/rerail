import { execFile } from "node:child_process";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolder } from "../../support/platform/remove-folder.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const HOST_SCRIPT = join(packageRoot, "scripts", "host", "windows-host.ps1");
const SMOKE_SCRIPT = join(packageRoot, "scripts", "host", "windows-install-smoke.ps1");
const run = promisify(execFile);
const folders: string[] = [];

afterEach(async () => {
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

async function scratchFolder(): Promise<string> {
  const folder = await mkdtemp(join(tmpdir(), "rerail-windows-host-"));
  folders.push(folder);
  return folder;
}

/**
 * Runs `script` in Windows PowerShell after windows-host.ps1. Started with no window and pipes for every stream, it
 * gets a console of its own, so a code page it sets reaches no other process. It runs by absolute path from a scratch
 * folder: the test's user profile does not exist, so PowerShell 5.1 would write its module cache relative to the
 * current folder, and Windows looks there for a bare program name before PATH.
 */
async function hostPowerShell(script: string, env: NodeJS.ProcessEnv = {}): Promise<string> {
  const folder = await scratchFolder();
  const command = `$ErrorActionPreference = 'Stop'\n. $env:RERAIL_TEST_HOST_SCRIPT\n${script}`;
  const powershell = join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const { stdout } = await run(powershell, ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(command, "utf16le").toString("base64")], {
    cwd: folder,
    env: { ...process.env, ...env, RERAIL_TEST_HOST_SCRIPT: HOST_SCRIPT, PSModuleAnalysisCachePath: join(folder, "ModuleAnalysisCache") },
    windowsHide: true,
    timeout: 60_000,
  });
  return stdout.trim();
}

describe.runIf(process.platform === "win32")("the Windows install checks' shared steps", () => {
  it("read a child's output as the UTF-8 node writes, on a console whose code page is 437", async () => {
    // A person's console, as on an en-US install; set-up's JSON names the sandbox profile "José García".
    const answer = await hostPowerShell(String.raw`
[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(437)
$name = "Jos" + [char]0x00E9 + " Garc" + [char]0x00ED + "a"
$run = Invoke-RerailCaptured $env:RERAIL_TEST_NODE @("-e", "process.stdout.write('Jos\u00e9 Garc\u00eda'); process.stderr.write('Jos\u00e9 Garc\u00eda')") $env:RERAIL_TEST_FOLDER
"stdout $($run.Out -ceq $name), stderr $($run.Err -ceq $name)"
`, { RERAIL_TEST_NODE: process.execPath, RERAIL_TEST_FOLDER: await scratchFolder() });
    expect(answer).toBe("stdout True, stderr True");
  }, 60_000);

  it("split the user Path entries inside a sandbox folder, in any case and with a trailing backslash, from the rest, which stays in place", async () => {
    const answer = await hostPowerShell(String.raw`
$sandbox = $env:RERAIL_TEST_SANDBOX
$path = '%SystemRoot%\system32;' + $sandbox + ';C:\Tools;' + $sandbox.ToUpperInvariant() + '\profile\.rerail\bin\;;%RERAIL_TEST_SANDBOX%\other;' + $sandbox + '-sibling\bin'
$split = Split-RerailPathUnder $path $sandbox
"$($split.Under -join '|')>$($split.Rest)"
`, { RERAIL_TEST_SANDBOX: String.raw`C:\Temp\rerail-windows-smoke-1` });
    expect(answer).toBe(String.raw`C:\Temp\rerail-windows-smoke-1|C:\TEMP\RERAIL-WINDOWS-SMOKE-1\profile\.rerail\bin\>%SystemRoot%\system32;C:\Tools;;%RERAIL_TEST_SANDBOX%\other;C:\Temp\rerail-windows-smoke-1-sibling\bin`);
  }, 60_000);

  it("keep a person's entry written with %USERPROFILE% while the smoke points USERPROFILE into its sandbox", async () => {
    // A new Windows account's user Path is `%USERPROFILE%\AppData\Local\Microsoft\WindowsApps`; the smoke sets
    // USERPROFILE to its sandbox profile before set-up, and clears the sandbox's entries with it still set.
    const answer = await hostPowerShell(String.raw`
$sandbox = $env:RERAIL_TEST_SANDBOX
$env:USERPROFILE = "$sandbox\profile"
$path = '%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;' + $sandbox + '\profile\.rerail\bin'
$split = Split-RerailPathUnder $path $sandbox
"$($split.Under -join '|')>$($split.Rest)"
`, { RERAIL_TEST_SANDBOX: String.raw`C:\Temp\rerail-windows-smoke-1` });
    expect(answer).toBe(String.raw`C:\Temp\rerail-windows-smoke-1\profile\.rerail\bin>%USERPROFILE%\AppData\Local\Microsoft\WindowsApps`);
  }, 60_000);
});

describe("the Windows install smoke", () => {
  it("takes its sandbox's launcher folder out of the user Path when it ends, passed or failed", async () => {
    // Set-up writes the folder into HKCU\Environment, which no variable the sandbox sets redirects. The call opens its
    // line, so a comment that names it does not count.
    const smoke = await readFile(SMOKE_SCRIPT, "utf8");
    const cleanup = smoke.slice(smoke.lastIndexOf("} finally {"));
    expect(cleanup).toMatch(/^\s*Clear-RerailUserPath -Folder \$base\b/mu);
  });
});
