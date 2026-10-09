import { describe, expect, it, vi } from "vitest";
import {
  ADAPTER_CAPABILITIES,
  type AdapterEvent,
  type AdapterSample,
  type AdapterSession,
} from "../../src/engine/index.js";
import {
  RerailHostRuntime,
  type HostRuntimePort,
  type HostRuntimeSessionState,
} from "../../src/runtime/host-runtime.js";

class RetryableEndPort implements HostRuntimePort {
  readonly events: AdapterEvent[] = [];
  closeCount = 0;
  #failEndOnce = true;

  async openSession(_session: AdapterSession): Promise<HostRuntimeSessionState> {
    return { processedEventIds: [] };
  }

  async closeSession(_sessionId: string): Promise<void> {
    this.closeCount += 1;
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    if (event.kind === "session-end" && this.#failEndOnce) {
      this.#failEndOnce = false;
      throw new Error("transcript unavailable");
    }
    this.events.push(event);
  }

  async persistTurn(_sessionId: string, _eventId: string): Promise<void> {}
  async persistDelivery(): Promise<void> {}
  async acceptSample(): Promise<void> {}
  async deliver(): Promise<void> {}
}

class ConcurrentOpenPort implements HostRuntimePort {
  readonly opened: string[] = [];
  readonly #blocked: Promise<void>;
  readonly #releaseBlocked: () => void;

  constructor() {
    const { promise, resolve } = Promise.withResolvers<void>();
    this.#blocked = promise;
    this.#releaseBlocked = resolve;
  }

  async openSession(session: AdapterSession): Promise<HostRuntimeSessionState> {
    this.opened.push(session.sessionId);
    if (session.sessionId === "blocked") await this.#blocked;
    return { processedEventIds: [] };
  }

  releaseBlocked(): void {
    this.#releaseBlocked();
  }

  async closeSession(): Promise<void> {}
  async persistEvent(): Promise<void> {}
  async persistTurn(): Promise<void> {}
  async persistDelivery(): Promise<void> {}
  async acceptSample(): Promise<void> {}
  async deliver(): Promise<void> {}
}

class OrderedEventPort implements HostRuntimePort {
  readonly observed: string[] = [];
  readonly #firstEvent = Promise.withResolvers<void>();

  async openSession(): Promise<HostRuntimeSessionState> {
    return { processedEventIds: [] };
  }

  releaseFirstEvent(): void {
    this.#firstEvent.resolve();
  }

  async closeSession(sessionId: string): Promise<void> {
    this.observed.push(`${sessionId}:close`);
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    this.observed.push(`${event.eventId}:persist:start`);
    if (event.eventId === "first") await this.#firstEvent.promise;
    this.observed.push(`${event.eventId}:persist:end`);
  }

  async persistTurn(): Promise<void> {}
  async persistDelivery(): Promise<void> {}

  async acceptSample(sample: AdapterSample): Promise<void> {
    this.observed.push(`${sample.eventId}:sample`);
  }

  async deliver(): Promise<void> {}
}

describe("RerailHostRuntime session shutdown", () => {
  it("retries session-end persistence before closing a session", async () => {
    const profile = ADAPTER_CAPABILITIES.codex;
    const session: AdapterSession = { host: profile.host, sessionId: "retryable-end", cwd: "/workspace" };
    const end: AdapterEvent = { kind: "session-end", eventId: "end-1", sessionId: session.sessionId };
    const port = new RetryableEndPort();
    const runtime = new RerailHostRuntime(profile, port);

    await runtime.ingest({ kind: "session-start", eventId: "start-1", session });
    await expect(runtime.ingest(end)).rejects.toThrow("transcript unavailable");
    await runtime.ingest(end);

    expect(port.events.map(event => event.eventId)).toEqual(["start-1", "end-1"]);
    expect(port.closeCount).toBe(1);
  });
});

