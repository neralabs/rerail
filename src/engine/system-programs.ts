import { win32 } from "node:path";

/**
 * The absolute path of a program Windows ships in System32. Rerail starts these by absolute path: Windows looks for a
 * bare program name in the current folder before PATH, and Rerail runs inside repositories it does not trust, so a
 * `powershell.exe` or `rundll32.exe` committed to one would run instead, and could read what Rerail hands it.
 */
export function windowsSystemProgram(name: string, env: NodeJS.ProcessEnv = process.env): string {
  return win32.join(env.SystemRoot || String.raw`C:\Windows`, "System32", name);
}

/** Windows PowerShell 5.1, which every Windows 10 and 11 install carries, by absolute path. */
export function windowsPowerShell(env: NodeJS.ProcessEnv = process.env): string {
  return windowsSystemProgram(win32.join("WindowsPowerShell", "v1.0", "powershell.exe"), env);
}
