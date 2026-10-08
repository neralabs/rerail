import { describe, expect, it } from "vitest";
import type {
  AdapterCapabilityProfile,
  AdapterDelivery,
  AdapterEvent,
  AdapterSample,
  AdapterSession,
} from "../../src/adapter-contract.js";
import { RULE_MODES } from "../../src/rule-mode.js";

export interface PersistedContractEntry {
  sessionId: string;
  kind: "event" | "delivery" | "turn";
  id: string;
  data?: unknown;
}

export interface HostAdapterContractDriver {
  readonly profile: AdapterCapabilityProfile;
  start(session: AdapterSession): Promise<void>;
  emit(event: AdapterEvent): Promise<void>;
  requestSteer(
    sessionId: string,
    requested: AdapterDelivery["requested"],
    content: string,
  ): Promise<void>;
  restart(session: AdapterSession): Promise<void>;
  stop(sessionId: string): Promise<void>;
  samples(): readonly AdapterSample[];
  deliveries(): readonly AdapterDelivery[];
  persisted(): readonly PersistedContractEntry[];
  modelLoads(): number;
}

export interface HostAdapterContractOptions {
  name: string;
  create(): HostAdapterContractDriver;
}

function session(host: AdapterCapabilityProfile["host"], sessionId = "session-a"): AdapterSession {
  return { host, sessionId, cwd: "/workspace" };
}

