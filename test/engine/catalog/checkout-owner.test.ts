import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCatalogs } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { pruneCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";

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
  it("drops a pruned plain root and keeps a second registration in the same resolver", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-plain-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
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
    expect(pruneCheckouts(catalogs.database)).toEqual([first]);
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
    cleanups.push(() => rm(root, { recursive: true, force: true }));
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
});
