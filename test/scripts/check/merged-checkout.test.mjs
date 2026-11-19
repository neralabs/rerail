// scripts/check/merged-checkout.mjs: the pre-commit checks run on the working tree merged into the local origin/main,
// as CI runs them on the pull request merged into main.

import { execFileSync, spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { prepareMergedCheckout, removeMergedCheckout } from "../../../scripts/check/merged-checkout.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

/** Git with a fixed identity and none of the machine's configuration. */
const ENV = {
  ...process.env,
  GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_AUTHOR_NAME: "T", GIT_AUTHOR_EMAIL: "t@example.invalid", GIT_COMMITTER_NAME: "T", GIT_COMMITTER_EMAIL: "t@example.invalid",
};
delete ENV.GIT_INDEX_FILE;

const git = (cwd, ...args) => execFileSync("git", args, { cwd, env: ENV, encoding: "utf8" }).trim();
const SCRIPT = fileURLToPath(new URL("../../../scripts/check/merged-checkout.mjs", import.meta.url));

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

/** A checkout on `feature`, branched from `main`, whose `origin/main` is `main`. */
function repository() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-merged-checkout-")));
  roots.push(root);
  git(root, "init", "-q", "-b", "main");
  writeFileSync(join(root, "a.txt"), "one\ntwo\nthree\n");
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "base");
  git(root, "update-ref", "refs/remotes/origin/main", "main");
  git(root, "switch", "-q", "-c", "feature");
  return root;
}

/** An empty folder to stand for the temporary folder, so what is left in it is what a run left. */
function temporaryFolder() {
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "rerail-merged-checkout-tmp-")));
  roots.push(folder);
  return folder;
}

/** `origin/main` gains `file` with `text`, as a fetch after main moved on would leave it. */
function mainGains(root, file, text) {
  git(root, "switch", "-q", "main");
  writeFileSync(join(root, file), text);
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", `main adds ${file}`);
  git(root, "update-ref", "refs/remotes/origin/main", "main");
  git(root, "switch", "-q", "feature");
}

const options = { env: ENV };

