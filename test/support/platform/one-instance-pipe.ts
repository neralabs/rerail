import { spawn } from "node:child_process";
import { once } from "node:events";

/** How a one-instance pipe's holder waits for its client: the .NET `PipeOptions` it is made with. */
export interface OneInstancePipe {
  /** `None` leaves `WaitForConnection` to wait for the client; `Asynchronous` lets `BeginWaitForConnection` return. */
  readonly options: "None" | "Asynchronous";
  /** The output buffer's size; 0, .NET's default, when not given. The input buffer is always 0. */
  readonly outBufferSize?: number;
}

/**
 * The Windows PowerShell lines that make `$pipe`, the one instance the pipe `name` (its path without `\\.\pipe\`)
 * allows, with no input buffer, through .NET calls alone. A holder calls no cmdlet before it says its pipe exists:
 * Windows PowerShell finds a cmdlet's module by reading every module on PSModulePath that it has not read before
 * ("Preparing modules for first use"), so a script that called `Add-Type` or `New-Object` first made its test wait,
 * before the pipe existed, for as long as the machine's modules take to read. That wait has no bound of its own and
 * grows with what the machine has installed.
 */
export function oneInstancePipeLines(name: string, pipe: OneInstancePipe): string[] {
  return [
    "$ErrorActionPreference = 'Stop'",
    `$pipe = [System.IO.Pipes.NamedPipeServerStream]::new('${name}', [System.IO.Pipes.PipeDirection]::InOut, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::${pipe.options}, 0, ${String(pipe.outBufferSize ?? 0)})`,
  ];
}

/**
 * A stand-in for a process that holds the Windows pipe `path` with one instance at most and does nothing else: Node
 * cannot cap a pipe's instances, so PowerShell makes it. Returns once the instance exists; `end` ends the holder. The
 * handshake ends on the holder's first output or its exit, whichever comes first; a holder that wrote anything else
 * is ended before the error that says so leaves, so no failed start leaves a PowerShell waiting on its input.
 */
export async function oneInstanceHolder(path: string): Promise<{ end: () => Promise<void> }> {
  // The pipe's name as PowerShell takes it: the path without `\\.\pipe\`.
  const name = path.replace(/^\\\\\.\\pipe\\/u, "");
  const script = [
    ...oneInstancePipeLines(name, { options: "Asynchronous" }),
    "[void]$pipe.BeginWaitForConnection($null, $null)",
    "[Console]::Out.WriteLine('holding'); [Console]::Out.Flush()",
    "[void][Console]::In.ReadLine()",
  ].join("\n");
  const holder = spawn("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")], {
    stdio: ["pipe", "pipe", "inherit"],
    windowsHide: true,
  });
  const closed = once(holder, "close");
  // A holder that has exited takes no input: ending its stdin then fails with EPIPE, and `closed` says it ended.
  holder.stdin!.on("error", () => undefined);
  try {
    const first = await Promise.race([once(holder.stdout!, "data"), closed]);
    if (!/^holding/u.test(String(first[0]))) throw new Error(`the one-instance pipe holder did not start: ${String(first[0])}`);
  } catch (error) {
    holder.kill();
    await closed.catch(() => undefined);
    throw error;
  }
  return {
    end: async () => {
      holder.stdin!.end();
      await closed;
    },
  };
}
