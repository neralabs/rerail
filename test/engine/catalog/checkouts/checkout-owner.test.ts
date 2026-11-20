import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../../src/engine/catalog/checkout-owner.js";
import { physicalPath } from "../../../../src/engine/catalog/checkout-control.js";
import { CheckoutCatalogs } from "../../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

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
  git(path, "add", "--", "README.md");
  git(path, "commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

describe("CheckoutOwners", () => {
  it("drops a forgotten plain root and keeps a second registration in the same resolver", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-plain-owner-")));
    cleanups.push(() => removeFolder(root));
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    const first = join(root, "first"), second = join(root, "second");
    for (const directory of [first, second]) {
      await mkdir(directory);
      recordCheckout(catalogs.database, directory);
    }
    const owners = new CheckoutOwners(catalogs.database);
    const owner = (root: string) => owners.ownerOf(join(root, "src", "a.py"))?.checkoutRoot;
    expect(owner(first)).toBe(first);
    expect(owner(second)).toBe(second);
    await rename(first, join(root, "moved"));
    // The catalog forgets the root (a pass keeps a plain root whose folder is gone, as it keeps a clone's only
    // checkout off disk: what forgets it is no part of this test). The owners follow the catalog's change.
    catalogs.database.$client.prepare("DELETE FROM checkouts WHERE checkout_path = ?").run(first);
    await mkdir(first);
    expect(owner(first)).toBeUndefined();
    expect(owner(second)).toBe(second);
    await rename(join(root, "moved", ".rerail"), join(first, ".rerail"));
    expect(owner(first)).toBeUndefined();
    recordCheckout(catalogs.database, first);
    expect(owner(first)).toBe(first);
  });

  it("finds a nested checkout recorded after the folder was first resolved", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const outer = await repository(join(root, "outer"));
    await mkdir(join(outer, "src"), { recursive: true });
    await writeFile(join(outer, "src", "a.py"), "x = 1\n");
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    recordCheckout(catalogs.database, outer);
    const owners = new CheckoutOwners(catalogs.database);

    expect(owners.ownerOf(join(outer, "src", "a.py"))).toEqual({ checkoutRoot: outer, path: "src/a.py" });

    // `src/` becomes its own checkout and Rerail records it (stopped or not, it now owns its files).
    const nested = await repository(join(outer, "src"));
    recordCheckout(catalogs.database, nested);

    expect(owners.ownerOf(join(outer, "src", "a.py"))).toEqual({ checkoutRoot: nested, path: "a.py" });
  });

  it("tells whether the recorded checkouts changed without preparing a statement on every lookup", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const outer = await repository(join(root, "outer"));
    const catalogs = new CheckoutCatalogs(join(root, "rerail.sqlite"));
    cleanups.push(async () => catalogs.close());
    const before = catalogs.recordedCheckoutsVersion();
    const prepare = vi.spyOn(catalogs.database.$client, "prepare");

    for (let lookup = 0; lookup < 50; lookup += 1) expect(catalogs.recordedCheckoutsVersion()).toBe(before);
    expect(prepare).not.toHaveBeenCalled();
    recordCheckout(catalogs.database, outer);
    const prepared = prepare.mock.calls.length;
    expect(catalogs.recordedCheckoutsVersion()).not.toBe(before);
    expect(prepare.mock.calls.length).toBe(prepared);
  });

  it.runIf(process.platform === "win32")("owns a file spelled with a lower-case drive letter, as VS Code spells a session's folder", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const outer = realpathSync.native(await repository(join(root, "outer")));
    await mkdir(join(outer, "src"), { recursive: true });
    await writeFile(join(outer, "src", "a.py"), "x = 1\n");
    const lower = (path: string): string => path.replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    const owners = new CheckoutOwners();

    expect(owners.ownerOf(lower(join(outer, "src", "a.py")))).toEqual({ checkoutRoot: outer, path: "src/a.py" });
    expect(owners.ownerOf(lower(outer))?.checkoutRoot).toBe(outer);
    expect(physicalPath(lower(outer).toUpperCase())).toBe(outer);
  });

  it("looks again above a file at a folder that became a checkout after a lookup passed it", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const file = join(root, "later", "src", "a.py");
    await mkdir(join(root, "later", "src"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgain()).toBe(false);

    const later = await repository(join(root, "later"));

    // The answers kept from the first lookup name no checkout until something drops them.
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgain()).toBe(true);
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: later, path: "src/a.py" });
    expect(owners.lookAgain()).toBe(false);
  });

  it("takes a folder that became a checkout after a lookup passed it only at the next point, at the folder itself too", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const file = join(root, "later", "src", "a.py");
    await mkdir(join(root, "later", "src"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();

    const later = await repository(join(root, "later"));

    // Between two points no change is taken: the folder's own lookup keeps the answer from before.
    expect(owners.ownerOf(join(root, "later"))).toBeUndefined();
    expect(owners.lookAgain()).toBe(true);
    expect(owners.ownerOf(join(root, "later"))).toEqual({ checkoutRoot: later, path: "" });
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: later, path: "src/a.py" });
  });

  it("looks again once at a folder that gained a .git entry Git does not take for a checkout", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const file = join(root, "stray", "a.py");
    await mkdir(join(root, "stray"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();

    // A worktree's `.git` file whose Git folder is gone: Git answers that it is no checkout, at a Git process each time.
    await writeFile(join(root, "stray", ".git"), `gitdir: ${join(root, "gone")}\n`);

    expect(owners.lookAgain()).toBe(true);
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgain()).toBe(false);
  });

  it("owns a worktree's files once Git finishes it, after a lookup landed between its .git file and its HEAD", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const main = await repository(join(root, "main"));
    git(main, "worktree", "add", "--quiet", "-b", "made", join(root, "made"));
    const made = await realpath(join(root, "made"));
    const file = join(made, "README.md");
    // `git worktree add` writes the worktree's `.git` file before the HEAD in the Git folder it names.
    const head = join(main, ".git", "worktrees", "made", "HEAD");
    const contents = await readFile(head, "utf8");
    await rm(head);
    const owners = new CheckoutOwners();

    // A walk's lookup of the folder lands in between: Git does not take it for a checkout yet.
    expect(owners.ownerOf(made)).toBeUndefined();
    expect(owners.lookAgain()).toBe(false);
    expect(owners.ownerOf(file)).toBeUndefined();

    await writeFile(head, contents);

    // The next Write in it looks again and finds the worktree.
    expect(owners.lookAgain()).toBe(true);
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: made, path: "README.md" });
    expect(owners.lookAgain()).toBe(false);
  });
});
