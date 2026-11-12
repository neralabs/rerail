/**
 * Files two branches change near each other, and a session runtime over a Merge Assist workspace, for the tests that
 * raise in-flight conflicts and follow them to a session's boundaries.
 */
import { join } from "node:path";
import type { Conflict } from "../../../../src/engine/index.js";
import { RerailRuntime, type MergeAssistRuntime } from "../../../../src/runtime/index.js";
import { testEmbedder } from "../../../support/hook-events.js";
import type { Workspace } from "../../../support/merge-assist/workspace.js";

/** `function <name>() {`, 38 numbered lines, `}`: a file whose line 20 both sides change. */
export const body = (name: string, changes: Readonly<Record<number, string>> = {}): string =>
  `${[`function ${name}() {`, ...Array.from({ length: 38 }, (_, index) => `  const ${name}${index + 2} = ${index + 2};`), "}"]
    .map((line, index) => changes[index + 1] ?? line).join("\n")}\n`;

/** Line 20 changed and `extra` lines built on it, as theirs commits it. */
export const grown = (name: string, extra: number): string =>
  body(name, { 20: [`  const ${name}20 = "theirs";`, ...Array.from({ length: extra }, (_, index) => `  const grown${index} = ${name}20;`)].join("\n") });

/** The conflicts not dismissed. */
export const open = (conflicts: readonly Conflict[]): Conflict[] => conflicts.filter(conflict => conflict.status === "open");

/** A session runtime over the workspace's database with Merge Assist `service`, and an evaluator that finds nothing. */
export function sessionRuntime(ws: Workspace, service: MergeAssistRuntime): RerailRuntime {
  return new RerailRuntime({
    autostart: false,
    workspaceRoot: ws.repo,
    storageRoot: join(ws.folder, "sessions"),
    databasePath: ws.databasePath,
    mergeAssist: service,
    embedder: testEmbedder,
    createAuditor: () => ({
      openWorkspace: async () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue", violations: [], failures: [] }),
      close: async () => undefined,
    }),
  });
}