export function hostAdapterContract(options: HostAdapterContractOptions): void {
  describe(`${options.name} host adapter contract`, () => {
    it("declares a complete, internally consistent capability profile", () => {
      const { profile } = options.create();
      expect(profile.host).toBeTruthy();
      expect(profile.lifecycleEvents.length).toBeGreaterThanOrEqual(1);
      expect(Object.keys(profile.sampleEvents).sort((left, right) => left.localeCompare(right))).toEqual(["assistant", "tool", "turn", "user"]);
      expect(Object.keys(profile.delivery).sort((left, right) => left.localeCompare(right))).toEqual(["block", "log", "steer"]);
      expect(profile.delivery.log).toBe("observe");
      expect(profile.delivery.block).toBe(profile.delivery.steer);
      expect(profile.assistantObservation === "streaming" || profile.assistantObservation === "completed").toBe(true);
    });

    it("loads session-scoped state once rather than once per event", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({ kind: "user-prompt", eventId: "u1", sessionId: active.sessionId, text: "First" });
      await driver.emit({ kind: "user-prompt", eventId: "u2", sessionId: active.sessionId, text: "Second" });
      expect(driver.modelLoads()).toBe(1);
    });

    it("normalizes user prompts without changing their text", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({ kind: "user-prompt", eventId: "u1", sessionId: active.sessionId, text: "  Preserve spacing.  " });
      expect(driver.samples()).toContainEqual({
        eventId: "u1",
        sessionId: active.sessionId,
        source: "user-input",
        boundary: "message",
        text: "  Preserve spacing.  ",
      });
    });

    it("normalizes nested tool inputs deterministically", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({
        kind: "call",
        eventId: "t1",
        toolCallId: "t1",
        sessionId: active.sessionId,
        toolName: "write",
        input: { z: 2, nested: { y: false, x: true }, a: 1 },
      });
      expect(driver.samples()).toContainEqual({
        eventId: "t1",
        sessionId: active.sessionId,
        source: "tool:write",
        boundary: "tool-call",
        text: '{"a":1,"nested":{"x":true,"y":false},"z":2}',
        input: '{"a":1,"nested":{"x":true,"y":false},"z":2}',
      });
    });

    it("preserves string tool results and normalizes structured results deterministically", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({
        kind: "result",
        eventId: "result-string",
        toolCallId: "result-string",
        sessionId: active.sessionId,
        toolName: "write",
        output: "plain result",
      });
      await driver.emit({
        kind: "result",
        eventId: "result-structured",
        toolCallId: "result-structured",
        sessionId: active.sessionId,
        toolName: "write",
        output: { z: 2, nested: { y: false, x: true }, a: 1 },
      });
      expect(driver.samples()).toEqual([
        {
          eventId: "result-string",
          sessionId: active.sessionId,
          source: "tool:write",
          boundary: "tool-call",
          text: "plain result",
        },
        {
          eventId: "result-structured",
          sessionId: active.sessionId,
          source: "tool:write",
          boundary: "tool-call",
          text: '{"a":1,"nested":{"x":true,"y":false},"z":2}',
        },
      ]);
    });

    it("contains malformed or blank tool values while persisting and deduplicating their events", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      const cyclic: { self?: unknown } = {};
      cyclic.self = cyclic;
      const malformedEvents: AdapterEvent[] = [
        {
          kind: "call",
          eventId: "call-cyclic",
          toolCallId: "call-cyclic",
          sessionId: active.sessionId,
          toolName: "write",
          input: cyclic,
        },
        {
          kind: "call",
          eventId: "call-undefined",
          toolCallId: "call-undefined",
          sessionId: active.sessionId,
          toolName: "write",
          input: undefined,
        },
        {
          kind: "result",
          eventId: "result-cyclic",
          toolCallId: "result-cyclic",
          sessionId: active.sessionId,
          toolName: "write",
          output: cyclic,
        },
        {
          kind: "result",
          eventId: "result-undefined",
          toolCallId: "result-undefined",
          sessionId: active.sessionId,
          toolName: "write",
          output: undefined,
        },
        {
          kind: "result",
          eventId: "result-blank",
          toolCallId: "result-blank",
          sessionId: active.sessionId,
          toolName: "write",
          output: " \n\t ",
        },
      ];

      for (const event of malformedEvents) {
        await expect(driver.emit(event)).resolves.toBeUndefined();
      }
      await driver.emit(malformedEvents[0]!);

      expect(driver.samples()).toEqual([]);
      expect(driver.persisted().filter(entry => entry.kind === "event").map(entry => entry.id)).toEqual([
        "call-cyclic",
        "call-undefined",
        "result-cyclic",
        "result-undefined",
        "result-blank",
      ]);
    });

    it("observes assistant output at the declared granularity", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      if (driver.profile.assistantObservation === "streaming") {
        await driver.emit({
          kind: "assistant-delta",
          eventId: "a1-0",
          sessionId: active.sessionId,
          messageId: "a1",
          delta: "First sentence. Second",
          final: false,
        });
        await driver.emit({
          kind: "assistant-delta",
          eventId: "a1-1",
          sessionId: active.sessionId,
          messageId: "a1",
          delta: " sentence.",
          final: true,
        });
        expect(driver.samples().filter(sample => sample.boundary === "sentence").map(sample => sample.text)).toEqual([
          "First sentence.",
          "Second sentence.",
        ]);
        expect(driver.samples()).toContainEqual(expect.objectContaining({
          source: "assistant-text",
          boundary: "message",
          text: "First sentence. Second sentence.",
        }));
      } else {
        await driver.emit({
          kind: "assistant-message",
          eventId: "a1",
          sessionId: active.sessionId,
          messageId: "a1",
          text: "First sentence. Second sentence.",
        });
        expect(driver.samples().filter(sample => sample.boundary === "sentence")).toEqual([]);
        expect(driver.samples()).toContainEqual(expect.objectContaining({
          source: "assistant-text",
          boundary: "message",
          text: "First sentence. Second sentence.",
        }));
      }
    });

    it("records turn boundaries independently from assistant messages", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({ kind: "turn-end", eventId: "turn-1", sessionId: active.sessionId, text: "Final answer" });
      expect(driver.samples()).toContainEqual({
        eventId: "turn-1",
        sessionId: active.sessionId,
        source: "assistant-text",
        boundary: "turn",
        text: "Final answer",
      });
      expect(driver.persisted()).toContainEqual(expect.objectContaining({ sessionId: active.sessionId, kind: "turn" }));
    });

    it("deduplicates replayed and concurrently delivered event ids", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      const duplicate: AdapterEvent = {
        kind: "user-prompt",
        eventId: "duplicate",
        sessionId: active.sessionId,
        text: "Only once",
      };
      await Promise.all([driver.emit(duplicate), driver.emit(duplicate), driver.emit(duplicate)]);
      expect(driver.samples().filter(sample => sample.eventId === "duplicate")).toHaveLength(1);
      expect(driver.persisted().filter(entry => entry.kind === "event" && entry.id === "duplicate")).toHaveLength(1);
    });

    it("isolates content, delivery, and persistence by session", async () => {
      const driver = options.create();
      const first = session(driver.profile.host, "session-a");
      const second = session(driver.profile.host, "session-b");
      await driver.start(first);
      await driver.start(second);
      await driver.emit({ kind: "user-prompt", eventId: "same-id", sessionId: first.sessionId, text: "Alpha" });
      await driver.emit({ kind: "user-prompt", eventId: "same-id", sessionId: second.sessionId, text: "Beta" });
      await driver.requestSteer(first.sessionId, "steer", "Only alpha");
      expect(driver.samples().filter(sample => sample.sessionId === first.sessionId).map(sample => sample.text)).toEqual(["Alpha"]);
      expect(driver.samples().filter(sample => sample.sessionId === second.sessionId).map(sample => sample.text)).toEqual(["Beta"]);
      expect(driver.deliveries().filter(delivery => delivery.sessionId === second.sessionId)).toEqual([]);
    });

    it("maps every rule mode explicitly and never emits log content", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      for (const requested of RULE_MODES) {
        await driver.requestSteer(active.sessionId, requested, `content:${requested}`);
      }
      expect(driver.deliveries()).toEqual([
        {
          sessionId: active.sessionId,
          requested: "steer",
          mechanism: driver.profile.delivery.steer,
          content: "content:steer",
          ruleIds: [],
        },
        {
          sessionId: active.sessionId,
          requested: "block",
          mechanism: driver.profile.delivery.block,
          content: "content:block",
          ruleIds: [],
        },
      ]);
      expect(driver.persisted().filter(entry => entry.kind === "delivery")).toHaveLength(3);
    });

    it("replays persisted deduplication state after restart", async () => {
      const driver = options.create();
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.emit({ kind: "user-prompt", eventId: "persisted", sessionId: active.sessionId, text: "Before restart" });
      await driver.restart(active);
      await driver.emit({ kind: "user-prompt", eventId: "persisted", sessionId: active.sessionId, text: "After restart" });
      expect(driver.samples().filter(sample => sample.eventId === "persisted")).toHaveLength(1);
    });

    it("rejects events for inactive sessions and releases session buffers on stop", async () => {
      const driver = options.create();
      await expect(driver.emit({ kind: "user-prompt", eventId: "orphan", sessionId: "missing", text: "No session" })).rejects.toThrow(/session/i);
      const active = session(driver.profile.host);
      await driver.start(active);
      await driver.stop(active.sessionId);
      await expect(driver.emit({ kind: "user-prompt", eventId: "late", sessionId: active.sessionId, text: "Too late" })).rejects.toThrow(/session/i);
    });
  });
}
