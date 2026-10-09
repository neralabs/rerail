import { StreamingSampleChunker } from "./chunker.js";
import { stableJsonText } from "../engine/stable-json.js";
import { SessionLanes } from "./session-lanes.js";
import { steers, type RuleMode } from "../engine/catalog/rule-mode.js";
import type {
  AdapterCapabilityProfile,
  AdapterDelivery,
  AdapterEvent,
  AdapterSample,
  AdapterSession,
} from "../engine/adapter-contract.js";

export interface HostRuntimeSessionState {
  processedEventIds: readonly string[];
}

export interface HostRuntimePort {
  openSession(session: AdapterSession): Promise<HostRuntimeSessionState>;
  closeSession(sessionId: string): Promise<void>;
  persistEvent(event: AdapterEvent): Promise<void>;
  persistTurn(sessionId: string, eventId: string): Promise<void>;
  persistDelivery(delivery: AdapterDelivery): Promise<void>;
  acceptSample(sample: AdapterSample): Promise<void>;
  deliver(delivery: AdapterDelivery): Promise<void>;
}

interface ActiveSession {
  processed: Set<string>;
  chunker: StreamingSampleChunker;
}

function stableText(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  return stableJsonText(value);
}

export class RerailHostRuntime {
  readonly profile: AdapterCapabilityProfile;
  readonly #port: HostRuntimePort;
  readonly #sessions = new Map<string, ActiveSession>();
  readonly #lanes = new SessionLanes();

  constructor(profile: AdapterCapabilityProfile, port: HostRuntimePort) {
    this.profile = profile;
    this.#port = port;
  }

  start(session: AdapterSession): Promise<void> {
    return this.#lanes.run(session.sessionId, async () => {
      if (session.host !== this.profile.host) throw new Error("Session host does not match adapter profile");
      const restored = await this.#port.openSession(session);
      this.#sessions.set(session.sessionId, {
        processed: new Set(restored.processedEventIds),
        chunker: new StreamingSampleChunker({ maxCharacters: 8192 }),
      });
    });
  }

  stop(sessionId: string): Promise<void> {
    return this.#lanes.run(sessionId, async () => {
      const active = this.#sessions.get(sessionId);
      active?.chunker.clear();
      this.#sessions.delete(sessionId);
      await this.#port.closeSession(sessionId);
    });
  }

  ingest(event: AdapterEvent): Promise<void> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    return this.#lanes.run(sessionId, () => this.#ingest(event));
  }

  async #ingest(event: AdapterEvent): Promise<void> {
    if (event.kind === "session-start") return this.#startEvent(event);
    if (event.kind === "session-end") return this.#endEvent(event);
    const active = this.#sessions.get(event.sessionId);
    if (active === undefined) throw new Error(`Inactive session: ${event.sessionId}`);
    if (active.processed.has(event.eventId)) return;
    active.processed.add(event.eventId);
    await this.#port.persistEvent(event);
    await this.#ingestContent(event, active);
  }

  async #startEvent(event: Extract<AdapterEvent, { kind: "session-start" }>): Promise<void> {
    if (event.session.host !== this.profile.host) throw new Error("Session host does not match adapter profile");
    const restored = await this.#port.openSession(event.session);
    const active = {
      processed: new Set(restored.processedEventIds),
      chunker: new StreamingSampleChunker({ maxCharacters: 8192 }),
    };
    this.#sessions.set(event.session.sessionId, active);
    if (active.processed.has(event.eventId)) return;
    active.processed.add(event.eventId);
    await this.#port.persistEvent(event);
  }

  async #endEvent(event: Extract<AdapterEvent, { kind: "session-end" }>): Promise<void> {
    const active = this.#sessions.get(event.sessionId);
    if (active !== undefined && !active.processed.has(event.eventId)) {
      await this.#port.persistEvent(event);
      active.processed.add(event.eventId);
    }
    active?.chunker.clear();
    this.#sessions.delete(event.sessionId);
    await this.#port.closeSession(event.sessionId);
  }

  async #ingestContent(
    event: Exclude<AdapterEvent, { kind: "session-start" | "session-end" }>,
    active: ActiveSession,
  ): Promise<void> {
    if (event.kind === "user-prompt") {
      await this.#sample(event, "user-input", "message", event.text);
    } else if (event.kind === "assistant-delta") {
      await this.#ingestAssistantDelta(event, active);
    } else if (event.kind === "assistant-message") {
      await this.#sample(event, "assistant-text", "message", event.text);
    } else if (event.kind === "assistant-abort") {
      active.chunker.abort(`${event.sessionId}:${event.messageId}`);
    } else if (event.kind === "call") {
      const text = stableJsonText(event.input);
      if (text !== undefined) await this.#sample(event, `tool:${event.toolName}`, "tool-call", text, text);
    } else if (event.kind === "result") {
      const text = stableText(event.output);
      if (text !== undefined) await this.#sample(event, `tool:${event.toolName}`, "tool-call", text);
    } else if (event.kind === "turn-end") {
      await this.#sample(event, "assistant-text", "turn", event.text);
      await this.#port.persistTurn(event.sessionId, event.eventId);
    }
  }

  async #ingestAssistantDelta(
    event: Extract<AdapterEvent, { kind: "assistant-delta" }>,
    active: ActiveSession,
  ): Promise<void> {
    if (this.profile.assistantObservation !== "streaming") return;
    const streamId = `${event.sessionId}:${event.messageId}`;
    for (const sentence of active.chunker.append(streamId, event.delta)) {
      await this.#sample(event, "assistant-text", "sentence", sentence);
    }
    if (!event.final) return;
    const finished = active.chunker.finish(streamId);
    if (finished === null) return;
    if (finished.remainder.length > 0) {
      await this.#sample(event, "assistant-text", "sentence", finished.remainder);
    }
    await this.#sample(event, "assistant-text", "message", finished.message);
  }

  requestSteer(
    sessionId: string,
    requested: RuleMode,
    content: string,
    ruleIds: readonly string[] = [],
  ): Promise<void> {
    return this.#lanes.run(sessionId, async () => {
      if (!this.#sessions.has(sessionId)) throw new Error(`Inactive session: ${sessionId}`);
      const delivery: AdapterDelivery = {
        sessionId,
        requested,
        mechanism: this.profile.delivery[requested],
        content,
        ruleIds,
      };
      await this.#port.persistDelivery(delivery);
      if (steers(requested)) await this.#port.deliver(delivery);
    });
  }


  async #sample(
    event: Exclude<AdapterEvent, { kind: "session-start" | "session-end" }>,
    source: AdapterSample["source"],
    boundary: AdapterSample["boundary"],
    text: string,
    input?: string,
  ): Promise<void> {
    if (text.trim() === "") return;
    await this.#port.acceptSample({
      eventId: event.eventId,
      sessionId: event.sessionId,
      source,
      boundary,
      text,
      ...(input === undefined ? {} : { input }),
    });
  }
}
