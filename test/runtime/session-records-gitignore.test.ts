import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
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

/** Each records folder whose repair a session started, and whether its .gitignore was there then. */
const repairs = vi.hoisted(() => [] as Array<{ folder: string; gitignore: boolean }>);
vi.mock("../../src/engine/storage/private-files.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/engine/storage/private-files.js")>("../../src/engine/storage/private-files.js");
  return {
    ...actual,
    keepFoundFolderPrivate: (folder: string, machine?: Parameters<typeof actual.keepFoundFolderPrivate>[1]) => {
      repairs.push({ folder, gitignore: existsSync(join(folder, ".gitignore")) });
      return actual.keepFoundFolderPrivate(folder, machine);
    },
  };
});

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
 * Whether accounts beyond the owner may read `path`, or list it when it is a folder: elsewhere than Windows, the group's
 * or others' read bit; on Windows, an entry of its ACL, read in SDDL, that lets Users (BU), Authenticated Users (AU) or
 * Everyone (WD) read it itself.
 */
async function readableByOthers(path: string): Promise<boolean> {
  if (process.platform !== "win32") return ((await stat(path)).mode & 0o044) !== 0;
  const sddl = execFileSync("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "(Get-Acl -LiteralPath $env:ACL_PATH).Sddl"], {
    encoding: "utf8", env: { ...process.env, ACL_PATH: path },
  });
  return [...sddl.matchAll(/\(A;([^;]*);([^;]*);;;(?:BU|AU|WD)\)/gu)].some(([, flags = "", rights = ""]) => !flags.includes("IO") && readsData(rights));
}

/**
 * Whether an entry's rights let its account read a file or list a folder: FILE_READ_DATA (bit 0x1), GENERIC_READ or
 * GENERIC_ALL, or a code carrying one. An entry that lets other accounts only pass through a records folder and make
 * a folder of their own there reads nothing.
 */
function readsData(rights: string): boolean {
  if (rights.startsWith("0x")) return (Number.parseInt(rights.slice(2), 16) & 0x9000_0001) !== 0;
  return (rights.match(/[A-Z]{2}/gu) ?? []).some(code => ["FA", "FR", "GA", "GR", "CC"].includes(code));
}

/**
 * Makes `file` one this account may not read, as one another account wrote private is to it: elsewhere than Windows
 * mode 000; on Windows an ACL of its own granting SYSTEM alone, so its owner keeps only the right to change the ACL.
 */
async function closeToThisAccount(file: string): Promise<void> {
  if (process.platform === "win32") execFileSync("icacls.exe", [file, "/inheritance:r", "/grant:r", "*S-1-5-18:F", "/q"]);
  else await chmod(file, 0o000);
}

/** Gives `file` back to this account, so the test's folder can be removed. */
async function openToThisAccount(file: string): Promise<void> {
  if (process.platform === "win32") execFileSync("icacls.exe", [file, "/reset", "/q"]);
  else await chmod(file, 0o600);
}

/**
 * Lets this account make a folder in `folder` but write no file there, as a records folder another account repaired
 * lets an account that could make a folder there before. On Windows an ACL of its own: this account may pass through,
 * read attributes and the ACL, and make a folder; CREATOR OWNER gives what it makes back to it; Administrators, whose
 * entry would let the test's account write, are left out. Elsewhere a folder's write bit covers files and folders
 * alike, so `inner` is made first and the folder is then mode 0555.
 */
async function letMakeFoldersOnly(folder: string, inner: string): Promise<void> {
  if (process.platform === "win32") {
    // Windows' own whoami by its path, as Git Bash puts a whoami of its own first on PATH; it prints
    // "<domain>\<name>","<SID>".
    const whoami = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "whoami.exe");
    const me = /"(S-1-[\d-]+)"\s*$/u.exec(execFileSync(whoami, ["/user", "/fo", "csv", "/nh"], { encoding: "utf8" }))?.[1] ?? "";
    execFileSync("icacls.exe", [folder, "/inheritance:r", "/grant:r", `*${me}:(X,AD,RA,RC,S)`, "*S-1-3-0:(OI)(CI)(IO)F", "*S-1-5-18:(OI)(CI)F", "/q"]);
  } else {
    await mkdir(inner, { recursive: true, mode: 0o700 });
    await chmod(folder, 0o555);
  }
}

/** Gives `folder` back to this account, so the test's folder can be removed. */
async function letWriteAgain(folder: string): Promise<void> {
  if (process.platform === "win32") execFileSync("icacls.exe", [folder, "/reset", "/q"]);
  else await chmod(folder, 0o700);
}

describe("session records in a checkout", () => {
  it("never show in git status, so git add -A cannot commit them", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect(execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: checkout, encoding: "utf8" })).toBe("");
  });

  it("write the .gitignore before the records folder's repair starts, so on Windows the ACL it passes on reaches it", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect(repairs.filter(({ folder }) => folder === storageRoot)).toEqual([{ folder: storageRoot, gitignore: true }]);
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
    // The session's own record is private; on Windows the folder's repair goes on after the session opens, and the
    // held-repair case in windows-acl.test.ts shows the record is private before it ends.
    expect(await readableByOthers(join(storageRoot, createHash("sha256").update("gitignore").digest("hex"), "events.jsonl"))).toBe(false);
    await vi.waitFor(async () => {
      expect(await readableByOthers(storageRoot)).toBe(false);
      // Elsewhere than Windows each session's folder is 0700 and no other user lists the records folder; on Windows the
      // record and the .gitignore, written before the repair starts, take the folder's new ACL.
      if (process.platform === "win32") {
        expect(await readableByOthers(earlier)).toBe(false);
        expect(await readableByOthers(join(storageRoot, ".gitignore"))).toBe(false);
      }
    }, { timeout: 30_000, interval: 200 });
    // Windows PowerShell's Get-Acl, which reads each ACL, can take seconds to start on a cold machine.
  }, 60_000);

  it("are kept where another account that records there wrote the .gitignore, which this one may not read", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    const gitignore = join(storageRoot, ".gitignore");
    await mkdir(storageRoot, { recursive: true });
    await writeFile(gitignore, "# Created by Rerail: session records stay out of Git.\n*\n");
    await closeToThisAccount(gitignore);
    try {
      await openOneSession(checkout, storageRoot, databasePath);
    } finally {
      await openToThisAccount(gitignore);
    }
    const ledger = await readFile(join(storageRoot, createHash("sha256").update("gitignore").digest("hex"), "events.jsonl"), "utf8");
    expect(ledger).toContain("Add a priority score to each todo");
  });

  it("are kept where this account may make its session's folder but no file, as in a records folder another account repaired, which writes the .gitignore", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    const folder = join(storageRoot, createHash("sha256").update("gitignore").digest("hex"));
    await mkdir(storageRoot, { recursive: true });
    await letMakeFoldersOnly(storageRoot, folder);
    try {
      await openOneSession(checkout, storageRoot, databasePath);
    } finally {
      await letWriteAgain(storageRoot);
    }
    expect(await readFile(join(folder, "events.jsonl"), "utf8")).toContain("Add a priority score to each todo");
    // The account that may list the records folder, the one whose Git could add what is in it, writes it.
    expect(existsSync(join(storageRoot, ".gitignore"))).toBe(false);
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
