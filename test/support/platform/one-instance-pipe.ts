import { spawn } from "node:child_process";
import { once } from "node:events";

/**
 * A stand-in for a process that holds the Windows pipe `path` with one instance at most and does nothing else: Node
 * cannot cap a pipe's instances, so PowerShell makes it. Returns once the instance exists; `end` ends the holder.
 */
export async function oneInstanceHolder(path: string): Promise<{ end: () => Promise<void> }> {
  // The pipe's name as PowerShell takes it: the path without `\\.\pipe\`.
  const name = path.replace(/^\\\\\.\\pipe\\/u, "");
  const script = [
    "$ErrorActionPreference = 'Stop'",
    "Add-Type -AssemblyName System.Core",
    `$pipe = New-Object System.IO.Pipes.NamedPipeServerStream('${name}', [System.IO.Pipes.PipeDirection]::InOut, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::Asynchronous)`,
    "[void]$pipe.BeginWaitForConnection($null, $null)",
    "[Console]::Out.WriteLine('holding'); [Console]::Out.Flush()",
    "[void][Console]::In.ReadLine()",
  ].join("\n");
  const holder = spawn("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")], {
    stdio: ["pipe", "pipe", "inherit"],
    windowsHide: true,
  });
  const closed = once(holder, "close");
  const first = await Promise.race([once(holder.stdout!, "data"), closed]);
  if (!/^holding/u.test(String(first[0]))) throw new Error(`the one-instance pipe holder did not start: ${String(first[0])}`);
  return {
    end: async () => {
      holder.stdin!.end();
      await closed;
    },
  };
}
