import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, rename, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { withHomeLock } from "../../../src/engine/native/file-lock.js";
import { makeLedgerFolder } from "../../../src/engine/storage/ledger-paths.js";
import { keepFolderPrivate, makeRerailHome, openToOthers } from "../../../src/engine/storage/private-files.js";
import { icaclsInherits, insideProfile, sddlOtherReaders, whoamiUserSid, WINDOWS_ACL, type WindowsAcl } from "../../../src/engine/storage/windows-acl.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const USER = "S-1-5-21-1-2-3-1001";
/** A Microsoft Entra ID (Azure AD) account's SID, as on a laptop joined to a company's Entra ID. */
const ENTRA_USER = "S-1-12-1-1111111111-2222222222-3333333333-4444444444";
/** A profile no test folder is under, so every folder here counts as outside it. */
const ELSEWHERE = String.raw`C:\Users\nobody-here`;

/** An ACL that records each folder it restricts and each it is asked about; `inherited` is what every read answers. */
function recordingAcl(inherited = true): WindowsAcl & { restricted: Array<[string, boolean]>; read: string[] } {
  const restricted: Array<[string, boolean]> = [];
  const read: string[] = [];
  return {
    restricted,
    read,
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
  };
}

const roots: string[] = [];
async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-acl-"));
  roots.push(root);
  return root;
}

afterEach(async () => {
  vi.restoreAllMocks();
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
    }, restrictFolderToOwner: () => Promise.resolve(), otherReaders: () => [], inherits: () => Promise.resolve(false) };
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
      engine: () => ({ tryLockExclusive: () => ({ release: () => undefined }) }),
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
    expect(acl.restricted).toEqual([[sessions, true]]);
    // Once per folder in a daemon's life: the next session there reads nothing.
    await makeLedgerFolder(join(sessions, "b".repeat(64)), { ...windows, repair: true });
    expect(acl.read).toEqual([sessions]);
    expect(acl.restricted).toHaveLength(1);
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
    expect(acl.restricted).toEqual([[sessions, true], [sessions, true]]);
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
    const acl: WindowsAcl = { ...recordingAcl(), restrictFolderToOwner: () => Promise.reject(new Error("Access is denied.")) };
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { acl, platform: "win32", profile: ELSEWHERE, repair: true });
    expect(written).toHaveBeenCalledWith(`rerail: could not keep ${sessions} to this user alone, so other accounts may read what Rerail writes there: Access is denied.\n`);
  });

  it("has a session that opens while its records folder is repaired wait for that one repair, and start no second", async () => {
    const checkout = await temporaryRoot();
    const sessions = join(checkout, ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    const repairs: string[] = [];
    const { promise: passedOn, resolve: passOn } = Promise.withResolvers<undefined>();
    const acl: WindowsAcl = { ...recordingAcl(), restrictFolderToOwner: path => {
      repairs.push(path);
      return passedOn;
    } };
    const windows = { acl, platform: "win32" as const, profile: ELSEWHERE, repair: true };
    const opened: string[] = [];
    const sessionsOpening = ["a", "b"].map(async id => {
      await makeLedgerFolder(join(sessions, id.repeat(64)), windows);
      opened.push(id);
    });
    await vi.waitFor(() => expect(repairs).toEqual([sessions]));
    // Neither session goes on to write while Windows passes the folder's new ACL on.
    await new Promise(resolve => setImmediate(resolve));
    expect(opened).toEqual([]);
    passOn(undefined);
    await Promise.all(sessionsOpening);
    expect(opened.toSorted()).toEqual(["a", "b"]);
    expect(repairs).toEqual([sessions]);
  });

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

  it.runIf(process.platform === "win32")("leaves the daemon's thread free while Windows passes a records folder's new ACL to the records in it", async () => {
    const root = await temporaryRoot();
    // Users may read the root, as under `C:\`, so each entry below inherits that and the repair has every one to rewrite.
    execFileSync("icacls.exe", [root, "/grant", "*S-1-5-32-545:(OI)(CI)RX", "/q"]);
    const sessions = join(root, ".rerail", "sessions");
    for (let i = 0; i < 1_500; i += 1) {
      await mkdir(join(sessions, String(i)), { recursive: true });
      await writeFile(join(sessions, String(i), "events.jsonl"), "{}\n");
    }
    let longest = 0;
    let last = performance.now();
    const ticks = setInterval(() => {
      longest = Math.max(longest, performance.now() - last);
      last = performance.now();
    }, 5);
    const started = performance.now();
    await makeLedgerFolder(join(sessions, "a".repeat(64)), { platform: "win32", profile: ELSEWHERE, repair: true });
    const took = performance.now() - started;
    clearInterval(ticks);
    expect(await WINDOWS_ACL.inherits(sessions)).toBe(false);
    // Windows passes the ACL to 3,000 entries here, at about 0.2 ms each; the thread never waits for most of it.
    expect(longest).toBeLessThan(took / 2);
  }, 120_000);

  it.runIf(process.platform === "win32")("leaves only this user, SYSTEM and Administrators able to read a folder", async () => {
    const folder = await temporaryRoot();
    expect(await WINDOWS_ACL.inherits(folder)).toBe(true);
    WINDOWS_ACL.restrictToOwner(folder, true);
    expect(await WINDOWS_ACL.inherits(folder)).toBe(false);
    expect(WINDOWS_ACL.otherReaders(folder)).toEqual([]);
    // Windows PowerShell can take seconds to start on a cold machine.
  }, 60_000);
});
