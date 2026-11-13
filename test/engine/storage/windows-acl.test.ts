import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { withHomeLock } from "../../../src/engine/native/file-lock.js";
import { makeLedgerFolder } from "../../../src/engine/storage/ledger-paths.js";
import { keepFolderPrivate, makeRerailHome, openToOthers } from "../../../src/engine/storage/private-files.js";
import { insideProfile, sddlOtherReaders, whoamiUserSid, WINDOWS_ACL, type WindowsAcl } from "../../../src/engine/storage/windows-acl.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const USER = "S-1-5-21-1-2-3-1001";
/** A Microsoft Entra ID (Azure AD) account's SID, as on a laptop joined to a company's Entra ID. */
const ENTRA_USER = "S-1-12-1-1111111111-2222222222-3333333333-4444444444";
/** A profile no test folder is under, so every folder here counts as outside it. */
const ELSEWHERE = String.raw`C:\Users\nobody-here`;

function recordingAcl(): WindowsAcl & { restricted: Array<[string, boolean]> } {
  const restricted: Array<[string, boolean]> = [];
  return { restricted, restrictToOwner: (path, folder) => restricted.push([path, folder]), otherReaders: () => [] };
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
    }, otherReaders: () => [] };
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

  it("names the accounts beyond the owner its ACL lets read a file, and none for an owner-only one", async () => {
    const reader = (readers: string[]) => ({ platform: "win32" as const, acl: { otherReaders: () => readers } });
    expect(await openToOthers(String.raw`C:\certs\key.pem`, reader(["BU", "AU"]))).toBe("can be read by other accounts (BU, AU)");
    expect(await openToOthers(String.raw`C:\certs\key.pem`, reader([]))).toBeUndefined();
    const unread = { platform: "win32" as const, acl: { otherReaders: (): string[] => {
      throw new Error("Get-Acl failed");
    } } };
    await expect(openToOthers(String.raw`C:\certs\key.pem`, unread)).rejects.toThrow("Get-Acl failed");
  });

  it.runIf(process.platform === "win32")("leaves only this user, SYSTEM and Administrators able to read a folder", async () => {
    const folder = await temporaryRoot();
    WINDOWS_ACL.restrictToOwner(folder, true);
    expect(WINDOWS_ACL.otherReaders(folder)).toEqual([]);
    // Windows PowerShell can take seconds to start on a cold machine.
  }, 60_000);
});
