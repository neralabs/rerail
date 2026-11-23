import { execFile, execFileSync } from "node:child_process";
import { type MakeDirectoryOptions, readFileSync, rmSync, writeFileSync } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, rename, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { withHomeLock } from "../../../src/engine/native/file-lock.js";
import { makeLedgerFolder } from "../../../src/engine/storage/ledger-paths.js";
import { keepFolderPrivate, keepFoundFolderPrivate, makeRerailHome, openToOthers } from "../../../src/engine/storage/private-files.js";
import {
  icaclsInherits, insideProfile, sddlInheritedWriters, sddlOtherReaders, sddlWriters, whoamiUserSid, WINDOWS_ACL, type WindowsAcl,
} from "../../../src/engine/storage/windows-acl.js";
import { cmdletsWithoutModule, commandScript } from "../../support/platform/powershell-commands.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const USER = "S-1-5-21-1-2-3-1001";
/** A Microsoft Entra ID (Azure AD) account's SID, as on a laptop joined to a company's Entra ID. */
const ENTRA_USER = "S-1-12-1-1111111111-2222222222-3333333333-4444444444";
/** A profile no test folder is under, so every folder here counts as outside it. */
const ELSEWHERE = String.raw`C:\Users\nobody-here`;

// Spied, each call still the real one, so a case can make the read of this user's home fail, stand in for `icacls`,
// or end a records folder's repair just as a session's folder is made.
vi.mock("node:os", { spy: true });
vi.mock("node:child_process", { spy: true });
vi.mock("node:fs/promises", { spy: true });

/** What `execFile` rejects with for an `icacls` it cut at its time limit. */
function timedOut(): Error {
  return Object.assign(new Error("Command failed: icacls.exe"), { code: null, killed: true, signal: "SIGTERM" });
}

/**
 * Changes what describes `folder` as a change of its access list does, which moves its change time and not its write
 * time: on Windows an entry of its ACL, elsewhere its mode, set to what it is. Again until its change time is no
 * longer the time an entry was last added there, which a clock that has not ticked since gives.
 */
async function changeAccess(folder: string): Promise<void> {
  for (let tries = 0; tries < 1000; tries += 1) {
    if (process.platform === "win32") execFileSync("icacls.exe", [folder, "/grant:r", `*S-1-5-11:${tries % 2 === 0 ? "(RX)" : "(R)"}`, "/q"]);
    else await chmod(folder, (await lstat(folder)).mode & 0o7777);
    const { ctimeNs, mtimeNs } = await lstat(folder, { bigint: true });
    if (ctimeNs !== mtimeNs) return;
    await new Promise(resolve => setTimeout(resolve, 1));
  }
  throw new Error(`the change time of ${folder} stayed its write time`);
}

/**
 * An ACL that records each folder it restricts or shares and each it is asked about; `inherited` is what every read
 * of inheritance answers, and `writers` the accounts every folder's ACL lets make a folder in it.
 */
function recordingAcl(inherited = true, writers: string[] = []): WindowsAcl & { restricted: Array<[string, boolean]>; read: string[]; calls: string[] } {
  const restricted: Array<[string, boolean]> = [];
  const read: string[] = [];
  const calls: string[] = [];
  return {
    restricted,
    read,
    calls,
    restrictToOwner: (path, folder) => restricted.push([path, folder]),
    restrictFolderToOwner: path => {
      restricted.push([path, true]);
      return Promise.resolve();
    },
    otherReaders: () => [],
    inherits: path => {
      read.push(path);
      return Promise.resolve(inherited);
    },
    writers: path => {
      calls.push(`writers ${path}`);
      return Promise.resolve(writers);
    },
    inheritedWriters: path => {
      calls.push(`inherited writers ${path}`);
      return Promise.resolve(writers);
    },
    keepEachOwnersOwn: (path, sharing) => {
      restricted.push([path, true]);
      calls.push(`each owner's own ${path} for ${sharing.join(" ")}`);
      return Promise.resolve();
    },
    letAddFiles: (path, sharing) => {
      calls.push(`add files ${path} for ${sharing.join(" ")}`);
      return Promise.resolve();
    },
  };
}

/** The entries of `path`'s DACL that `icacls /save` writes, as `[flags, access mask, SID or short name]`. */
function savedEntries(path: string): Array<[string, number, string]> {
  const saved = join(tmpdir(), `rerail-acl-test-${process.pid}-${String(Date.now())}.txt`);
  try {
    execFileSync("icacls.exe", [path, "/save", saved, "/q"]);
    const sddl = readFileSync(saved).toString("utf16le");
    const codes: Record<string, number> = { FA: 0x1F_01FF };
    return [...sddl.matchAll(/\(A;([A-Z]*);([^;]*);;;([^)]*)\)/gu)].map(([, flags = "", rights = "", sid = ""]) =>
      [flags, rights.startsWith("0x") ? Number.parseInt(rights.slice(2), 16) : codes[rights] ?? -1, sid]);
  } finally {
    rmSync(saved, { force: true });
  }
}

