import { EventEmitter } from "node:events";
import type { FSWatcher, watch as watchFolder } from "node:fs";

/**
 * Folder watches a test drives in place of `node:fs`'s: `report(folder, name)` is the OS saying `name` in `folder`
 * changed, so a test says when a notice comes, before or after what it tells of, on every system alike.
 */
export function drivenWatch(): { watch: typeof watchFolder; report: (folder: string, name: string) => void } {
  const listeners = new Map<string, (event: string, name: string) => void>();
  const watch = ((folder: string, _options: unknown, onChange: (event: string, name: string) => void) => {
    listeners.set(folder, onChange);
    const watcher = new EventEmitter() as FSWatcher;
    watcher.close = () => { listeners.delete(folder); };
    return watcher;
  }) as unknown as typeof watchFolder;
  return { watch, report: (folder, name) => listeners.get(folder)?.("change", name) };
}
