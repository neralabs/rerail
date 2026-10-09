/**
 * Entry point of the Windows Claude Code hooks: `node <dist>/hook-launcher.js <recorded node> <dist>/<hook>.js`.
 *
 * Claude Code spawns these exec-form hooks without a shell, so the `cmd` fallback the shell-form registrations use
 * is unavailable. Claude Code resolves `node` from PATH and starts this file. The launcher then runs the hook under the node
 * recorded at registration, falling back to this node once that binary is gone. A version-manager upgrade (nvm-windows,
 * Volta, fnm) removes it and would otherwise break every hook until `rerail setup` runs again. The recorded-first
 * order keeps the shell launchers' version guarantee, so the hook still loads the SQLite binding built for that node when
 * a project pins an older node on PATH. Kept free of dependencies so whatever node PATH supplies can run it.
 */
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { pathToFileURL } from "node:url";

function sameFile(left: string, right: string): boolean {
  try {
    return realpathSync.native(left) === realpathSync.native(right);
  } catch {
    return false;
  }
}

const [recordedNode, hookScript] = process.argv.slice(2);
if (recordedNode === undefined || hookScript === undefined) {
  process.stderr.write("Usage: node hook-launcher.js <recorded node> <hook script>\n");
  process.exit(1);
}

if (existsSync(recordedNode) && !sameFile(recordedNode, process.execPath)) {
  // Inherited stdio hands the event JSON on stdin straight to the hook and its decision straight back to Claude Code.
  const result = spawnSync(recordedNode, [hookScript], { stdio: "inherit", windowsHide: true });
  if (result.error !== undefined) process.stderr.write(`${result.error.message}\n`);
  process.exitCode = result.status ?? 1;
} else {
  // Run in this process exactly as `node <hook>` would: a hook serves only when it is the launched script, argv[1].
  // The hook is chosen by the registration's argument, so it cannot be a static import.
  process.argv.splice(1, 3, hookScript);
  await import(pathToFileURL(hookScript).href);
}
