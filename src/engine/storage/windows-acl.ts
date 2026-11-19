/**
 * Owner-only access on Windows, where mode bits mean nothing: `mkdir`'s and `open`'s modes are ignored, and a new file
 * or folder takes its parent's access control list (ACL). Under the profile folder that list grants this user, SYSTEM
 * and Administrators alone; a folder Windows makes elsewhere, under `C:\` say, lets every local user (Users,
 * Authenticated Users) read, and often change, what is in it. Rerail keeps private data in its home and in each
 * checkout's `.rerail\sessions`, and reads a daemon's TLS private key, wherever the person points them.
 *
 * Both work through programs Windows ships, by absolute path and with no window: `icacls` sets an ACL and tells
 * whether one is inherited, Windows PowerShell's `Get-Acl` reads one in SDDL, which names every account by SID whatever
 * the system's language. This is the Windows half of `private-files.ts`, which callers use.
 */
import { randomUUID } from "node:crypto";
import { readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, win32 } from "node:path";
import { promisify } from "node:util";
import { execFile, execFileSync } from "../child-process.js";
import { windowsPowerShell, windowsSystemProgram } from "../system-programs.js";

const SYSTEM_SID = "S-1-5-18";
const ADMINISTRATORS_SID = "S-1-5-32-544";
/** CREATOR OWNER: an inheritable entry for it grants each new file or folder's maker, and no one else, what it names. */
const CREATOR_OWNER_SID = "S-1-3-0";
const OWNER_RIGHTS_SID = "S-1-3-4";
/** The SDDL short names of SYSTEM, Administrators and OWNER RIGHTS, which an owner-only ACL may grant besides the owner. */
const TRUSTED_ALIASES: ReadonlySet<string> = new Set(["SY", "BA", "OW"]);

/** What Rerail asks of Windows' access control; tests pass their own. */
export interface WindowsAcl {
  /**
   * Gives `path` an ACL of its own that inherits nothing and grants full control to this user, SYSTEM and Administrators
   * alone. A folder's entries pass to everything in it, files already there included.
   */
  restrictToOwner(path: string, folder: boolean): void;
  /** The accounts, by SID or SDDL short name, besides this user, SYSTEM and Administrators, that may read `path`. */
  otherReaders(path: string): string[];
  /**
   * Whether `path`'s ACL takes entries from its parent's, as the ACL of a folder Git or `mkdir` made does until it is
   * given one of its own. It holds no thread while `icacls` reads it.
   */
  inherits(path: string): Promise<boolean>;
  /**
   * Gives folder `path` the ACL {@link restrictToOwner} gives, holding no thread while Windows passes it to everything
   * already in the folder, which takes time in step with what is there: about 0.1 to 0.2 ms an entry. Windows sets the
   * folder's own ACL after everything in it, so a run cut short, as by the daemon stopping, leaves the folder still
   * inheriting, and the next daemon sets it again. Nobody waits for it on a records folder; a session waits for it
   * only on its own new folder, whose few entries take one `icacls` start, so it has no time limit.
   */
  restrictFolderToOwner(path: string): Promise<void>;
  /**
   * The accounts, by SID, that `path`'s ACL lets add a folder in it, besides this user, SYSTEM, Administrators and the
   * entries for whoever makes something there: who could record their own sessions in a records folder. Read with one
   * `icacls /save`, holding no thread.
   */
  writers(path: string): Promise<string[]>;
  /**
   * Gives records folder `path` an ACL of its own under which what each account makes there is that account's alone:
   * this user may list the folder, each of `writers` may only pass through it, read its attributes and ACL and make a
   * folder there, and what is made there grants its maker (CREATOR OWNER), SYSTEM and Administrators alone. Windows
   * passes it to what the folder holds as {@link restrictFolderToOwner} does, each entry its owner's.
   */
  keepEachOwnersOwn(path: string, writers: readonly string[]): Promise<void>;
  /**
   * Lets each of `writers` add files to folder `path`, pass through it and read its attributes, and nothing more: an
   * entry on that folder alone, which a folder every account's hooks share needs once its parent stops letting them
   * list it.
   */
  letAddFiles(path: string, writers: readonly string[]): Promise<void>;
}

