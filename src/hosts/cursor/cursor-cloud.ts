import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { replaceFile } from "../../engine/file-replace.js";
import { defaultRerailDatabasePath } from "../../engine/storage/database.js";

/**
 * The marker `rerail start --cursor-cloud` leaves in the Rerail home: this machine is a Cursor cloud agent's VM, where
 * Cursor fires no `sessionStart` or `sessionEnd` (docs/cursor-cloud-agents-prd.md). Its Cursor hooks then wait for the
 * daemon as a session's start does, and a session's first hooks deliver the note its start would have.
 */
export const CURSOR_CLOUD_MARKER = "cursor-cloud.json";

/** The marker's path, beside the database the hooks use: `$RERAIL_HOME/cursor-cloud.json` by default. */
export function cursorCloudMarkerPath(databasePath: string = defaultRerailDatabasePath()): string {
  return join(dirname(databasePath), CURSOR_CLOUD_MARKER);
}

/** Whether this machine is a Cursor cloud VM, as `rerail start --cursor-cloud` marked it. One `stat` per hook. */
export function isCursorCloud(databasePath?: string): boolean {
  return existsSync(cursorCloudMarkerPath(databasePath));
}

/** Marks this machine as a Cursor cloud VM. Writing it again keeps the first time it was marked. */
export async function markCursorCloud(databasePath?: string, now: number = Date.now()): Promise<void> {
  const path = cursorCloudMarkerPath(databasePath);
  if (existsSync(path)) return;
  await mkdir(dirname(path), { recursive: true });
  await replaceFile(path, `${JSON.stringify({ schema: "rerail.cursor-cloud/1", markedAt: now })}\n`, { mode: 0o600 });
}
