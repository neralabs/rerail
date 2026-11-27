/**
 * A Rerail home and user home of their own, with the repository `app` set up in it, for cases that run the built CLI
 * and a real daemon as a person does; and a second copy of the package sharing that home, as the version `rerail
 * update` replaced, or a managed install, runs from its own folder.
 */
import { execFile, execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const run = promisify(execFile);
export const PACKAGE_ROOT = realpathSync.native(process.cwd());
const CLI = join("dist", "cli", "cli.js");
const places: Place[] = [];

/** A Rerail home and user home of their own, and the repository `app` set up there with no rules. */
export interface Place {
  root: string;
  home: string;
  user: string;
  repo: string;
  repositoryKey: string;
  env: NodeJS.ProcessEnv;
}

/** How a place is made: Rerail started in `app` or not, and autostart on, as a person's shell has it, or `0`. */
export interface PlaceOptions {
  started?: boolean;
  autostart?: "on" | "0";
}

/** A new place, removed by {@link removePlaces}. */
export function makePlace({ started = true, autostart = "on" }: PlaceOptions = {}): Place {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-start-link-")));
  const user = join(root, "user-home");
  const home = join(root, "rerail-home");
  const repo = join(root, "app");
  for (const folder of [user, home, repo]) mkdirSync(folder);
  const env: NodeJS.ProcessEnv = { ...process.env, RERAIL_HOME: home, HOME: user, USERPROFILE: user, BROWSER: "none", RERAIL_AUTOSTART: autostart };
  // A person's shell has autostart on; the test run's own setting keeps every other test from starting a daemon.
  if (autostart === "on") delete env.RERAIL_AUTOSTART;
  writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-11-01T00:00:00.000Z" } }));
  writeFileSync(join(repo, "a.py"), "x = 1\n");
  const git = ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid"];
  for (const args of [["init", "-q"], ["add", "a.py"], ["commit", "-q", "-m", "base"]]) execFileSync("git", [...git, ...args], { cwd: repo, stdio: "ignore" });
  const repositoryKey = setUpTestRepository(repo, started ? {} : { start: false }, join(home, "rerail.sqlite"));
  const place = { root, home, user, repo, repositoryKey, env };
  places.push(place);
  return place;
}

/** Stops whichever daemon each place made since the last call runs, this package's or the other's, then removes it. */
export async function removePlaces(): Promise<void> {
  for (const place of places.splice(0)) {
    await rerail(PACKAGE_ROOT, place, ["daemon", "stop"]);
    await removeFolder(place.root);
  }
}

/** `rerail <args>` in the repository, from the package at `packageRoot`: its exit code and stdout. */
export async function rerail(packageRoot: string, place: Place, args: readonly string[]): Promise<{ code: number; stdout: string }> {
  try {
    return { code: 0, stdout: (await run(process.execPath, [join(packageRoot, CLI), ...args], { cwd: place.repo, env: place.env })).stdout };
  } catch (error) {
    const failed = error as { code?: unknown; stdout?: string };
    return { code: typeof failed.code === "number" ? failed.code : -1, stdout: failed.stdout ?? "" };
  }
}

/** The daemon the home records: its pid, the real path of the package it runs from, and its origin. */
export function recorded(place: Place): { pid: number; packageRoot: string; url: string } | undefined {
  try {
    const { pid, packageRoot, url } = JSON.parse(readFileSync(join(place.home, "daemon.json"), "utf8")) as { pid: number; packageRoot: string; url: string };
    return { pid, packageRoot: realpathSync.native(packageRoot), url };
  } catch {
    return undefined;
  }
}

/**
 * A copy of this package in `folder` (the place's `other-package` when none is named), another daemon runs from: its
 * own `dist` and `package.json`, and links to the rest.
 */
export function otherPackage(place: Place, folder = join(place.root, "other-package")): string {
  mkdirSync(folder, { recursive: true });
  cpSync(join(PACKAGE_ROOT, "package.json"), join(folder, "package.json"));
  cpSync(join(PACKAGE_ROOT, "dist"), join(folder, "dist"), { recursive: true });
  for (const entry of readdirSync(PACKAGE_ROOT)) {
    // A junction on Windows, which an account without the right to make symbolic links can make.
    if (["node_modules", "native", "model", "packs", "docs", "hooks", "skills"].includes(entry)) symlinkSync(join(PACKAGE_ROOT, entry), join(folder, entry), "junction");
  }
  return realpathSync.native(folder);
}

/** Whether `link` answers as a browser opening it finds: any HTTP answer, or the error that came instead. */
export async function answerOf(link: string): Promise<string> {
  try {
    await fetch(link, { signal: AbortSignal.timeout(10_000) });
    return "answers";
  } catch (error) {
    return `no answer (${(error as { cause?: { code?: string } }).cause?.code ?? String(error)})`;
  }
}
