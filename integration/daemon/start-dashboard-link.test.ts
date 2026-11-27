/**
 * `rerail start` and `rerail start --json` in a repository set up and started before, through the built CLI and a real
 * daemon, as a person runs them: autostart on, a Rerail home and a user home of their own, no browser. `--json` changes
 * the form of the answer only: each form ends on the dashboard's link and leaves the daemon as the other does. In OMP
 * and Pi the extension attaches its session again right after `/rerail-start`, and the link it showed still answers.
 */
import { execFile, execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, test, vi } from "vitest";
import { createNativeSession, type NativeSessionClient } from "../../src/harness/index.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const run = promisify(execFile);
const PACKAGE_ROOT = realpathSync.native(process.cwd());
const CLI = join("dist", "cli", "cli.js");
const ALREADY_ON = "● Rerail is already on in app. 0 rules are on.";
/** Six commands and up to three daemon starts, a few seconds each on a loaded Windows machine. */
const TEST_TIMEOUT_MS = 120_000;
const places: Place[] = [];

/** A Rerail home and user home of their own, and the repository `app` set up and started there with no rules. */
interface Place {
  root: string;
  home: string;
  repo: string;
  repositoryKey: string;
  env: NodeJS.ProcessEnv;
}

/** What a start left: its exit code, the lines it showed with the daemon's origin as `<daemon>/`, and that daemon. */
interface Outcome {
  code: number;
  lines: string[];
  daemon: string;
}

afterEach(async () => {
  for (const place of places.splice(0)) {
    // Whichever daemon runs, this package's or the other's, holds files under the folder until it ends.
    await rerail(PACKAGE_ROOT, place, ["daemon", "stop"]);
    await removeFolder(place.root);
  }
});

function makePlace(): Place {
  const root = realpathSync.native(mkdtempSync(join(tmpdir(), "rerail-start-link-")));
  const user = join(root, "user-home");
  const home = join(root, "rerail-home");
  const repo = join(root, "app");
  for (const folder of [user, home, repo]) mkdirSync(folder);
  const env: NodeJS.ProcessEnv = { ...process.env, RERAIL_HOME: home, HOME: user, USERPROFILE: user, BROWSER: "none" };
  // A person's shell has autostart on; the test run's own setting keeps every other test from starting a daemon.
  delete env.RERAIL_AUTOSTART;
  writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-11-01T00:00:00.000Z" } }));
  writeFileSync(join(repo, "a.py"), "x = 1\n");
  const git = ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid"];
  for (const args of [["init", "-q"], ["add", "a.py"], ["commit", "-q", "-m", "base"]]) execFileSync("git", [...git, ...args], { cwd: repo, stdio: "ignore" });
  const place = { root, home, repo, repositoryKey: setUpTestRepository(repo, {}, join(home, "rerail.sqlite")), env };
  places.push(place);
  return place;
}

/** `rerail <args>` in the repository, from the package at `packageRoot`: its exit code and stdout. */
async function rerail(packageRoot: string, place: Place, args: readonly string[]): Promise<{ code: number; stdout: string }> {
  try {
    return { code: 0, stdout: (await run(process.execPath, [join(packageRoot, CLI), ...args], { cwd: place.repo, env: place.env })).stdout };
  } catch (error) {
    const failed = error as { code?: unknown; stdout?: string };
    return { code: typeof failed.code === "number" ? failed.code : -1, stdout: failed.stdout ?? "" };
  }
}

/** The daemon the home records: its pid, the real path of the package it runs from, and its origin. */
function recorded(place: Place): { pid: number; packageRoot: string; url: string } | undefined {
  try {
    const { pid, packageRoot, url } = JSON.parse(readFileSync(join(place.home, "daemon.json"), "utf8")) as { pid: number; packageRoot: string; url: string };
    return { pid, packageRoot: realpathSync.native(packageRoot), url };
  } catch {
    return undefined;
  }
}

/** What a start showed: its text, or with `--json` its step's `screen`; a step not `done` shows whole, saying what it was. */
function shownLines(stdout: string, json: boolean): string[] {
  const step = json ? JSON.parse(stdout) as { step: string; screen: string; question: unknown } : undefined;
  const done = step !== undefined && step.step === "done" && step.question === null;
  return (done ? step.screen : stdout.trimEnd()).split("\n");
}

/** `rerail start`, or with `json` its `--json` form, from this package: the lines it showed and the daemon after it. */
async function start(place: Place, json: boolean): Promise<Outcome> {
  const { code, stdout } = await rerail(PACKAGE_ROOT, place, json ? ["start", "--json"] : ["start"]);
  const lines = shownLines(stdout, json);
  const daemon = recorded(place);
  if (daemon === undefined) return { code, lines, daemon: "none" };
  const whose = daemon.packageRoot === PACKAGE_ROOT ? "this" : "another";
  return { code, lines: lines.map(line => line.replace(daemon.url, "<daemon>/")), daemon: `${whose} package's, pid ${daemon.pid}` };
}

