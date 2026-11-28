import { accessSync, constants, existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { LAUNCHER_DIR } from "../engine/storage/rerail-home.js";

/** The line of `<home>/bin/rerail` naming the node set-up recorded (`shLauncher`): `NODE='<path>'`, each `'` as `'\''`. */
const SH_NODE = /^NODE='(.*)'$/mu;
/**
 * The line of `<home>/bin/rerail.cmd` naming it (`launcherShim`): `set "RERAIL_NODE=<path>"`, each `%` as `%%`, a
 * non-ASCII path inside the Rerail home spelled from the launcher's own folder, and nothing for one it cannot spell.
 */
const BATCH_NODE = /^set "RERAIL_NODE=(.*)"\r?$/mu;
const FROM_LAUNCHER_FOLDER = "%~dp0..\\";

/**
 * The node the `rerail` launcher in `home` starts whatever PATH holds: the one set-up recorded in it, while the
 * launcher's own test finds that file (`[ -x ]` in `bin/rerail`; `if exist` in `bin/rerail.cmd`, which cmd and
 * PowerShell run on Windows). Undefined where the launcher runs `node` from PATH instead, since that file is gone or it
 * records none it could spell, and where there is no launcher to read.
 */
export function launcherRecordedNode(home: string): string | undefined {
  const recorded = process.platform === "win32" ? batchRecordedNode(home) : shRecordedNode(home);
  return recorded !== undefined && launcherFinds(recorded) ? recorded : undefined;
}

function launcherText(home: string, name: string): string {
  try {
    return readFileSync(join(home, LAUNCHER_DIR, name), "utf8");
  } catch {
    return "";
  }
}

function shRecordedNode(home: string): string | undefined {
  return SH_NODE.exec(launcherText(home, "rerail"))?.[1]?.replaceAll(String.raw`'\''`, "'");
}

function batchRecordedNode(home: string): string | undefined {
  const spelled = BATCH_NODE.exec(launcherText(home, "rerail.cmd"))?.[1];
  if (spelled === undefined || spelled === "") return undefined;
  const unescaped = (text: string): string => text.replaceAll("%%", "%");
  return spelled.startsWith(FROM_LAUNCHER_FOLDER) ? join(home, unescaped(spelled.slice(FROM_LAUNCHER_FOLDER.length))) : unescaped(spelled);
}

function launcherFinds(path: string): boolean {
  if (process.platform === "win32") return existsSync(path);
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
