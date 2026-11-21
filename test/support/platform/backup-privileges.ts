import { execFileSync } from "node:child_process";
import { join } from "node:path";

/**
 * The Windows privileges that let a program open a file whatever its ACL says. Node opens every file with
 * FILE_FLAG_BACKUP_SEMANTICS, so while Backup is on a read passes a deny entry, and while Restore is on a write or a new
 * file does. An elevated administrator's token holds both, off; Git Bash turns them on in each process it starts, and
 * so in the Node that `npm test` starts under it, as on GitHub's Windows runner.
 */
const BACKUP_PRIVILEGES = ["SeBackupPrivilege", "SeRestorePrivilege"] as const;

/**
 * Windows PowerShell: sets the privileges named in RERAIL_TEST_PRIVILEGES, in the token of process RERAIL_TEST_PID, to
 * RERAIL_TEST_STATE (0 off, 2 on), and prints each one that was on before. One the token does not hold is passed over.
 */
const SET_PRIVILEGES = String.raw`
Add-Type -Namespace RerailTest -Name Token -MemberDefinition @'
[StructLayout(LayoutKind.Sequential, Pack = 4)] public struct One { public uint Count; public uint Low; public int High; public uint Attributes; }
[DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
[DllImport("advapi32.dll", SetLastError = true)] public static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
[DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool LookupPrivilegeValue(string system, string name, out long luid);
[DllImport("advapi32.dll", SetLastError = true)] public static extern bool AdjustTokenPrivileges(IntPtr token, bool all, ref One state, uint length, ref One previous, out uint returned);
'@
$process = [RerailTest.Token]::OpenProcess(0x0400, $false, [int]$env:RERAIL_TEST_PID)
$token = [IntPtr]::Zero
if (-not [RerailTest.Token]::OpenProcessToken($process, 0x28, [ref]$token)) { throw "OpenProcessToken failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
foreach ($name in $env:RERAIL_TEST_PRIVILEGES.Split(' ')) {
  $luid = [long]0
  if (-not [RerailTest.Token]::LookupPrivilegeValue($null, $name, [ref]$luid)) { throw "LookupPrivilegeValue failed for $name" }
  $state = New-Object RerailTest.Token+One
  $state.Count = 1; $state.Low = [uint32]($luid -band 0xffffffffL); $state.High = [int]($luid -shr 32); $state.Attributes = [uint32]$env:RERAIL_TEST_STATE
  $previous = New-Object RerailTest.Token+One
  $length = [uint32]0
  if (-not [RerailTest.Token]::AdjustTokenPrivileges($token, $false, [ref]$state, 16, [ref]$previous, [ref]$length)) { throw "AdjustTokenPrivileges failed for $name" }
  if ($previous.Count -eq 1 -and ($previous.Attributes -band 2)) { $name }
}
`;

/** Sets `names` in this process's token to `state`, by {@link SET_PRIVILEGES}; returns those that were on. */
function setPrivileges(names: readonly string[], state: 0 | 2): string[] {
  const powershell = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const printed = execFileSync(powershell, [
    "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(SET_PRIVILEGES, "utf16le").toString("base64"),
  ], { encoding: "utf8", env: { ...process.env, RERAIL_TEST_PID: String(process.pid), RERAIL_TEST_PRIVILEGES: names.join(" "), RERAIL_TEST_STATE: String(state) } });
  return printed.split(/\s+/u).filter(name => names.includes(name));
}

/**
 * Runs `body` with this process's Backup and Restore privileges off ({@link BACKUP_PRIVILEGES}), and then turns on again
 * those that were on, so an ACL a test sets binds this process as it binds every account that holds neither on: a
 * person's program, an elevated one started from PowerShell or cmd included. Elsewhere than Windows it runs `body`.
 */
export async function withoutBackupPrivileges<T>(body: () => Promise<T>): Promise<T> {
  if (process.platform !== "win32") return await body();
  const wereOn = setPrivileges(BACKUP_PRIVILEGES, 0);
  try {
    return await body();
  } finally {
    if (wereOn.length > 0) setPrivileges(wereOn, 2);
  }
}