/** The rules page's link as a start shows it, scoped to the repository and its checkout. */
function rulesLink(place: Place): string {
  return `  <daemon>/?${new URLSearchParams({ repository: place.repositoryKey, checkout: place.repo })}#rules`;
}

/**
 * A copy of this package another daemon runs from, as a version `rerail update` replaced runs from its own folder: its
 * own `dist` and `package.json`, and links to the rest.
 */
function otherPackage(place: Place): string {
  const other = join(place.root, "other-package");
  mkdirSync(other);
  cpSync(join(PACKAGE_ROOT, "package.json"), join(other, "package.json"));
  cpSync(join(PACKAGE_ROOT, "dist"), join(other, "dist"), { recursive: true });
  for (const entry of readdirSync(PACKAGE_ROOT)) {
    // A junction on Windows, which an account without the right to make symbolic links can make.
    if (["node_modules", "native", "model", "packs", "docs", "hooks", "skills"].includes(entry)) symlinkSync(join(PACKAGE_ROOT, entry), join(other, entry), "junction");
  }
  return other;
}

/**
 * The attach OMP's and Pi's extension makes once `/rerail-start` ends (`createNativeSession`), from this package, with
 * the place's Rerail home and user home; the session it opens is closed again.
 */
async function attachAsTheExtension(place: Place): Promise<void> {
  for (const name of ["RERAIL_HOME", "HOME", "USERPROFILE"] as const) vi.stubEnv(name, place.env[name]);
  const opened: NativeSessionClient[] = [];
  try {
    await createNativeSession({
      current: () => undefined,
      matchesCurrent: () => false,
      close: async () => undefined,
      activate: client => opened.push(client),
      entries: () => [],
      appendEntry: () => undefined,
      deliver: () => undefined,
      reattach: async () => undefined,
    }, {}, { host: "omp", sessionId: "rerail-start-link", cwd: place.repo }, false, {});
  } finally {
    await Promise.all(opened.map(async client => await client.close()));
    vi.unstubAllEnvs();
  }
}

/** Whose daemon the home records, and whether it serves the origin of `link`. */
function daemonServing(place: Place, link: string): string {
  const daemon = recorded(place);
  if (daemon === undefined) return "none";
  const whose = daemon.packageRoot === PACKAGE_ROOT ? "this" : "another";
  return `${whose} package's, ${link.startsWith(daemon.url) ? "on the link's origin" : `on ${daemon.url}`}`;
}

/** Whether `link` answers as a browser opening it finds: any HTTP answer, or the error that came instead. */
async function answerOf(link: string): Promise<string> {
  try {
    await fetch(link, { signal: AbortSignal.timeout(10_000) });
    return "answers";
  } catch (error) {
    return `no answer (${(error as { cause?: { code?: string } }).cause?.code ?? String(error)})`;
  }
}

describe("rerail start in a repository set up before, with and without --json", () => {
  test("starts the active package's daemon where none runs, and ends on its link", async () => {
    const place = makePlace();
    const ended = { code: 0, lines: [ALREADY_ON, rulesLink(place)], daemon: expect.stringMatching(/^this package's, pid \d+$/u) };
    expect(await start(place, false)).toEqual(ended);
    expect((await rerail(PACKAGE_ROOT, place, ["daemon", "stop"])).code).toBe(0);
    expect(await start(place, true)).toEqual(ended);
  }, TEST_TIMEOUT_MS);

  // The daemon serves every repository and session of the home; a coding agent's next hook replaces it with this
  // package's, as `rerail update` does unless it was started with options a restart would drop.
  test("leaves running a daemon of another package, and ends on its link", async () => {
    const place = makePlace();
    const other = otherPackage(place);
    expect((await rerail(other, place, ["daemon", "start", "--port", "0"])).code).toBe(0);
    const before = recorded(place);
    expect(before?.packageRoot).toBe(realpathSync.native(other));
    const ended = { code: 0, lines: [ALREADY_ON, rulesLink(place)], daemon: `another package's, pid ${before?.pid}` };
    expect(await start(place, false)).toEqual(ended);
    expect(await start(place, true)).toEqual(ended);
  }, TEST_TIMEOUT_MS);

  // OMP's and Pi's `/rerail-start` runs `rerail start --json` from the extension's package, then the extension attaches
  // its session again, which replaces a daemon of another package with its own. The person opens the link next.
  test("in OMP and Pi, the link /rerail-start shows still answers once the extension attaches again", async () => {
    const place = makePlace();
    expect((await rerail(otherPackage(place), place, ["daemon", "start", "--port", "0"])).code).toBe(0);
    const step = await rerail(PACKAGE_ROOT, place, ["start", "--json"]);
    const link = shownLines(step.stdout, true).map(line => line.trim()).find(line => line.startsWith("http")) ?? `no link in ${step.stdout}`;
    await attachAsTheExtension(place);
    expect({ link: await answerOf(link), daemon: daemonServing(place, link) })
      .toEqual({ link: "answers", daemon: "this package's, on the link's origin" });
  }, TEST_TIMEOUT_MS);
});
