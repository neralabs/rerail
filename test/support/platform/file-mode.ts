/**
 * Whether files and folders here carry POSIX permission bits. Windows keeps none: Node reports 0o666 for a file, or 0o444
 * once it is read-only, whatever mode made it. A private file there is one in a folder whose ACL grants its owner alone
 * (`keepFolderToOwner`), which test/engine/storage/windows-acl.test.ts checks.
 */
export const POSIX_MODES = process.platform !== "win32";

/** Why a test of permission bits alone does not run on Windows. */
export const NO_POSIX_MODES = "Windows keeps no POSIX permission bits; a private file there is kept so by its folder's ACL";
