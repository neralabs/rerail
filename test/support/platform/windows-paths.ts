import { win32 } from "node:path";

/**
 * `folder` in a form Windows reads as fully qualified, naming one folder from any current folder: as it is on Windows;
 * on Linux and macOS as a server path (`//tmp/...`), which they read as the same folder. A test that plays Windows
 * there hands it to a PATH search or a program start that reads paths as Windows does.
 */
export function fullyQualified(folder: string): string {
  return process.platform === "win32" ? folder : `/${folder}`;
}

/**
 * `folder` from the root of the current drive, as Windows reads a path such as `\Users\me`: without its drive. A Linux
 * or macOS path already is one.
 */
export function fromDriveRoot(folder: string): string {
  return folder.slice(win32.parse(folder).root.length - 1);
}
