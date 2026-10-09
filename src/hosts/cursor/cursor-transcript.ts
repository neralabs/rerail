import { mkdir, open, readFile, rename, writeFile, type FileHandle } from "node:fs/promises";
import { dirname } from "node:path";
import { z } from "zod";
import type { AdapterEvent, AdapterSession, RerailEventLoop } from "./adapter-contract.js";
import { createHash } from "node:crypto";
import { pendingPath, type HookInput, persistPending } from "./hook-adapter.js";
import { steers } from "./rule-mode.js";

const CursorTranscriptStateSchema = z
  .object({
    path: z.string().nullable(),
    offset: z.number().int().nonnegative(),
    latestAssistantText: z.string(),
    nativeTextMode: z.boolean(),
  })
  .strict();
type CursorTranscriptState = z.infer<typeof CursorTranscriptStateSchema>;

const TranscriptPartSchema = z
  .object({
    type: z.string(),
    text: z.string().optional(),
  })
  .passthrough();
const TranscriptRecordSchema = z
  .object({
    role: z.string().optional(),
    type: z.string().optional(),
    status: z.string().optional(),
    message: z
      .object({
        content: z.array(TranscriptPartSchema),
      })
      .passthrough()
      .optional(),
  })
  .passthrough();
type TranscriptRecord = z.infer<typeof TranscriptRecordSchema>;

const INITIAL_STATE: CursorTranscriptState = {
  path: null,
  offset: 0,
  latestAssistantText: "",
  nativeTextMode: false,
};

function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && (error as NodeJS.ErrnoException).code === code;
}

function transcriptStatePath(storageRoot: string, sessionId: string): string {
  return `${pendingPath(storageRoot, "cursor", sessionId)}.transcript`;
}

async function readState(path: string): Promise<CursorTranscriptState> {
  try {
    return CursorTranscriptStateSchema.parse(JSON.parse(await readFile(path, "utf8")) as unknown);
  } catch (error) {
    if (isErrno(error, "ENOENT")) return { ...INITIAL_STATE };
    throw error;
  }
}

async function writeState(path: string, state: CursorTranscriptState): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, JSON.stringify(state), { mode: 0o600 });
  await rename(temporary, path);
}

function userText(record: TranscriptRecord): string | undefined {
  if (record.role !== "user" || record.message === undefined) return undefined;
  let text = "";
  for (const part of record.message.content) {
    if (part.type === "text" && part.text !== undefined) text += part.text;
  }
  const match = /^<timestamp>[^\r\n]*<\/timestamp>\n<user_query>\n([\s\S]*)\n<\/user_query>$/.exec(text);
  return match?.[1];
}

function assistantText(record: TranscriptRecord): string | undefined {
  if (record.role !== "assistant" || record.message === undefined) return undefined;
  let text = "";
  let hasText = false;
  for (const part of record.message.content) {
    if (part.type === "text" && part.text !== undefined) {
      hasText = true;
      text += part.text;
    }
  }
  return hasText ? text : undefined;
}

function recordEventId(sessionId: string, offset: number, line: Buffer): string {
  const hash = createHash("sha256").update(line).digest("hex");
  return `${sessionId}:transcript:${offset}:${hash}`;
}

function eventForRecord(
  record: TranscriptRecord,
  descriptor: AdapterSession,
  offset: number,
  line: Buffer,
  latestAssistantText: string,
): AdapterEvent | undefined {
  const eventId = recordEventId(descriptor.sessionId, offset, line);
  const prompt = userText(record);
  if (prompt !== undefined) {
    return { kind: "user-prompt", eventId, sessionId: descriptor.sessionId, session: descriptor, text: prompt };
  }
  const assistant = assistantText(record);
  if (assistant !== undefined) {
    return {
      kind: "assistant-message",
      eventId,
      sessionId: descriptor.sessionId,
      session: descriptor,
      messageId: eventId,
      text: assistant,
    };
  }
  if (record.type === "turn_ended" && record.status === "success") {
    return {
      kind: "turn-end",
      eventId,
      sessionId: descriptor.sessionId,
      session: descriptor,
      text: latestAssistantText,
    };
  }
  return undefined;
}

