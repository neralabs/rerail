import { z } from "zod/v4";
import type { AdapterDelivery, AdapterEvent } from "./adapter-contract.js";

export const RERAIL_EVENT_PROTOCOL_PREFACE = "RERAIL-EVENTS/1";
export const RERAIL_EVENT_MAX_FRAME_BYTES = 1024 * 1024;

export const SessionSchema = z.object({
  host: z.enum(["omp", "pi", "claude-code", "codex", "cursor"]),
  sessionId: z.string().min(1),
  cwd: z.string().min(1),
}).strict();

export const AdapterEventSchema: z.ZodType<AdapterEvent> = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("session-start"), eventId: z.string().min(1), session: SessionSchema }).strict(),
  z.object({ kind: z.literal("session-end"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional() }).strict(),
  z.object({ kind: z.literal("user-prompt"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), text: z.string() }).strict(),
  z.object({ kind: z.literal("assistant-delta"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), messageId: z.string().min(1), delta: z.string(), final: z.boolean() }).strict(),
  z.object({ kind: z.literal("assistant-message"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), messageId: z.string().min(1), text: z.string() }).strict(),
  z.object({ kind: z.literal("assistant-abort"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), messageId: z.string().min(1) }).strict(),
  z.object({ kind: z.literal("tool-call"), eventId: z.string().min(1), toolCallId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), toolName: z.string().min(1), input: z.unknown() }).strict(),
  z.object({ kind: z.literal("tool-result"), eventId: z.string().min(1), toolCallId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), toolName: z.string().min(1), output: z.unknown(), failed: z.boolean().optional() }).strict(),
  z.object({ kind: z.literal("turn-end"), eventId: z.string().min(1), sessionId: z.string().min(1), session: SessionSchema.optional(), text: z.string() }).strict(),
]);

const DeliverySchema: z.ZodType<AdapterDelivery> = z.object({
  sessionId: z.string().min(1),
  requested: z.enum(["observe", "steer", "followUp", "nextTurn"]),
  mechanism: z.enum(["observe", "native-queue", "additional-context", "continuation", "deferred-context"]),
  content: z.string(),
  // A kernel from a build without re-rail attribution sends no rule ids.
  ruleIds: z.array(z.string()).default([]),
}).strict();

const EventFrameSchema = z.object({ event: AdapterEventSchema }).strict();
const EventSuccessFrameSchema = z.object({
  eventId: z.string().min(1),
  deliveries: z.array(DeliverySchema),
}).strict();
const EventErrorFrameSchema = z.object({
  eventId: z.string().min(1).nullable(),
  error: z.object({
    code: z.enum(["invalid-event", "inactive-session", "unsupported-version", "internal-error", "overloaded"]),
    message: z.string(),
  }).strict(),
}).strict();

export type RerailEventFrame = z.infer<typeof EventFrameSchema>;
export type RerailEventSuccessFrame = z.infer<typeof EventSuccessFrameSchema>;
export type RerailEventErrorFrame = z.infer<typeof EventErrorFrameSchema>;
export type RerailEventResultFrame = RerailEventSuccessFrame | RerailEventErrorFrame;

export function decodeRerailEventResult(text: string): RerailEventSuccessFrame {
  return EventSuccessFrameSchema.parse(JSON.parse(text));
}

export function decodeRerailEventFrame(line: string): RerailEventFrame {
  if (Buffer.byteLength(line, "utf8") > RERAIL_EVENT_MAX_FRAME_BYTES) {
    throw new RangeError(`Rerail event frame exceeds ${RERAIL_EVENT_MAX_FRAME_BYTES} bytes`);
  }
  return EventFrameSchema.parse(JSON.parse(line));
}

export function encodeRerailEventResult(frame: RerailEventResultFrame): string {
  const parsed = "error" in frame
    ? EventErrorFrameSchema.parse(frame)
    : EventSuccessFrameSchema.parse(frame);
  return JSON.stringify(parsed);
}
