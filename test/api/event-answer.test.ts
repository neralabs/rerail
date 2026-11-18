import { describe, expect, it } from "vitest";
import { eventAnswerFacts } from "../../src/api/event-answer.js";
import type { AdapterDelivery, AdapterEvent, HeldSession } from "../../src/engine/index.js";

const SESSION = { host: "claude-code" as const, sessionId: "answered", cwd: "/work/app/src" };
/** What the runtime that took the event holds of a session in the checkout /work/app of repository acme/app. */
const HELD: HeldSession = { checkout: { repositoryKey: "acme/app", checkoutPath: "/work/app" } };

/** A prompt of the session whose answer steers about one rule, so the hook links it. */
function steeredPrompt(): { event: AdapterEvent; deliveries: AdapterDelivery[] } {
  return {
    event: { kind: "user-prompt", eventId: "prompt", sessionId: SESSION.sessionId, session: SESSION, text: "Add the classifier." },
    deliveries: [{
      sessionId: SESSION.sessionId, requested: "steer", mechanism: "additional-context", content: "Remove print.",
      ruleIds: ["acme:no-print"], rules: [{ id: "acme:no-print", state: "steered" }],
    }],
  };
}

describe("the facts of the daemon's answer to a hook's event", () => {
  it("links a steer to the checkout's own dashboard page", () => {
    const { event, deliveries } = steeredPrompt();
    expect(eventAnswerFacts("http://127.0.0.1:4737", event, deliveries, HELD))
      .toEqual({ dashboard: "http://127.0.0.1:4737/?repository=acme%2Fapp&checkout=%2Fwork%2Fapp" });
  });

  it("gives no link where the daemon listens where a browser should not be sent, as the hook refuses that address", () => {
    const { event, deliveries } = steeredPrompt();
    expect(eventAnswerFacts("http://0.0.0.0:4737", event, deliveries, HELD)).toEqual({ dashboard: null });
  });

  it("says nothing for an answer with nothing to link", () => {
    expect(eventAnswerFacts("http://127.0.0.1:4737", steeredPrompt().event, [], HELD)).toEqual({});
  });

  it("gives a session's start its note's rules and the bare dashboard where Rerail records no checkout", () => {
    const start: AdapterEvent = { kind: "session-start", eventId: "start", session: SESSION };
    const held: HeldSession = { checkout: null, sessionNote: { setUp: true, ruleCount: 8 } };
    expect(eventAnswerFacts("http://127.0.0.1:4737", start, [], held))
      .toEqual({ dashboard: "http://127.0.0.1:4737/", sessionNote: { setUp: true, ruleCount: 8 } });
  });

  it("says nothing where no runtime holds the event's session, so the hook reads both after the answer", () => {
    const { event, deliveries } = steeredPrompt();
    expect(eventAnswerFacts("http://127.0.0.1:4737", event, deliveries, undefined)).toEqual({});
  });
});
