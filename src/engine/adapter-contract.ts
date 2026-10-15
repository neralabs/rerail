import { randomUUID } from "node:crypto";
import { steers, type RuleMode } from "./catalog/rule-mode.js";

/** Every coding agent Rerail works with. */
export const RERAIL_HOSTS = ["omp", "pi", "claude-code", "codex", "cursor"] as const;
export type RerailHost = (typeof RERAIL_HOSTS)[number];

/** One coding agent as `rerail` sees it on this computer; set-up computes it, the API reports it. */
export interface CodingAgentState {
  id: RerailHost;
  name: string;
  /** Its config folder exists under the user's home. */
  found: boolean;
  /** Registering it again would write nothing: Rerail's hooks and commands are in place. */
  connected: boolean;
  /** Why its connection could not be checked, such as a config file that is not valid JSON; null when it could. */
  problem: string | null;
}
export type AssistantObservation = "streaming" | "completed";
export type ToolSurface = "native" | "event";
export type PersistenceSurface = "session-entries" | "jsonl-ledger";
export type DeliveryMechanism =
  | "observe"
  | "native-queue"
  | "additional-context"
  | "deferred-context";

export interface AdapterCapabilityProfile {
  host: RerailHost;
  assistantObservation: AssistantObservation;
  toolSurface: ToolSurface;
  persistence: PersistenceSurface;
  delivery: Readonly<Record<RuleMode, DeliveryMechanism>>;
  lifecycleEvents: readonly string[];
  sampleEvents: Readonly<Record<"user" | "assistant" | "tool" | "turn", string>>;
}

export const ADAPTER_CAPABILITIES = {
  omp: {
    host: "omp",
    assistantObservation: "streaming",
    toolSurface: "native",
    persistence: "session-entries",
    delivery: {
      log: "observe",
      steer: "native-queue",
      block: "native-queue",
    },
    lifecycleEvents: ["session_start", "session_switch", "session_branch", "session_tree", "session_shutdown"],
    sampleEvents: {
      user: "input",
      assistant: "message_update/message_end",
      tool: "tool_call",
      turn: "turn_end",
    },
  },
  pi: {
    host: "pi",
    assistantObservation: "streaming",
    toolSurface: "native",
    persistence: "session-entries",
    delivery: {
      log: "observe",
      steer: "native-queue",
      block: "native-queue",
    },
    lifecycleEvents: ["session_start", "session_tree", "session_shutdown"],
    sampleEvents: {
      user: "input",
      assistant: "message_update/message_end",
      tool: "tool_call",
      turn: "turn_end",
    },
  },
  "claude-code": {
    host: "claude-code",
    assistantObservation: "streaming",
    toolSurface: "event",
    persistence: "jsonl-ledger",
    delivery: {
      log: "observe",
      steer: "additional-context",
      block: "additional-context",
    },
    lifecycleEvents: ["SessionStart", "SessionEnd"],
    sampleEvents: {
      user: "UserPromptSubmit",
      assistant: "MessageDisplay/Stop/StopFailure",
      tool: "PreToolUse/PostToolUse/PostToolUseFailure",
      turn: "Stop/StopFailure",
    },
  },
  codex: {
    host: "codex",
    assistantObservation: "completed",
    toolSurface: "event",
    persistence: "jsonl-ledger",
    delivery: {
      log: "observe",
      steer: "additional-context",
      block: "additional-context",
    },
    lifecycleEvents: ["SessionStart"],
    sampleEvents: {
      user: "UserPromptSubmit",
      assistant: "Stop",
      tool: "PreToolUse/PostToolUse",
      turn: "Stop",
    },
  },
  cursor: {
    host: "cursor",
    assistantObservation: "completed",
    toolSurface: "event",
    persistence: "jsonl-ledger",
    delivery: {
      log: "observe",
      steer: "deferred-context",
      block: "deferred-context",
    },
    lifecycleEvents: ["sessionStart", "sessionEnd"],
    sampleEvents: {
      user: "beforeSubmitPrompt",
      assistant: "afterAgentResponse",
      tool: "preToolUse/postToolUse/postToolUseFailure",
      turn: "stop",
    },
  },
} as const satisfies Record<RerailHost, AdapterCapabilityProfile>;

export interface AdapterSession {
  host: RerailHost;
  sessionId: string;
  cwd: string;
}

/** Host hook payloads are untyped JSON; nonblank text is the only accepted field value. */
export function hookText(host: string) {
  return {
    required(input: Readonly<Record<string, unknown>>, field: string): string {
      const value = input[field];
      if (typeof value !== "string" || value.trim() === "") throw new Error(`${host} hook omitted ${field}`);
      return value;
    },
    optional(input: Readonly<Record<string, unknown>>, field: string): string | undefined {
      const value = input[field];
      return typeof value === "string" && value.trim() !== "" ? value : undefined;
    },
  };
}

type ContextualAdapterEvent = {
  eventId: string;
  sessionId: string;
  session?: AdapterSession | undefined;
};

