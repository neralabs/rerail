import { describe, expect, it } from "vitest";
import {
  decodeRerailEventFrame,
  decodeRerailEventResult,
  encodeRerailEventResult,
  RERAIL_EVENT_MAX_FRAME_BYTES,
} from "../src/event-protocol.js";

describe("Rerail event protocol", () => {
  it("decodes one strict lifecycle event frame", () => {
    expect(decodeRerailEventFrame(JSON.stringify({
      event: {
        kind: "user-prompt",
        eventId: "session-1:prompt-1",
        sessionId: "session-1",
        text: "Keep the session active",
      },
    }))).toEqual({
      event: {
        kind: "user-prompt",
        eventId: "session-1:prompt-1",
        sessionId: "session-1",
        text: "Keep the session active",
      },
    });
  });

  it("rejects unknown fields and oversized frames", () => {
    expect(() => decodeRerailEventFrame(JSON.stringify({
      event: {
        kind: "session-start",
        eventId: "start",
        session: { host: "claude-code", sessionId: "session-1", cwd: "/workspace" },
        method: "tools/call",
      },
    }))).toThrow();
    expect(() => decodeRerailEventFrame("x".repeat(RERAIL_EVENT_MAX_FRAME_BYTES + 1))).toThrow(/exceeds/);
  });

  it("encodes the minimal correlated result", () => {
    expect(JSON.parse(encodeRerailEventResult({
      eventId: "session-1:prompt-1",
      deliveries: [],
    }))).toEqual({ eventId: "session-1:prompt-1", deliveries: [] });
  });

  it("carries delivery rule ids across the engine socket and tolerates an engine service that sends none", () => {
    const delivery = {
      sessionId: "session-1",
      requested: "steer" as const,
      mechanism: "additional-context" as const,
      content: "No raw SQL.",
      ruleIds: ["rerail:no-raw-sql", "team:api-auth"],
    };
    const encoded = encodeRerailEventResult({ eventId: "session-1:prompt-1", deliveries: [delivery] });
    expect(JSON.parse(encoded).deliveries[0].ruleIds).toEqual(["rerail:no-raw-sql", "team:api-auth"]);
    expect(decodeRerailEventResult(encoded).deliveries).toEqual([delivery]);

    const { ruleIds: _omitted, ...legacy } = delivery;
    expect(decodeRerailEventResult(JSON.stringify({ eventId: "session-1:prompt-1", deliveries: [legacy] })).deliveries)
      .toEqual([{ ...legacy, ruleIds: [] }]);
  });
});
