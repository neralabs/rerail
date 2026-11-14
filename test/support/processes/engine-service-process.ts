// One engine service in a process of its own, as a daemon runs it, so a test can kill it or keep it running beside the
// test's own: node --import tsx engine-service-process.ts <socket> <storage root> <database> <idle ms>
// Its clock is the wall clock plus an offset a message sets; a message can also run its idle close. It answers each
// message with { done: true, now }, and says { ready: true, pid, socketPath } once it listens.
import type { CodeRuleAuditor, SemanticEmbedder } from "../../../src/engine/index.js";
import { startEngineService } from "../../../src/runtime/index.js";

const [socketPath, storageRoot, databasePath, idleMs] = process.argv.slice(2) as [string, string, string, string];
// As `testEmbedder` (hook-events.ts), which imports vitest and so does not load outside a test.
const testEmbedder: SemanticEmbedder = {
  fingerprint: () => ({ modelId: "test", revision: "1", dimensions: 2, weightsSha256: "test", tokenizerSha256: "test", kernelFormatVersion: 1 }),
  embed: async texts => texts.map(() => new Float32Array([1, 0])),
};
const quietAuditor = (): CodeRuleAuditor => ({
  openWorkspace: async () => undefined,
  acceptsPath: () => true,
  auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
  close: async () => undefined,
});
let offset = 0;
const service = await startEngineService({
  socketPath, storageRoot, databasePath, embedder: testEmbedder, idleSessionMs: Number(idleMs), idleRuntimeMs: 0,
  now: () => Date.now() + offset, createAuditor: quietAuditor,
});
process.on("message", (message: { offset?: number; sweep?: boolean }) => {
  void (async () => {
    if (typeof message.offset === "number") offset = message.offset;
    if (message.sweep === true) await service.closeIdleSessions();
    process.send?.({ done: true, now: Date.now() + offset });
  })();
});
process.send?.({ ready: true, pid: process.pid, socketPath: service.socketPath });
