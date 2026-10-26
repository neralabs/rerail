import { execFileSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { gitCandidates, readGitBlobs } from "../../src/engine/git.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

async function repository(files: Record<string, string>): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-blobs-"));
  roots.push(root);
  git(root, "init", "--quiet");
  for (const [path, text] of Object.entries(files)) await writeFile(join(root, path), text);
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

describe("readGitBlobs", () => {
  it("keeps every blob it was asked for when another in the batch is larger than the limit", async () => {
    const root = await repository({ "big.py": "x".repeat(5_000), "small.py": "print(1)\n" });
    await writeFile(join(root, "small.py"), "print(2)\n");
    git(root, "add", "small.py");
    const blobs = await readGitBlobs(root, [":./big.py", ":./small.py", "HEAD:./small.py", ":./missing.py"], { maxBlobBytes: 1_000 });
    expect(Object.fromEntries([...blobs].map(([revision, blob]) => [revision, blob.toString("utf8")])))
      .toEqual({ ":./small.py": "print(2)\n", "HEAD:./small.py": "print(1)\n" });
  });

  it("never fetches a blob a partial clone does not hold", async () => {
    const origin = await repository({ "a.py": "print(1)\n" });
    git(origin, "config", "uploadpack.allowFilter", "true");
    const clone = join(origin, "..", `${origin.split("/").pop()}-clone`);
    roots.push(clone);
    git(origin, "clone", "--quiet", "--filter=blob:none", "--no-checkout", `file://${origin}`, clone);
    const oid = git(clone, "rev-parse", "HEAD:a.py").trim();
    expect(await readGitBlobs(clone, ["HEAD:./a.py"], { maxBlobBytes: 1_000 })).toEqual(new Map());
    // Still missing: asking for it fetched nothing from the remote.
    expect(() => execFileSync("git", ["cat-file", "-e", oid], { cwd: clone, env: { ...process.env, GIT_NO_LAZY_FETCH: "1" }, stdio: "ignore" })).toThrow();
  });
});

describe("gitCandidates", () => {
  it("looks in the per-user installs too, never on PATH or anywhere the environment points", () => {
    // A repository's environment (direnv, a devcontainer) can set these as easily as PATH.
    vi.stubEnv("LOCALAPPDATA", String.raw`C:\repo\evil`);
    vi.stubEnv("PATH", "/home/me/repo/bin");
    try {
      expect(gitCandidates("win32", String.raw`C:\Users\me`)).toEqual([
        String.raw`C:\Program Files\Git\cmd\git.exe`,
        String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
        String.raw`C:\Users\me\AppData\Local\Programs\Git\cmd\git.exe`,
        String.raw`C:\Users\me\scoop\apps\git\current\cmd\git.exe`,
      ]);
      expect(gitCandidates("linux", "/home/me")).toEqual([
        "/usr/bin/git",
        "/usr/local/bin/git",
        "/opt/homebrew/bin/git",
        "/home/linuxbrew/.linuxbrew/bin/git",
        "/run/current-system/sw/bin/git",
        "/home/me/.nix-profile/bin/git",
      ]);
    } finally {
      vi.unstubAllEnvs();
    }
  });
});
