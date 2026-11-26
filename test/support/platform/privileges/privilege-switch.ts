import { type ChildProcessByStdio, spawn } from "node:child_process";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";

/** A program and the arguments that go before the switch's own: Windows PowerShell, or a test's stand-in for it. */
export type Program = readonly [string, ...string[]];

/** Windows PowerShell 5.1, which every Windows 10 and 11 machine holds. */
export const WINDOWS_POWERSHELL: Program = [
  join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe"),
];

/**
 * Windows PowerShell: opens the token of process RERAIL_TEST_PID, prints `ready`, then takes one request per line of its
 * input, a state (0 off, 2 on) and the privileges to set to it, sets each and prints `done` and each one that was on
 * before. One the token does not hold is passed over. It reads its input through `$input`, which PowerShell's host feeds
 * line by line as the lines come, and ends when that input ends: when the process that started it closes it or ends.
 */
const SWITCH = String.raw`
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
[Console]::Out.WriteLine('ready')
[Console]::Out.Flush()
foreach ($line in $input) {
  $wanted, $names = $line.Split(' ')
  $wereOn = foreach ($name in $names) {
    $luid = [long]0
    if (-not [RerailTest.Token]::LookupPrivilegeValue($null, $name, [ref]$luid)) { throw "LookupPrivilegeValue failed for $name" }
    $state = New-Object RerailTest.Token+One
    $state.Count = 1; $state.Low = [uint32]($luid -band 0xffffffffL); $state.High = [int]($luid -shr 32); $state.Attributes = [uint32]$wanted
    $previous = New-Object RerailTest.Token+One
    $length = [uint32]0
    if (-not [RerailTest.Token]::AdjustTokenPrivileges($token, $false, [ref]$state, 16, [ref]$previous, [ref]$length)) { throw "AdjustTokenPrivileges failed for $name" }
    if ($previous.Count -eq 1 -and ($previous.Attributes -band 2)) { $name }
  }
  [Console]::Out.WriteLine("done $wereOn")
  [Console]::Out.Flush()
}
`;

/** The most of the switch's error output kept for its failure's message: its end, where PowerShell's error is. */
const ERROR_OUTPUT_KEPT = 16_384;

/**
 * What PowerShell wrote to its error output, as text. Started with `-EncodedCommand`, Windows PowerShell writes its
 * error and progress records there as CLIXML (`#< CLIXML`, then `<Objs>`): this gives the text of its error records,
 * with the characters CLIXML escapes as `_xHHHH_` and XML as entities put back.
 */
export function powerShellWords(errorOutput: string): string {
  if (!errorOutput.startsWith("#< CLIXML")) return errorOutput.trim();
  const errors = [...errorOutput.matchAll(/<S S="Error">([^<]*)<\/S>/gu)].map(match => match[1] ?? "").join("");
  return errors
    .replace(/_x([0-9A-Fa-f]{4})_/gu, (_, code: string) => String.fromCharCode(Number.parseInt(code, 16)))
    .replace(/&(lt|gt|quot|apos|amp);/gu, (_, name: string) => ({ lt: "<", gt: ">", quot: '"', apos: "'", amp: "&" })[name] ?? "")
    .trim();
}

/** One reply the switch owes: the next line it prints settles the oldest. */
interface Owed {
  resolve: (line: string) => void;
  reject: (error: Error) => void;
}

/**
 * One Windows PowerShell kept open, which sets privileges in this process's token when asked. Its replies come in the
 * order of the requests, so each line it prints answers the oldest request still owed.
 */
export class PrivilegeSwitch {
  readonly #program: string;
  readonly #child: ChildProcessByStdio<Writable, Readable, Readable>;
  readonly #owed: Owed[] = [];
  readonly #exited: Promise<void>;
  #errorOutput = "";
  #ended: Error | undefined;
  #ready = false;

  constructor(program: Program) {
    const [file, ...before] = program;
    this.#program = file;
    this.#child = spawn(file, [...before, "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(SWITCH, "utf16le").toString("base64")], {
      env: { ...process.env, RERAIL_TEST_PID: String(process.pid) },
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    this.#exited = new Promise(resolve => {
      this.#child.once("error", error => resolve(this.#fail(`${file} could not start: ${error.message}`)));
      this.#child.once("close", (code, signal) => resolve(this.#fail(this.#endedMessage(code, signal))));
    });
    createInterface({ input: this.#child.stdout, crlfDelay: Infinity }).on("line", line => this.#owed.shift()?.resolve(line.trim()));
    this.#child.stderr.setEncoding("utf8").on("data", (text: string) => {
      this.#errorOutput = (this.#errorOutput + text).slice(-ERROR_OUTPUT_KEPT);
    });
    // Writing to a switch that has ended fails with EPIPE; the request's own rejection says why it ended.
    this.#child.stdin.on("error", () => undefined);
  }

  /** Its process id, once started. */
  get pid(): number | undefined {
    return this.#child.pid;
  }

  /** Resolves once it says it is ready; rejects, in PowerShell's own words, when it ends first. */
  async ready(): Promise<void> {
    const line = await this.#reply();
    if (line !== "ready") throw new Error(`${this.#program} printed "${line}" where it says it is ready`);
    this.#ready = true;
  }

  /** Sets `names` to `state` (0 off, 2 on) in this process's token; resolves to those of them that were on before. */
  async set(names: readonly string[], state: 0 | 2): Promise<string[]> {
    const reply = this.#reply();
    this.#child.stdin.write(`${state} ${names.join(" ")}\n`);
    const line = await reply;
    if (!line.startsWith("done")) throw new Error(`${this.#program} printed "${line}" where it answers a request`);
    return line.split(" ").filter(name => names.includes(name));
  }

  /**
   * Closes its input, on which a ready switch ends by itself, and resolves once it has exited. One that never said it
   * was ready, still starting or stuck, has touched no token: it is ended at once.
   */
  async end(): Promise<void> {
    this.#child.stdin.end();
    if (!this.#ready) this.#child.kill();
    await this.#exited;
  }

  /** The next line it prints, or its end's reason when it ends first. */
  #reply(): Promise<string> {
    if (this.#ended !== undefined) return Promise.reject(this.#ended);
    return new Promise((resolve, reject) => this.#owed.push({ resolve, reject }));
  }

  /** Keeps the first reason it ended for, and rejects with it every reply still owed. */
  #fail(why: string): void {
    this.#ended ??= new Error(why);
    for (const owed of this.#owed.splice(0)) owed.reject(this.#ended);
  }

  #endedMessage(code: number | null, signal: NodeJS.Signals | null): string {
    const how = code === null ? `signal ${signal ?? "unknown"}` : `exit code ${code}`;
    const words = powerShellWords(this.#errorOutput);
    return `${this.#program} ended (${how}): ${words === "" ? "it wrote nothing to its error output" : words}`;
  }
}

/**
 * Starts a {@link PrivilegeSwitch} on `program` and resolves once it is ready, with no bound of its own: the caller's
 * hook bounds the wait. When it ends first, it rejects with PowerShell's own words; either way no process is left.
 */
export async function startPrivilegeSwitch(program: Program = WINDOWS_POWERSHELL): Promise<PrivilegeSwitch> {
  const started = new PrivilegeSwitch(program);
  try {
    await started.ready();
  } catch (error) {
    await started.end();
    throw error;
  }
  return started;
}
