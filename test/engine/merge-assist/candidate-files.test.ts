import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  CANDIDATE_DIRECTORY,
  MAX_CANDIDATE_BYTES,
  MAX_CANDIDATES_PER_BRANCH,
  serializeCandidate,
  type Candidate,
} from "../../../src/engine/merge-assist/candidate.js";
import {
  candidatesIgnored,
  committedCandidates,
  readTreeCandidates,
  readWorktreeCandidates,
  writeWorktreeCandidate,
} from "../../../src/engine/merge-assist/candidate-files.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-candidates-"));
  roots.push(root);
  return root;
}

async function repository(): Promise<string> {
  const root = await temporaryRoot();
  git(root, "init", "--quiet", "-b", "main");
  writeFileSync(join(root, "README.md"), "seed\n");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

function candidate(slug: string, decision = "Retries live in the transport layer."): Candidate {
  return {
    format: "rerail.candidate/v1",
    id: `candidate:${slug}`,
    decision,
    generality: "repository",
    scope: ["src/api/**"],
    author: "tbelanger",
    createdAt: "2026-11-03T14:02:11Z",
    state: "active",
    supersedes: null,
  };
}

function writeCandidateFile(root: string, name: string, text: string): void {
  mkdirSync(join(root, CANDIDATE_DIRECTORY), { recursive: true });
  writeFileSync(join(root, CANDIDATE_DIRECTORY, name), text);
}

describe("readWorktreeCandidates and writeWorktreeCandidate", () => {
  it("reads back what it wrote, as a committable 0644 file, and a missing folder as no candidates", async () => {
    const root = await temporaryRoot();
    expect(await readWorktreeCandidates(root)).toEqual({ candidates: [], problems: [], truncated: false });

    const first = candidate("retries-3f9a1c");
    const path = await writeWorktreeCandidate(root, first);
    expect(path).toBe(join(root, CANDIDATE_DIRECTORY, "retries-3f9a1c.json"));
    expect(statSync(path).mode & 0o777).toBe(0o644);
    expect(await readFile(path, "utf8")).toBe(serializeCandidate(first));

    // Rewriting replaces the file in place; no temporary file is left behind.
    const rejected = { ...first, state: "rejected" as const };
    await writeWorktreeCandidate(root, rejected);
    await writeWorktreeCandidate(root, candidate("errors-as-results-a1b2c3", "API errors are returned as Result values."));
    expect(await readWorktreeCandidates(root)).toEqual({
      candidates: [candidate("errors-as-results-a1b2c3", "API errors are returned as Result values."), rejected],
      problems: [],
      truncated: false,
    });
    expect(readdirSync(join(root, CANDIDATE_DIRECTORY)).sort()).toEqual(["errors-as-results-a1b2c3.json", "retries-3f9a1c.json"]);
  });

  it("follows no symlink: not .rerail, not the candidates folder, not a candidate file", async () => {
    const elsewhere = await temporaryRoot();
    writeCandidateFile(elsewhere, "retries-3f9a1c.json", serializeCandidate(candidate("retries-3f9a1c")));

    const linkedRerail = await temporaryRoot();
    symlinkSync(join(elsewhere, ".rerail"), join(linkedRerail, ".rerail"));
    expect(await readWorktreeCandidates(linkedRerail)).toEqual({
      candidates: [],
      problems: [".rerail: not a folder (a symbolic link, which Rerail does not follow)"],
      truncated: false,
    });
    await expect(writeWorktreeCandidate(linkedRerail, candidate("another-0a0a0a"))).rejects.toThrow(/\.rerail is not a folder \(a symbolic link/u);

    const linkedFolder = await temporaryRoot();
    mkdirSync(join(linkedFolder, ".rerail"));
    symlinkSync(join(elsewhere, CANDIDATE_DIRECTORY), join(linkedFolder, CANDIDATE_DIRECTORY));
    expect((await readWorktreeCandidates(linkedFolder)).problems).toEqual([".rerail/candidates: not a folder (a symbolic link, which Rerail does not follow)"]);
    await expect(writeWorktreeCandidate(linkedFolder, candidate("another-0a0a0a"))).rejects.toThrow(/\.rerail\/candidates is not a folder/u);

    const linkedFile = await temporaryRoot();
    mkdirSync(join(linkedFile, CANDIDATE_DIRECTORY), { recursive: true });
    symlinkSync(join(elsewhere, CANDIDATE_DIRECTORY, "retries-3f9a1c.json"), join(linkedFile, CANDIDATE_DIRECTORY, "retries-3f9a1c.json"));
    expect(await readWorktreeCandidates(linkedFile)).toEqual({ candidates: [], problems: ["retries-3f9a1c.json: not a regular file"], truncated: false });
  });

  it("reports an oversize file and files beyond the limit without reading them, and an invalid one with its reason", async () => {
    const root = await temporaryRoot();
    writeCandidateFile(root, "aaa-big.json", `${JSON.stringify(candidate("aaa-big"))}${" ".repeat(MAX_CANDIDATE_BYTES)}`);
    writeCandidateFile(root, "aab-invalid.json", JSON.stringify({ ...candidate("aab-invalid"), scope: ["../outside/**"] }));
    writeCandidateFile(root, "aac-not-json.json", "{ nope");
    writeCandidateFile(root, "aad-renamed.json", serializeCandidate(candidate("something-else")));
    // Names Rerail does not read are left alone.
    writeCandidateFile(root, "README.md", "notes\n");
    writeCandidateFile(root, "UPPER.json", serializeCandidate(candidate("upper")));
    for (let index = 0; index < MAX_CANDIDATES_PER_BRANCH; index += 1) {
      const slug = `b-${String(index).padStart(3, "0")}`;
      writeCandidateFile(root, `${slug}.json`, serializeCandidate(candidate(slug)));
    }

    const set = await readWorktreeCandidates(root);

    expect(set.truncated).toBe(true);
    expect(set.candidates.map(entry => entry.id)).toEqual(
      Array.from({ length: MAX_CANDIDATES_PER_BRANCH - 4 }, (_, index) => `candidate:b-${String(index).padStart(3, "0")}`),
    );
    expect(set.problems).toEqual([
      `aaa-big.json: larger than ${MAX_CANDIDATE_BYTES} bytes`,
      "aab-invalid.json: scope.0: a scope pattern stays inside the repository",
      "aac-not-json.json: not JSON",
      "aad-renamed.json: its id candidate:something-else belongs in something-else.json",
      `${CANDIDATE_DIRECTORY}: 4 more candidate files not read; a branch holds at most ${MAX_CANDIDATES_PER_BRANCH}`,
    ]);
  });

  it("refuses to write a candidate that would not read back", async () => {
    const root = await temporaryRoot();
    await expect(writeWorktreeCandidate(root, { ...candidate("bad-scope"), scope: ["/etc/**"] })).rejects.toThrow(/was not written: .*relative to the repository/u);
  });
});

describe("readTreeCandidates", () => {
  it("reads the candidates in a commit's tree, regular files only, and reports an unknown commit", async () => {
    const root = await repository();
    await writeWorktreeCandidate(root, candidate("retries-3f9a1c"));
    writeCandidateFile(root, "big.json", " ".repeat(MAX_CANDIDATE_BYTES + 1));
    writeCandidateFile(root, "bad.json", "{}");
    symlinkSync("retries-3f9a1c.json", join(root, CANDIDATE_DIRECTORY, "link.json"));
    mkdirSync(join(root, CANDIDATE_DIRECTORY, "nested.json"));
    writeFileSync(join(root, CANDIDATE_DIRECTORY, "nested.json", "inner.json"), serializeCandidate(candidate("inner")));
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "candidates");
    const commit = git(root, "rev-parse", "HEAD").trim();
    // A later working-tree change is not in the commit.
    await writeWorktreeCandidate(root, candidate("later-000000"));

    const set = await readTreeCandidates(root, commit);

    expect(set.candidates).toEqual([candidate("retries-3f9a1c")]);
    expect(set.truncated).toBe(false);
    expect(set.problems).toEqual([
      expect.stringMatching(/^bad\.json: /u),
      `big.json: larger than ${MAX_CANDIDATE_BYTES} bytes`,
      "link.json: not a regular file",
      "nested.json: not a regular file",
    ]);
    await expect(readTreeCandidates(root, "main")).resolves.toEqual(set);
    await expect(readTreeCandidates(root, "0".repeat(40))).resolves.toEqual({ candidates: [], problems: [`${"0".repeat(40)}: not a commit in this repository`], truncated: false });
    expect((await readTreeCandidates(root, "--output=x")).problems).toEqual(["--output=x: not a commit in this repository"]);
  });

  it("reads many candidates in one pass and keeps problems in name order", async () => {
    const root = await repository();
    const slugs = Array.from({ length: 20 }, (_, index) => `decision-${String(index).padStart(2, "0")}`);
    for (const slug of slugs) await writeWorktreeCandidate(root, candidate(slug, `Decision number ${slug}.`));
    writeCandidateFile(root, "decision-05a.json", "not json");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "candidates");

    const set = await readTreeCandidates(root, "HEAD");

    expect(set.candidates.map(found => found.id)).toEqual(slugs.map(slug => `candidate:${slug}`));
    expect(set.problems).toEqual(["decision-05a.json: not JSON"]);
  });

  it("finds nothing behind a symlinked candidates folder in the tree", async () => {
    const root = await repository();
    writeCandidateFile(join(root, "real"), "retries-3f9a1c.json", serializeCandidate(candidate("retries-3f9a1c")));
    mkdirSync(join(root, ".rerail"));
    symlinkSync("../real/.rerail/candidates", join(root, CANDIDATE_DIRECTORY));
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "linked");
    await expect(readTreeCandidates(root, "HEAD")).resolves.toEqual({ candidates: [], problems: [], truncated: false });
  });
});

