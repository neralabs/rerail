import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { commitStagingNote, gitCommitIn, shellCommandOf, unstagedDecisionFiles } from "../../../src/engine/merge-assist/commit-staging.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

function write(root: string, files: Record<string, string>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

describe("commit staging", () => {
  it("reads the command of a shell tool call only", () => {
    expect(shellCommandOf("Bash", { command: "git commit -m x" })).toBe("git commit -m x");
    expect(shellCommandOf("exec_command", { cmd: ["git", "commit", "-am", "x"] })).toBe("git commit -am x");
    expect(shellCommandOf("Edit", { command: "git commit" })).toBeNull();
    expect(shellCommandOf("Bash", null)).toBeNull();
  });

  it("recognises git commit in any simple command, with its -a flag, and nothing else", () => {
    expect(gitCommitIn("git commit -m \"fix; then git push\"")).toEqual({ all: false });
    expect(gitCommitIn("npm test && git -C . -c core.hooksPath=/dev/null commit -am 'x'")).toEqual({ all: true });
    expect(gitCommitIn("GIT_AUTHOR_NAME=x git commit --all")).toEqual({ all: true });
    expect(gitCommitIn("git commit --amend --no-edit")).toEqual({ all: false });
    expect(gitCommitIn("git log --grep commit")).toBeNull();
    expect(gitCommitIn("echo git commit")).toBeNull();
    expect(gitCommitIn("git status")).toBeNull();
  });

  it("lists decision and candidate files a commit would leave out: untracked, or changed and not staged unless -a takes them, ledger entries only with the ledger", async () => {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-commit-staging-")));
    roots.push(root);
    git(root, "init", "--quiet", "-b", "main");
    write(root, { "README.md": "seed\n", ".rerail/candidates/kept-1a2b3c.json": "{}\n", ".rerail/candidates/edited-4d5e6f.json": "{}\n" });
    git(root, "add", "-A");
    git(root, "commit", "--quiet", "-m", "seed");
    expect(unstagedDecisionFiles(root, false, true)).toEqual([]);

    const entry = `.rerail/decisions/${"a".repeat(32)}.json`;
    const staged = `.rerail/decisions/${"b".repeat(32)}.json`;
    write(root, { [entry]: "{}\n", [staged]: "{}\n", ".rerail/candidates/edited-4d5e6f.json": "{\"x\":1}\n", ".rerail/candidates/new-7a8b9c.json": "{}\n", "src/code.ts": "x\n" });
    git(root, "add", "--", staged);
    expect(unstagedDecisionFiles(root, false, true)).toEqual([".rerail/candidates/edited-4d5e6f.json", ".rerail/candidates/new-7a8b9c.json", entry]);
    expect(unstagedDecisionFiles(root, true, true)).toEqual([".rerail/candidates/new-7a8b9c.json", entry]);
    // Without the experimental decision ledger, only the candidates.
    expect(unstagedDecisionFiles(root, false, false)).toEqual([".rerail/candidates/edited-4d5e6f.json", ".rerail/candidates/new-7a8b9c.json"]);
    expect(commitStagingNote([entry])).toBe(`Rerail: include these decision files in this commit: ${entry}`);
  });
});
