import { createHash, randomBytes } from "node:crypto";
import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { loadNativeBinding, type NativeBinding, type NativeNamedPipeServer } from "../../engine/native/native.js";

/**
 * What a Rerail engine pipe is on Windows: the name the engine of one Rerail database listens on, the fallback it moves
 * to when another process holds that name, and how a process tells from the pipes that exist whether one is there. The
 * daemon writes these names and the daemon's control reads them, so both take them from here.
 */

/** The start of every named pipe's path, and the folder Windows lists its named pipes in. */
export const PIPE_PREFIX = String.raw`\\.\pipe` + "\\";

/** 24 hex characters that name the engine of the Rerail database at `databasePath`, as a pipe or a Unix socket. */
export function engineIdentity(databasePath: string): string {
  return createHash("sha256").update(resolve(databasePath)).digest("hex").slice(0, 24);
}

/** The engine pipe of the Rerail database at `databasePath`: `\\.\pipe\rerail-<24 hex>`. */
export function enginePipeName(databasePath: string): string {
  return `${PIPE_PREFIX}rerail-${engineIdentity(databasePath)}`;
}

/** The pipe the engine listens on when another process holds `pipe`: `<pipe>-<16 hex>`, from 8 random bytes. */
export function fallbackPipe(pipe: string, random: () => Buffer = () => randomBytes(8)): string {
  return `${pipe}-${random().toString("hex")}`;
}

/** A Rerail engine pipe's name as `\\.\pipe\` lists it: `rerail-<24 hex>`, or the fallback a held one moves to. */
const ENGINE_PIPE = /^rerail-[0-9a-f]{24}(?:-[0-9a-f]+)?$/u;

/** Whether `name`, as `\\.\pipe\` lists it, whatever its case, is a Rerail engine's pipe. */
export function isEnginePipe(name: string): boolean {
  return ENGINE_PIPE.test(name.toLowerCase());
}

/** Whether `path` names a Windows named pipe. */
export function isNamedPipe(path: string): boolean {
  return path.startsWith(PIPE_PREFIX);
}

/** The name `\\.\pipe\` lists the pipe at `path` by. */
export function pipeNameOf(path: string): string {
  return path.slice(PIPE_PREFIX.length);
}

/** The named pipes that exist, read from `\\.\pipe\` without connecting to any. */
export function listPipes(): readonly string[] {
  return readdirSync(PIPE_PREFIX);
}

/** The pipes `pipes` lists, or undefined when they cannot be listed. */
function listed(pipes: () => readonly string[]): readonly string[] | undefined {
  try {
    return pipes();
  } catch {
    return undefined;
  }
}

/** Whether the pipe at `path` is among `pipes`, whatever its case; pipes that cannot be listed count as holding it. */
export function pipeListed(path: string, pipes: () => readonly string[] = listPipes): boolean {
  const name = pipeNameOf(path).toLowerCase();
  return listed(pipes)?.some(pipe => pipe.toLowerCase() === name) ?? true;
}

/**
 * Whether the engine pipe of the Rerail database at `databasePath` is among `pipes`, under its own name or the fallback
 * its engine moves to ({@link fallbackPipe}), whatever its case; pipes that cannot be listed count as holding it.
 */
export function databasePipeListed(databasePath: string, pipes: () => readonly string[] = listPipes): boolean {
  const name = pipeNameOf(enginePipeName(databasePath)).toLowerCase();
  return listed(pipes)?.some(pipe => {
    const listedName = pipe.toLowerCase();
    return listedName === name || listedName.startsWith(`${name}-`);
  }) ?? true;
}

/** Whether some Rerail engine pipe is among `pipes`; pipes that cannot be listed count as holding one. */
export function enginePipeListed(pipes: () => readonly string[] = listPipes): boolean {
  return listed(pipes)?.some(isEnginePipe) ?? true;
}

/**
 * What Windows says of the server end of a named pipe ({@link pipeServer}): the process holding it and when that process
 * started; with no pid, other clients kept every instance taken for the whole wait, so no instance could be opened to ask.
 */
export type PipeServer = NativeNamedPipeServer;

/**
 * What {@link pipeServer} gives: the server end Windows names ({@link PipeServer}); `no-pipe` where no pipe of that name
 * took a connection or Windows named no holder; `not-asked` where Windows could not be asked, since the native binding
 * would not load in this process, predates the question or threw.
 */
export type PipeServerAnswer = PipeServer | "no-pipe" | "not-asked";

/**
 * The process holding the server end of the Windows named pipe at `path` and when it started, as Windows records them
 * for each pipe instance (the native binding's `namedPipeServer`). While other clients hold every instance it waits for
 * one to free and opens again, as Node's own client does, until `timeoutMs` has passed, on a thread of the worker pool,
 * so this process's own thread and its timers run on meanwhile. Any local user can create a pipe of any name, but not
 * one Windows records as another process's. `no-pipe` where no such pipe takes a connection; `not-asked` where the
 * binding (`binding`, the engine's own unless a test stands in one) predates the question, will not load in this
 * process or throws. Either way the engine's proof decides, and with no capability in daemon.json nothing can; the
 * first load failure or throw is reported ({@link reportPipeServerFailure}).
 */
export async function pipeServer(
  path: string,
  timeoutMs: number,
  binding: () => Pick<NativeBinding, "namedPipeServer"> = loadNativeBinding,
): Promise<PipeServerAnswer> {
  try {
    const native = binding();
    if (native.namedPipeServer === undefined) return "not-asked";
    return await native.namedPipeServer(path, timeoutMs) ?? "no-pipe";
  } catch (error) {
    reportPipeServerFailure(path, error);
    return "not-asked";
  }
}

/** Whether a failure to ask Windows which process serves a pipe was reported in this process. */
let pipeServerFailureReported = false;

/**
 * Writes the first failure to ask Windows which process serves a pipe to stderr, once per process: without that answer a
 * hook sends an event there only once the engine on it proves it holds the capability daemon.json records, and where
 * daemon.json records none, nothing can prove it, so the hook sends nothing and says so.
 */
function reportPipeServerFailure(path: string, error: unknown): void {
  if (pipeServerFailureReported) return;
  pipeServerFailureReported = true;
  const reason = error instanceof Error ? error.message : String(error);
  process.stderr.write(`rerail: the engine could not ask Windows which process serves the pipe ${path}, so the engine on it must prove it holds the capability daemon.json records, and where daemon.json records none nothing is sent to it: ${reason}\n`);
}
