import { createHash, randomBytes } from "node:crypto";
import { readdirSync } from "node:fs";
import { resolve } from "node:path";

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

/** Whether some Rerail engine pipe is among `pipes`; pipes that cannot be listed count as holding one. */
export function enginePipeListed(pipes: () => readonly string[] = listPipes): boolean {
  return listed(pipes)?.some(isEnginePipe) ?? true;
}