describe("RerailHostRuntime session concurrency", () => {
  it("orders each session independently instead of blocking the host", async () => {
    const profile = ADAPTER_CAPABILITIES.codex;
    const port = new ConcurrentOpenPort();
    const runtime = new RerailHostRuntime(profile, port);
    const blocked = { host: profile.host, sessionId: "blocked", cwd: "/workspace" };
    const independent = { host: profile.host, sessionId: "independent", cwd: "/workspace" };

    const blockedStart = runtime.ingest({ kind: "session-start", eventId: "blocked-start", session: blocked });
    await vi.waitFor(() => expect(port.opened).toEqual(["blocked"]));
    const independentStart = runtime.ingest({ kind: "session-start", eventId: "independent-start", session: independent });
    await vi.waitFor(() => expect(port.opened).toEqual(["blocked", "independent"]));

    port.releaseBlocked();
    await Promise.all([blockedStart, independentStart]);
  });

  it("does not overlap events submitted to the same session", async () => {
    const profile = ADAPTER_CAPABILITIES.codex;
    const port = new OrderedEventPort();
    const runtime = new RerailHostRuntime(profile, port);
    const session = { host: profile.host, sessionId: "ordered", cwd: "/workspace" };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });
    port.observed.length = 0;

    const first = runtime.ingest({ kind: "user-prompt", eventId: "first", sessionId: session.sessionId, text: "first" });
    const second = runtime.ingest({ kind: "user-prompt", eventId: "second", sessionId: session.sessionId, text: "second" });
    await vi.waitFor(() => expect(port.observed).toEqual(["first:persist:start"]));

    const stopped = runtime.stop(session.sessionId);
    port.releaseFirstEvent();
    await Promise.all([first, second, stopped]);
    expect(port.observed).toEqual([
      "first:persist:start",
      "first:persist:end",
      "first:sample",
      "second:persist:start",
      "second:persist:end",
      "second:sample",
      "ordered:close",
    ]);
  });

  it("deduplicates event ids within a session rather than across sessions", async () => {
    const profile = ADAPTER_CAPABILITIES.codex;
    const port = new RetryableEndPort();
    const runtime = new RerailHostRuntime(profile, port);
    const sessions = ["first-session", "second-session"].map(sessionId => ({
      host: profile.host,
      sessionId,
      cwd: "/workspace",
    }));
    await Promise.all(sessions.map(session =>
      runtime.ingest({ kind: "session-start", eventId: `start:${session.sessionId}`, session }),
    ));

    await Promise.all(sessions.map(session =>
      runtime.ingest({ kind: "user-prompt", eventId: "shared-event-id", sessionId: session.sessionId, text: "continue" }),
    ));

    expect(port.events.filter(event => event.eventId === "shared-event-id").map(event => (
      event.kind === "session-start" ? event.session.sessionId : event.sessionId
    )).sort()).toEqual(["first-session", "second-session"]);
  });
});

class SamplePort implements HostRuntimePort {
  readonly samples: AdapterSample[] = [];

  async openSession(): Promise<HostRuntimeSessionState> {
    return { processedEventIds: [] };
  }

  async closeSession(): Promise<void> {}
  async persistEvent(): Promise<void> {}
  async persistTurn(): Promise<void> {}
  async persistDelivery(): Promise<void> {}

  async acceptSample(sample: AdapterSample): Promise<void> {
    this.samples.push(sample);
  }

  async deliver(): Promise<void> {}
}

describe("RerailHostRuntime tool samples", () => {
  it("attaches a tool call's arguments as structured input and leaves tool results without it", async () => {
    const profile = ADAPTER_CAPABILITIES.codex;
    const port = new SamplePort();
    const runtime = new RerailHostRuntime(profile, port);
    const session = { host: profile.host, sessionId: "tools", cwd: "/workspace" };
    await runtime.ingest({ kind: "session-start", eventId: "start", session });

    await runtime.ingest({
      kind: "call", eventId: "call", sessionId: session.sessionId, toolCallId: "t1", toolName: "Bash",
      input: { command: "git push --force", description: "push" },
    });
    await runtime.ingest({
      kind: "result", eventId: "result", sessionId: session.sessionId, toolCallId: "t1", toolName: "Bash",
      output: "rejected: git push --force",
    });

    expect(port.samples).toEqual([
      {
        eventId: "call", sessionId: "tools", source: "tool:Bash", boundary: "tool-call",
        text: '{"command":"git push --force","description":"push"}',
        input: '{"command":"git push --force","description":"push"}',
      },
      { eventId: "result", sessionId: "tools", source: "tool:Bash", boundary: "tool-call", text: "rejected: git push --force" },
    ]);
  });
});
