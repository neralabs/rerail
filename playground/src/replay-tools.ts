import { constants, type Stats } from "node:fs";
import { lstat, mkdir, open, readdir, realpath } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";
import { z } from "zod";
import type { ExtensionFactory } from "@oh-my-pi/pi-coding-agent";
import type { PlaygroundScenario } from "./protocol.js";
import { commandResultSchema } from "./protocol.js";
import { validateScenarioFixtures } from "./experiments.js";
import picomatch from "picomatch";
import { workerControl } from "./ipc.js";

const MAX_BYTES = 2 * 1024 * 1024;
const RESERVED: Record<string, true> = { ".rerail": true, ".playground-home": true, ".git": true };
export const REPLAY_TOOL_NAMES = ["read", "grep", "glob", "write", "edit"] as const;

function existingEntry(path: string): Promise<Stats | undefined> {
  return lstat(path).catch((error) => {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    return undefined;
  });
}

function isOrdinaryEntry(entry: Stats): boolean {
  if (entry.isSymbolicLink()) return false;
  return entry.isDirectory() || (entry.isFile() && entry.nlink === 1);
}

/** No URI, selector, absolute path, link, or private runtime directory is a tool target. */
export async function safePath(root: string, input: string, createParents = false): Promise<string> {
  if (!input || isAbsolute(input) || /[\\:]|\p{Cc}/u.test(input)) throw new Error("Use a relative sandbox file path.");
  const parts = input.split("/");
  if (parts.some((part) => part === ".." || Object.hasOwn(RESERVED, part.normalize("NFC").toLowerCase())))
    throw new Error("Private runtime and parent paths are not accessible.");
  const target = resolve(root, input);
  if (target !== root && !target.startsWith(root + sep)) throw new Error("Path escapes sandbox.");
  let current = root;
  for (const part of relative(root, target).split(sep).filter(Boolean)) {
    current = join(current, part);
    let entry = await existingEntry(current);
    if (!entry && createParents && current !== target) {
      await mkdir(current, { mode: 0o700 });
      entry = await lstat(current);
    }
    if (!entry) continue;
    if (!isOrdinaryEntry(entry)) {
      throw new Error("Only ordinary sandbox files and directories are accessible; links and devices are forbidden.");
    }
    if ((await realpath(current)) !== current) throw new Error("Path resolves outside its canonical sandbox location.");
  }
  return target;
}

/** Confine a supervisor- or model-supplied path, relative or absolute, to the sandbox root before any fs use. */
export async function sandboxFilePath(root: string, input: string): Promise<string> {
  root = await realpath(root);
  const target = resolve(root, input);
  // The sandbox may be named through a linked temporary directory; canonicalize the existing parent first.
  const parent = await realpath(dirname(target)).catch(() => dirname(target));
  const canonical = join(parent, basename(target));
  if (canonical !== root && !canonical.startsWith(root + sep)) throw new Error("Path escapes sandbox.");
  return safePath(root, relative(root, canonical));
}

async function textFile(root: string, path: string): Promise<string> {
  const file = await open(await safePath(root, path), constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.nlink !== 1 || stat.size > MAX_BYTES)
      throw new Error("Read requires a regular unlinked file at most 2 MiB.");
    const text = await file.readFile("utf8");
    if (text.includes("\0")) throw new Error("Binary file reading is unavailable in replay tools.");
    return text;
  } finally {
    await file.close();
  }
}

async function putFile(root: string, path: string, content: string): Promise<void> {
  if (Buffer.byteLength(content) > MAX_BYTES) throw new Error("Writes are limited to 2 MiB.");
  const file = await open(
    await safePath(root, path, true),
    constants.O_WRONLY | constants.O_CREAT | constants.O_NOFOLLOW,
    0o600,
  );
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.nlink !== 1) throw new Error("Write requires an ordinary unlinked file.");
    await file.truncate(0);
    await file.writeFile(content);
  } finally {
    await file.close();
  }
}

export async function installReplayFixtures(root: string, scenario: PlaygroundScenario): Promise<void> {
  validateScenarioFixtures(scenario);
  for (const fixture of scenario.fixtures) await putFile(root, fixture.path, fixture.content);
}