async function readUnreadSnapshot(
  path: string,
  offset: number,
): Promise<{ bytes: Buffer; offset: number } | undefined> {
  let handle: FileHandle;
  try {
    handle = await open(path, "r");
  } catch (error) {
    if (isErrno(error, "ENOENT")) return undefined;
    throw error;
  }
  try {
    const size = (await handle.stat()).size;
    const start = offset > size ? 0 : offset;
    const bytes = Buffer.alloc(size - start);
    const { bytesRead } = await handle.read(bytes, 0, bytes.length, start);
    return { bytes: bytes.subarray(0, bytesRead), offset: start };
  } finally {
    await handle.close();
  }
}

async function ingestCompleteLines(
  complete: Buffer,
  offset: number,
  state: CursorTranscriptState,
  descriptor: AdapterSession,
  runtime: RerailEventLoop,
  pending: string,
): Promise<void> {
  let lineStart = 0;
  while (lineStart < complete.length) {
    const lineOffset = offset + lineStart;
    const lineEnd = complete.indexOf(0x0a, lineStart);
    const line = complete.subarray(lineStart, lineEnd);
    lineStart = lineEnd + 1;
    if (line.length === 0 || (line.length === 1 && line[0] === 0x0d)) continue;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line.toString("utf8")) as unknown;
    } catch (error) {
      throw new Error(
        `Cursor transcript contains invalid JSON at byte ${lineOffset}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
    const event = eventForRecord(
      TranscriptRecordSchema.parse(parsed),
      descriptor,
      lineOffset,
      line,
      state.latestAssistantText,
    );
    if (event === undefined) continue;
    if (event.kind === "user-prompt") state.latestAssistantText = "";
    await persistPending(
      pending,
      (await runtime.ingest(event)).filter((delivery) => steers(delivery.requested)),
    );
    if (event.kind === "assistant-message") state.latestAssistantText = event.text;
  }
}

/**
 * Recovers Cursor's append-only JSONL transcript when the CLI does not supply text hooks.
 * The caller holds Cursor's per-session hook lock, so the cursor sidecar is single-writer.
 */
export async function ingestCursorTranscript(
  input: HookInput,
  descriptor: AdapterSession,
  runtime: RerailEventLoop,
  storageRoot: string,
): Promise<boolean> {
  const statePath = transcriptStatePath(storageRoot, descriptor.sessionId);
  const state = await readState(statePath);

  if (input.hook_event_name === "beforeSubmitPrompt") {
    if (!state.nativeTextMode) await writeState(statePath, { ...state, nativeTextMode: true });
    return false;
  }
  if (state.nativeTextMode) return false;

  const path =
    typeof input.transcript_path === "string" && input.transcript_path.trim() !== ""
      ? input.transcript_path
      : undefined;
  if (path === undefined) return false;
  if (state.path !== path) {
    state.path = path;
    state.offset = 0;
    state.latestAssistantText = "";
  }

  const snapshot = await readUnreadSnapshot(path, state.offset);
  const previousOffset = state.offset;
  if (snapshot === undefined) {
    await writeState(statePath, state);
    return false;
  }

  state.offset = snapshot.offset;
  if (state.offset < previousOffset) state.latestAssistantText = "";
  const lastNewline = snapshot.bytes.lastIndexOf(0x0a);
  if (lastNewline === -1) {
    await writeState(statePath, state);
    return true;
  }

  const complete = snapshot.bytes.subarray(0, lastNewline + 1);
  await ingestCompleteLines(
    complete,
    state.offset,
    state,
    descriptor,
    runtime,
    pendingPath(storageRoot, "cursor", descriptor.sessionId),
  );

  state.offset += complete.length;
  await writeState(statePath, state);
  return true;
}
