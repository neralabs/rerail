/**
 * A file whose delete is pending, as Windows leaves one deleted while a handle with the delete on it stays open. A delete
 * that is not POSIX keeps the name until every handle closes: FAT, exFAT and network volumes delete that way, and so does
 * any program that deletes through FileDispositionInfo. A POSIX delete keeps it until the deleting handle closes. The name
 * stays, refusing every open, meanwhile.
 */
import { spawn } from "node:child_process";
import { once } from "node:events";
import { join } from "node:path";

const HOLDER = String.raw`
$sig = @'
using System; using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles;
public static class DeletePending {
  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  public static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
  [DllImport("kernel32.dll", SetLastError = true)]
  public static extern bool SetFileInformationByHandle(SafeFileHandle handle, int infoClass, ref uint info, int size);
  public static SafeFileHandle Handle;
  // GENERIC_READ and DELETE, every share mode, an existing file; then FileDispositionInfo (class 4) with Delete set, or
  // FileDispositionInfoEx (class 21) with FILE_DISPOSITION_FLAG_DELETE and FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.
  public static string Mark(string path, bool posix) {
    Handle = CreateFileW(path, 0x80000000u | 0x00010000u, 7u, IntPtr.Zero, 3u, 0u, IntPtr.Zero);
    if (Handle.IsInvalid) return "open failed " + Marshal.GetLastWin32Error();
    uint info = posix ? 3u : 1u;
    return SetFileInformationByHandle(Handle, posix ? 21 : 4, ref info, posix ? 4 : 1) ? "marked" : "mark failed " + Marshal.GetLastWin32Error();
  }
}
'@
Add-Type -TypeDefinition $sig
[DeletePending]::Mark($env:RERAIL_PENDING_PATH, $env:RERAIL_PENDING_POSIX -eq "1")
[void][Console]::In.ReadLine()
if ($null -ne [DeletePending]::Handle) { [DeletePending]::Handle.Close() }
`;

/**
 * Opens the file `path` with DELETE access and every share mode, marks it for deletion, through FileDispositionInfo (not
 * POSIX) or with `posix` through FileDispositionInfoEx's POSIX semantics, and holds the handle until `release`, which
 * closes it: the file is gone once it returns. Windows only.
 */
export async function holdDeletePending(path: string, { posix = false }: { posix?: boolean } = {}): Promise<{ release(): Promise<void> }> {
  const powershell = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const script = Buffer.from(HOLDER, "utf16le").toString("base64");
  const holder = spawn(powershell, ["-NoProfile", "-NonInteractive", "-EncodedCommand", script], {
    env: { ...process.env, RERAIL_PENDING_PATH: path, RERAIL_PENDING_POSIX: posix ? "1" : "0" }, stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  const closed = once(holder, "close");
  let said = "";
  holder.stdout.setEncoding("utf8");
  const marked = new Promise<void>((resolve, reject) => {
    holder.stdout.on("data", (chunk: string) => {
      said += chunk;
      if (said.includes("marked")) {
        resolve();
      } else if (/failed/u.test(said)) {
        holder.stdin.end();
        reject(new Error(`could not hold ${path} pending: ${said.trim()}`));
      }
    });
    void closed.then(() => reject(new Error(`the holder of ${path} ended: ${said.trim()}`)));
  });
  await marked;
  return {
    async release() {
      holder.stdin.end("\r\n");
      await closed;
    },
  };
}
