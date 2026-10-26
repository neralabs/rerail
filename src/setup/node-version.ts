import { renderNodeTooOld, type SetupLane } from "../harness/setup-screens.js";

/** The oldest Node major Rerail runs on; matches `engines.node` in package.json. */
export const REQUIRED_NODE_MAJOR = 24;

export type NodeVersionCheck =
  | { readonly ok: true; readonly major: number }
  | { readonly ok: false; readonly found: string; readonly requiredMajor: number; readonly message: string };

/** `24`, `v24.1.0`, `24.0.0-nightly20250101abc`; the major is the leading integer. */
const NODE_VERSION = /^v?(\d+)(?:\.\d+){0,2}(?:[-+][0-9A-Za-z.-]*)?$/;

/** Where the running Node came from: its binary, and the environment a version manager names itself in. */
export interface NodeOrigin {
  readonly execPath: string;
  readonly env: NodeJS.ProcessEnv;
}

/**
 * The command that upgrades Node the way this one was installed, or undefined when nothing says how. The binary's
 * own path decides first (Node resolves it, so it names the version manager's or Homebrew's directory); a version
 * manager's variable alone still means the person has it.
 */
export function nodeUpgradeCommand(origin: NodeOrigin, major: number): string | undefined {
  const path = origin.execPath.replaceAll("\\", "/");
  const under = (...roots: (string | undefined)[]): boolean =>
    roots.some(root => root !== undefined && root !== "" && path.startsWith(`${root.replaceAll("\\", "/").replace(/\/+$/u, "")}/`));
  const { VOLTA_HOME, FNM_DIR, NVM_DIR, NVM_HOME, NVM_SYMLINK } = origin.env;
  const volta = `volta install node@${major}`;
  const fnm = `fnm install ${major} && fnm use ${major}`;
  const nvm = `nvm install ${major}`;
  if (under(VOLTA_HOME) || path.includes("/.volta/")) return volta;
  if (under(FNM_DIR) || path.includes("/fnm/node-versions/")) return fnm;
  if (under(NVM_DIR, NVM_HOME, NVM_SYMLINK) || path.includes("/.nvm/")) return nvm;
  // `node` is Homebrew's current Node; a versioned formula (`node@22`) stays on its major, so the answer is `node`.
  if (path.includes("/Cellar/node/")) return "brew upgrade node";
  if (path.includes("/Cellar/") || path.startsWith("/opt/homebrew/") || path.startsWith("/home/linuxbrew/.linuxbrew/")) return "brew install node";
  if (VOLTA_HOME) return volta;
  if (FNM_DIR) return fnm;
  if (NVM_DIR || NVM_HOME) return nvm;
  return undefined;
}

/**
 * The explicit check npm's `engines` only warns about. Pass
 * `process.versions.node` (or `process.version`). A version that cannot be
 * read fails the check: set-up stops rather than guessing.
 */
export function checkNodeVersion(
  found: string,
  lane: SetupLane,
  requiredMajor: number = REQUIRED_NODE_MAJOR,
  origin: NodeOrigin = { execPath: process.execPath, env: process.env },
): NodeVersionCheck {
  const version = found.trim();
  const majorText = NODE_VERSION.exec(version)?.[1];
  const major = majorText === undefined ? Number.NaN : Number.parseInt(majorText, 10);
  if (major >= requiredMajor) return { ok: true, major };
  const shown = version.replace(/^v/, "") || "unknown";
  const upgradeCommand = nodeUpgradeCommand(origin, requiredMajor);
  return {
    ok: false,
    found: shown,
    requiredMajor,
    message: renderNodeTooOld({ found: shown, requiredMajor, ...(upgradeCommand === undefined ? {} : { upgradeCommand }) }, lane),
  };
}
