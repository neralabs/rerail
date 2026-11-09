import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { checkoutRealpath, physicalPath } from "../../../src/engine/catalog/checkout-control.js";
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

  it("keys checkouts through the realpath that spells names as stored on Windows", () => {
    expect(checkoutRealpath("win32")).toBe(realpathSync.native);
    expect(checkoutRealpath("darwin")).toBe(realpathSync);
    expect(checkoutRealpath("linux")).toBe(realpathSync);
  });

  it.runIf(process.platform === "win32")("owns a file spelled with a lower-case drive letter, as VS Code spells a session's folder", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-checkout-owner-")));
    cleanups.push(() => rm(root, { recursive: true, force: true }));
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