let userSid: string | undefined;

/**
 * The SID in what `whoami /user /fo csv /nh` prints, `"<domain>\<name>","<SID>"`: its last field. Any SID counts,
 * `S-1-`, the identifier authority, then the sub-authorities: a local or domain account's starts `S-1-5-21-`, a
 * Microsoft Entra ID (Azure AD) account's, as on most company laptops, `S-1-12-1-`.
 */
export function whoamiUserSid(output: string): string {
  const sid = /"(S-1-\d+(?:-\d+)+)"\s*$/u.exec(output)?.[1];
  if (sid === undefined) throw new Error("whoami did not report this user's SID");
  return sid;
}

/**
 * This process's user SID, from `whoami /user`, which reads it from the process's token as .NET's `WindowsIdentity`
 * does, for any kind of account, and starts in milliseconds where Windows PowerShell takes up to seconds.
 */
function currentUserSid(): string {
  userSid ??= whoamiUserSid(execFileSync(windowsSystemProgram("whoami.exe"), ["/user", "/fo", "csv", "/nh"], {
    encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 10_000,
  }));
  return userSid;
}

/** Access-mask bits that let an account read a file's data: FILE_READ_DATA, GENERIC_ALL and GENERIC_READ. */
const READ_BITS = 0x1 | 0x1000_0000 | 0x8000_0000;
/** SDDL right codes that carry one of `READ_BITS`; `CC` is the directory-service name for bit 0x1. */
const READ_CODES: ReadonlySet<string> = new Set(["FA", "FR", "GA", "GR", "CC", "KA", "KR"]);

function grantsRead(rights: string): boolean {
  if (/^0x[0-9a-f]+$/iu.test(rights)) return (Number.parseInt(rights.slice(2), 16) & READ_BITS) !== 0;
  return (rights.match(/[A-Z]{2}/gu) ?? []).some(code => READ_CODES.has(code));
}

/** The account an access control entry lets read the object itself: an allow entry, not inherit-only (`IO`), that grants a read right. */
function readerOf(ace: string): string | undefined {
  const [type = "", flags = "", rights = "", , , sid = ""] = ace.split(";");
  const allowsObject = (type === "A" || type === "OA") && !/IO/u.test(flags);
  return allowsObject && grantsRead(rights) ? sid : undefined;
}

/**
 * The entries of an SDDL security descriptor's DACL, or null for a NULL DACL (`D:NO_ACCESS_CONTROL`), which gives
 * everyone full control. Throws for a descriptor with no DACL to read, an empty one included.
 */
function daclEntries(sddl: string): string[] | null {
  const parsed = /D:([A-Z_]*)((?:\([^)]*\))*)/u.exec(sddl);
  if (parsed === null) throw new Error("the security descriptor has no DACL to read");
  if (parsed[1]?.includes("NO_ACCESS_CONTROL") === true) return null;
  return [...(parsed[2] ?? "").matchAll(/\(([^)]*)\)/gu)].map(([, ace]) => ace ?? "");
}

/**
 * The accounts an SDDL security descriptor's DACL lets read the object, besides its owner (`O:`), `user`, SYSTEM,
 * Administrators and OWNER RIGHTS. Only allow entries that apply to the object itself count: an inherit-only entry
 * (`IO`) is for what a folder will hold. A NULL DACL reads as Everyone (`WD`): read as a DACL with no entries it would
 * pass as owner-only. A descriptor with no DACL to read throws, as the caller cannot tell who may read the object.
 */
export function sddlOtherReaders(sddl: string, user: string): string[] {
  const entries = daclEntries(sddl);
  if (entries === null) return ["WD"];
  const owner = /^O:([^:]+?)(?=[GDS]:|$)/u.exec(sddl)?.[1];
  const trusted = new Set([owner, user, SYSTEM_SID, ADMINISTRATORS_SID, ...TRUSTED_ALIASES]);
  const readers = entries.map(readerOf).filter((sid): sid is string => sid !== undefined && !trusted.has(sid));
  return [...new Set(readers)];
}