describe("candidatesIgnored and committedCandidates", () => {
  it("tells whether Git ignores the candidates folder", async () => {
    const root = await repository();
    expect(candidatesIgnored(root)).toBe(false);
    writeFileSync(join(root, ".gitignore"), ".rerail/\n");
    expect(candidatesIgnored(root)).toBe(true);
  });

  it("holds an id only while its working file matches HEAD's, for all ids at once", async () => {
    const root = await repository();
    const first = candidate("retries-3f9a1c");
    const second = candidate("errors-9b8c7d", "Errors carry a code.");
    await writeWorktreeCandidate(root, first);
    expect(committedCandidates(root, [first.id])).toEqual(new Set());

    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "candidate");
    await writeWorktreeCandidate(root, second);
    expect(committedCandidates(root, [first.id, second.id, "candidate:never-written"])).toEqual(new Set([first.id]));

    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "second");
    await writeWorktreeCandidate(root, { ...first, state: "rejected" });
    expect(committedCandidates(root, [first.id, second.id])).toEqual(new Set([second.id]));
    expect(committedCandidates(root, [])).toEqual(new Set());
  });

  it("holds nothing before the first commit", async () => {
    const root = await temporaryRoot();
    git(root, "init", "--quiet", "-b", "main");
    const first = candidate("retries-3f9a1c");
    await writeWorktreeCandidate(root, first);
    expect(committedCandidates(root, [first.id])).toEqual(new Set());
  });
});