async function files(root: string, input = "."): Promise<string[]> {
  const start = await safePath(root, input);
  const result: string[] = [];
  const visit = async (path: string): Promise<void> => {
    if (result.length >= 4096) throw new Error("Search is limited to 4096 files.");
    const stat = await lstat(path);
    if (stat.isFile()) {
      result.push(relative(root, path));
      return;
    }
    if (!stat.isDirectory()) throw new Error("Search requires a regular file or directory.");
    for (const entry of await readdir(path, { withFileTypes: true })) {
      if (Object.hasOwn(RESERVED, entry.name.normalize("NFC").toLowerCase())) continue;
      const child = relative(root, join(path, entry.name));
      await visit(await safePath(root, child));
    }
  };
  await visit(start);
  return result.sort((a, b) => a.localeCompare(b));
}

/** Keep the tool lane held until the supervisor confirms command completion or cancellation. */
async function requestSandboxCommand(command: string, signal?: AbortSignal): Promise<string> {
  signal?.throwIfAborted();
  const id = randomUUID();
  const ipc = workerControl;
  const result = Promise.withResolvers<string>();
  const receive = (message: unknown) => {
    const parsed = commandResultSchema.safeParse(message);
    if (!parsed.success || parsed.data.id !== id) return;
    if (signal?.aborted) result.reject(signal.reason);
    else if (parsed.data.ok) result.resolve(parsed.data.output);
    else result.reject(new Error(parsed.data.output));
  };
  const cancel = () => workerControl.send({ type: "cancel-command", id });
  const disconnected = () => result.reject(new Error("Sandbox command supervisor disconnected."));
  ipc.on("message", receive);
  ipc.once("disconnect", disconnected);
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    workerControl.send({ type: "command", id, command });
    if (signal?.aborted) cancel();
    return await result.promise;
  } finally {
    ipc.off("message", receive);
    ipc.off("disconnect", disconnected);
    signal?.removeEventListener("abort", cancel);
  }
}

async function grepFiles(root: string, pattern: string, input?: string): Promise<string> {
  const matches: string[] = [];
  let bytes = 0;
  for (const file of await files(root, input)) {
    const text = await textFile(root, file);
    bytes += Buffer.byteLength(text);
    if (bytes > 8 * MAX_BYTES) throw new Error("Search exceeded its 16-MiB scan limit.");
    for (const [index, line] of text.split("\n").entries())
      if (line.includes(pattern)) {
        matches.push(`${file}:${index + 1}:${line}`);
        if (matches.length === 200) return matches.join("\n");
      }
  }
  return matches.join("\n");
}

async function editFile(root: string, path: string, oldText: string, newText: string): Promise<string> {
  const text = await textFile(root, path);
  const at = text.indexOf(oldText);
  if (at < 0 || text.includes(oldText, at + 1)) throw new Error("oldText must match exactly once.");
  await putFile(root, path, text.slice(0, at) + newText + text.slice(at + oldText.length));
  return `Edited ${path}`;
}

