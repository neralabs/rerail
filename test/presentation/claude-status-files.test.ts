import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseApiStores } from "../../src/api/index.js";
import { defaultRerailDatabasePath, loadRuleCatalog, openRerailDatabase, ruleConfigurationSignature } from "../../src/engine/index.js";
import { keepClaudeStatusFiles, statusFileText, type ClaudeStatusFiles } from "../../src/presentation/claude-status-files.js";
import { fallbackPresentation, stoppedDaemonPresentation } from "../../src/presentation/session-presentation.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { NO_POSIX_MODES, POSIX_MODES } from "../support/platform/file-mode.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { setUpTestRepository } from "../support/repository-rules.js";
import { startTestRepository, stopTestRepository } from "../support/started-repository.js";

const ORIGIN = "http://127.0.0.1:4737";
const DAY_MS = 24 * 60 * 60 * 1_000;
/** A line as a terminal with no colour and no links shows it. */
const shown = (line: string): string => line.replace(/\u001b\]8;;[^\u0007]*\u0007|\u001b\[[0-9;]*m/gu, "");

let root: string;
let home: string;
let folder: string;
let workspace: string;
let databasePath: string;
let store: SqliteSessionStore;
let stores: DatabaseApiStores;
let keepers: ClaudeStatusFiles[];

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-status-files-"));
  home = join(root, "home");
  folder = join(home, "status");
  workspace = join(root, "workspace");
  await mkdir(workspace);
  vi.stubEnv("RERAIL_HOME", home);
  databasePath = defaultRerailDatabasePath();
  setUpTestRepository(workspace);
  store = new SqliteSessionStore(databasePath);
  stores = new DatabaseApiStores(databasePath, {
    health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    origin: () => ORIGIN,
  });
  keepers = [];
});

afterEach(async () => {
  await Promise.all(keepers.map(keeper => keeper.stop()));
  stores.close();
  store.close();
  vi.unstubAllEnvs();
  await removeFolder(root);
});

/** A keeper that draws only when the test asks, as the daemon's does on each change it sees. */
function keeper(): ClaudeStatusFiles {
  const kept = keepClaudeStatusFiles({ folder, stores, checkEveryMs: false });
  keepers.push(kept);
  return kept;
}

/** Records a Claude Code session in `cwd` as its session-start hook leaves it; with `rules`, its rule count too. */
function recordSession(sessionId: string, cwd = workspace, rules?: number): { host: "claude-code"; sessionId: string; cwd: string } {
  const session = { host: "claude-code" as const, sessionId, cwd };
  store.openSession(session);
  store.startHudTracking(session, true);
  if (rules === undefined) return session;
  const database = openRerailDatabase(databasePath);
  try {
    const checkout = stores.checkouts()(cwd)!;
    const ownerCheckoutPaths = [realpathSync(cwd)];
    const ruleSignature = ruleConfigurationSignature(database, loadRuleCatalog(), checkout.repositoryKey, ownerCheckoutPaths);
    store.recordRuleCoverage(session, { repositoryId: checkout.repositoryKey, ownerCheckoutPaths, activeRuleCount: rules, ruleSignature });
  } finally {
    database.$client.close();
  }
  return session;
}

/** Records a Claude Code session whose one event was observed at `lastEventAt`, and which never ended. */
function recordSilentSession(sessionId: string, lastEventAt: number): void {
  const session = { host: "claude-code" as const, sessionId, cwd: workspace };
  store.openSession(session, lastEventAt);
  store.append({ sessionId, kind: "user-prompt", payload: {}, observedAt: lastEventAt }, session);
}

/** The session's status file as its three lines; undefined when it has none. */
function drawn(sessionId: string): string[] | undefined {
  const path = join(folder, sessionId);
  return existsSync(path) ? readFileSync(path, "utf8").split("\n").slice(0, -1) : undefined;
}

