import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, realpath, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import type { AuditRecorder } from "../../../src/engine/audit/audit.js";
import { nativeGitAccess } from "../../../src/engine/native/git-access.js";
import { GitIgnoreMatchers } from "../../../src/engine/native/git-ignore-matchers.js";
import type { NativeGitIgnoreMatcher } from "../../../src/engine/native/native.js";
import { setUpTestRepository } from "../../support/repository-rules.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A committed repository at `root`/session with an `api` folder, recorded in a database beside it. */
async function checkout(): Promise<{ root: string; session: string; databasePath: string }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-ignore-answers-")));
  cleanups.push(() => removeFolder(root));
  const session = join(root, "session");
  await mkdir(join(session, "api", "inner"), { recursive: true });
  await writeFile(join(session, "README.md"), "seed\n");
  git(session, "init", "--quiet");
  git(session, "add", "--", "README.md");
  git(session, "commit", "--quiet", "-m", "seed");
  return { root, session: await realpath(session), databasePath: join(root, "rerail.sqlite") };
}

/** A rule that forbids `NEEDLE` in every Python file, so the checkout's lanes answer for it. */
function needle(): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "block",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  });
}

const recorder: AuditRecorder = { start: () => "0", finish: () => undefined, progress: () => undefined, interrupt: () => undefined };

/** A session's router in `session`, which follows `.gitignore` unless told not to, sharing `matchers` as the daemon's sessions do. */
async function opened(session: string, databasePath: string, matchers: GitIgnoreMatchers, followGitignore = true): Promise<CheckoutCodeRuleRouter> {
  const owners = new CheckoutOwners();
  const catalogs = new CheckoutCatalogs(databasePath, {});
  const auditor = new CheckoutCodeRuleRouter({
    sessionRoot: session,
    startingCheckout: startingCheckout(owners, session).checkoutRoot,
    owners,
    catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: recorder,
    followGitignore,
    gitIgnoreMatchers: matchers,
  });
  cleanups.push(async () => {
    await auditor.close();
    catalogs.close();
  });
  await auditor.openWorkspace(`session-${cleanups.length}`);
  return auditor;
}

/**
 * Each question put to the engine's Git ignore matchers from now on, as "folder <path>" or "file <path>": every matcher
 * the process's owner of Git opens is wrapped to count them, until the case ends.
 */
function countedIgnoreQuestions(): string[] {
  const asked: string[] = [];
  const access = nativeGitAccess();
  const open = access.openGitIgnoreMatcher;
  const counted = (matcher: NativeGitIgnoreMatcher): NativeGitIgnoreMatcher => ({
    ignored: (path, directory) => {
      asked.push(`${directory ? "folder" : "file"} ${path}`);
      return matcher.ignored(path, directory);
    },
    ignoredBy: (path, directory) => matcher.ignoredBy(path, directory),
    sourcesUnchanged: () => matcher.sourcesUnchanged(),
  });
  const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementation(root => {
    const matcher = open.call(access, root);
    return matcher === null ? null : counted(matcher);
  });
  cleanups.push(async () => { opens.mockRestore(); });
  return asked;
}

/** What one walk of `auditor` answers of each of `paths` (a folder's ends with `/`): whether Git ignores it. */
function walk(auditor: CheckoutCodeRuleRouter, ...paths: string[]): boolean[] {
  const view = auditor.walkView();
  return paths.map(path => (path.endsWith("/") ? view.ignoresPath(path.slice(0, -1), true) : view.ignoresPath(path, false)));
}

/** The four answers a walk gets of the folders `scratch` and `api` and the files `api/x.py` and `run.log`. */
const FOUR = ["scratch/", "api/", "api/x.py", "run.log"];
/**
 * The questions the engine is asked of them where no answer is kept. Before a walk's first answer to keep in a folder
 * the engine is asked about `.git`, which has it read again the `.gitignore` of each folder on the way
 * (`GitIgnoreAnswers`): once for the root's `scratch`, `api` and `run.log`, once for `api/x.py`.
 */
const FOUR_LEARNT = ["folder .git", "folder scratch", "folder api", "folder .git", "file api/x.py", "file run.log"];

