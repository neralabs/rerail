import { realpath } from "node:fs/promises";
import { join } from "node:path";
import { spawn } from "node:child_process";
import type { Readable } from "node:stream";
import { captureSession, stopSession } from "./process.js";
/** The supervisor supplies the workspace; neither the worker nor browser chooses it. */
export async function executeSandboxCommand(root: string, command: string, signal: AbortSignal): Promise<string> {
  signal.throwIfAborted();
  root = await realpath(root);
  const executables = [
    "/bin/bash",
    "/bin/cat",
    "/bin/echo",
    "/bin/pwd",
    "/bin/mkdir",
    "/bin/rm",
    "/bin/cp",
    "/bin/mv",
    "/bin/ls",
    "/usr/bin/sed",
    "/usr/bin/cut",
    "/usr/bin/sort",
    "/usr/bin/uniq",
    "/usr/bin/wc",
    "/usr/bin/touch",
    "/usr/bin/tr",
    "/usr/bin/head",
    "/usr/bin/tail",
    "/usr/bin/printf",
    "/usr/bin/true",
    "/usr/bin/false",
    "/usr/bin/tee",
    "/usr/bin/diff",
    "/usr/bin/grep",
  ];
  // Inline policy cannot be replaced by a worker-writable profile file. macOS
  // cannot reapply seatbelt inside an already sandboxed worker, hence the broker.
  const executableLiterals = executables.map((path) => `(literal ${JSON.stringify(path)})`).join(" ");
  const profile = [
    "(version 1)",
    "(deny default)",
    "(allow process-fork)",
    `(allow process-exec ${executableLiterals})`,
    "(allow sysctl-read)",
    // dyld opens the root directory itself, not its descendants.
    '(allow file-read* (literal "/"))',
    '(allow file-read* (subpath "/bin") (subpath "/usr/bin") (subpath "/usr/lib") (subpath "/System/Library") (literal "/dev/null") (literal "/dev/urandom"))',
    `(allow file-read* file-write* (subpath ${JSON.stringify(root)}))`,
    ...[".rerail", ".playground-home", ".git"].map(
      (name) => `(deny file-read* file-write* (subpath ${JSON.stringify(join(root, name))}))`,
    ),
    `(deny file-map-executable (subpath ${JSON.stringify(root)}))`,
    '(allow file-write* (literal "/dev/null"))',
    "(deny network*)",
  ].join("\n");
  signal.throwIfAborted();
  // Keep the session leader alive until its real SID is captured. The gate
  // runs outside seatbelt; only the fixed sandbox-exec command follows it.
  const child = spawn("/bin/bash", [
    "--noprofile", "--norc", "-c", 'read -r gate; exec "$@"', "playground-command",
    "/usr/bin/sandbox-exec", "-p", profile, "/bin/bash", "--noprofile", "--norc", "-c", command,
  ], {
    cwd: root,
    env: { PATH: "/usr/bin:/bin", HOME: "/nonexistent", TMPDIR: root, LANG: "en_US.UTF-8" },
    stdio: ["pipe", "pipe", "pipe"],
    detached: true,
  });
  const exited = Promise.withResolvers<number>();
  child.once("error", exited.reject);
  child.once("exit", code => exited.resolve(code ?? 1));
  let session: number;
  try {
    session = await captureSession(child.pid!);
  } catch (error) {
    child.kill("SIGKILL");
    await exited.promise;
    throw error;
  }
  child.stdin.end("start\n");
  let exceeded = false;
  let size = 0;
  // Job control can create another process group, but the allowlisted programs
  // cannot create another session. Reclaim the whole detached spawn session.
  let stopping: Promise<void> | undefined;
  const stop = () => (stopping ??= stopSession(session));
  const cancelled = Promise.withResolvers<void>();
  const kill = () => {
    void stop().then(cancelled.resolve, cancelled.reject);
  };
  const timer = setTimeout(() => {
    exceeded = true;
    kill();
  }, 10_000);
  signal.addEventListener("abort", kill, { once: true });
  if (signal.aborted) kill();
  const collect = async (stream: Readable): Promise<string> => {
    const chunks: Uint8Array[] = [];
    for await (const chunk of stream) {
      size += chunk.byteLength;
      if (size > 131_072) {
        exceeded = true;
        kill();
        break;
      }
      chunks.push(chunk);
    }
    return Buffer.concat(chunks).toString("utf8");
  };
  try {
    const [stdout, stderr, code] = await Promise.race([
      Promise.all([collect(child.stdout), collect(child.stderr), exited.promise]),
      cancelled.promise.then(() => {
        signal.throwIfAborted();
        throw new Error("Command exceeded its 10-second or 128-KiB output limit.");
      }),
    ]);
    signal.throwIfAborted();
    if (exceeded) throw new Error("Command exceeded its 10-second or 128-KiB output limit.");
    if (code !== 0) throw new Error(`Command exited ${code}: ${stderr || stdout}`);
    return stdout + stderr;
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", kill);
    await stop();
  }
}