describe("the checkout the pre-commit checks run in", () => {
  it("is the checkout itself while it holds origin/main", () => {
    const root = repository();
    expect(prepareMergedCheckout(root, options)).toEqual({ checkout: root, throwaway: false });
  });

  it("is the checkout itself, said, without an origin/main", () => {
    const root = repository();
    git(root, "update-ref", "-d", "refs/remotes/origin/main");
    expect(prepareMergedCheckout(root, options)).toMatchObject({ checkout: root, throwaway: false, note: expect.stringMatching(/no origin\/main/u) });
  });

  it("holds main's newer files with the branch's committed, staged, unstaged and untracked work, and leaves the checkout as it was", () => {
    const root = repository();
    mainGains(root, "main.txt", "from main\n");
    writeFileSync(join(root, "committed.txt"), "committed\n");
    git(root, "add", "-A");
    git(root, "commit", "-q", "-m", "work");
    writeFileSync(join(root, "staged.txt"), "staged\n");
    git(root, "add", "staged.txt");
    writeFileSync(join(root, "a.txt"), "one\ntwo\nthree, changed\n");
    writeFileSync(join(root, "untracked.txt"), "untracked\n");
    const status = git(root, "status", "--porcelain");

    const { checkout, throwaway } = prepareMergedCheckout(root, options);

    expect(throwaway).toBe(true);
    for (const [file, text] of [["main.txt", "from main\n"], ["committed.txt", "committed\n"], ["staged.txt", "staged\n"], ["untracked.txt", "untracked\n"], ["a.txt", "one\ntwo\nthree, changed\n"]]) {
      expect(readFileSync(join(checkout, file), "utf8")).toBe(text);
    }
    expect(git(root, "status", "--porcelain")).toBe(status);
    expect(git(root, "rev-parse", "--abbrev-ref", "HEAD")).toBe("feature");

    removeMergedCheckout(root, checkout, options);
    expect(existsSync(checkout)).toBe(false);
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(1);
  });

  it("takes what is being committed from the index a hook is handed", () => {
    const root = repository();
    mainGains(root, "main.txt", "from main\n");
    writeFileSync(join(root, ".gitignore"), "*.log\n");
    writeFileSync(join(root, "kept.log"), "kept\n");
    // Another index, as `git commit -a` hands one, staging an ignored file the working tree alone would leave out.
    const hookEnv = { ...ENV, GIT_INDEX_FILE: ".git/hook-index" };
    copyFileSync(join(root, ".git", "index"), join(root, ".git", "hook-index"));
    execFileSync("git", ["add", "--force", "kept.log"], { cwd: root, env: hookEnv });

    const { checkout } = prepareMergedCheckout(root, { env: hookEnv });

    expect(readFileSync(join(checkout, "kept.log"), "utf8")).toBe("kept\n");
    expect(git(root, "diff", "--cached", "--name-only")).toBe("");
    removeMergedCheckout(root, checkout, options);
  });

  it("is the checkout itself, naming the files, when the work conflicts with origin/main", () => {
    const root = repository();
    mainGains(root, "a.txt", "one\ntwo\nmain's three\n");
    writeFileSync(join(root, "a.txt"), "one\ntwo\nfeature's three\n");

    expect(prepareMergedCheckout(root, options)).toMatchObject({
      checkout: root, throwaway: false, note: expect.stringMatching(/conflicts with origin\/main in a\.txt/u),
    });
  });

  it("is the checkout itself, saying why, when git merge-tree fails other than on a conflict", () => {
    const root = repository();
    git(root, "update-ref", "refs/remotes/origin/main", git(root, "commit-tree", "HEAD^{tree}", "-m", "unrelated"));
    const tmp = temporaryFolder();

    const run = spawnSync("node", [SCRIPT, "prepare"], { cwd: root, env: { ...ENV, TMPDIR: tmp }, encoding: "utf8" });

    expect(run.status, run.stderr).toBe(0);
    // The root as Git prints it (`D:/a/b` on Windows), which pre-commit.sh compares with Git's own.
    expect(resolve(run.stdout.trim())).toBe(resolve(root));
    expect(run.stderr.trim()).toMatch(
      /^pre-commit: could not merge your work with origin\/main \(git merge-tree: fatal: refusing to merge unrelated histories; it needs Git 2\.38 or later\), so your branch is checked alone$/u,
    );
    expect(readdirSync(tmp)).toEqual([]);
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(1);
  });

  it("removes the worktree it made when a later step fails", () => {
    const root = repository();
    mainGains(root, "main.txt", "from main\n");
    // A node_modules folder in the work leaves no room for the link to this checkout's.
    mkdirSync(join(root, "node_modules"));
    writeFileSync(join(root, "node_modules", "x.txt"), "x\n");

    expect(() => prepareMergedCheckout(root, options)).toThrow(/EEXIST/u);
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(1);
  });

  it("removes nothing it did not make", () => {
    const root = repository();
    removeMergedCheckout(root, root, options);
    expect(existsSync(join(root, "a.txt"))).toBe(true);
  });
});

/**
 * A stub on PATH: `npm run --silent check` lists what Git sees, and every other check passes (`just`, and `python3`
 * for the engine manifests' feature check, which this repository has none of).
 */
const STUBS = { npm: '#!/bin/sh\n[ "$3" != check ] || exec git ls-files\n', just: "#!/bin/sh\nexit 0\n", python3: "#!/bin/sh\nexit 0\n" };

/** `root` with the real pre-commit scripts committed on `feature`, and `main.txt` on `origin/main` only. */
function hookRepository() {
  const root = repository();
  mkdirSync(join(root, "scripts", "check"), { recursive: true });
  mkdirSync(join(root, "scripts", "release"));
  for (const script of ["pre-commit.sh", "merged-checkout.mjs"]) {
    copyFileSync(fileURLToPath(new URL(`../../../scripts/check/${script}`, import.meta.url)), join(root, "scripts", "check", script));
  }
  writeFileSync(join(root, "scripts", "release", "core-changelog-check.mjs"), "");
  writeFileSync(join(root, "branch.txt"), "branch\n");
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "hook");
  mainGains(root, "main.txt", "from main\n");
  const bin = join(root, ".git", "stub-bin");
  mkdirSync(bin);
  for (const [name, text] of Object.entries(STUBS)) writeFileSync(join(bin, name), text, { mode: 0o755 });
  return { root, path: `${bin}:${process.env.PATH}` };
}

