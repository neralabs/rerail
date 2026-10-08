import { afterEach, describe, expect, test } from "vitest";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import Database from "better-sqlite3";
import { NativeSemanticEmbedder, SemanticWorkspace } from "../src/semantic-primitives.js";
import { OmpMessageContentSource, OmpWatermarkStore } from "../src/omp-semantic.js";
import { defaultRerailDatabasePath } from "../src/database.js";
import { openScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import type { ConversationRule } from "../src/rule-schema.js";
import { SqlitePresentationPreferenceStore } from "../src/presentation-preference-store.js";
import { OmpHost } from "./omp-host.js";
import type { HostEvidence } from "./omp-host.js";

const originalRerailHome = process.env.RERAIL_HOME;
const temporaryDirectories: string[] = [];
afterEach(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});
async function isolate(): Promise<string> {
  const cwd = await mkdtemp(join(tmpdir(), "rerail-omp-contract-"));
  temporaryDirectories.push(cwd);
  process.env.RERAIL_HOME = join(cwd, "home");
  return cwd;
}
function checkoutRule(id: string): ConversationRule {
  return { id, mode: "steer", reference: "Guest checkout remains available.", match: { operator: "gte", threshold: 0.99 },
    sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 1 },
    action: { content: "Check checkout requirements." }, repeat: { mode: "once" } };
}
function matches(evidence: HostEvidence) {
  return evidence.branch.filter(entry => entry.type === "custom" && "customType" in entry && entry.customType === "rerail.match");
}
function expectNoExtensionErrors(host: OmpHost): void {
  expect(host.frames.filter(frame => frame.type === "extension_error")).toEqual([]);
  expect(host.errors.join("")).not.toMatch(/extension.*(?:failed|error)|failed.*extension/iu);
}

