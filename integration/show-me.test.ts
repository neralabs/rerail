import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ingestEngineEvent, startEngineService } from "../src/runtime/index.js";
import type { AdapterDelivery } from "../src/engine/index.js";
import { runShowMeCommand, SHOW_ME_FILE, SHOW_ME_FIXED, SHOW_ME_MESSAGE, SHOW_ME_RULE_ID, SHOW_ME_VIOLATING } from "../src/cli/index.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { awaitCodeRulesOpen } from "./support/code-rules-open.js";
import { escapeRegExp } from "./support/steer-text.js";

const originalRerailHome = process.env.RERAIL_HOME;
let isolatedRerailHome: string;

beforeAll(async () => {
  isolatedRerailHome = await mkdtemp(join(tmpdir(), "rerail-show-me-home-"));
  process.env.RERAIL_HOME = isolatedRerailHome;
});

afterAll(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await rm(isolatedRerailHome, { recursive: true, force: true });
});

const demoSteers = (deliveries: readonly AdapterDelivery[]): AdapterDelivery[] =>
  deliveries.filter(delivery => delivery.requested === "steer" && delivery.ruleIds.includes(SHOW_ME_RULE_ID));

function showMe(args: string[]): number {
  return runShowMeCommand(args, () => "usage", { stdout: { write: () => true }, stderr: { write: () => true } });
}

describe("/rerail-show-me in a running session", () => {
  it("steers on the demo rule added after the session started, and stops once cleaned up", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-show-me-"));
    setUpTestRepository(root);
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail", "sessions");
    const sessionId = "show-me";
    const file = join(root, SHOW_ME_FILE);
    let service: { close(): Promise<void> } | undefined;
    let step = 0;
    const write = async (content: string): Promise<AdapterDelivery[]> => {
      const toolCallId = `write-${++step}`;
      await ingestEngineEvent(socketPath, { kind: "call", eventId: `call:${toolCallId}`, toolCallId, sessionId, toolName: "write", input: { path: SHOW_ME_FILE } });
      await writeFile(file, content);
      return [...await ingestEngineEvent(socketPath, { kind: "result", eventId: `result:${toolCallId}`, toolCallId, sessionId, toolName: "write", output: { ok: true } })];
    };
    // A turn's end settles every pending check, so what it returns is all the turn still had to say.
    const endTurn = async (): Promise<readonly AdapterDelivery[]> =>
      ingestEngineEvent(socketPath, { kind: "turn-end", eventId: `turn:${++step}`, sessionId, text: "done" });

    try {
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      await ingestEngineEvent(socketPath, { kind: "session-start", eventId: "start", session: { host: "omp", sessionId, cwd: root } });
      await awaitCodeRulesOpen(socketPath, { host: "omp", sessionId, cwd: root });

      expect(showMe(["--repo", root, "--json"])).toBe(0);
      const broken = [...await write(SHOW_ME_VIOLATING), ...await endTurn()];
      expect(demoSteers(broken)).toEqual([
        expect.objectContaining({ content: expect.stringMatching(new RegExp(`^- ${escapeRegExp(SHOW_ME_FILE)}:2:\\d+ — ${escapeRegExp(SHOW_ME_MESSAGE)} \\[${escapeRegExp(SHOW_ME_RULE_ID)}\\]$`, "mu")) }),
      ]);

      expect(demoSteers([...await write(SHOW_ME_FIXED), ...await endTurn()])).toEqual([]);

      expect(showMe(["--cleanup", "--repo", root])).toBe(0);
      expect(demoSteers([...await write(SHOW_ME_VIOLATING), ...await endTurn()])).toEqual([]);
      await ingestEngineEvent(socketPath, { kind: "session-end", eventId: "end", sessionId });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
