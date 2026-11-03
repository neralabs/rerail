import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { SENSITIVE_FILE_NAMES, SENSITIVE_FOLDER_NAMES } from "../../../src/engine/path-boundary.js";

const capabilities = JSON.stringify({
  format: "rerail.policy-capabilities/v1",
  ast: { engine: "ast-grep-core", engineVersion: "0.45", languages: { python: "tree-sitter-python@0.23" } },
  factRelations: {},
  limits: { maxAstPatternBytes: 16384, maxFiles: 100, maxGraphDepth: 16, maxRegexPatternBytes: 16384, maxSourceBytes: 1_000_000, maxVisitedNodes: 100_000, maxWitnesses: 10_000 },
});

const needle = JSON.stringify({
  format: "rerail.policy/v1", id: "secret-needle", revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] },
  clauses: [{ id: "needle", forbid: { regex: { pattern: "NEEDLE", source: "file.content" } }, violation: { anchor: "witness.span", code: "NEEDLE", message: "needle" } }],
});

describe("content store admission", () => {
  it("evaluates a file named like a secret but never writes it to the content store", async () => {
    const root = mkdtempSync(join(tmpdir(), "rerail-secret-names-"));
    const storePath = join(root, "store", "content.sqlite");
    const checkout = join(root, "checkout");
    const workspace = new (loadNativeBinding().CodeRuleDslWorkspace!)();
    try {
      execFileSync("git", ["init", "--quiet", checkout]);
      writeFileSync(join(checkout, "seed.py"), "value = 1\n");
      execFileSync("git", ["add", "."], { cwd: checkout });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: checkout });
      workspace.addCodeRule(loadNativeBinding().CodeRuleDslRuntime!.compile(needle, "json", capabilities));
      const attachment = await workspace.enableStorageAsync(checkout, storePath, { files: SENSITIVE_FILE_NAMES, folders: SENSITIVE_FOLDER_NAMES });
      expect(attachment.attached).toBe(true);
      const source = "a = 'NEEDLE'\n";
      const files = [
        { path: "config/secrets.py", language: "python", source: "token = 'NEEDLE in a secret'\n" },
        { path: ".aws/settings.py", language: "python", source: "key = 'NEEDLE in a credential folder'\n" },
        { path: "src/a.py", language: "python", source },
      ];
      const applied = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [] }))) as {
        evaluation: { violations: Array<{ path: string }> };
      };
      expect(applied.evaluation.violations.map(violation => violation.path).sort()).toEqual([".aws/settings.py", "config/secrets.py", "src/a.py"]);

      const store = new Database(storePath, { readonly: true });
      try {
        const paths = store.prepare(
          "SELECT strings.value AS path FROM workspace_overlay JOIN paths USING (path_id) JOIN strings ON strings.string_id = paths.string_id",
        ).all() as Array<{ path: string }>;
        expect(paths.map(row => row.path)).toEqual(["src/a.py"]);
        const sources = store.prepare("SELECT raw_length AS bytes FROM source_objects").all() as Array<{ bytes: number }>;
        expect(sources.map(row => row.bytes)).toEqual([Buffer.byteLength(source)]);
      } finally {
        store.close();
      }
    } finally {
      workspace.close();
      rmSync(root, { force: true, recursive: true });
    }
  });
});