describe("installed OMP extension in standalone host", () => {
  test.each(["compact", "silent"] as const)(
    "interrupts a real streamed session and persists the steer with %s presentation",
    async presence => {
      const cwd = await isolate();
      const preferences = new SqlitePresentationPreferenceStore(defaultRerailDatabasePath());
      preferences.upsert({ repository: cwd, host: "omp", presence });
      preferences.close();
      await mkdir(join(cwd, ".omp"));
      await writeFile(join(cwd, ".omp", "conversation-rules.json"), JSON.stringify({ version: 1, rules: [checkoutRule("checkout")] }));
    const host = await OmpHost.start(cwd, [
      { content: ["Guest checkout remains available. Next"] }, { content: ["Requirement acknowledged."] },
    ]);
    let sessionId: string;
    try {
      await host.prompt("Describe the checkout change.");
      const evidence = await host.snapshot();
      sessionId = evidence.sessionId;
      expectNoExtensionErrors(host);
      expect(evidence.calls).toHaveLength(2);
      expect(matches(evidence)).toHaveLength(1);
      expect(JSON.stringify(evidence.calls[1]?.context.messages)).toContain("Check checkout requirements.");
    } finally { await host.close(); }
    const transcript = new Database(defaultRerailDatabasePath(), { readonly: true });
    try {
      const recorded = transcript.prepare("SELECT kind, payload_json AS payloadJson FROM events WHERE session_id = ? ORDER BY sequence")
        .all(sessionId!) as Array<{ kind: string; payloadJson: string }>;
      const kinds = recorded.map(event => event.kind);
      const sessionStartIndex = kinds.indexOf("session-start");
      const sessionEndIndex = kinds.lastIndexOf("session-end");
      expect(sessionStartIndex).toBeGreaterThanOrEqual(0);
      expect(sessionEndIndex).toBeGreaterThan(sessionStartIndex);
      expect(recorded.at(-1)?.kind).toBe("session-end");
      expect(kinds).toEqual(expect.arrayContaining(["assistant-delta", "user-prompt", "turn-end"]));
      expect(recorded.filter(event => event.kind === "steering-delivered")).toHaveLength(1);
      const messages = recorded.filter(event => event.kind === "user-prompt" || event.kind === "turn-end").map(event => ({
        role: event.kind === "user-prompt" ? "user" : "assistant", text: (JSON.parse(event.payloadJson) as { text: string }).text,
      }));
      expect(messages).toEqual(expect.arrayContaining([
        { role: "user", text: "Describe the checkout change." },
        { role: "assistant", text: "Guest checkout remains available. Next" },
        { role: "assistant", text: "Requirement acknowledged." },
      ]));
      expect(transcript.prepare("SELECT host, cwd, ended_at AS endedAt FROM sessions WHERE session_id = ?").get(sessionId!))
        .toEqual({ host: "omp", cwd, endedAt: expect.any(Number) });
    } finally { transcript.close(); }
    },
    90_000,
  );

  test("steers on a conversation rule enabled in the catalog after the session started", async () => {
    const cwd = await isolate();
    const store = openScopedRuleCatalog(cwd);
    store.upsertConversationRule(checkoutRule("db:checkout"), { enabled: false });
    let host: OmpHost | undefined;
    try {
      host = await OmpHost.start(cwd, [
        { content: ["Guest checkout remains available. First"] }, { content: ["Guest checkout remains available. Second"] },
        { content: ["Requirement acknowledged."] },
      ]);
      await host.prompt("Describe the checkout change.");
      const before = await host.snapshot();
      expectNoExtensionErrors(host);
      expect(before.calls).toHaveLength(1);
      expect(matches(before)).toHaveLength(0);
      expect(store.setEnabled({ ids: ["db:checkout"] }, true)).toBe(1);
      await host.prompt("Describe it again.");
      const after = await host.snapshot();
      expectNoExtensionErrors(host);
      expect(after.calls).toHaveLength(3);
      expect(matches(after)).toHaveLength(1);
      expect(JSON.stringify(after.calls[2]?.context.messages)).toContain("Check checkout requirements.");
    } finally { try { await host?.close(); } finally { store.close(); } }
  }, 90_000);

  test("survives a rerail reset run underneath it and keeps recording", async () => {
    const cwd = await isolate();
    const databasePath = defaultRerailDatabasePath();
    const host = await OmpHost.start(cwd, [{ content: ["First answer."] }, { content: ["Second answer."] }]);
    try {
      await host.prompt("First.");
      const { sessionId } = await host.snapshot();
      const recorded = () => {
        const transcript = new Database(databasePath, { readonly: true });
        try { return (transcript.prepare("SELECT kind FROM events WHERE session_id = ? ORDER BY sequence").all(sessionId) as Array<{ kind: string }>).map(row => row.kind); }
        finally { transcript.close(); }
      };
      expectNoExtensionErrors(host);
      expect(recorded()).toContain("turn-end");
      const before = await stat(databasePath);
      const reset = spawnSync(process.execPath, [resolve("dist/cli.js"), "reset"], {
        cwd, env: { ...process.env, RERAIL_HOME: join(cwd, "home") }, encoding: "utf8", timeout: 30_000,
      });
      expect(reset.status, reset.stderr).toBe(0);
      expect((await stat(databasePath)).ino).toBe(before.ino);
      expect(recorded()).toEqual([]);
      await host.prompt("Second.");
      expectNoExtensionErrors(host);
      expect(recorded()).toContain("user-prompt");
      expect(recorded()).toContain("turn-end");
      expect(recorded()).not.toContain("session-start");
    } finally { await host.close(); }
  }, 90_000);

  test("marks and projects real OMP branch messages", async () => {
    const cwd = await isolate();
    const host = await OmpHost.start(cwd, [{ content: ["Use an append-only watermark ledger."] }], { extension: "none" });
    try {
      await host.prompt("Design the selector.");
      let evidence = await host.snapshot();
      const pending: Array<{ customType: string; data: unknown }> = [];
      const workspace = new SemanticWorkspace({
        sources: [new OmpMessageContentSource({ sessionId: evidence.sessionId, entries: () => evidence.branch })],
        watermarks: new OmpWatermarkStore({ entries: () => evidence.branch, appendEntry: (customType, data) => { pending.push({ customType, data }); } }),
        embedder: new NativeSemanticEmbedder(resolve("model")),
      });
      const userMessage = (await workspace.select({ from: { owners: ["user"] } })).items[0];
      expect(userMessage).toBeDefined();
      await workspace.mark({ kind: "attach", targets: [userMessage!.ref], watermarks: [{
        namespace: "integration", key: "embed", value: true, owner: "test", visibility: "private", createdAt: Date.now(),
      }] });
      for (const entry of pending) await host.append(entry.customType, entry.data);
      evidence = await host.snapshot();
      const selection = await workspace.select({ where: { mark: { namespace: "integration", key: "embed", value: true } } });
      const projection = await workspace.project(selection, { unit: { kind: "item" }, render: { kind: "text-only" } });
      expect(selection.items.map(item => item.text)).toEqual(["Design the selector."]);
      expect(projection.vectors).toHaveLength(1);
      expect(projection.vectors[0]?.values).toHaveLength(projection.model.dimensions);
    } finally { await host.close(); }
  }, 90_000);
});
