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

  it("looks again above a file at a folder that became a checkout after a lookup passed it", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const file = join(root, "later", "src", "a.py");
    await mkdir(join(root, "later", "src"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgainAt(file)).toBe(false);

    const later = await repository(join(root, "later"));

    // The answers kept from the first lookup name no checkout until something drops them.
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgainAt(file)).toBe(true);
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: later, path: "src/a.py" });
    expect(owners.lookAgainAt(file)).toBe(false);
  });

  it("looks again once at a folder that gained a .git entry Git does not take for a checkout", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
    const file = join(root, "stray", "a.py");
    await mkdir(join(root, "stray"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();

    // A worktree's `.git` file whose Git folder is gone: Git answers that it is no checkout, at a Git process each time.
    await writeFile(join(root, "stray", ".git"), `gitdir: ${join(root, "gone")}\n`);

    expect(owners.lookAgainAt(file)).toBe(true);
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgainAt(file)).toBe(false);
  });
});
