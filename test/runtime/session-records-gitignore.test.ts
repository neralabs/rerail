import { execFileSync } from "node:child_process";
import { lstat, mkdir, readdir, readFile, realpath, stat, symlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { RerailRuntime } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";

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

  it("replace a .gitignore that is a symlink instead of writing through it", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    const outside = join(dirname(checkout), "outside.txt");
    await writeFile(outside, "kept\n");
    await mkdir(storageRoot, { recursive: true });
    await symlink(outside, join(storageRoot, ".gitignore"));
    await openOneSession(checkout, storageRoot, databasePath);
    expect(await readFile(outside, "utf8")).toBe("kept\n");
    expect((await lstat(join(storageRoot, ".gitignore"))).isFile()).toBe(true);
  });

  it("write the .gitignore readable by the person only, like the ledgers", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect((await stat(join(storageRoot, ".gitignore"))).mode & 0o777).toBe(0o600);
  });

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
});