/** Runs scripts/check/pre-commit.sh in `cwd` with `env` as Git hands it to a hook, returning what the checks listed. */
function runHook(cwd, env) {
  const run = spawnSync("sh", ["scripts/check/pre-commit.sh"], { cwd, env, encoding: "utf8" });
  expect(run.status, run.stderr).toBe(0);
  return run.stdout.split("\n").filter(Boolean);
}

describe.skipIf(process.platform === "win32")("the pre-commit hook", () => {
  it("checks the merged tree from a clone's own checkout, given the hook's relative index", { timeout: 30_000 }, () => {
    const { root, path } = hookRepository();

    const listed = runHook(root, { ...ENV, PATH: path, GIT_INDEX_FILE: ".git/index" });

    expect(listed).toEqual(expect.arrayContaining(["a.txt", "branch.txt", "main.txt"]));
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(1);
  });

  // A stub git, ahead of the real one on PATH, that stops for a Ctrl-C at one point of `prepare`.
  it.each([
    ["merging", '*" merge-tree "*) : >"$MARKER"; sleep 30 ;;'],
    ["with the worktree made", '*" worktree add "*) "$REAL" "$@"; : >"$MARKER"; sleep 30 ;;'],
  ])("leaves no folder or worktree behind when stopped %s", { timeout: 30_000 }, async (_phase, stop) => {
    const { root, path } = hookRepository();
    const tmp = temporaryFolder();
    const marker = join(root, ".git", "stopped-here");
    const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
    const bin = join(root, ".git", "stub-git");
    mkdirSync(bin);
    writeFileSync(join(bin, "git"), `#!/bin/sh\ncase " $* " in\n  ${stop}\nesac\nexec "$REAL" "$@"\n`, { mode: 0o755 });

    const hook = spawn("sh", ["scripts/check/pre-commit.sh"], {
      cwd: root, env: { ...ENV, PATH: `${bin}:${path}`, TMPDIR: tmp, REAL: real, MARKER: marker }, detached: true, stdio: "ignore",
    });
    const exited = once(hook, "exit");
    try {
      while (!existsSync(marker) && hook.exitCode === null) await new Promise((wake) => setTimeout(wake, 50));
      expect(existsSync(marker)).toBe(true);
      expect(readdirSync(tmp)).toEqual([expect.stringMatching(/^rerail-pre-commit-/u)]);
      // A Ctrl-C reaches every process of the terminal's foreground group: the hook, node and git.
      process.kill(-hook.pid, "SIGINT");
      expect(await exited).toEqual([130, null]);
    } finally {
      try {
        process.kill(-hook.pid, "SIGKILL");
      } catch {
        // Already gone.
      }
    }

    expect(readdirSync(tmp)).toEqual([]);
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(1);
  });

  it("checks the merged tree from a linked worktree, given the hook's GIT_DIR", { timeout: 30_000 }, () => {
    const { root, path } = hookRepository();
    const parent = realpathSync(mkdtempSync(join(tmpdir(), "rerail-merged-checkout-linked-")));
    roots.push(parent);
    const linked = join(parent, "linked");
    git(root, "worktree", "add", "-q", "-b", "linked", linked, "feature");
    const admin = join(root, ".git", "worktrees", "linked");

    const listed = runHook(linked, { ...ENV, PATH: path, GIT_DIR: admin, GIT_INDEX_FILE: join(admin, "index") });

    expect(listed).toEqual(expect.arrayContaining(["a.txt", "branch.txt", "main.txt"]));
    expect(git(root, "worktree", "list").split("\n")).toHaveLength(2);
  });
});