/** Each union member owns both its schema and implementation; `root` is already canonical. */
function replayToolDefinitions(root: string, allowCommands: boolean) {
  const path = z.string().min(1).max(1024);
  return [
    {
      name: "read",
      description: "Read a UTF-8 sandbox fixture or generated file, optionally by one-based line range.",
      parameters: z
        .object({
          path,
          offset: z.number().int().min(1).optional(),
          limit: z.number().int().min(1).max(2000).optional(),
        })
        .strict(),
      run: async (args: { path: string; offset?: number; limit?: number }) =>
        (await textFile(root, args.path))
          .split("\n")
          .slice((args.offset ?? 1) - 1, (args.offset ?? 1) - 1 + (args.limit ?? 2000))
          .join("\n"),
    },
    {
      name: "write",
      description: "Create or replace a UTF-8 file inside the sandbox.",
      parameters: z.object({ path, content: z.string().max(MAX_BYTES) }).strict(),
      run: async (args: { path: string; content: string }) => {
        await putFile(root, args.path, args.content);
        return `Wrote ${args.path}`;
      },
    },
    {
      name: "edit",
      description: "Replace exactly one occurrence of oldText in a sandbox file; ambiguous edits fail.",
      parameters: z
        .object({ path, oldText: z.string().min(1).max(MAX_BYTES), newText: z.string().max(MAX_BYTES) })
        .strict(),
      run: (args: { path: string; oldText: string; newText: string }) =>
        editFile(root, args.path, args.oldText, args.newText),
    },
    {
      name: "glob",
      description: "List sandbox files matching a glob (not an external path).",
      parameters: z.object({ pattern: z.string().min(1).max(1024), path: path.optional() }).strict(),
      run: async (args: { pattern: string; path?: string }) => {
        const matcher = picomatch(args.pattern, { dot: true });
        return (await files(root, args.path)).filter(file => matcher(file)).join("\n");
      },
    },
    {
      name: "grep",
      description:
        "Find literal text in sandbox UTF-8 files. This bounded search does not evaluate regular expressions.",
      parameters: z.object({ pattern: z.string().min(1).max(1024), path: path.optional() }).strict(),
      run: (args: { pattern: string; path?: string }) => grepFiles(root, args.pattern, args.path),
    },
    ...(allowCommands
      ? [
          {
            name: "bash",
            description:
              "Run an explicitly authorized supervisor-owned OS-sandboxed command: no network, original checkout, or credentials. Limit 10 seconds and 128 KiB output.",
            parameters: z.object({ command: z.string().min(1).max(16_384) }).strict(),
            run: (args: { command: string }, signal?: AbortSignal) => requestSandboxCommand(args.command, signal),
          },
        ]
      : []),
  ];
}
type ReplayToolDefinition = ReturnType<typeof replayToolDefinitions>[number];

type ScriptedToolCall = PlaygroundScenario["steps"][number]["toolCalls"][number];

/** A scripted call must name an available tool with valid arguments and an admissible sandbox path. */
async function validateScriptedCall(
  root: string,
  definitions: readonly ReplayToolDefinition[],
  call: ScriptedToolCall,
): Promise<void> {
  const definition = definitions.find((tool) => tool.name === call.name);
  if (!definition)
    throw new Error(
      `Unavailable tool ${call.name}${call.name === "bash" ? "; enable Allow commands explicitly" : ""}.`,
    );
  const parsed = definition.parameters.safeParse(call.arguments);
  if (!parsed.success) throw new Error(`Invalid ${call.name} arguments: ${parsed.error.message}`);
  if ("path" in parsed.data && typeof parsed.data.path === "string") await safePath(root, parsed.data.path);
}

async function validateScriptedCalls(
  root: string,
  definitions: readonly ReplayToolDefinition[],
  scenario: PlaygroundScenario,
): Promise<void> {
  for (const step of scenario.steps)
    for (const call of step.toolCalls) await validateScriptedCall(root, definitions, call);
}

/**
 * Serialize shell and filesystem tools so no subprocess can swap a checked
 * directory for a link between validation and an in-process read or write.
 */
class ToolLane {
  #tail = Promise.resolve();
  async run<T>(work: () => Promise<T>): Promise<T> {
    const previous = this.#tail;
    const gate = Promise.withResolvers<void>();
    this.#tail = gate.promise;
    await previous;
    try {
      return await work();
    } finally {
      gate.resolve();
    }
  }
}

async function executeReplayTool(definition: ReplayToolDefinition, args: unknown, signal: AbortSignal | undefined) {
  if (signal?.aborted) throw signal.reason;
  const parsed = definition.parameters.parse(args);
  const run = definition.run as (input: typeof parsed, signal?: AbortSignal) => Promise<string>;
  const text = await run(parsed, signal);
  return { content: [{ type: "text" as const, text: text.slice(0, 32_768) }], details: {} };
}

export async function replayTools(
  root: string,
  allowCommands: boolean,
  scenario: PlaygroundScenario,
): Promise<ExtensionFactory> {
  root = await realpath(root);
  const definitions = replayToolDefinitions(root, allowCommands);
  await validateScriptedCalls(root, definitions, scenario);
  return (api) => {
    const lane = new ToolLane();
    for (const definition of definitions) {
      api.registerTool({
        name: definition.name,
        label: definition.name,
        description: definition.description,
        parameters: z.toJSONSchema(definition.parameters),
        approval: "write",
        strict: true,
        execute: (_id, args, signal) => lane.run(() => executeReplayTool(definition, args, signal)),
      });
    }
  };
}
