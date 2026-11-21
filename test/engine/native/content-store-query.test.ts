import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { SENSITIVE_FILE_NAMES, SENSITIVE_FOLDER_NAMES } from "../../../src/engine/path-boundary.js";
import { queryContentStore } from "../../support/engine/content-store-query.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const capabilities = JSON.stringify({
  format: "rerail.policy-capabilities/v1",
  ast: { engine: "ast-grep-core", engineVersion: "0.45", languages: { python: "tree-sitter-python@0.23" } },
  factRelations: {},
  limits: { maxAstPatternBytes: 16384, maxFiles: 100, maxGraphDepth: 16, maxRegexPatternBytes: 16384, maxSourceBytes: 1_000_000, maxVisitedNodes: 100_000, maxWitnesses: 10_000 },
});

const needle = JSON.stringify({
  format: "rerail.policy/v1", id: "store-query-needle", revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] },
  clauses: [{ id: "needle", forbid: { regex: { pattern: "NEEDLE", source: "file.content" } }, violation: { anchor: "witness.span", code: "NEEDLE", message: "needle" } }],
});

/** Opens the store read-write in another process, reads it and closes it: the last connection to close removes the WAL. */
function openAndCloseElsewhere(storePath: string): void {
  const driver = createRequire(import.meta.url).resolve("better-sqlite3");
  execFileSync(process.execPath, [
    "-e",
    "const Database = require(process.argv[1]); const store = new Database(process.argv[2]); store.prepare('SELECT count(*) FROM sqlite_master').get(); store.close();",
    driver,
    storePath,
  ]);
}

describe("reading the engine's content store", () => {
  // Read in this process, through better-sqlite3, the store lost the engine's locks: the next process to close it took
  // itself for the last connection and removed the WAL and the wal-index from under the engine, and one opening it cut
  // the wal-index the engine maps (SIGBUS in the engine, code-rule-enforcer-mechanical.test.ts on CI).
  it("leaves the engine holding the store, so another process's close is not the last one", async () => {
    const root = mkdtempSync(join(tmpdir(), "rerail-store-query-"));
    const storePath = join(root, "store", "content.sqlite");
    const checkout = join(root, "checkout");
    const binding = loadNativeBinding();
    const workspace = new binding.CodeRuleDslWorkspace!();
    try {
      execFileSync("git", ["init", "--quiet", checkout]);
      writeFileSync(join(checkout, "seed.py"), "value = 1\n");
      execFileSync("git", ["add", "."], { cwd: checkout });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: checkout });
      workspace.addCodeRule(binding.CodeRuleDslRuntime!.compile(needle, "json", capabilities));
      expect((await workspace.enableStorageAsync(checkout, storePath, { files: SENSITIVE_FILE_NAMES, folders: SENSITIVE_FOLDER_NAMES })).attached).toBe(true);
      const source = "a = 'NEEDLE'\n";
      await workspace.applyAsync(JSON.stringify({ revision: 1, files: [{ path: "src/a.py", language: "python", source }], removed: [] }));
      expect(existsSync(`${storePath}-wal`) && existsSync(`${storePath}-shm`)).toBe(true);

      expect(queryContentStore(storePath, "SELECT raw_length AS bytes FROM source_objects")).toEqual([{ bytes: Buffer.byteLength(source) }]);
      openAndCloseElsewhere(storePath);
      expect({ wal: existsSync(`${storePath}-wal`), walIndex: existsSync(`${storePath}-shm`) }).toEqual({ wal: true, walIndex: true });
    } finally {
      workspace.close();
      removeFolderSync(root);
    }
  });
});
