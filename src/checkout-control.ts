import { existsSync } from "node:fs";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { isAbsolute, join, relative, resolve, sep } from "node:path";

export const RERAIL_DISABLED_MARKER = join(".rerail", "disabled");

export function rerailDisabledMarker(workspace: string): string {
  return resolve(workspace, RERAIL_DISABLED_MARKER);
}

export function isRerailDisabled(workspace: string): boolean {
  return existsSync(rerailDisabledMarker(workspace));
}

/** True when `candidate` is `root` or below it; canonicalize both first when symlinked identity matters. */
export function withinCheckout(root: string, candidate: string): boolean {
  const path = relative(root, resolve(candidate));
  return !isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`);
}

export async function disableRerail(workspace: string): Promise<string> {
  const marker = rerailDisabledMarker(workspace);
  await mkdir(join(resolve(workspace), ".rerail"), { recursive: true });
  await writeFile(marker, "");
  return marker;
}

export async function enableRerail(workspace: string): Promise<string> {
  const marker = rerailDisabledMarker(workspace);
  await rm(marker, { force: true });
  return marker;
}
