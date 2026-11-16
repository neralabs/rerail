import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
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
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  return await realpath(path);
}

describe("CheckoutOwners", () => {
  it("finds a nested checkout recorded after the folder was first resolved", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
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
    expect(owners.lookAgainAt(file)).toBe(false);

    const later = await repository(join(root, "later"));

    // The answers kept from the first lookup name no checkout until something drops them.
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgainAt(file)).toBe(true);
    expect(owners.ownerOf(file)).toEqual({ checkoutRoot: later, path: "src/a.py" });
    expect(owners.lookAgainAt(file)).toBe(false);
  });

  it("finds the checkout a folder became after a lookup passed it once a lookup starts at that folder", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => removeFolder(root));
    const file = join(root, "later", "src", "a.py");
    await mkdir(join(root, "later", "src"), { recursive: true });
    await writeFile(file, "x = 1\n");
    const owners = new CheckoutOwners();
    expect(owners.ownerOf(file)).toBeUndefined();

    const later = await repository(join(root, "later"));

    // A walk asks for each folder before the files in it, so the folder's own lookup is where it finds the checkout.
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

    expect(owners.lookAgainAt(file)).toBe(true);
    expect(owners.ownerOf(file)).toBeUndefined();
    expect(owners.lookAgainAt(file)).toBe(false);
  });
});