const roots: string[] = [];
async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-acl-"));
  roots.push(root);
  return root;
}

afterEach(async () => {
  vi.restoreAllMocks();
  vi.mocked(execFile).mockReset();
  vi.mocked(execFileSync).mockReset();
  vi.mocked(mkdir).mockReset();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

describe("who an ACL lets read a file", () => {
  it("names every account beyond the owner, this user, SYSTEM and Administrators that may read it", () => {
    const ownerOnly = `O:${USER}G:${USER}D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;${USER})`;
    expect(sddlOtherReaders(ownerOnly, USER)).toEqual([]);
    // What a folder made under C:\ passes on: Users read and execute, Authenticated Users modify.
    const underDrive = `O:${USER}G:${USER}D:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;0x1200a9;;;BU)(A;ID;0x1301bf;;;AU)`;
    expect(sddlOtherReaders(underDrive, USER)).toEqual(["BU", "AU"]);
    // An entry for what a folder will hold, a denial, and a grant that reads nothing do not let anyone read this.
    const harmless = `O:${USER}D:(A;OICIIO;GA;;;CO)(D;;FA;;;WD)(A;;0x100000;;;WD)(A;;FA;;;OW)`;
    expect(sddlOtherReaders(harmless, USER)).toEqual([]);
    expect(sddlOtherReaders(`O:BAD:(A;;FR;;;S-1-5-21-9-9-9-1002)`, USER)).toEqual(["S-1-5-21-9-9-9-1002"]);
    const entraOnly = `O:${ENTRA_USER}D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;${ENTRA_USER})`;
    expect(sddlOtherReaders(entraOnly, ENTRA_USER)).toEqual([]);
  });

  it("reads a NULL DACL, which gives everyone full control, as Everyone", () => {
    expect(sddlOtherReaders(`O:${USER}G:${USER}D:NO_ACCESS_CONTROL`, USER)).toEqual(["WD"]);
  });

  it("cannot tell who reads a descriptor with no DACL to read", () => {
    expect(() => sddlOtherReaders("", USER)).toThrow("no DACL");
    expect(() => sddlOtherReaders(`O:${USER}G:${USER}`, USER)).toThrow("no DACL");
  });
});

describe("this user's SID", () => {
  it("is read from whoami for a local, domain or Microsoft Entra ID account", () => {
    expect(whoamiUserSid(`"desktop-1\\me","${USER}"\r\n`)).toBe(USER);
    expect(whoamiUserSid(`"azuread\\janedoe","${ENTRA_USER}"\r\n`)).toBe(ENTRA_USER);
    expect(() => whoamiUserSid("")).toThrow("whoami did not report this user's SID");
  });
});

describe("the read of who may read a file", () => {
  // A cmdlet named alone makes Windows PowerShell read every installed module it has not read yet before it runs: 47 s
  // with 40 large ones, past the read's 30 s limit, so it read nothing.
  it("names Get-Acl's module, so PowerShell reads no other installed module first", async () => {
    const scripts: Array<string | undefined> = [];
    // Only PowerShell is stood in for: on Windows `whoami` stays the real one, since this user's SID, once read, is kept.
    const { execFileSync: realExecFileSync } = await vi.importActual<{ execFileSync: typeof execFileSync }>("node:child_process");
    vi.mocked(execFileSync).mockImplementation(((file: string, args: readonly string[], options: Parameters<typeof realExecFileSync>[2]) => {
      if (/powershell\.exe$/iu.test(file)) {
        scripts.push(commandScript(args));
        return "O:SYD:PAI(A;;FA;;;SY)\r\n";
      }
      return process.platform === "win32" ? realExecFileSync(file, args, options) : `"desktop-1\\me","${USER}"\r\n`;
    }) as unknown as typeof execFileSync);
    expect(WINDOWS_ACL.otherReaders(String.raw`C:\nowhere\key.pem`)).toEqual([]);
    expect(scripts).toHaveLength(1);
    expect(scripts[0]).toMatch(/\\Get-Acl /u);
    expect(cmdletsWithoutModule(scripts[0] ?? "")).toEqual([]);
  });
});

describe("an owner-only ACL on Windows", () => {
  it("goes on a folder outside the profile, never inside it, and never on macOS or Linux", () => {
    const acl = recordingAcl();
    keepFolderPrivate(String.raw`D:\rerail`, { acl, platform: "win32", profile: String.raw`C:\Users\me`, made: true });
    keepFolderPrivate(String.raw`c:\users\ME\.rerail`, { acl, platform: "win32", profile: String.raw`C:\Users\me`, made: true });
    keepFolderPrivate("/srv/rerail", { acl, platform: "linux", profile: "/home/me", made: true });
    expect(acl.restricted).toEqual([[String.raw`D:\rerail`, true]]);
    expect(insideProfile(String.raw`C:\Users\me2\x`, String.raw`C:\Users\me`)).toBe(false);
  });

  it("goes on only a folder just made, or one a rare command repairs, so a hook's call starts no program", () => {
    const acl = recordingAcl();
    keepFolderPrivate(String.raw`D:\rerail`, { acl, platform: "win32", profile: ELSEWHERE });
    keepFolderPrivate(String.raw`D:\rerail`, { acl, platform: "win32", profile: ELSEWHERE, made: false });
    expect(acl.restricted).toEqual([]);
    keepFolderPrivate(String.raw`D:\rerail`, { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(acl.restricted).toEqual([[String.raw`D:\rerail`, true]]);
  });

  it("never fails its caller when Windows refuses it, and says so", () => {
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const acl: WindowsAcl = { restrictToOwner: () => {
      throw new Error("icacls failed");
    }, restrictFolderToOwner: () => Promise.resolve(), otherReaders: () => [], inherits: () => Promise.resolve(false),
    writers: () => Promise.resolve([]), inheritedWriters: () => Promise.resolve([]), keepEachOwnersOwn: () => Promise.resolve(), letAddFiles: () => Promise.resolve() };
    expect(() => keepFolderPrivate(String.raw`D:\rerail`, { acl, platform: "win32", profile: ELSEWHERE, made: true })).not.toThrow();
    expect(written).toHaveBeenCalledWith(expect.stringContaining("icacls failed"));
  });

  it("goes on a Rerail home Windows creates, which mode 0700 does nothing for", async () => {
    const home = join(await temporaryRoot(), "rerail-home");
    const acl = recordingAcl();
    makeRerailHome(home, { acl, platform: "win32", profile: ELSEWHERE });
    expect(acl.restricted).toEqual([[home, true]]);
    makeRerailHome(home, { acl, platform: "win32", profile: ELSEWHERE });
    expect(acl.restricted).toHaveLength(1);
  });

  it("goes on a Rerail home a lock makes first, as a first `rerail install` or `rerail activate` does", async () => {
    const home = join(await temporaryRoot(), "rerail-home");
    const acl = recordingAcl();
    const lock = {
      name: "work",
      busy: () => new Error("busy"),
      engine: () => ({ lockExclusive: async () => ({ release: () => undefined }) }),
      machine: { acl, platform: "win32" as const, profile: ELSEWHERE },
    };
    await withHomeLock(home, lock, async () => undefined);
    expect(acl.restricted).toEqual([[home, true]]);
    await withHomeLock(home, lock, async () => undefined);
    expect(acl.restricted).toHaveLength(1);
  });

  it("goes on a checkout's session records folder when it is made, which every record then inherits", async () => {
    const checkout = await temporaryRoot();
    const acl = recordingAcl();
    await makeLedgerFolder(join(checkout, ".rerail", "sessions", "claude-code-locks"), { acl, platform: "win32", profile: ELSEWHERE });
    expect(acl.restricted).toEqual([[join(checkout, ".rerail", "sessions"), true]]);
    await makeLedgerFolder(join(checkout, ".rerail", "sessions", "codex-locks"), { acl, platform: "win32", profile: ELSEWHERE });
    expect(acl.restricted).toHaveLength(1);
  });

  it("goes on a session records folder Git, a person or an earlier release made, once a daemon opens a session there", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE };
    // A hook's call reads no ACL: it starts no program for a folder it did not make.
    await makeLedgerFolder(join(sessions, "claude-code-locks"), windows);
    expect(acl.read).toEqual([]);
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { ...windows, repair: true });
    expect(acl.restricted).toContainEqual([sessions, true]);
    const restricted = acl.restricted.length;
    // Once per folder in a daemon's life: the next session there, once the repair has ended, reads and sets nothing.
    await makeLedgerFolder(join(sessions, "b".repeat(64)), { ...windows, repair: true });
    expect(acl.read).toEqual([sessions]);
    expect(acl.restricted).toHaveLength(restricted);
  });

  it("reads a session records folder again once another folder has taken its place", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    // Moved aside, the first folder keeps its file id, which the new one then cannot take.
    await rename(sessions, `${sessions}.aside`);
    await mkdir(sessions);
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    expect(acl.restricted.filter(([path]) => path === sessions)).toHaveLength(2);
  });

  it("reads no ACL of a records folder outside any .rerail folder, which the person named and may keep their own files in", async () => {
    const root = join(await temporaryRoot(), "records");
    const acl = recordingAcl();
    await makeLedgerFolder(join(root, "a".repeat(64)), { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(acl.read).toEqual([]);
  });

  it("reads no ACL of a session records folder under the profile folder, whose own ACL is the person's alone", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { acl, platform: "win32", profile: checkout, repair: true });
    expect(acl.read).toEqual([]);
  });

  it("leaves a session records folder with an ACL of its own as it is", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl(false);
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(acl.read).toEqual([sessions]);
    expect(acl.restricted).toEqual([]);
  });

  it("says when it cannot keep a session records folder private, and still makes the session's folder", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const acl: WindowsAcl = { ...recordingAcl(), keepEachOwnersOwn: () => Promise.reject(new Error("Access is denied.")) };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(written).toHaveBeenCalledWith(`rerail: could not keep ${sessions} to this user alone, so other accounts may read what Rerail writes there: Access is denied.\n`);
  });

  it("gives each later session's folder its owner-only ACL once the records folder's repair, or the read of its ACL, failed", async () => {
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const refused = () => Promise.reject(new Error("Access is denied."));
    for (const failing of [{ keepEachOwnersOwn: refused }, { inherits: refused }]) {
      const sessions = join(await temporaryRoot(), ".rerail", "sessions");
      await mkdir(sessions, { recursive: true });
      const acl = { ...recordingAcl(), ...failing };
      const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
      await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
      // Once the failure is named, the folder still has its inherited ACL, which a new session's folder would take.
      while (!written.mock.calls.some(([line]) => String(line).startsWith(`rerail: could not keep ${sessions} `))) await new Promise(resolve => setImmediate(resolve));
      await makeLedgerFolder(join(sessions, "b".repeat(64)), windows);
      expect(acl.restricted, Object.keys(failing)[0]).toContainEqual([join(sessions, "b".repeat(64)), true]);
    }
  });

  it("reads and repairs a records folder again at the next session once the read of its ACL, or its repair, failed in a way that may pass", async () => {
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const notStarted = Object.assign(new Error("spawn icacls.exe EAGAIN"), { code: "EAGAIN", syscall: "spawn icacls.exe" });
    // The read and `icacls /save` (`inheritedWriters`) each have a time limit; any `icacls` may not start.
    for (const [failing, error] of [["inherits", timedOut()], ["inheritedWriters", timedOut()], ["inheritedWriters", notStarted]] as const) {
      const sessions = join(await temporaryRoot(), ".rerail", "sessions");
      await mkdir(sessions, { recursive: true });
      const acl = recordingAcl();
      vi.spyOn(acl, failing).mockRejectedValueOnce(error);
      const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
      for (const id of ["a", "b"]) {
        await makeLedgerFolder(join(sessions, id.repeat(64)), windows);
        await new Promise(resolve => setImmediate(resolve));
      }
      expect(acl.calls, `${failing}: ${error.message}`).toContain(`each owner's own ${sessions} for `);
    }
  });

  it("keeps a records folder's refused repair, read and named once, until what describes the folder changes", async () => {
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const sessions = join(await temporaryRoot(), ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    // What `execFile` rejects with when Windows refuses `icacls`: it ran, and exited with Access is denied's code.
    const refused = Object.assign(new Error(`Command failed: icacls.exe\n${sessions}: Access is denied.`), { code: 5, killed: false, signal: null });
    const repaired = vi.spyOn(acl, "keepEachOwnersOwn").mockRejectedValue(refused);
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    const named = (): number => written.mock.calls.filter(([line]) => String(line).startsWith(`rerail: could not keep ${sessions} `)).length;
    for (const id of ["a", "b", "c"]) {
      await makeLedgerFolder(join(sessions, id.repeat(64)), windows);
      while (named() === 0) await new Promise(resolve => setImmediate(resolve));
    }
    expect([acl.read, repaired.mock.calls.length, named()]).toEqual([[sessions], 1, 1]);
    // Each session's own folder still gets the owner-only ACL of its own.
    expect(acl.restricted).toContainEqual([join(sessions, "c".repeat(64)), true]);
    await changeAccess(sessions);
    await makeLedgerFolder(join(sessions, "d".repeat(64)), windows);
    expect(acl.read).toEqual([sessions, sessions]);
  });

  it("reads a records folder again at the next session once its repair could not start", async () => {
    const sessions = join(await temporaryRoot(), ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    // With no profile given, the repair reads this user's home first.
    vi.mocked(homedir).mockImplementationOnce(() => {
      throw new Error("no home folder");
    });
    await expect(makeLedgerFolder(join(sessions, "a".repeat(64)), { acl, platform: "win32", repair: true })).rejects.toThrow("no home folder");
    await makeLedgerFolder(join(sessions, "b".repeat(64)), { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(acl.read).toEqual([sessions]);
  });

  it("keeps the repair of a records folder that took another's place when the earlier one's repair fails after it", async () => {
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const sessions = join(await temporaryRoot(), ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    const { promise: firstRepair, reject: failFirst } = Promise.withResolvers<undefined>();
    const repaired = vi.spyOn(acl, "keepEachOwnersOwn").mockReturnValueOnce(firstRepair);
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    while (repaired.mock.calls.length === 0) await new Promise(resolve => setImmediate(resolve));
    await rename(sessions, `${sessions}.aside`);
    await mkdir(sessions);
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    while (repaired.mock.calls.length === 1) await new Promise(resolve => setImmediate(resolve));
    failFirst(timedOut());
    await new Promise(resolve => setImmediate(resolve));
    await makeLedgerFolder(join(sessions, "b".repeat(64)), windows);
    expect(acl.read).toEqual([sessions, sessions]);
  });

  it("lets sessions that open while their records folder is repaired go on at once, each one's folder its owner's first", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const started: string[] = [];
    const ownersAlone: string[] = [];
    // The records folder's repair stays under way until the case releases it, as one over many earlier records does.
    const { promise: passedOn, resolve: passOn } = Promise.withResolvers<undefined>();
    const acl: WindowsAcl = { ...recordingAcl(), keepEachOwnersOwn: path => {
      started.push(path);
      return passedOn;
    }, restrictFolderToOwner: async path => {
      started.push(path);
      // A session's own folder, new or nearly empty, has its ACL a turn of the event loop later.
      await new Promise(resolve => setImmediate(resolve));
      ownersAlone.push(path);
    } };
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    const open = async (id: string): Promise<void> => {
      const folder = join(sessions, id.repeat(64));
      await makeLedgerFolder(folder, windows);
      // What the session writes next goes into a folder that is already its owner's alone.
      expect(ownersAlone).toContain(folder);
    };
    // The session that starts the repair, and a second one opening meanwhile, each go on while it is held.
    await Promise.all([open("a"), open("b")]);
    expect(started.filter(path => path === sessions)).toHaveLength(1);
    passOn(undefined);
    await new Promise(resolve => setImmediate(resolve));
    // Once the repair has ended, a session there takes the folder's ACL and starts no icacls of its own.
    await makeLedgerFolder(join(sessions, "c".repeat(64)), windows);
    expect(started).toHaveLength(3);
  });

  it("gives a session's folder its owner-only ACL when the records folder's repair ends after that folder was made", async () => {
    const sessions = join(await temporaryRoot(), ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const acl = recordingAcl();
    const { promise: passedOn, resolve: passOn } = Promise.withResolvers<undefined>();
    vi.spyOn(acl, "keepEachOwnersOwn").mockReturnValueOnce(passedOn);
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    // The repair ends once the next session's folder is there, before that session asks whether it has: Windows may
    // have gone over the records folder before that folder was made, which then keeps the open ACL it took.
    const { mkdir: makeFolder } = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");
    vi.mocked(mkdir).mockImplementationOnce((async (path: string, options?: MakeDirectoryOptions) => {
      const made = await makeFolder(path, options);
      passOn(undefined);
      await new Promise(resolve => setImmediate(resolve));
      return made;
    }) as typeof mkdir);
    await makeLedgerFolder(join(sessions, "b".repeat(64)), windows);
    expect(acl.restricted).toContainEqual([join(sessions, "b".repeat(64)), true]);
    // A session's folder made once the repair has ended takes the records folder's new ACL.
    await makeLedgerFolder(join(sessions, "c".repeat(64)), windows);
    expect(acl.restricted).not.toContainEqual([join(sessions, "c".repeat(64)), true]);
  });

  it("repairs a records folder again at the next session, its ACL unchanged meanwhile, when a folder every account shares there could not be shared in a way that may pass", async () => {
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const sessions = join(await temporaryRoot(), ".rerail", "sessions");
    const locks = join(sessions, "claude-code-locks");
    await mkdir(locks, { recursive: true });
    const acl = recordingAcl(true, ["S-1-5-11"]);
    vi.spyOn(acl, "letAddFiles").mockRejectedValueOnce(timedOut());
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), windows);
    while (!written.mock.calls.some(([line]) => String(line).startsWith(`rerail: could not let other accounts that record here use ${locks}:`))) {
      await new Promise(resolve => setImmediate(resolve));
    }
    await new Promise(resolve => setImmediate(resolve));
    await makeLedgerFolder(join(sessions, "b".repeat(64)), windows);
    while (!acl.calls.some(call => call.startsWith("each owner's own "))) await new Promise(resolve => setImmediate(resolve));
    // The first repair ended before the records folder's list changed, so other accounts' hooks kept their way into
    // the shared folder; the second shared it first, then changed that list.
    expect(acl.calls).toEqual([
      `inherited writers ${sessions}`, `inherited writers ${sessions}`, `add files ${locks} for S-1-5-11`, `each owner's own ${sessions} for S-1-5-11`,
    ]);
  });

  it("keeps for each account that could make a folder in a found records folder that right, after letting it add files to each folder every account shares there", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    for (const name of ["claude-code-locks", "codex-pending", "a".repeat(64)]) await mkdir(join(sessions, name), { recursive: true });
    await writeFile(join(sessions, "notes-locks"), "a file, not a folder\n");
    const acl = recordingAcl(true, ["S-1-5-11"]);
    const repair = await keepFoundFolderPrivate(sessions, { acl, platform: "win32", profile: ELSEWHERE, shared: name => name.endsWith("-locks") || name.endsWith("-pending") });
    await repair.kept;
    expect(acl.calls.slice(0, 1)).toEqual([`inherited writers ${sessions}`]);
    expect(acl.calls.slice(1, 3).sort()).toEqual([`add files ${join(sessions, "claude-code-locks")} for S-1-5-11`, `add files ${join(sessions, "codex-pending")} for S-1-5-11`]);
    // The records folder's own list changes last, once every account that records there can still lock and hold steers.
    expect(acl.calls.slice(3)).toEqual([`each owner's own ${sessions} for S-1-5-11`]);
  });

  it("lets the accounts that record in a records folder add files to a folder every account shares there, whoever makes it, and to no session's folder", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    // Another account's view: its hook makes the agent host's lock folder in a records folder the first account repaired.
    const acl = recordingAcl(false, ["S-1-5-21-1-2-3-1001", "S-1-5-11"]);
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE };
    await makeLedgerFolder(join(sessions, "claude-code-locks"), windows);
    expect(acl.calls).toEqual([`writers ${sessions}`, `add files ${join(sessions, "claude-code-locks")} for S-1-5-21-1-2-3-1001 S-1-5-11`]);
    // A hook's call on a folder already there, and a session's own folder, start no program.
    await makeLedgerFolder(join(sessions, "claude-code-locks"), windows);
    await makeLedgerFolder(join(sessions, "b".repeat(64)), windows);
    expect(acl.calls).toHaveLength(2);
    // Under the profile folder, or elsewhere than Windows, nothing is shared.
    await makeLedgerFolder(join(sessions, "codex-locks"), { ...windows, profile: checkout });
    await makeLedgerFolder(join(sessions, "cursor-locks"), { ...windows, platform: "linux" });
    expect(acl.calls).toHaveLength(2);
  });

  it("names the accounts a folder's ACL lets make a folder in it, by SID, beyond this user, SYSTEM, Administrators and the creator's entry", () => {
    // A folder made under `C:\`: Users may read, Authenticated Users may change, and a named account may write.
    const underDrive = "D:AI(A;OICIID;FA;;;BA)(A;OICIID;FA;;;SY)(A;OICIID;0x1200a9;;;BU)(A;ID;0x1301bf;;;AU)(A;OICIIOID;SDGXGWGR;;;AU)"
      + `(A;OICIIOID;GA;;;CO)(A;;FA;;;${USER})(A;;FW;;;S-1-5-21-1-2-3-1002)(D;;FA;;;S-1-5-21-1-2-3-1003)(A;;LC;;;S-1-5-21-1-2-3-1004)`;
    expect(sddlWriters(underDrive, USER)).toEqual(["S-1-5-11", "S-1-5-21-1-2-3-1002", "S-1-5-21-1-2-3-1004"]);
    // Repaired: this user lists it, Authenticated Users may only make a folder there.
    expect(sddlWriters(`D:PAI(A;;FA;;;${USER})(A;OICIIO;FA;;;CO)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;;0x1200a4;;;AU)`, USER)).toEqual(["S-1-5-11"]);
    expect(sddlWriters(`D:PAI(A;OICI;FA;;;${USER})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`, USER)).toEqual([]);
    expect(sddlWriters("D:NO_ACCESS_CONTROL", USER)).toEqual(["S-1-1-0"]);
  });

  it("keeps, as a records folder stops inheriting, the right to make a folder there for the accounts an inherited entry gives it, an allow entry set on the folder for one of them included, never one a deny entry set there names", () => {
    // Authenticated Users, Users and another account may change what is in it by inheritance; entries set on the folder
    // let one account that inherits nothing write, let Users only read and run what is there, and deny the other one
    // writing.
    const found = "D:AI(A;;FW;;;S-1-5-21-1-2-3-1002)(A;;0x1200a9;;;BU)(D;;WD;;;S-1-5-21-1-2-3-1003)(A;OICIID;FA;;;BA)(A;OICIID;FA;;;SY)"
      + "(A;ID;0x1301bf;;;AU)(A;OICIID;0x1301bf;;;BU)(A;ID;0x1301bf;;;S-1-5-21-1-2-3-1003)";
    // Users keeps the right beside its own read entry, which `icacls /grant` adds it to; the denied account gets none.
    expect(sddlInheritedWriters(found, USER)).toEqual(["S-1-5-11", "S-1-5-32-545"]);
    expect(sddlWriters(found, USER)).toEqual(["S-1-5-21-1-2-3-1002", "S-1-5-11", "S-1-5-32-545", "S-1-5-21-1-2-3-1003"]);
    // A deny entry the folder inherits is not one set there.
    expect(sddlInheritedWriters("D:AI(D;ID;WD;;;BU)(A;OICIID;0x1301bf;;;BU)", USER)).toEqual(["S-1-5-32-545"]);
    expect(sddlInheritedWriters("D:NO_ACCESS_CONTROL", USER)).toEqual([]);
  });

  it("runs each icacls of a records folder's repair on a link there itself, never on what it points to, and each but those that change the folder's own ACL within its time limit", async () => {
    const runs: Array<[readonly string[], number | undefined]> = [];
    vi.mocked(execFile).mockImplementation(((_file: string, args: readonly string[], options: { timeout?: number }, done: (error: Error | null, stdout: string, stderr: string) => void) => {
      runs.push([args, options.timeout]);
      const save = args.indexOf("/save");
      if (save >= 0) writeFileSync(args[save + 1] ?? "", Buffer.from("sessions\r\nD:AI(A;OICIID;0x1301bf;;;AU)\r\n", "utf16le"));
      done(null, "", "");
    }) as unknown as typeof execFile);
    // This user's SID, which whoami gives on Windows.
    if (process.platform !== "win32") vi.mocked(execFileSync).mockImplementation(() => `"desktop-1\\me","${USER}"\r\n`);
    const sessions = String.raw`C:\src\demo\.rerail\sessions`;
    await WINDOWS_ACL.inherits(sessions);
    expect(await WINDOWS_ACL.inheritedWriters(sessions)).toEqual(["S-1-5-11"]);
    expect(await WINDOWS_ACL.writers(sessions)).toEqual(["S-1-5-11"]);
    await WINDOWS_ACL.letAddFiles(`${sessions}\\claude-code-locks`, ["S-1-5-11"]);
    await WINDOWS_ACL.keepEachOwnersOwn(sessions, ["S-1-5-11"]);
    await WINDOWS_ACL.restrictFolderToOwner(`${sessions}\\${"a".repeat(64)}`);
    expect(runs.map(([args]) => [args[0], args.includes("/l")])).toEqual([
      [sessions, true], [sessions, true], [sessions, true], [`${sessions}\\claude-code-locks`, true], [sessions, true], [sessions, true],
      [`${sessions}\\${"a".repeat(64)}`, true],
    ]);
    // Windows passes the records folder's ACL again to all it holds on each change of it, so those two have no time limit.
    expect(runs.map(([, timeout]) => timeout)).toEqual([30_000, 30_000, 30_000, 30_000, undefined, undefined, 30_000]);
    expect(runs[3]?.[0]).toEqual([`${sessions}\\claude-code-locks`, "/grant", "*S-1-5-11:(X,WD,RA,S)", "/l", "/q"]);
    // Each account that may make a folder there gets that right on the folder itself before it stops inheriting the
    // entry that gave it, added to any entry already set there for that account.
    expect(runs[4]?.[0]).toEqual([sessions, "/grant", "*S-1-5-11:(X,AD,RA,RC,S)", "/l", "/q"]);
    expect(runs[5]?.[0]).toEqual([
      sessions, "/inheritance:r", "/grant:r", expect.stringMatching(/^\*S-1-[\d-]+:F$/u), "*S-1-3-0:(OI)(CI)(IO)F",
      "*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F", "/l", "/q",
    ]);
  });

  it.runIf(process.platform === "win32")("keeps each account's records its own in a records folder under a drive, and lets each account that could make a folder there still make one", async () => {
    const root = await temporaryRoot();
    // Authenticated Users may change what the root holds, as under `C:\`, so the records folder inherits that.
    execFileSync("icacls.exe", [root, "/grant", "*S-1-5-11:(OI)(CI)M", "/q"]);
    const sessions = join(root, ".rerail", "sessions");
    await mkdir(join(sessions, "0".repeat(64)), { recursive: true });
    await writeFile(join(sessions, "0".repeat(64), "events.jsonl"), "{}\n");
    await mkdir(join(sessions, "claude-code-locks"));
    const repair = await keepFoundFolderPrivate(sessions, { profile: ELSEWHERE, shared: name => name.endsWith("-locks") });
    await repair.kept;
    const records = savedEntries(sessions);
    expect(records.filter(([flags]) => flags.includes("ID"))).toEqual([]);
    expect(records).toContainEqual(["OICIIO", 0x1F_01FF, "CO"]);
    // Authenticated Users: pass through, read attributes and the ACL, make a folder; never list it.
    expect(records).toContainEqual(["", 0x12_00A4, "AU"]);
    expect(savedEntries(join(sessions, "claude-code-locks"))).toContainEqual(["", 0x10_00A2, "AU"]);
    expect(savedEntries(join(sessions, "0".repeat(64), "events.jsonl")).map(([, , sid]) => sid)).not.toContain("AU");
    // A folder every account shares, made after the repair, gets the same entry; a session's folder gets none.
    await makeLedgerFolder(join(sessions, "codex-locks"), { profile: ELSEWHERE });
    expect(savedEntries(join(sessions, "codex-locks"))).toContainEqual(["", 0x10_00A2, "AU"]);
    await makeLedgerFolder(join(sessions, "1".repeat(64)), { profile: ELSEWHERE });
    expect(savedEntries(join(sessions, "1".repeat(64))).map(([, , sid]) => sid)).not.toContain("AU");
  }, 60_000);

  it("tells an inherited entry from what icacls prints, in any language and whatever the path", () => {
    const madeUnderDrive = [
      String.raw`C:\src\demo(I)\.rerail\sessions BUILTIN\Administrators:(I)(OI)(CI)(F)`,
      String.raw`                             VORDEFINIERT\Benutzer:(I)(OI)(CI)(RX)`,
      "",
      "Successfully processed 1 files; Failed processing 0 files",
    ].join("\r\n");
    expect(icaclsInherits(madeUnderDrive)).toBe(true);
    const ownerOnly = [
      String.raw`C:\src\demo(I)\.rerail\sessions BUILTIN\Administrators:(OI)(CI)(F)`,
      String.raw`                             NT AUTHORITY\SYSTEM:(OI)(CI)(F)`,
      String.raw`                             DESKTOP-1\me:(OI)(CI)(F)`,
      "",
      "Traitement réussi de 1 fichiers ; échec du traitement de 0 fichiers",
    ].join("\r\n");
    expect(icaclsInherits(ownerOnly)).toBe(false);
  });

  it("names the accounts beyond the owner its ACL lets read a file, and none for an owner-only one", async () => {
    const reader = (readers: string[]) => ({ platform: "win32" as const, acl: { otherReaders: () => readers } });
    expect(await openToOthers(String.raw`C:\certs\key.pem`, reader(["BU", "AU"]))).toBe("can be read by other accounts (BU, AU)");
    expect(await openToOthers(String.raw`C:\certs\key.pem`, reader([]))).toBeUndefined();
    const unread = { platform: "win32" as const, acl: { otherReaders: (): string[] => {
      throw new Error("Get-Acl failed");
    } } };
    await expect(openToOthers(String.raw`C:\certs\key.pem`, unread)).rejects.toThrow("Get-Acl failed");
  });

  it.runIf(process.platform === "win32")("says why Windows refused an owner-only ACL, in the words icacls gives", async () => {
    const missing = join(await temporaryRoot(), "missing");
    let message = "";
    try {
      WINDOWS_ACL.restrictToOwner(missing, true);
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    // The first line names the command; icacls's own reason follows it, starting with the path it refused.
    expect(message.split(/\r?\n/u).slice(1).join("\n")).toContain(`${missing}:`);
    const later = await WINDOWS_ACL.restrictFolderToOwner(missing).then(() => "", (error: unknown) => (error instanceof Error ? error.message : String(error)));
    expect(later.split(/\r?\n/u).slice(1).join("\n")).toContain(`${missing}:`);
  });

  it.runIf(process.platform === "win32")("lets the daemon's other work run while Windows passes a records folder's new ACL to the records in it", async () => {
    const root = await temporaryRoot();
    // Users may read the root, as under `C:\`, so the record below inherits that and the repair has it to rewrite.
    execFileSync("icacls.exe", [root, "/grant", "*S-1-5-32-545:(OI)(CI)RX", "/q"]);
    const sessions = join(root, ".rerail", "sessions");
    await mkdir(join(sessions, "0".repeat(64)), { recursive: true });
    await writeFile(join(sessions, "0".repeat(64), "events.jsonl"), "{}\n");
    let turned = false;
    setImmediate(() => {
      turned = true;
    });
    await WINDOWS_ACL.restrictFolderToOwner(sessions);
    // The event loop turned while icacls ran, as it must for a daemon to answer other sessions meanwhile.
    expect(turned).toBe(true);
    expect(await WINDOWS_ACL.inherits(sessions)).toBe(false);
    expect(WINDOWS_ACL.otherReaders(join(sessions, "0".repeat(64), "events.jsonl"))).toEqual([]);
    // Windows PowerShell, which reads the record's ACL, can take seconds to start on a cold machine.
  }, 60_000);

  it.runIf(process.platform === "win32")("leaves only this user, SYSTEM and Administrators able to read a folder", async () => {
    const folder = await temporaryRoot();
    expect(await WINDOWS_ACL.inherits(folder)).toBe(true);
    WINDOWS_ACL.restrictToOwner(folder, true);
    expect(await WINDOWS_ACL.inherits(folder)).toBe(false);
    expect(WINDOWS_ACL.otherReaders(folder)).toEqual([]);
    // Windows PowerShell can take seconds to start on a cold machine.
  }, 60_000);
});
