import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { stopDaemon } from "../src/daemon-control.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const run = promisify(execFile);
const homes: string[] = [];

afterEach(async () => {
  await Promise.all(homes.splice(0).map(async home => {
    await stopDaemon(home);
    await rm(home, { recursive: true, force: true });
  }));
});

/**
 * PowerShell holds an inheritable pipe client, as a Rust launcher such as `just` holds its own stdout, and starts
 * the launcher with handle inheritance. Once the launcher exits and PowerShell drops its copy, the pipe reaches
 * end of file only if the daemon did not inherit the client handle too.
 */
const PARENT_SCRIPT = String.raw`
$server = New-Object System.IO.Pipes.AnonymousPipeServerStream([System.IO.Pipes.PipeDirection]::In, [System.IO.HandleInheritability]::Inheritable)
$start = New-Object System.Diagnostics.ProcessStartInfo($env:RERAIL_TEST_NODE)
$start.Arguments = '--import tsx "' + $env:RERAIL_TEST_LAUNCHER + '"'
$start.WorkingDirectory = $env:RERAIL_TEST_ROOT
$start.UseShellExecute = $false
$launcher = [System.Diagnostics.Process]::Start($start)
$launcher.WaitForExit()
if ($launcher.ExitCode -ne 0) { "launcher exited with $($launcher.ExitCode)"; exit 1 }
$server.DisposeLocalCopyOfClientHandle()
$read = $server.ReadAsync((New-Object byte[] 1), 0, 1)
if ($read.Wait(5000)) { "closed" } else { "held" }
`;

describe.runIf(process.platform === "win32")("Windows daemon launch", () => {
  it("does not hand the daemon a pipe its launcher inherited", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-windows-launch-"));
    homes.push(home);
    const launcher = join(home, "launch.mjs");
    await writeFile(launcher, [
      `import { startDaemon } from ${JSON.stringify(pathToFileURL(join(packageRoot, "src", "daemon-control.ts")).href)};`,
      `await startDaemon({ home: ${JSON.stringify(home)}, packageRoot: ${JSON.stringify(packageRoot)} },`,
      `  { dev: true, transcriptDatabasePath: ${JSON.stringify(join(home, "transcripts.sqlite"))}, port: 0 });`,
    ].join("\n"));

    const { stdout } = await run(
      "powershell.exe",
      ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(PARENT_SCRIPT, "utf16le").toString("base64")],
      { env: { ...process.env, RERAIL_TEST_NODE: process.execPath, RERAIL_TEST_LAUNCHER: launcher, RERAIL_TEST_ROOT: packageRoot } },
    );
    expect(stdout.trim()).toBe("closed");
  }, 60_000);
});
