import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, lstat, mkdir, readdir, readFile, realpath, stat, symlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { RerailRuntime } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";
import { resolveSessionPaths } from "../../src/runtime/session-paths.js";
import { NO_POSIX_MODES, POSIX_MODES } from "../support/platform/file-mode.js";
import { symlinkOrSkip } from "../support/platform/links.js";

const temporaryDirectory = temporaryDirectories("rerail-session-records-gitignore-");

/** A Git checkout with no ignores of its own, and the session storage root where hooks put it by default. */
async function checkoutWithSessionStorage(): Promise<{ checkout: string; storageRoot: string; databasePath: string }> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const checkout = join(root, "checkout");
  const databasePath = join(root, "rerail.sqlite");
  await mkdir(checkout);
  execFileSync("git", ["init", "--quiet"], { cwd: checkout });
  startTestRepository(checkout, databasePath);
  return { checkout, storageRoot: join(checkout, ".rerail", "sessions"), databasePath };
}

async function openOneSession(checkout: string, storageRoot: string, databasePath: string): Promise<void> {
  const runtime = new RerailRuntime({ storageRoot, databasePath, workspaceRoot: checkout, autostart: false, embedder: testEmbedder });
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "gitignore", cwd: checkout } });
    await runtime.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: "gitignore", text: "Add a priority score to each todo" });
  } finally {
    await runtime.dispose();
  }
}

/**
 * Lets other accounts read `folder` and what is later made in it, as Git or `mkdir` leave a folder under `C:\` or with
 * a loose umask: on Windows an entry for Users that everything in it inherits, elsewhere mode 0755. The test's user home
 * is a folder of its own, so Rerail counts a checkout here as outside the profile folder.
 */
async function openToOtherAccounts(folder: string): Promise<void> {
  if (process.platform === "win32") execFileSync("icacls.exe", [folder, "/grant", "*S-1-5-32-545:(OI)(CI)RX", "/q"]);
  else await chmod(folder, 0o755);
}

/**
 * Whether accounts beyond the owner may read `path`: elsewhere than Windows, any group or other bit; on Windows, an
 * entry of its ACL, read in SDDL, that lets Users (BU), Authenticated Users (AU) or Everyone (WD) read it itself.
 */
async function readableByOthers(path: string): Promise<boolean> {
  if (process.platform !== "win32") return ((await stat(path)).mode & 0o077) !== 0;
  const sddl = execFileSync("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "(Get-Acl -LiteralPath $env:ACL_PATH).Sddl"], {
    encoding: "utf8", env: { ...process.env, ACL_PATH: path },
  });
  return /\(A;(?![^;]*IO)[^)]*;(?:BU|AU|WD)\)/u.test(sddl);
}

describe("session records in a checkout", () => {
  it("never show in git status, so git add -A cannot commit them", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect(execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: checkout, encoding: "utf8" })).toBe("");
  });

  it("rewrite a .gitignore a repository planted in the storage root, so its !* cannot commit them", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await mkdir(storageRoot, { recursive: true });
    await writeFile(join(storageRoot, ".gitignore"), "!*\n");
    await openOneSession(checkout, storageRoot, databasePath);
    expect(await readFile(join(storageRoot, ".gitignore"), "utf8")).toBe("# Created by Rerail: session records stay out of Git.\n*\n");
    expect(execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: checkout, encoding: "utf8" })).toBe("");
  });

  it("replace a .gitignore that is a symlink instead of writing through it", async context => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    const outside = join(dirname(checkout), "outside.txt");
    await writeFile(outside, "kept\n");
    await mkdir(storageRoot, { recursive: true });
    symlinkOrSkip(context, outside, join(storageRoot, ".gitignore"));
    await openOneSession(checkout, storageRoot, databasePath);
    expect(await readFile(outside, "utf8")).toBe("kept\n");
    expect((await lstat(join(storageRoot, ".gitignore"))).isFile()).toBe(true);
  });

  it("write the .gitignore readable by the person only, like the ledgers", async ({ skip }) => {
    skip(!POSIX_MODES, NO_POSIX_MODES);
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect((await stat(join(storageRoot, ".gitignore"))).mode & 0o777).toBe(0o600);
  });

  it("are kept private once a session opens in a records folder Git, a person or an earlier release made, earlier ones included", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openToOtherAccounts(checkout);
    const earlier = join(storageRoot, "0".repeat(64), "events.jsonl");
    await mkdir(dirname(earlier), { recursive: true });
    await writeFile(earlier, "{}\n");
    // On Windows the folder takes the checkout's entry for Users, as a folder Git or mkdir makes there does; elsewhere
    // a loose umask, as Git's or a person's may be, leaves it 0755.
    if (POSIX_MODES) await openToOtherAccounts(storageRoot);
    expect(await readableByOthers(storageRoot)).toBe(true);
    await openOneSession(checkout, storageRoot, databasePath);
    // The session's own record is private at once; on Windows the folder's repair goes on after the session opens.
    expect(await readableByOthers(join(storageRoot, createHash("sha256").update("gitignore").digest("hex"), "events.jsonl"))).toBe(false);
    await vi.waitFor(async () => {
      expect(await readableByOthers(storageRoot)).toBe(false);
      // Elsewhere than Windows the folder's 0700 keeps other users from reaching a record in it; on Windows the record
      // takes the folder's new ACL.
      if (process.platform === "win32") expect(await readableByOthers(earlier)).toBe(false);
    }, { timeout: 30_000, interval: 200 });
    // Windows PowerShell's Get-Acl, which reads each ACL, can take seconds to start on a cold machine.
  }, 60_000);

  it("leave a custom storage root outside a .rerail folder without a .gitignore, since it may hold the person's files", async () => {
    const { checkout, databasePath } = await checkoutWithSessionStorage();
    const storageRoot = join(checkout, "records");
    await openOneSession(checkout, storageRoot, databasePath);
    await expect(stat(join(storageRoot, ".gitignore"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it.skipIf(process.platform === "win32").each([".rerail", ".rerail/sessions"])("refuse a %s symlink and write nothing where it leads", async (linked) => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    const elsewhere = join(dirname(checkout), "elsewhere");
    await mkdir(elsewhere);
    await mkdir(dirname(join(checkout, linked)), { recursive: true });
    await symlink(elsewhere, join(checkout, linked));
    await expect(openOneSession(checkout, storageRoot, databasePath)).rejects.toThrow(`through ${join(checkout, linked)}: it is a symbolic link`);
    expect(await readdir(elsewhere)).toEqual([]);
  });

  it("leave a custom storage root inside a .rerail folder without a .gitignore, since it is still not the default", async () => {
    const { checkout, databasePath } = await checkoutWithSessionStorage();
    const elsewhere = join(checkout, "..", "elsewhere");
    await mkdir(elsewhere);
    const storageRoot = join(elsewhere, ".rerail", "sessions");
    await openOneSession(checkout, storageRoot, databasePath);
    await expect(stat(join(storageRoot, ".gitignore"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("ignore the default root in Git when RERAIL_STORAGE_ROOT names it, relative to the checkout", async () => {
    const { checkout, storageRoot: defaultRoot, databasePath } = await checkoutWithSessionStorage();
    const { storageRoot } = resolveSessionPaths(checkout, {}, { RERAIL_STORAGE_ROOT: ".rerail/sessions" });
    expect(storageRoot).toBe(defaultRoot);
    await openOneSession(checkout, storageRoot, databasePath);
    expect(execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: checkout, encoding: "utf8" })).toBe("");
  });
});
