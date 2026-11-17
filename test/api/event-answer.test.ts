import { mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { eventAnswerFacts } from "../../src/api/event-answer.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import type { AdapterDelivery, AdapterEvent } from "../../src/engine/index.js";
import { startTestRepository } from "../support/started-repository.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const roots: string[] = [];
const opened: DatabaseApiStores[] = [];
afterEach(async () => {
  for (const stores of opened.splice(0)) stores.close();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** The stores of a daemon whose dashboard is at `origin`, over a database recording one started checkout. */
async function daemonAt(origin: string) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-event-answer-")));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const repositoryKey = startTestRepository(root, databasePath);
  const stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => origin,
  });
  opened.push(stores);
  return { root, repositoryKey, stores };
}

/** A prompt of the session at `cwd` whose answer steers about one rule, so the hook links it. */
function steeredPrompt(cwd: string): { event: AdapterEvent; deliveries: AdapterDelivery[] } {
  const session = { host: "claude-code" as const, sessionId: "answered", cwd };
  return {
    event: { kind: "user-prompt", eventId: "prompt", sessionId: session.sessionId, session, text: "Add the classifier." },
    deliveries: [{
      sessionId: session.sessionId, requested: "steer", mechanism: "additional-context", content: "Remove print.",
      ruleIds: ["acme:no-print"], rules: [{ id: "acme:no-print", state: "steered" }],
    }],
  };
}

describe("the facts of the daemon's answer to a hook's event", () => {
  it("links a steer to the checkout's own dashboard page", async () => {
    const { root, repositoryKey, stores } = await daemonAt("http://127.0.0.1:4737");
    const { event, deliveries } = steeredPrompt(root);
    const { dashboard } = await eventAnswerFacts(stores, event, deliveries);
    const link = new URL(dashboard ?? "about:blank");
    expect(`${link.origin}${link.pathname}`).toBe("http://127.0.0.1:4737/");
    expect(link.searchParams.get("repository")).toBe(repositoryKey);
    expect(link.searchParams.get("checkout")).not.toBeNull();
  });

  it("gives no link where the daemon listens where a browser should not be sent, as the hook refuses that address", async () => {
    const { root, stores } = await daemonAt("http://0.0.0.0:4737");
    const { event, deliveries } = steeredPrompt(root);
    expect(await eventAnswerFacts(stores, event, deliveries)).toEqual({ dashboard: null });
  });

  it("says nothing for an answer with nothing to link", async () => {
    const { root, stores } = await daemonAt("http://127.0.0.1:4737");
    expect(await eventAnswerFacts(stores, steeredPrompt(root).event, [])).toEqual({});
  });
});