describe("the Claude Code status files the daemon keeps", () => {
  it("draws a session's line once for each colour depth, with the links the status line has", async () => {
    recordSession("session-one", workspace, 3);
    await keeper().refresh();
    const [truecolor, palette, plain] = drawn("session-one")!;
    expect([truecolor, palette, plain].map(line => shown(line!))).toEqual(Array(3).fill("R E R A I L ● 3 rules"));
    expect(truecolor).toContain("\u001b[38;2;165;173;251mR");
    expect(palette).toContain("\u001b[38;5;147mR");
    expect(plain).not.toContain("\u001b[");
    const { repositoryKey } = stores.checkouts()(workspace)!;
    const scope = new URLSearchParams({ repository: repositoryKey, checkout: realpathSync(workspace) });
    expect(plain).toContain(`\u001b]8;;${ORIGIN}/?${scope}#rules\u00073 rules\u001b]8;;\u0007`);
    expect(plain).toMatch(/^\u001b\]8;;http:\/\/127\.0\.0\.1:4737\/\?[^\u0007]*sessionId=session-one&host=claude-code[^\u0007]*\u0007R E R A I L\u001b\]8;;\u0007 ● /u);
  });

  it.skipIf(!POSIX_MODES)(`keeps the files to their owner (${NO_POSIX_MODES} otherwise)`, async () => {
    recordSession("session-one");
    await keeper().refresh();
    expect(statSync(folder).mode & 0o777).toBe(0o700);
    expect(statSync(join(folder, "session-one")).mode & 0o777).toBe(0o600);
  });

  it("writes a file again when what its line shows changes, and only then", async () => {
    const session = recordSession("session-one", workspace, 3);
    const kept = keeper();
    await kept.refresh();
    const first = statSync(join(folder, "session-one")).ino;
    await kept.refresh();
    expect(statSync(join(folder, "session-one")).ino).toBe(first);
    store.recordSteer(session, "receipt", ["team:one", "team:two"]);
    await kept.refresh();
    expect(shown(drawn("session-one")![2]!)).toBe("R E R A I L ● 3 rules · steered 2×");
    expect(statSync(join(folder, "session-one")).ino).not.toBe(first);
  });

  it("draws nothing for a session where Rerail is not on or the footer is silent, and says a repository is not set up", async () => {
    const started = join(root, "started");
    const silent = join(root, "silent");
    for (const path of [started, silent]) mkdirSync(path);
    startTestRepository(started);
    setUpTestRepository(silent);
    recordSession("stopped-here", workspace, 3);
    recordSession("not-set-up", started);
    recordSession("silent", silent, 3);
    stopTestRepository(workspace);
    stores.preferences().upsert({ repository: realpathSync(silent), host: "claude-code", presence: "silent" });
    await keeper().refresh();
    expect(readFileSync(join(folder, "stopped-here"), "utf8")).toBe("");
    expect(readFileSync(join(folder, "silent"), "utf8")).toBe("");
    expect(drawn("not-set-up")!.map(shown)).toEqual(Array(3).fill("R E R A I L ○ not set up · rerail start"));
  });

  it("removes a session's file when the session ends", async () => {
    const session = recordSession("session-one", workspace, 3);
    recordSession("session-two", workspace, 3);
    const kept = keeper();
    await kept.refresh();
    store.closeSession(session.sessionId);
    await kept.refresh();
    expect(drawn("session-one")).toBeUndefined();
    expect(drawn("session-two")).toHaveLength(3);
    // The session goes on (Claude Code resumed it): its next event opens it again.
    store.openSession(session);
    await kept.refresh();
    expect(drawn("session-one")).toHaveLength(3);
  });

  it("draws the file of a session an earlier daemon's stop ended, and removes the ones no session will read again", async () => {
    mkdirSync(folder, { recursive: true });
    for (const sessionId of ["still-running", "silent-for-days", "reset-away"]) writeFileSync(join(folder, sessionId), "an earlier daemon's line\n");
    writeFileSync(join(folder, "still-running.123.2f0e9b7a-0000-4000-8000-000000000000.tmp"), "a write cut short");
    recordSession("still-running", workspace, 3);
    store.closeSession("still-running", Date.now() - 1_000);
    recordSilentSession("silent-for-days", Date.now() - 8 * DAY_MS);
    store.closeSession("silent-for-days", Date.now() - 1_000);
    const kept = keeper();
    await kept.refresh();
    expect(shown(drawn("still-running")![2]!)).toBe("R E R A I L ● 3 rules");
    expect(readdirSync(folder)).toEqual(["still-running"]);
    // Its Claude Code then exits without another event: the end reaches a session the database already held as ended.
    store.closeSession("still-running");
    await kept.refresh();
    expect(readdirSync(folder)).toEqual([]);
    // An ended session with no file gets none back on a later pass.
    await kept.refresh();
    expect(readdirSync(folder)).toEqual([]);
  });

  // Claude Code often exits before its end hook has run, so the database holds such a session open for good.
  it("keeps no file for a session with no event in a week whose end never arrived, and draws it again at its next event", async () => {
    recordSilentSession("gone", Date.now() - 8 * DAY_MS);
    recordSilentSession("away-for-the-weekend", Date.now() - 3 * DAY_MS);
    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, "gone"), "the line it had\n");
    const kept = keeper();
    await kept.refresh();
    expect(readdirSync(folder)).toEqual(["away-for-the-weekend"]);
    store.append({ sessionId: "gone", kind: "user-prompt", payload: {} });
    await kept.refresh();
    expect(readdirSync(folder).sort()).toEqual(["away-for-the-weekend", "gone"]);
  });

  it("draws a session with no event in half an hour once a minute, not on every change", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      recordSilentSession("quiet", Date.now() - 40 * 60 * 1_000);
      recordSession("active", workspace, 3);
      const kept = keeper();
      await kept.refresh();
      const quietLine = readFileSync(join(folder, "quiet"), "utf8");
      expect(quietLine).not.toBe("");
      stores.preferences().upsert({ repository: realpathSync(workspace), host: "claude-code", presence: "silent" });
      await kept.refresh();
      expect(readFileSync(join(folder, "active"), "utf8")).toBe("");
      expect(readFileSync(join(folder, "quiet"), "utf8")).toBe(quietLine);
      vi.setSystemTime(Date.now() + 61_000);
      await kept.refresh();
      expect(readFileSync(join(folder, "quiet"), "utf8")).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves every file saying what its session shows with no daemon when the daemon stops", async () => {
    const elsewhere = join(root, "elsewhere");
    mkdirSync(elsewhere);
    setUpTestRepository(elsewhere);
    recordSession("on-here", workspace, 3);
    recordSession("not-on-here", elsewhere, 3);
    stopTestRepository(elsewhere);
    const kept = keeper();
    await kept.refresh();
    await kept.stop();
    expect(readFileSync(join(folder, "on-here"), "utf8")).toBe(statusFileText(stoppedDaemonPresentation("started")));
    expect(drawn("on-here")!.map(shown)).toEqual(Array(3).fill("R E R A I L ✕ not running"));
    expect(readFileSync(join(folder, "not-on-here"), "utf8")).toBe("");
    // A stopped keeper draws no more.
    store.recordSteer({ host: "claude-code", sessionId: "on-here", cwd: workspace }, "late");
    await kept.refresh();
    expect(drawn("on-here")!.map(shown)).toEqual(Array(3).fill("R E R A I L ✕ not running"));
  });

  it("gives a session whose id cannot name a file no file", async () => {
    recordSession("../outside", workspace, 3);
    recordSession("with.dot", workspace, 3);
    await keeper().refresh();
    expect(existsSync(folder) ? readdirSync(folder) : []).toEqual([]);
    expect(existsSync(join(home, "outside"))).toBe(false);
  });
});