export type AdapterEvent =
  | { kind: "session-start"; eventId: string; session: AdapterSession }
  | (ContextualAdapterEvent & { kind: "session-end" })
  | (ContextualAdapterEvent & { kind: "user-prompt"; text: string })
  | (ContextualAdapterEvent & {
      kind: "assistant-delta";
      messageId: string;
      delta: string;
      final: boolean;
    })
  | (ContextualAdapterEvent & { kind: "assistant-message"; messageId: string; text: string })
  | (ContextualAdapterEvent & { kind: "assistant-abort"; messageId: string })
  | (ContextualAdapterEvent & { kind: "call"; toolCallId: string; toolName: string; input: unknown })
  | (ContextualAdapterEvent & { kind: "result"; toolCallId: string; toolName: string; output: unknown; failed?: boolean | undefined })
  | (ContextualAdapterEvent & { kind: "turn-end"; text: string });

export function userPromptEvent(sessionId: string, eventId: string, text: string): AdapterEvent {
  return { kind: "user-prompt", eventId, sessionId, text };
}

export function assistantDeltaEvent(
  sessionId: string,
  eventId: string,
  messageId: string,
  delta: string,
  final: boolean,
): AdapterEvent {
  return { kind: "assistant-delta", eventId, sessionId, messageId, delta, final };
}

export function assistantAbortEvent(sessionId: string, eventId: string, messageId: string): AdapterEvent {
  return { kind: "assistant-abort", eventId, sessionId, messageId };
}

export function assistantMessageEvent(sessionId: string, eventId: string, messageId: string, text: string): AdapterEvent {
  return { kind: "assistant-message", eventId, sessionId, messageId, text };
}

/** Whether a message ended without finishing: its stream is abandoned rather than completed. */
export function assistantAborted(stopReason: string | undefined): boolean {
  return stopReason === "aborted" || stopReason === "error";
}

export function assistantCompletionEvent(
  sessionId: string,
  messageId: string,
  streamed: boolean,
  text: () => string,
  stopReason: string | undefined,
  nonce: string,
): AdapterEvent {
  if (assistantAborted(stopReason)) {
    return assistantAbortEvent(sessionId, `assistant-abort:${nonce}`, messageId);
  }
  return assistantDeltaEvent(sessionId, `assistant-final:${nonce}`, messageId, streamed ? "" : text(), true);
}

export function callEvent(
  sessionId: string,
  eventId: string,
  toolCallId: string,
  toolName: string,
  input: unknown,
): AdapterEvent {
  return { kind: "call", eventId, sessionId, toolCallId, toolName, input };
}

export function resultEvent(
  sessionId: string,
  eventId: string,
  toolCallId: string,
  toolName: string,
  output: unknown,
  failed?: boolean,
): AdapterEvent {
  return { kind: "result", eventId, sessionId, toolCallId, toolName, output, failed };
}

export function turnEndEvent(sessionId: string, eventId: string, text: string): AdapterEvent {
  return { kind: "turn-end", eventId, sessionId, text };
}

export interface AdapterSample {
  eventId: string;
  sessionId: string;
  source: `tool:${string}` | "user-input" | "assistant-text";
  boundary: "sentence" | "message" | "turn" | "tool-call";
  text: string;
  /** A tool call's arguments as stable JSON, for structural rules; absent for every other sample, tool results included. */
  input?: string;
}

export interface AdapterDelivery {
  sessionId: string;
  requested: RuleMode;
  mechanism: DeliveryMechanism;
  content: string;
  /** Distinct rule ids whose violations were rendered into `content`, first-seen order; empty when the delivery is not a code-rule steer. */
  ruleIds: readonly string[];
}

/** The `customType` every host records Rerail rule guidance under in its own session transcript. */
export const RULE_MESSAGE_TYPE = "rerail.rule" as const;

type NativeRuleMessage = {
  customType: typeof RULE_MESSAGE_TYPE;
  content: string;
  display: true;
  details: Pick<AdapterDelivery, "requested" | "mechanism" | "sessionId">;
};

export async function sendNativeDeliveries(
  deliveries: readonly AdapterDelivery[],
  send: (message: NativeRuleMessage, options: { deliverAs: "steer" }) => void | Promise<void>,
  record: (sessionId: string, receiptId: string, ruleIds: readonly string[]) => void | Promise<void>,
): Promise<void> {
  for (const delivery of deliveries) {
    if (!steers(delivery.requested)) continue;
    await send(
      {
        customType: RULE_MESSAGE_TYPE,
        content: delivery.content,
        display: true,
        details: {
          requested: delivery.requested,
          mechanism: delivery.mechanism,
          sessionId: delivery.sessionId,
        },
      },
      { deliverAs: "steer" },
    );
    if (delivery.content.trim() === "") continue;
    try {
      await record(delivery.sessionId, randomUUID(), delivery.ruleIds);
    } catch (error) {
      // A successful steer stays successful even if bookkeeping cannot persist it.
      console.error("rerail steering receipt:", error);
    }
  }
}

export interface RerailEventLoop {
  ingest(event: AdapterEvent): Promise<readonly AdapterDelivery[]>;
}
