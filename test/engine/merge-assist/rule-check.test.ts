import { execFileSync } from "node:child_process";
import { mkdirSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkCodeRule } from "../../../src/engine/code-rule/rule-check.js";
import { codeRuleSchema, type CodeRule } from "../../../src/engine/code-rule/code-rule-schema.js";

/** `checkCodeRule` is Merge Assist's check of a drafted or peer rule; `rerail rules test` reports through the same core. */

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

async function repository(files: Record<string, string>): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-rule-check-"));
  roots.push(root);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: root });
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

function noRetryLoop(pattern = String.raw`retry\(`): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1",
    id: "candidate:retries-in-transport-3f9a1c",
    revision: 1,
    mode: "steer",
    description: "Retries live in the transport layer.",
    scope: { include: ["src/**/*.ts"], exclude: [] },
    clauses: [{
      id: "no-retry",
      forbid: { regex: { pattern, source: "file.content" } },
      violation: { anchor: "witness.span", code: "NO_RETRY", message: "Retries live in the transport layer." },
    }],
  }) as CodeRule;
}

const FILES = {
  "src/api/a.ts": "export const a = 1;\nretry(call);\n",
  "src/api/b.ts": "retry(other);\n",
  "src/api/clean.ts": "export const clean = 1;\n",
  "lib/c.ts": "retry(outside);\n",
  "src/ignored/d.ts": "retry(ignored);\n",
  ".gitignore": "src/ignored/\n",
};

describe("checkCodeRule", () => {
  it("runs the cases and reports every match below the root, the list capped at maxMatches", async () => {
    const root = await repository(FILES);
    const result = await checkCodeRule(noRetryLoop(), {
      root,
      maxMatches: 1,
      cases: [
        { expected: "caught", label: "the agent's loop", path: "src/api/x.ts", content: "retry(fetch);\n" },
        { expected: "passed", label: "the corrected call", path: "src/api/x.ts", content: "transport.send();\n" },
        { expected: "caught", label: "outside its scope", path: "lib/x.ts", content: "retry(fetch);\n" },
      ],
    });

    expect(result).toEqual({
      valid: true,
      error: null,
      cases: [
        { expected: "caught", label: "the agent's loop", caught: true, ok: true },
        { expected: "passed", label: "the corrected call", caught: false, ok: true },
        { expected: "caught", label: "outside its scope (outside the files this rule covers)", caught: false, ok: false },
      ],
      matches: [{ path: "src/api/a.ts", line: 2, column: 1, message: "Retries live in the transport layer." }],
      // A file Git ignores stays out while the repository follows .gitignore.
      matchCount: 2,
    });
  });

  it("limits the repository pass to the given paths, and still never reads past the data boundary", async () => {
    const root = await repository({ ...FILES, ".env.ts": "retry(secret);\n" });
    mkdirSync(join(root, "src", "linked"));
    symlinkSync(join(root, "src", "api", "b.ts"), join(root, "src", "linked", "b.ts"));
    symlinkSync(join(root, "src", "api"), join(root, "src", "folder-link"));

    const limited = await checkCodeRule(noRetryLoop(), {
      root,
      cases: [],
      paths: ["src/api/b.ts", "src/api/clean.ts", "src/api/deleted.ts", "lib/c.ts", "src/ignored/d.ts", "../outside.ts", "/etc/passwd", "src/linked/b.ts", "src/folder-link/a.ts", ".env.ts"],
    });
    expect(limited).toMatchObject({ valid: true, matchCount: 1, matches: [{ path: "src/api/b.ts", line: 1 }] });

    const unfollowed = await checkCodeRule(noRetryLoop(), { root, cases: [], paths: ["src/ignored/d.ts"], followGitignore: false });
    expect(unfollowed).toMatchObject({ valid: true, matchCount: 1, matches: [{ path: "src/ignored/d.ts" }] });
  });

  it("reports a rule the Engine refuses instead of throwing", async () => {
    const root = await repository(FILES);
    const result = await checkCodeRule(noRetryLoop("(unclosed"), { root, cases: [{ expected: "caught", label: "x", path: "src/x.ts", content: "retry(" }] });
    expect(result).toMatchObject({ valid: false, cases: [], matches: [], matchCount: 0 });
    expect(result.error).toContain("candidate:retries-in-transport-3f9a1c does not compile");
  });
});
