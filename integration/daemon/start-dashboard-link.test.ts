/**
 * `rerail start` and `rerail start --json` in a repository set up and started before, through the built CLI and a real
 * daemon, as a person runs them: autostart on, a Rerail home and a user home of their own, no browser. `--json` changes
 * the form of the answer only: each form ends on the dashboard's link and leaves the daemon as the other does. In OMP
 * and Pi the extension attaches its session again right after `/rerail-start`, and a coding agent's hook may replace
 * another package's daemon after `rerail start`: the link shown before still answers afterwards.
 */
import { spawn } from "node:child_process";
import { once } from "node:events";
import { join } from "node:path";
import { text } from "node:stream/consumers";
import { afterEach, describe, expect, test, vi } from "vitest";
import { createNativeSession, type NativeSessionClient } from "../../src/harness/index.js";
import { answerOf, makePlace, otherPackage, PACKAGE_ROOT, recorded, removePlaces, rerail, type Place } from "../support/rerail-place.js";

const ALREADY_ON = "● Rerail is already on in app. 0 rules are on.";
/** Six commands and up to three daemon starts, a few seconds each on a loaded Windows machine. */
const TEST_TIMEOUT_MS = 120_000;
/** Claude Code's hook command, as set-up writes it: the launcher, the node it runs on, and the hook. */
const CLAUDE_HOOK = [join(PACKAGE_ROOT, "dist", "harness", "hook-launcher.js"), process.execPath, join(PACKAGE_ROOT, "dist", "hosts", "claude", "claude-hook.js")];

/** What a start left: its exit code, the lines it showed with the daemon's origin as `<daemon>/`, and that daemon. */
interface Outcome {
  code: number;
  lines: string[];
  daemon: string;
}

afterEach(removePlaces);

/** What a start showed: its text, or with `--json` its step's `screen`; a step not `done` shows whole, saying what it was. */
function shownLines(stdout: string, json: boolean): string[] {
  const step = json ? JSON.parse(stdout) as { step: string; screen: string; question: unknown } : undefined;
  const done = step !== undefined && step.step === "done" && step.question === null;
  return (done ? step.screen : stdout.trimEnd()).split("\n");
}

/** The first link among a start's lines, as a person copies it. */
function linkIn(lines: readonly string[]): string {
  return lines.map(line => line.trim()).find(line => line.startsWith("http")) ?? `no link in ${lines.join(" | ")}`;
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
 * The attach OMP's and Pi's extension makes once `/rerail-start` ends (`createNativeSession`), from this package, with
 * the place's Rerail home and user home; the session it opens is closed again.
 */
async function attachAsTheExtension(place: Place): Promise<void> {
  for (const name of ["RERAIL_HOME", "HOME", "USERPROFILE"] as const) vi.stubEnv(name, place.env[name]);
  vi.stubEnv("RERAIL_AUTOSTART", "1");
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

/** A Claude Code session's start in the repository, through this package's built hook as Claude Code runs it. */
async function claudeSessionStart(place: Place): Promise<void> {
  const child = spawn(process.execPath, CLAUDE_HOOK, {
    cwd: place.repo, env: { ...place.env, CLAUDE_PROJECT_DIR: place.repo }, stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.end(JSON.stringify({ session_id: "s1", cwd: place.repo, hook_event_name: "SessionStart", source: "startup" }));
  await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
}

/** Whose daemon the home records, and whether it serves the origin of `link`. */
function daemonServing(place: Place, link: string): string {
  const daemon = recorded(place);
  if (daemon === undefined) return "none";
  const whose = daemon.packageRoot === PACKAGE_ROOT ? "this" : "another";
  return `${whose} package's, ${link.startsWith(daemon.url) ? "on the link's origin" : `on ${daemon.url}`}`;
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
    expect(before?.packageRoot).toBe(other);
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
    const link = linkIn(shownLines(step.stdout, true));
    await attachAsTheExtension(place);
    expect({ link: await answerOf(link), daemon: daemonServing(place, link) })
      .toEqual({ link: "answers", daemon: "this package's, on the link's origin" });
  }, TEST_TIMEOUT_MS);

  // Another package's daemon on a free port, as its OMP or Pi extension starts one: the home remembers no port for it. A
  // Claude Code, Codex or Cursor session of this package then replaces it with this package's.
  test("the link rerail start showed still answers once a coding agent's hook replaces another package's daemon on a free port", async () => {
    const place = makePlace();
    expect((await rerail(otherPackage(place), place, ["daemon", "start", "--port", "0"])).code).toBe(0);
    const link = linkIn(shownLines((await rerail(PACKAGE_ROOT, place, ["start"])).stdout, false));
    await claudeSessionStart(place);
    expect({ link: await answerOf(link), daemon: daemonServing(place, link) })
      .toEqual({ link: "answers", daemon: "this package's, on the link's origin" });
  }, TEST_TIMEOUT_MS);
});
