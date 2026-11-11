import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCatalogs } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** A committed Git repository at `path`; returns its physical path. */
async function repository(path: string): Promise<string> {
  await mkdir(path, { recursive: true });
  await writeFile(join(path, "README.md"), "seed\n");
  git(path, "init", "--quiet");
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

describe("CheckoutOwners", () => {
  it("finds a nested checkout recorded after the folder was first resolved", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const outer = await repository(join(root, "outer"));
    await mkdir(join(outer, "src"), { recursive: true });
    await writeFile(join(outer, "src", "a.py"), "x = 1\n");
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    recordCheckout(catalogs.database, outer);
    const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());

    expect(owners.ownerOf(join(outer, "src", "a.py"))).toEqual({ checkoutRoot: outer, path: "src/a.py" });

    // `src/` becomes its own checkout and Rerail records it (stopped or not, it now owns its files).
    const nested = await repository(join(outer, "src"));
    recordCheckout(catalogs.database, nested);

    expect(owners.ownerOf(join(outer, "src", "a.py"))).toEqual({ checkoutRoot: nested, path: "a.py" });
  });

  it("lets a worktree go once Git's folder for it is gone, at the next change in the recorded checkouts", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const main = await repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "--detach", join(root, "w"), "HEAD");
    const worktree = await realpath(join(root, "w"));
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    recordCheckout(catalogs.database, main);
    const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
    expect(owners.ownerOf(join(worktree, "README.md"))).toEqual({ checkoutRoot: worktree, path: "README.md" });

    // The worktree's `.git` file stays and names a Git folder that is gone, so Git no longer takes it for a checkout.
    await rm(join(main, ".git", "worktrees", "w"), { recursive: true, force: true });
    recordCheckout(catalogs.database, await repository(join(root, "other")));

    expect(owners.ownerOf(join(worktree, "README.md"))).toBeUndefined();
  });

  it("lets a worktree go once Git's folder for it is left empty, at the next change in the recorded checkouts", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const main = await repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "--detach", join(root, "w"), "HEAD");
    const worktree = await realpath(join(root, "w"));
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    recordCheckout(catalogs.database, main);
    const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
    expect(owners.ownerOf(join(worktree, "README.md"))).toEqual({ checkoutRoot: worktree, path: "README.md" });

    // The folder the worktree's `.git` file names is still there, but without a HEAD Git does not take it for a Git
    // directory, nor the worktree for a checkout.
    await emptyFolder(join(main, ".git", "worktrees", "w"));
    recordCheckout(catalogs.database, await repository(join(root, "other")));

    expect(owners.ownerOf(join(worktree, "README.md"))).toBeUndefined();
  });

  it("gives a nested checkout's files to the checkout around it once its .git folder is left empty", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const outer = await repository(join(root, "outer"));
    const nested = await repository(join(outer, "src"));
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    recordCheckout(catalogs.database, outer);
    recordCheckout(catalogs.database, nested);
    const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
    expect(owners.ownerOf(join(nested, "README.md"))).toEqual({ checkoutRoot: nested, path: "README.md" });

    // An empty `.git` folder is no Git directory: Git looks further up and finds the checkout around it.
    await emptyFolder(join(nested, ".git"));
    recordCheckout(catalogs.database, await repository(join(root, "other")));

    expect(owners.ownerOf(join(nested, "README.md"))).toEqual({ checkoutRoot: outer, path: "src/README.md" });
  });
});

/** Removes everything inside `folder` and leaves it there, empty. */
async function emptyFolder(folder: string): Promise<void> {
  await rm(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  await mkdir(folder);
}
