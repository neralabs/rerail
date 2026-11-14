// scripts/check/merged-checkout.mjs: the pre-commit checks run on the working tree merged into the local origin/main,
// as CI runs them on the pull request merged into main.

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
    writeFileSync(join(root, "staged.txt"), "staged\n");
    git(root, "add", "staged.txt");
    const index = join(root, ".git", "index");

    const { checkout } = prepareMergedCheckout(root, { env: { ...ENV, GIT_INDEX_FILE: index } });

    expect(readFileSync(join(checkout, "staged.txt"), "utf8")).toBe("staged\n");
    expect(git(root, "diff", "--cached", "--name-only")).toBe("staged.txt");
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

  it("removes nothing it did not make", () => {
    const root = repository();
    removeMergedCheckout(root, root, options);
    expect(existsSync(join(root, "a.txt"))).toBe(true);
  });
});
