import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { physicalPath } from "../../../src/engine/catalog/checkout-control.js";
import { CheckoutCatalogs } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

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
});
