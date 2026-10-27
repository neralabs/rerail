import { readFileSync } from "node:fs";
import { gitExecutable, GitUnavailableError } from "../engine/git.js";
import {
  renderGitMissing,
  renderUnsupportedPlatform,
  renderWslNote,
  type SetupLane,
  type UnsupportedPlatform,
} from "../harness/setup-screens.js";

/**
 * The oldest glibc Rerail runs on: what Node 24's own Linux binaries need. The engines load on older ones still
 * (`.github/workflows/package.yml` builds them with napi's glibc 2.17 toolchain and fails the build if one needs more
 * than 2.28, the same number), so change the two together.
 */
export const MIN_LINUX_GLIBC = "2.28";

/** The systems and processors `package.yml` builds an engine for. */
const SUPPORTED: Readonly<Partial<Record<NodeJS.Platform, readonly string[]>>> = {
  darwin: ["arm64", "x64"],
  linux: ["arm64", "x64"],
  win32: ["arm64", "x64"],
};

const SYSTEM_NAMES: Readonly<Partial<Record<NodeJS.Platform, string>>> = { darwin: "macOS", linux: "Linux", win32: "Windows" };

export interface PlatformFacts {
  readonly platform: NodeJS.Platform;
  readonly arch: string;
  /** Linux: the glibc Node runs on; undefined on musl, which has none. */
  readonly glibc?: string;
}

export function platformFacts(): PlatformFacts {
  if (process.platform !== "linux") return { platform: process.platform, arch: process.arch };
  // Node reports the glibc it runs on, and leaves the field out on musl.
  const report = process.report.getReport();
  const header = "header" in report && typeof report.header === "object" && report.header !== null ? report.header : {};
  const glibc = "glibcVersionRuntime" in header && typeof header.glibcVersionRuntime === "string" ? header.glibcVersionRuntime : undefined;
  return { platform: process.platform, arch: process.arch, ...(glibc === undefined ? {} : { glibc }) };
}

/** `2.36` against `2.39`, numerically part by part. */
function olderThan(version: string, minimum: string): boolean {
  const found = version.split(".").map(part => Number.parseInt(part, 10));
  const wanted = minimum.split(".").map(part => Number.parseInt(part, 10));
  for (let index = 0; index < wanted.length; index += 1) {
    const difference = (found[index] ?? 0) - wanted[index]!;
    if (Number.isNaN(difference)) return true;
    if (difference !== 0) return difference < 0;
  }
  return false;
}

/** Why Rerail's engine cannot load here, or undefined where it can. */
export function unsupportedPlatform(facts: PlatformFacts): UnsupportedPlatform | undefined {
  if (!SUPPORTED[facts.platform]?.includes(facts.arch)) {
    return { kind: "system", platform: SYSTEM_NAMES[facts.platform] ?? facts.platform, arch: facts.arch };
  }
  if (facts.platform !== "linux") return undefined;
  if (facts.glibc === undefined) return { kind: "musl" };
  return olderThan(facts.glibc, MIN_LINUX_GLIBC) ? { kind: "glibc", found: facts.glibc } : undefined;
}

/** The command that installs Git here, or undefined when set-up can't tell which package manager this Linux uses. */
export function gitInstallCommand(platform: NodeJS.Platform, osRelease: string | undefined, root: boolean): string | undefined {
  if (platform === "darwin") return "xcode-select --install";
  if (platform === "win32") return "winget install --id Git.Git -e";
  if (osRelease === undefined) return undefined;
  const ids = new Set(
    osRelease.split("\n")
      .map(line => /^(?:ID|ID_LIKE)=(.*)$/u.exec(line.trim())?.[1])
      .filter(value => value !== undefined)
      .flatMap(value => value.replace(/^["']|["']$/gu, "").split(/\s+/u)),
  );
  const sudo = root ? "" : "sudo ";
  if (ids.has("debian") || ids.has("ubuntu")) return `${sudo}apt-get install git`;
  if (ids.has("fedora") || ids.has("rhel") || ids.has("centos")) return `${sudo}dnf install git`;
  if (ids.has("arch")) return `${sudo}pacman -S git`;
  if (ids.has("suse") || ids.has("opensuse")) return `${sudo}zypper install git`;
  return undefined;
}

function readOptional(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

/**
 * What stops set-up before it installs anything, as the screen to show; undefined when nothing does. An engine that
 * cannot load here would otherwise fail the install's health check with a loader stack, and without Git the install
 * fails to start its health-check repository.
 */
export function setupPreflight(lane: SetupLane, facts: PlatformFacts = platformFacts()): string | undefined {
  const problem = unsupportedPlatform(facts);
  if (problem !== undefined) return renderUnsupportedPlatform({ problem, minGlibc: MIN_LINUX_GLIBC }, lane);
  try {
    gitExecutable();
  } catch (error) {
    if (!(error instanceof GitUnavailableError)) throw error;
    const installCommand = gitInstallCommand(
      facts.platform,
      facts.platform === "linux" ? readOptional("/etc/os-release") : undefined,
      process.getuid?.() === 0,
    );
    return renderGitMissing({ looked: error.looked, ...(installCommand === undefined ? {} : { installCommand }) }, lane);
  }
  return undefined;
}

/** Whether this is Linux inside WSL: WSL names the distribution, and its kernel says Microsoft. */
export function insideWsl(): boolean {
  if (process.platform !== "linux") return false;
  if (process.env.WSL_DISTRO_NAME) return true;
  return /microsoft/iu.test(readOptional("/proc/version") ?? "");
}

/** Inside WSL, the note for a folder on a Windows drive (`/mnt/c/…`); undefined anywhere else. */
export function wslNote(folder: string, lane: SetupLane, wsl: boolean = insideWsl()): string | undefined {
  if (!wsl || !/^\/mnt\/[a-z](?:\/|$)/iu.test(folder)) return undefined;
  return renderWslNote({ folder }, lane);
}
