/**
 * Entry point of the Windows Claude Code hooks: `<node> <dist>/hook-launcher.js <recorded node> <dist>/<hook>.js`.
 *
 * Claude Code spawns these exec-form hooks without a shell, so the `cmd` command line the shell-form registrations use
 * is unavailable. The entry starts cmd.exe with each word as an argument of its own, and cmd starts the recorded node
 * itself as `<node>` while it exists and `node.exe` from PATH once it is gone: one node a hook event, whatever PATH
 * Claude Code has. Where cmd.exe cannot start the paths, `<node>` is `node`, which Claude Code resolves from its PATH,
 * and so it is in every entry 0.1.4 and earlier wrote and in those 0.1.5 to 0.2.1 wrote where set-up's PATH found the
 * recorded node; where it found another, 0.1.5 to 0.2.1 wrote the cmd.exe entry.
 * The launcher runs the hook under the node
 * recorded at registration, in this process when that node started it, falling back to this node once that binary is
 * gone. A version-manager upgrade (nvm-windows, Volta, fnm) removes it and would otherwise break every hook until
 * `rerail setup` runs again. The recorded-first order keeps the shell launchers' version guarantee, so the hook still
 * loads the SQLite binding built for that node when a project pins an older node on PATH. Kept free of dependencies,
 * Node's built-ins, Rerail's child-process helper and `same-file.ts` aside, so whatever node PATH supplies can run it.
 */
import { existsSync } from "node:fs";
import * as nodeModule from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "../engine/child-process.js";
import { sameFile } from "./same-file.js";

/**
 * Turns on Node's compile cache in this process (`module.enableCompileCache`) in the folder Node takes itself:
 * `node-compile-cache` in the temporary folder, or the one `NODE_COMPILE_CACHE` names, where Node turned the cache on
 * as it started. Each hook event starts a Node that loads the hook's bundle of over 2 MB, which V8 would otherwise
 * parse and compile again in every process. It changes no environment, so whatever the hook starts runs as it did
 * before. The cache stays off where `NODE_DISABLE_COMPILE_CACHE` turns it off, and a node without one, or a cache
 * folder that cannot be made, runs the hook as before.
 */
function turnOnCompileCache(): void {
  try {
    // A node before 22.8 has no such export, so the call throws there.
    nodeModule.enableCompileCache();
  } catch {
    // The hook runs without the cache, as on a node that has none.
  }
}

/**
 * A hook that could not run, or failed, answers nothing rather than failing: an agent shows a failed hook as an error
 * under every tool call. `{}` stands in for an answer the hook never wrote.
 */
function quiet(reason: string, answered: boolean): void {
  process.stderr.write(`rerail hook: ${reason}\n`);
  if (!answered) process.stdout.write("{}\n");
  process.exitCode = 0;
}

/**
 * Whether the hook has written to stdout since the call, as the answer a launcher collects from the one it starts
 * tells: it counts what goes through `process.stdout.write`, which a hook's answer and `console.log` use.
 */
function watchAnswer(): () => boolean {
  const write = process.stdout.write.bind(process.stdout);
  let written = false;
  // One signature stands in for the method's overloads: it passes their arguments on as they came.
  process.stdout.write = ((...args: Parameters<typeof write>) => {
    if (args[0].length > 0) written = true;
    return write(...args);
  }) as typeof process.stdout.write;
  return () => written;
}

// What the launcher passes in the recorded node's place when it starts itself under that node: that launcher runs the
// hook in its own process, so a recorded node whose process names another binary (a version manager's stand-in) starts
// no third launcher.
const HERE = "--here";

const [recordedNode, hookScript] = process.argv.slice(2);
if (recordedNode === undefined || hookScript === undefined) {
  process.stderr.write("Usage: node hook-launcher.js <recorded node> <hook script>\n");
  process.exit(1);
}

if (recordedNode !== HERE && existsSync(recordedNode) && !sameFile(recordedNode, process.execPath)) {
  // The launcher starts itself under the recorded node, as this node started it, so the hook runs in that launcher's
  // process with the compile cache on whatever this node has. The event JSON on stdin goes straight to it; its answer
  // comes back through here, so a launcher that wrote none can be answered for. This node's own flags go on with it,
  // so the `--import tsx` the tests start the launcher with reaches the second launcher; neither registered entry
  // (`nodeHookEntry`, `cmdHookLine` in `harness-registration.ts`) passes a Node flag, so for a person's hooks the list
  // is empty.
  const result = spawnSync(recordedNode, [...process.execArgv, fileURLToPath(import.meta.url), HERE, hookScript], { stdio: ["inherit", "pipe", "inherit"] });
  const answer = result.stdout ?? Buffer.alloc(0);
  process.stdout.write(answer);
  if (result.error !== undefined) quiet(result.error.message, answer.length > 0);
  else if (result.status !== 0) quiet(result.status === null ? `killed by ${result.signal ?? "a signal"}` : `exited ${result.status}`, answer.length > 0);
} else {
  // Run in this process exactly as `node <hook>` would: a hook serves only when it is the launched script, argv[1].
  // The hook is chosen by the registration's argument, so it cannot be a static import.
  turnOnCompileCache();
  process.argv.splice(1, 3, hookScript);
  const answered = watchAnswer();
  // A hook that fails here exits 0 all the same, with `{}` when it wrote nothing. Whether it ends the process itself,
  // throws after it has loaded or leaves a failing exit code is known once the process ends.
  process.on("exit", code => {
    if (code !== 0) quiet(`exited ${code}`, answered());
  });
  try {
    await import(pathToFileURL(hookScript).href);
  } catch (error) {
    quiet(error instanceof Error ? error.message : String(error), answered());
  }
}