const EVERYONE_SID = "S-1-1-0";
/** The SIDs of the SDDL short names a folder's ACL may name an account by; `icacls /grant` takes a SID, not these. */
const ALIAS_SIDS: Readonly<Record<string, string>> = {
  WD: EVERYONE_SID, AU: "S-1-5-11", BU: "S-1-5-32-545", IU: "S-1-5-4", NU: "S-1-5-2", BG: "S-1-5-32-546", PU: "S-1-5-32-547",
  RD: "S-1-5-32-555", AN: "S-1-5-7", AC: "S-1-15-2-1", SY: SYSTEM_SID, BA: ADMINISTRATORS_SID, CO: CREATOR_OWNER_SID, OW: OWNER_RIGHTS_SID,
};
/** FILE_ADD_SUBDIRECTORY, GENERIC_ALL and GENERIC_WRITE: the access-mask bits that let an account make a folder. */
const ADD_FOLDER_BITS = 0x4 | 0x1000_0000 | 0x4000_0000;
/** SDDL right codes that carry one of `ADD_FOLDER_BITS`; `LC` is the directory-service name for bit 0x4. */
const ADD_FOLDER_CODES: ReadonlySet<string> = new Set(["FA", "FW", "GA", "GW", "LC"]);

function grantsAddFolder(rights: string): boolean {
  if (/^0x[0-9a-f]+$/iu.test(rights)) return (Number.parseInt(rights.slice(2), 16) & ADD_FOLDER_BITS) !== 0;
  return (rights.match(/[A-Z]{2}/gu) ?? []).some(code => ADD_FOLDER_CODES.has(code));
}

/** The account, by SID or short name, an access control entry lets make a folder in the folder itself: an allow entry, not inherit-only (`IO`). */
function writerOf(ace: string): string | undefined {
  const [type = "", flags = "", rights = "", , , account = ""] = ace.split(";");
  const allowsObject = (type === "A" || type === "OA") && !/IO/u.test(flags);
  return allowsObject && grantsAddFolder(rights) ? account : undefined;
}

/**
 * The accounts, by SID, an SDDL security descriptor's DACL lets make a folder in the folder itself, besides `user`,
 * SYSTEM, Administrators, CREATOR OWNER and OWNER RIGHTS. A NULL DACL lets everyone (`S-1-1-0`). A short name with no
 * SID known here is left out, as `icacls` could not grant it.
 */
export function sddlWriters(sddl: string, user: string): string[] {
  const entries = daclEntries(sddl);
  if (entries === null) return [EVERYONE_SID];
  const trusted = new Set([user, SYSTEM_SID, ADMINISTRATORS_SID, CREATOR_OWNER_SID, OWNER_RIGHTS_SID]);
  const sids = entries.map(writerOf).map(account => (account === undefined ? undefined : ALIAS_SIDS[account] ?? account));
  return [...new Set(sids.filter((sid): sid is string => sid?.startsWith("S-1-") === true && !trusted.has(sid)))];
}

/**
 * Whether what `icacls <path>` prints marks an entry inherited. Each entry is a line ending in `<account>:` and the
 * entry's codes in brackets, which `icacls` prints alike in every language: `(I)` for an inherited one. Only the text
 * after a line's last colon is read, so neither the path, which starts the first line, nor an account name counts.
 */
export function icaclsInherits(output: string): boolean {
  return output.split(/\r?\n/u).some(line => line.slice(line.lastIndexOf(":") + 1).includes("(I)"));
}

/** The `icacls` arguments that give `path` an ACL of its own granting this user, SYSTEM and Administrators alone. */
function ownerOnlyArguments(path: string, folder: boolean): string[] {
  const inherit = folder ? "(OI)(CI)" : "";
  return [
    path, "/inheritance:r", "/grant:r",
    `*${currentUserSid()}:${inherit}F`, `*${SYSTEM_SID}:${inherit}F`, `*${ADMINISTRATORS_SID}:${inherit}F`, "/q",
  ];
}