/**
 * The status script, read by the shell Claude Code starts for the status command (`. <script>`). PATH is empty, so a
 * line it prints came from shell built-ins alone: any program it tried to start would not be found.
 */
describe.skipIf(process.platform === "win32")("the status script Claude Code's shell reads", () => {
  const script = join(process.cwd(), "src", "hosts", "claude", "claude-status.sh");
  const shells = ["/bin/sh", "/bin/dash", "/bin/bash"].filter(shell => existsSync(shell));
  /** The colour depths the script tells apart, in the order of a status file's lines. */
  const terminals: Record<string, string>[] = [{ COLORTERM: "truecolor" }, {}, { NO_COLOR: "1", COLORTERM: "truecolor" }];

  function statusLine(shell: string, input: unknown, env: Record<string, string> = {}, before = ""): string {
    const run = spawnSync(shell, ["-c", `${before}. "${script}"`], {
      input: typeof input === "string" ? input : JSON.stringify(input), encoding: "utf8",
      env: { PATH: "", HOME: root, RERAIL_HOME: home, ...env },
    });
    expect({ status: run.status, stderr: run.stderr }).toEqual({ status: 0, stderr: "" });
    return run.stdout;
  }

  /** What Claude Code writes to the status command's stdin, as far as the script reads it. */
  const input = (sessionId: string): object => ({ cwd: workspace, session_id: sessionId, session_name: "a \"session_id\": \"decoy\"", workspace: { project_dir: workspace } });

  it.each(shells)("%s prints the line the daemon drew for the terminal's colour depth", async (shell) => {
    recordSession("0f9c1d2e-aaaa-4bbb-8ccc-123456789abc", workspace, 3);
    await keeper().refresh();
    const lines = drawn("0f9c1d2e-aaaa-4bbb-8ccc-123456789abc")!;
    for (const [depth, env] of terminals.entries()) {
      expect(statusLine(shell, input("0f9c1d2e-aaaa-4bbb-8ccc-123456789abc"), env)).toBe(`${lines[depth]}\n`);
    }
    // Claude Code's JSON on several lines reads the same.
    expect(statusLine(shell, JSON.stringify(input("0f9c1d2e-aaaa-4bbb-8ccc-123456789abc"), null, 2), terminals[2])).toBe(`${lines[2]}\n`);
  });

  it.each(shells)("%s prints nothing for a session whose file is empty", async (shell) => {
    recordSession("stopped-here", workspace, 3);
    stopTestRepository(workspace);
    await keeper().refresh();
    expect(statusLine(shell, input("stopped-here"))).toBe("");
  });

  it.each(shells)("%s says not running for a session with no file while no daemon is recorded, and nothing while one is", (shell) => {
    const notRunning = statusFileText(stoppedDaemonPresentation("started")).split("\n");
    for (const [depth, env] of terminals.entries()) expect(statusLine(shell, input("no-file"), env)).toBe(`${notRunning[depth]}\n`);
    expect(statusLine(shell, input("no-file"), { NO_COLOR: "1" })).toBe("R E R A I L ✕ not running\n");
    // A daemon that runs keeps no line for a session where Rerail is not on, or whose start it has not recorded yet.
    writeFileSync(join(home, "daemon.json"), "{}");
    expect(statusLine(shell, input("no-file"))).toBe("");
  });

  it.each(shells)("%s takes no session id that could name a file outside the status folder", (shell) => {
    mkdirSync(join(folder, "nested"), { recursive: true });
    writeFileSync(join(folder, "nested", "session"), "a nested file\na nested file\na nested file\n");
    writeFileSync(join(home, "daemon.json"), "the daemon's capabilities\nthe daemon's capabilities\nthe daemon's capabilities\n");
    const unavailable = statusFileText(fallbackPresentation("session unavailable")).split("\n");
    for (const sessionId of ["nested/session", "../daemon.json", "..", ""]) {
      for (const [depth, env] of terminals.entries()) expect(statusLine(shell, input(sessionId), env)).toBe(`${unavailable[depth]}\n`);
    }
    expect(statusLine(shell, "not JSON", { NO_COLOR: "1" })).toBe("R E R A I L ● session unavailable\n");
  });

  it.each(shells)("%s draws nothing when the person's own status line runs Rerail's again", (shell) => {
    recordSession("session-one", workspace, 3);
    writeFileSync(join(home, "daemon.json"), "{}");
    // The marker names the process that started the person's status line; one that is gone does not count.
    expect(statusLine(shell, input("no-file"), { _RERAIL_CLAUDE_STATUS_PARENT: String(process.pid) })).toBe("");
    const gone = spawnSync(process.execPath, ["-e", ""]).pid;
    rmSync(join(home, "daemon.json"));
    expect(statusLine(shell, input("no-file"), { NO_COLOR: "1", _RERAIL_CLAUDE_STATUS_PARENT: String(gone) })).toBe("R E R A I L ✕ not running\n");
  });

  it.each(shells)("%s starts node with the Node status script where the person has a status line of their own", (shell) => {
    // Stands in for node: prints the words it was started with, one a line.
    const node = join(root, "recorded node");
    writeFileSync(node, "#!/bin/sh\nprintf 'recorded %s\\n' \"$@\"\n", { mode: 0o755 });
    const words = `"${node}" "/package/claude status.js" /project/settings.local.json`;
    expect(statusLine(shell, input("session-one"), {}, `set -- ${words}; `)).toBe("recorded /package/claude status.js\nrecorded /project/settings.local.json\n");
    // Once the recorded node is gone, `node` from PATH.
    const bin = join(root, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "node"), "#!/bin/sh\nprintf 'from PATH %s\\n' \"$@\"\n", { mode: 0o755 });
    expect(statusLine(shell, input("session-one"), { PATH: bin }, `set -- "${join(root, "gone node")}" /package/claude-status.js; `))
      .toBe("from PATH /package/claude-status.js\n");
  });

  /**
   * A folder for PATH whose only program is a `sleep` that returns at once, lists each pause it was asked for in
   * `slept`, and, with `fileAt`, copies `line` to the session's status file on that call, as the daemon writes it.
   */
  function fakeSleep(sessionId: string, fileAt?: number): { path: string; slept: () => string[] } {
    const bin = join(root, "sleep-bin");
    const log = join(root, "slept");
    const line = join(root, "line");
    mkdirSync(bin);
    mkdirSync(folder, { recursive: true });
    writeFileSync(log, "");
    writeFileSync(line, "24-bit\n256\nplain\n");
    const write = fileAt === undefined ? "" : [
      "calls=0", `while IFS= read -r pause; do calls=$((calls + 1)); done < "${log}"`,
      `if [ "$calls" -eq ${fileAt} ]; then while IFS= read -r text; do printf '%s\\n' "$text"; done < "${line}" > "${join(folder, sessionId)}"; fi`,
    ].join("\n");
    writeFileSync(join(bin, "sleep"), `#!/bin/sh\nprintf '%s\\n' "$1" >> "${log}"\n${write}\n`, { mode: 0o755 });
    return { path: bin, slept: () => readFileSync(log, "utf8").split("\n").filter(pause => pause !== "") };
  }

  /** Claude Code's input for a session that has had no reply yet, or has had one. */
  const inputAt = (sessionId: string, apiMs: number): object => ({ ...input(sessionId), cost: { total_duration_ms: 11_761, total_api_duration_ms: apiMs } });

  // Claude Code draws a new session's status line about a second before its start hook has told the daemon of it.
  it.each(shells)("%s waits, at a session's first draw, for the file the daemon writes once the session's start hook has run", (shell) => {
    const sleep = fakeSleep("new-session", 2);
    writeFileSync(join(home, "daemon.json"), "{}");
    expect(statusLine(shell, inputAt("new-session", 0), { PATH: sleep.path, NO_COLOR: "1" })).toBe("plain\n");
    expect(sleep.slept()).toEqual(["0.2", "0.3"]);
  });

  it.each(shells)("%s gives up after six sleeps where no file comes, and never waits once the session has had a reply", (shell) => {
    const sleep = fakeSleep("no-file");
    expect(statusLine(shell, inputAt("no-file", 0), { PATH: sleep.path, NO_COLOR: "1" })).toBe("R E R A I L ✕ not running\n");
    expect(sleep.slept()).toEqual(["0.2", "0.3", "0.5", "1", "2", "3"]);
    for (const answered of [inputAt("no-file", 2_300), inputAt("no-file", 0.5), input("no-file")]) {
      expect(statusLine(shell, answered, { PATH: sleep.path, NO_COLOR: "1" })).toBe("R E R A I L ✕ not running\n");
    }
    expect(sleep.slept()).toHaveLength(6);
  });
});
