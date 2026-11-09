/**
 * Owner-only access on Windows, where mode bits mean nothing: `mkdir`'s and `open`'s modes are ignored, and a new file
 * or folder takes its parent's access control list (ACL). Under the profile folder that list grants this user, SYSTEM
 * and Administrators alone; a folder Windows makes elsewhere, under `C:\` say, lets every local user (Users,
 * Authenticated Users) read, and often change, what is in it. Rerail keeps private data in its home and in each
 * checkout's `.rerail\sessions`, and reads a daemon's TLS private key, wherever the person points them.
 *
 * Both work through programs Windows ships, by absolute path and with no window: `icacls` sets an ACL, Windows
 * PowerShell's `Get-Acl` reads one in SDDL, which names every account by SID whatever the system's language. This is
 * the Windows half of `private-files.ts`, which callers use.
 */
import { win32 } from "node:path";
import { execFileSync } from "../child-process.js";
import { windowsPowerShell, windowsSystemProgram } from "../system-programs.js";

const SYSTEM_SID = "S-1-5-18";
const ADMINISTRATORS_SID = "S-1-5-32-544";
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
}

let userSid: string | undefined;

/** This process's user SID, from `whoami /user`. */
function currentUserSid(): string {
  if (userSid !== undefined) return userSid;
  const output = execFileSync(windowsSystemProgram("whoami.exe"), ["/user", "/fo", "csv", "/nh"], {
    encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 10_000,
  });
  const sid = /S-1-5-\d+(?:-\d+)*/u.exec(output)?.[0];
  if (sid === undefined) throw new Error("whoami did not report this user's SID");
  userSid = sid;
  return sid;
}

/** Access-mask bits that let an account read a file's data: FILE_READ_DATA, GENERIC_ALL and GENERIC_READ. */
const READ_BITS = 0x1 | 0x1000_0000 | 0x8000_0000;
/** SDDL right codes that carry one of `READ_BITS`; `CC` is the directory-service name for bit 0x1. */
const READ_CODES: ReadonlySet<string> = new Set(["FA", "FR", "GA", "GR", "CC", "KA", "KR"]);

function grantsRead(rights: string): boolean {
  if (/^0x[0-9a-f]+$/iu.test(rights)) return (Number.parseInt(rights.slice(2), 16) & READ_BITS) !== 0;
  return (rights.match(/[A-Z]{2}/gu) ?? []).some(code => READ_CODES.has(code));
}

/**
 * The accounts an SDDL security descriptor's DACL lets read the object, besides its owner (`O:`), `user`, SYSTEM,
 * Administrators and OWNER RIGHTS. Only allow entries that apply to the object itself count: an inherit-only entry
 * (`IO`) is for what a folder will hold.
 */
export function sddlOtherReaders(sddl: string, user: string): string[] {
  const owner = /^O:([^:]+?)(?=[GDS]:|$)/u.exec(sddl)?.[1];
  const dacl = /D:[A-Z]*((?:\([^)]*\))*)/u.exec(sddl)?.[1] ?? "";
  const readers = new Set<string>();
  for (const [, ace] of dacl.matchAll(/\(([^)]*)\)/gu)) {
    const [type = "", flags = "", rights = "", , , sid = ""] = (ace ?? "").split(";");
    if ((type !== "A" && type !== "OA") || /IO/u.test(flags) || !grantsRead(rights)) continue;
    if (sid === owner || sid === user || sid === SYSTEM_SID || sid === ADMINISTRATORS_SID || TRUSTED_ALIASES.has(sid)) continue;
    readers.add(sid);
  }
  return [...readers];
}

/** Windows' own access control, through `icacls` and `Get-Acl`. */
export const WINDOWS_ACL: WindowsAcl = {
  restrictToOwner(path, folder) {
    const inherit = folder ? "(OI)(CI)" : "";
    execFileSync(windowsSystemProgram("icacls.exe"), [
      path, "/inheritance:r", "/grant:r",
      `*${currentUserSid()}:${inherit}F`, `*${SYSTEM_SID}:${inherit}F`, `*${ADMINISTRATORS_SID}:${inherit}F`, "/q",
    ], { stdio: "ignore", timeout: 30_000 });
  },
  otherReaders(path) {
    // The path goes in the environment, so no quoting of it reaches PowerShell's parser.
    const sddl = execFileSync(windowsPowerShell(), [
      "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "(Get-Acl -LiteralPath $env:RERAIL_ACL_PATH).Sddl",
    ], { encoding: "utf8", env: { ...process.env, RERAIL_ACL_PATH: path }, stdio: ["ignore", "pipe", "pipe"], timeout: 30_000 });
    return sddlOtherReaders(sddl.trim(), currentUserSid());
  },
};

/** Whether `path` is under this user's profile folder, whose ACL already grants this user, SYSTEM and Administrators alone. */
export function insideProfile(path: string, profile: string): boolean {
  const below = win32.relative(profile.toLowerCase(), path.toLowerCase());
  return below === "" || (!below.startsWith("..") && !win32.isAbsolute(below));
}
