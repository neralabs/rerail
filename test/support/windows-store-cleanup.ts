import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { TestProject } from "vitest/node";

declare module "vitest" {
  interface ProvidedContext {
    windowsStoreRoot: string;
  }
}

export default function setup(project: TestProject): void {
  const root = mkdtempSync(join(tmpdir(), "rerail-vitest-stores-"));
  project.provide("windowsStoreRoot", root);
  // Vitest closes its worker pool before onClose. All native SQLite handles are then closed.
  project.vitest.onClose(() => rmSync(root, { recursive: true, force: true, maxRetries: 3 }));
}