const icaclsLater = promisify(execFile);

/** Runs `icacls` with `args`, holding no thread; a refusal's error message carries what `icacls` said on stderr. */
async function icacls(args: readonly string[]): Promise<string> {
  const { stdout } = await icaclsLater(windowsSystemProgram("icacls.exe"), args, { encoding: "utf8", timeout: 30_000 });
  return stdout;
}

/**
 * The SDDL `icacls <path> /save` writes, in UTF-16, to a file of its own in the temporary folder: the path's DACL with
 * every account by SID or short name, which the lines `icacls <path>` prints name in the system's language.
 */
async function savedSddl(path: string): Promise<string> {
  const saved = join(tmpdir(), `rerail-acl-${process.pid}-${randomUUID()}.txt`);
  try {
    await icacls([path, "/save", saved, "/q"]);
    const lines = (await readFile(saved)).toString("utf16le").split(/\r?\n/u);
    return lines.find(line => line.includes("D:")) ?? "";
  } finally {
    await rm(saved, { force: true });
  }
}

/** Windows' own access control, through `icacls` and `Get-Acl`. */
export const WINDOWS_ACL: WindowsAcl = {
  restrictToOwner(path, folder) {
    // `icacls` says why it failed ("Access is denied.") on stderr, which the thrown error's message then carries.
    execFileSync(windowsSystemProgram("icacls.exe"), ownerOnlyArguments(path, folder), { stdio: ["ignore", "ignore", "pipe"], timeout: 30_000 });
  },
  async restrictFolderToOwner(path) {
    // The rejected error's message carries `icacls`'s stderr, as `restrictToOwner`'s does.
    await icaclsLater(windowsSystemProgram("icacls.exe"), ownerOnlyArguments(path, true), { encoding: "utf8" });
  },
  otherReaders(path) {
    // The path goes in the environment, so no quoting of it reaches PowerShell's parser.
    const sddl = execFileSync(windowsPowerShell(), [
      "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "(Get-Acl -LiteralPath $env:RERAIL_ACL_PATH).Sddl",
    ], { encoding: "utf8", env: { ...process.env, RERAIL_ACL_PATH: path }, stdio: ["ignore", "pipe", "pipe"], timeout: 30_000 });
    return sddlOtherReaders(sddl.trim(), currentUserSid());
  },
  async inherits(path) {
    // `icacls` starts in milliseconds where Windows PowerShell's `Get-Acl` takes up to seconds.
    return icaclsInherits(await icacls([path]));
  },
  async writers(path) {
    return sddlWriters(await savedSddl(path), currentUserSid());
  },
  async keepEachOwnersOwn(path, writers) {
    // No time limit: Windows passes the new ACL to everything already in the folder, as `restrictFolderToOwner` does.
    await icaclsLater(windowsSystemProgram("icacls.exe"), [
      path, "/inheritance:r", "/grant:r", `*${currentUserSid()}:F`, `*${CREATOR_OWNER_SID}:(OI)(CI)(IO)F`,
      `*${SYSTEM_SID}:(OI)(CI)F`, `*${ADMINISTRATORS_SID}:(OI)(CI)F`, ...writers.map(sid => `*${sid}:(X,AD,RA,RC,S)`), "/q",
    ], { encoding: "utf8" });
  },
  async letAddFiles(path, writers) {
    await icacls([path, "/grant", ...writers.map(sid => `*${sid}:(X,WD,RA,S)`), "/q"]);
  },
};

/** Whether `path` is under this user's profile folder, whose ACL already grants this user, SYSTEM and Administrators alone. */
export function insideProfile(path: string, profile: string): boolean {
  const below = win32.relative(profile.toLowerCase(), path.toLowerCase());
  return below === "" || (!below.startsWith("..") && !win32.isAbsolute(below));
}
