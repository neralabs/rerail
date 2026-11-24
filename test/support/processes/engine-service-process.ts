// One engine service in a process of its own, as a daemon runs it, so a test can kill it or keep it running beside the
// test's own: node --import tsx engine-service-process.ts <socket> <storage root> <database> <idle ms> [audits]
// With `audits`, each batch of a session's edits is recorded as a check, started and finished through the runtime's own
// audit store, as the real evaluator records its checks, and it says { opened: <session id> } once that session's
// evaluator has opened; without it, nothing is recorded.
// Its clock is the wall clock plus an offset a message sets; a message can also run its idle close. It answers each
// message with { done: true, now }, and says { ready: true, pid, socketPath } once it listens.
import type { AuditRecorder, CodeRuleAuditor, SemanticEmbedder } from "../../../src/engine/index.js";
import { startEngineService } from "../../../src/runtime/index.js";

const [socketPath, storageRoot, databasePath, idleMs, recorded] = process.argv.slice(2) as [string, string, string, string, string?];
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
const recordingAuditor = ({ workspaceRoot, audits }: { workspaceRoot: string; audits: AuditRecorder }): CodeRuleAuditor => ({
  ...quietAuditor(),
  // A session's first start does not wait for its evaluator: an edit that comes while it opens is answered unchecked
  // (runtime.ts `#codeRuleWorkspaceNow`), so a test that needs the edit's check waits for this message first. The rest
  // of the evaluator's open runs in this turn once this returns, before this process reads the test's next request.
  openWorkspace: async sessionId => {
    process.send?.({ opened: sessionId });
  },
  auditGeneratedEdits: async (sessionId, batch) => {
    const auditId = audits.start({
      sessionId, workspaceRoot, scope: "touched-files", mode: "steer", revision: batch.revision, catalogRevision: null, ruleRevisions: [], paths: [],
    });
    audits.finish(auditId, { status: "completed", violations: [], failures: [] });
    return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
  },
});
let offset = 0;
const service = await startEngineService({
  socketPath, storageRoot, databasePath, embedder: testEmbedder, idleSessionMs: Number(idleMs), idleRuntimeMs: 0,
  now: () => Date.now() + offset, createAuditor: recorded === "audits" ? recordingAuditor : quietAuditor,
});
process.on("message", (message: { offset?: number; sweep?: boolean }) => {
  void (async () => {
    if (typeof message.offset === "number") offset = message.offset;
    if (message.sweep === true) await service.closeIdleSessions();
    process.send?.({ done: true, now: Date.now() + offset });
  })();
});
process.send?.({ ready: true, pid: process.pid, socketPath: service.socketPath });