/** The index file of the checkout `root` with a `.git` folder. */
const indexOf = (root: string): string => join(root, ".git", "index");

/**
 * Puts zero bytes in place of the SHA-1 checksum at the end of `root`'s index, as Git 2.40 and later write it where
 * `index.skipHash` is set; Git from 2.13 reads such an index. An older Git writes the checksum whatever the setting.
 */
function withoutChecksum(root: string): void {
  const bytes = readFileSync(indexOf(root));
  bytes.fill(0, bytes.length - 20);
  writeFileSync(indexOf(root), bytes);
}

describe("Git's ignore answers kept per checkout", () => {
  // A walk asks Git's ignore rules about every folder and file it finds, and every session walks its checkout again at
  // each turn's end: on a machine with 17 repositories and about 180 worktrees those questions held a third of the
  // daemon's busy thread. One answer serves every walk of every session until a fact ends it.
  it("asks Git's ignore rules once about each path for two sessions' two walks of a checkout that runs no rules", async () => {
    const { session, databasePath } = await checkout();
    await writeFile(join(session, ".gitignore"), "scratch/\n*.log\n");
    startTestRepository(session, databasePath);
    const asked = countedIgnoreQuestions();
    const matchers = new GitIgnoreMatchers();
    const sessions = [await opened(session, databasePath, matchers), await opened(session, databasePath, matchers)];

    for (const auditor of sessions) for (let walks = 0; walks < 2; walks += 1) expect(walk(auditor, ...FOUR)).toEqual([true, false, false, true]);
    expect(asked).toEqual(FOUR_LEARNT);
  });

  it("asks Git's ignore rules once about each path for two sessions' two walks of a checkout its lanes answer, a lane's own question included", async () => {
    const { session, databasePath } = await checkout();
    await writeFile(join(session, ".gitignore"), "scratch/\n*.log\n");
    setUpTestRepository(session, { library: [{ kind: "code", rule: needle() }] }, databasePath);
    const asked = countedIgnoreQuestions();
    const matchers = new GitIgnoreMatchers();
    const sessions = [await opened(session, databasePath, matchers), await opened(session, databasePath, matchers)];
    // Each lane reads its own checkout as it opens; the walks' questions come after.
    asked.length = 0;

    for (const auditor of sessions) for (let walks = 0; walks < 2; walks += 1) expect(walk(auditor, ...FOUR)).toEqual([true, false, false, true]);
    // A walk asks a file whether Git ignores it, then whether a lane reads it: the second question takes the first's answer.
    const view = sessions[1]!.walkView();
    expect([view.ignoresPath("api/y.py", false), view.acceptsPath("api/y.py"), view.acceptsPath("api/x.py")]).toEqual([false, true, true]);
    expect(asked).toEqual([...FOUR_LEARNT, "folder .git", "file api/y.py"]);
  });

  it("asks a linked worktree's Git ignore rules once about a file for two walks, though another checkout's lane answers whether it is read", async () => {
    const { root, session, databasePath } = await checkout();
    setUpTestRepository(session, { library: [{ kind: "code", rule: needle() }] }, databasePath);
    const worktree = join(root, "elsewhere", "w");
    git(session, "worktree", "add", "--quiet", "-b", "linked", worktree);
    const file = relative(session, join(await realpath(worktree), "api", "x.py")).split(sep).join("/");
    const asked = countedIgnoreQuestions();
    const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
    asked.length = 0;

    for (let walks = 0; walks < 2; walks += 1) {
      const view = auditor.walkView();
      expect([view.ignoresPath(file, false), view.acceptsPath(file)]).toEqual([false, true]);
    }
    expect(asked).toEqual(["folder .git", "file api/x.py"]);
  });

  describe("what ends an answer", () => {
    it("answers that Git ignores no path its index tracks since a walk learnt Git ignored it", async () => {
      const { session, databasePath } = await checkout();
      await writeFile(join(session, ".gitignore"), "*.log\n");
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "run.log")).toEqual([true]);

      await writeFile(join(session, "run.log"), "kept\n");
      git(session, "add", "--force", "--", "run.log");
      expect(walk(auditor, "run.log")).toEqual([false]);
    });

    it("answers that Git ignores a path its index no longer tracks since a walk learnt Git left it in", async () => {
      const { session, databasePath } = await checkout();
      await writeFile(join(session, ".gitignore"), "*.log\n");
      await writeFile(join(session, "run.log"), "kept\n");
      git(session, "add", "--force", "--", "run.log");
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "run.log")).toEqual([false]);

      git(session, "rm", "--cached", "--quiet", "--", "run.log");
      expect(walk(auditor, "run.log")).toEqual([true]);
    });

    // A linked worktree's index lives in the repository's Git folder, under the name its `.git` file gives.
    it("answers that Git ignores no path a linked worktree's index tracks since a walk learnt Git ignored it", async () => {
      const { root, session, databasePath } = await checkout();
      setUpTestRepository(session, { library: [{ kind: "code", rule: needle() }] }, databasePath);
      const worktree = join(root, "elsewhere", "w");
      git(session, "worktree", "add", "--quiet", "-b", "linked", worktree);
      await writeFile(join(worktree, ".gitignore"), "*.log\n");
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      const log = relative(session, join(await realpath(worktree), "run.log")).split(sep).join("/");
      expect(walk(auditor, log)).toEqual([true]);

      await writeFile(join(worktree, "run.log"), "kept\n");
      git(worktree, "add", "--force", "--", "run.log");
      expect(walk(auditor, log)).toEqual([false]);
    });

    // Git's checksum at the index's end tells one index from another; the file's times say nothing a walk needs.
    it("keeps the answers while the index holds the bytes a walk learnt them under, though its file's times moved", async () => {
      const { session, databasePath } = await checkout();
      await writeFile(join(session, ".gitignore"), "*.log\n");
      startTestRepository(session, databasePath);
      const asked = countedIgnoreQuestions();
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "run.log", "api/x.py")).toEqual([true, false]);
      const { mtime } = await stat(indexOf(session));
      const later = new Date(mtime.getTime() + 10_000);
      await utimes(indexOf(session), later, later);
      asked.length = 0;

      expect([walk(auditor, "run.log", "api/x.py"), asked]).toEqual([[true, false], []]);
    });

    // `index.skipHash`, which `feature.manyFiles` turns on, has Git write zero bytes where its checksum would be.
    it("answers by the files an index with no checksum tracks now, though it kept its size", async () => {
      const { session, databasePath } = await checkout();
      git(session, "config", "index.skipHash", "true");
      await writeFile(join(session, ".gitignore"), "*.log\n");
      await writeFile(join(session, "a.log"), "kept\n");
      await writeFile(join(session, "b.log"), "kept\n");
      git(session, "add", "--force", "--", ".gitignore", "a.log");
      withoutChecksum(session);
      const size = (await stat(indexOf(session))).size;
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "a.log", "b.log")).toEqual([false, true]);

      git(session, "rm", "--cached", "--quiet", "--", "a.log");
      git(session, "add", "--force", "--", "b.log");
      withoutChecksum(session);
      expect((await stat(indexOf(session))).size).toBe(size);
      expect(walk(auditor, "a.log", "b.log")).toEqual([true, false]);
    });

    // The engine reads a folder's `.gitignore` as a question enters the folder, and keeps it while questions stay there.
    it("asks Git's ignore rules again about a folder's files once a .gitignore appeared there or above since a walk learnt them", async () => {
      const { session, databasePath } = await checkout();
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      const paths = ["api/", "api/inner/", "api/inner/z.py", "api/x.py", "api/y.py"];
      expect(walk(auditor, ...paths)).toEqual([false, false, false, false, false]);

      await writeFile(join(session, "api", ".gitignore"), "x.py\ninner/z.py\n");
      expect(walk(auditor, ...paths)).toEqual([false, false, true, true, false]);
    });

    // A session that starts below its checkout's root walks from its own folder, and asks about no folder above it.
    it("asks Git's ignore rules again about a session's files once a .gitignore above the session's folder changed", async () => {
      const { session, databasePath } = await checkout();
      const below = join(session, "pkg", "app");
      await mkdir(below, { recursive: true });
      startTestRepository(session, databasePath);
      const auditor = await opened(await realpath(below), databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "x.py")).toEqual([false]);

      await writeFile(join(session, "pkg", ".gitignore"), "app/x.py\n");
      expect(walk(auditor, "x.py")).toEqual([true]);
    });

    it("asks Git's ignore rules again about a folder's files once a .gitignore went from there or changed below it", async () => {
      const { session, databasePath } = await checkout();
      await writeFile(join(session, "api", ".gitignore"), "x.py\n");
      await writeFile(join(session, "api", "inner", ".gitignore"), "z.py\n");
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      const paths = ["README.md", "api/", "api/x.py", "api/inner/", "api/inner/z.py", "api/inner/w.py"];
      expect(walk(auditor, ...paths)).toEqual([false, false, true, false, true, false]);

      await rm(join(session, "api", ".gitignore"));
      await writeFile(join(session, "api", "inner", ".gitignore"), "w.py\n");
      expect(walk(auditor, ...paths)).toEqual([false, false, false, false, false, true]);
    });

    // The engine reads a rules file by its text: a rewrite at the same size within one tick of the file clock is a
    // change, and a touch is none.
    it("keeps the answers a walk learnt of a folder's files until its .gitignore's text changes", async () => {
      const { session, databasePath } = await checkout();
      const rules = join(session, "api", ".gitignore");
      await writeFile(rules, "x.py\n");
      startTestRepository(session, databasePath);
      const asked = countedIgnoreQuestions();
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(walk(auditor, "README.md", "api/", "api/x.py")).toEqual([false, false, true]);
      const { mtime } = await stat(rules);
      const later = new Date(mtime.getTime() + 10_000);
      await utimes(rules, later, later);
      asked.length = 0;
      expect([walk(auditor, "README.md", "api/", "api/x.py"), asked]).toEqual([[false, false, true], []]);

      await writeFile(rules, "y.py\n");
      await utimes(rules, later, later);
      expect(walk(auditor, "README.md", "api/", "api/x.py")).toEqual([false, false, false]);
    });

    // The engine reads a folder's `.gitignore` as a question enters the folder: a question outside the walks (a tool
    // call's file) can leave it resting in a folder whose `.gitignore` changes after.
    it("answers by a folder's .gitignore as it is when a walk learns there, though a question outside the walks left the engine in that folder", async () => {
      const { session, databasePath } = await checkout();
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      expect(auditor.ignoresPath("api/x.py", false)).toBe(false);

      await writeFile(join(session, "api", ".gitignore"), "x.py\n");
      expect([walk(auditor, "api/x.py", "README.md"), walk(auditor, "api/x.py")]).toEqual([[true, false], [true]]);
    });

    // A walk learns as it goes: a question outside the walks can take the engine into a folder the walk has not reached
    // yet, whose `.gitignore` then changes before the walk reads it.
    it("answers by a folder's .gitignore as the walk reads it, though a question outside the walks entered the folder during the walk", async () => {
      const { session, databasePath } = await checkout();
      startTestRepository(session, databasePath);
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      const view = auditor.walkView();
      expect(view.ignoresPath("README.md", false)).toBe(false);
      expect(auditor.ignoresPath("api/x.py", false)).toBe(false);

      await writeFile(join(session, "api", ".gitignore"), "x.py\n");
      expect([view.ignoresPath("api/x.py", false), walk(auditor, "README.md", "api/x.py")]).toEqual([true, [false, true]]);
    });

    // The engine reads the root `.gitignore`, `info/exclude` and the excludes file again only with a new index, and a
    // session that asks for the checkout's matcher once one changed gets a new matcher, which keeps answers of its own.
    const sources: Array<[string, (session: string, root: string) => Promise<string>]> = [
      ["the root .gitignore", async session => join(session, ".gitignore")],
      ["info/exclude", async session => {
        await mkdir(join(session, ".git", "info"), { recursive: true });
        return join(session, ".git", "info", "exclude");
      }],
      ["core.excludesFile", async (session, root) => {
        const excludes = join(root, "excludes");
        await writeFile(excludes, "");
        git(session, "config", "core.excludesFile", excludes.split("\\").join("/"));
        return excludes;
      }],
    ];
    for (const [name, place] of sources) {
      it(`answers a session that opens once ${name} changed by the new rules, while an earlier session's walks keep its matcher's answers until the index changes`, async () => {
        const { root, session, databasePath } = await checkout();
        const rules = await place(session, root);
        startTestRepository(session, databasePath);
        const matchers = new GitIgnoreMatchers();
        const earlier = await opened(session, databasePath, matchers);
        expect(walk(earlier, "run.log")).toEqual([false]);

        await writeFile(rules, "*.log\n");
        const later = await opened(session, databasePath, matchers);
        expect([walk(later, "run.log"), walk(earlier, "run.log")]).toEqual([[true], [false]]);
        await writeFile(join(session, "README.md"), "changed\n");
        git(session, "add", "--", "README.md");
        expect(walk(earlier, "run.log")).toEqual([true]);
      });

      it(`asks Git's ignore rules at each walk once ${name} changed with a folder's .gitignore, until the index changes`, async () => {
        const { root, session, databasePath } = await checkout();
        const rules = await place(session, root);
        startTestRepository(session, databasePath);
        const asked = countedIgnoreQuestions();
        const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
        expect(walk(auditor, "api/", "api/x.py")).toEqual([false, false]);
        await writeFile(rules, "*.tmp\n");
        await writeFile(join(session, "api", ".gitignore"), "x.py\n");
        walk(auditor, "api/", "api/x.py");
        asked.length = 0;
        walk(auditor, "api/", "api/x.py");
        walk(auditor, "api/", "api/x.py");
        expect(asked).toEqual(["folder api", "file api/x.py", "folder api", "file api/x.py"]);

        await writeFile(join(session, "api", "y.py"), "y\n");
        git(session, "add", "--", "api/y.py");
        asked.length = 0;
        expect([walk(auditor, "api/", "api/x.py"), walk(auditor, "api/", "api/x.py")]).toEqual([[false, true], [false, true]]);
        expect(asked).toEqual(["folder .git", "folder api", "folder .git", "file api/x.py"]);
      });
    }

    // A worktree removed and added again at its place has a new index, whatever its rules are.
    it("answers a worktree added again at the place of one removed by the rules it holds now", async () => {
      const { root, session, databasePath } = await checkout();
      setUpTestRepository(session, { library: [{ kind: "code", rule: needle() }] }, databasePath);
      const worktree = join(root, "elsewhere", "w");
      git(session, "worktree", "add", "--quiet", "-b", "logs-ignored", worktree);
      await writeFile(join(worktree, ".gitignore"), "*.log\n");
      git(worktree, "add", "--", ".gitignore");
      git(worktree, "commit", "--quiet", "-m", "logs ignored");
      const auditor = await opened(session, databasePath, new GitIgnoreMatchers());
      const log = relative(session, join(await realpath(worktree), "run.log")).split(sep).join("/");
      expect(walk(auditor, log)).toEqual([true]);

      git(session, "worktree", "remove", "--force", worktree);
      git(session, "worktree", "add", "--quiet", "-b", "logs-kept", worktree, "HEAD");
      expect(walk(auditor, log)).toEqual([false]);
    });

    // A session that does not follow `.gitignore` asks nothing of the answers other sessions keep, and one that follows
    // it again takes them only as each walk's checks leave them.
    it("answers no path Git ignores for a session that does not follow .gitignore, and the rules as they are now for one that follows it again", async () => {
      const { session, databasePath } = await checkout();
      await writeFile(join(session, ".gitignore"), "*.log\n");
      startTestRepository(session, databasePath);
      const asked = countedIgnoreQuestions();
      const matchers = new GitIgnoreMatchers();
      const following = await opened(session, databasePath, matchers);
      expect(walk(following, "run.log", "api/", "api/x.py")).toEqual([true, false, false]);
      asked.length = 0;

      const off = await opened(session, databasePath, matchers, false);
      expect([walk(off, "run.log", "api/", "api/x.py"), asked]).toEqual([[false, false, false], []]);
      await writeFile(join(session, "api", ".gitignore"), "x.py\n");
      const on = await opened(session, databasePath, matchers);
      expect([walk(on, "run.log", "api/", "api/x.py"), asked]).toEqual([[true, false, true], ["folder .git", "file api/x.py"]]);
    });
  });
});
