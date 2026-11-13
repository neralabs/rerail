import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runFeedbackCommand } from "../../src/cli/feedback-cli.js";
import { defaultRerailDatabasePath, openRerailDatabase, steerFeedback } from "../../src/engine/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { installPack } from "../support/installed-packs.js";
import { startTestRepository } from "../support/started-repository.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const RULE = "bandit-b101-assert-used";
const OTHER_RULE = "bandit-b102-exec-used";
const directories: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(directories.splice(0).map(directory => removeFolder(directory)));
});

/**
 * A Rerail home with the Python pack installed and a started git repository in which a Claude Code session was
 * steered about `RULE`; `run` is `rerail feedback` from that repository.
 */
async function steeredRepository() {
  const root = await mkdtemp(join(tmpdir(), "rerail-feedback-"));
  directories.push(root);
  const home = join(root, "home");
  vi.stubEnv("RERAIL_HOME", home);
  installPack("python", home);
  const workspace = join(root, "repo");
  mkdirSync(workspace);
  execFileSync("git", ["init", "-q", workspace]);
  const repositoryId = startTestRepository(workspace);
  const store = new SqliteSessionStore(defaultRerailDatabasePath(), { repositoryId });
  const session = { host: "claude-code" as const, sessionId: "steered", cwd: workspace };
  store.openSession(session);
  store.recordSteer(session, "receipt", [RULE]);
  store.close();

  const stdin = { value: "" };
  async function run(...args: string[]): Promise<string> {
    let out = "";
    await runFeedbackCommand(args, () => "usage", {
      packageVersion: "9.9.9",
      cwd: workspace,
      env: { RERAIL_HOME: home },
      stdout: { write: (text: string) => (out += text) },
      stdin: () => stdin.value,
    });
    return out;
  }
  const notes = () => {
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      return database.select().from(steerFeedback).orderBy(sql`rowid`).all();
    } finally {
      database.$client.close();
    }
  };
  return { root, run, notes, stdin };
}

describe("rerail feedback", () => {
  it("keeps the note with its rule's pack version and the session the rule steered, on this computer only", async () => {
    const { run, notes } = await steeredRepository();

    const output = await run(RULE, "pytest", "asserts\nare the   repo convention");

    expect(notes()).toEqual([expect.objectContaining({
      ruleId: RULE,
      packId: "python",
      packVersion: JSON.parse(readFileSync(join("packs", "python", "package.json"), "utf8")).version,
      host: "claude-code",
      sessionId: "steered",
      repositoryId: expect.stringMatching(/^clone:/u),
      reason: "pytest asserts are the repo convention",
      rerailVersion: "9.9.9",
    })]);
    expect(output).toMatch(/^Rerail kept your note on bandit-b101-assert-used on this computer and sends it nowhere\. Nothing more to do\. /u);
    // Sharing is the person's to do, through an issue, with the command that prints the note.
    expect(output).toContain(`\`rerail feedback list --rule ${RULE}\` prints it for a rule misfire issue (https://github.com/neralabs/rerail/issues/new?template=2-rule-misfire.yml)`);
  });

  it("refuses control, bidi and zero-width characters in the reason or the rule id, keeping nothing", async () => {
    const { run, notes } = await steeredRepository();

    await expect(run(RULE, "clear\u001b[2Jthe screen")).rejects.toThrow(/control or invisible characters/u);
    // A right-to-left override or a zero-width space would make the pasted line read otherwise than it is.
    await expect(run(RULE, "fine \u202eevil")).rejects.toThrow(/control or invisible characters/u);
    await expect(run(RULE, "zero\u200bwidth")).rejects.toThrow(/control or invisible characters/u);
    await expect(run(`${RULE}\u0007`, "a reason")).rejects.toThrow(/control or invisible characters/u);
    await expect(run(`${RULE}\u2066`, "a reason")).rejects.toThrow(/control or invisible characters/u);
    expect(notes()).toEqual([]);
  });

  it("refuses a rule Rerail does not know, keeping nothing", async () => {
    const { run, notes } = await steeredRepository();

    await expect(run("no-such-rule", "it does not apply")).rejects.toThrow(/no rule no-such-rule/u);
    expect(notes()).toEqual([]);
  });

  it("refuses a note outside any repository Rerail knows, since a reset could never delete it", async () => {
    const { root, notes } = await steeredRepository();
    const outside = join(root, "outside");
    mkdirSync(outside);

    await expect(runFeedbackCommand([RULE, "not in a repository"], () => "usage", {
      packageVersion: "9.9.9",
      cwd: outside,
      env: { RERAIL_HOME: join(root, "home") },
      stdout: { write: () => true },
    })).rejects.toThrow(/knows no repository at .*outside; run rerail feedback in the repository the steer came from/u);
    expect(notes()).toEqual([]);
  });

  it("takes the reason on stdin for a line with a quote in it", async () => {
    const { run, notes, stdin } = await steeredRepository();
    stdin.value = "I won't: it's the repo's convention\n";

    await run(RULE, "-");

    expect(notes().map(note => note.reason)).toEqual(["I won't: it's the repo's convention"]);
  });

  it("lists the notes oldest first for an issue, one rule's with --rule, without the session or the repository", async () => {
    const { run } = await steeredRepository();
    await run(RULE, "asserts are the test style");
    await run(OTHER_RULE, "exec runs a fixed template");
    await run(RULE, "still the test style");

    const all = await run("list");
    const one = await run("list", "--rule", RULE);

    const version = JSON.parse(readFileSync(join("packs", "python", "package.json"), "utf8")).version as string;
    const entry = (rule: string, agent: string, reason: string) =>
      new RegExp(`^\\d{4}-\\d\\d-\\d\\dT[\\d:.]+Z  ${rule}  \\(python@${version.replaceAll(".", "\\.")}, ${agent}, Rerail 9\\.9\\.9\\)\\n  ${reason}$`, "u");
    const blocks = (output: string) => output.trimEnd().split(/\n(?=\d{4}-)/u);
    expect(blocks(all)).toEqual([
      expect.stringMatching(entry(RULE, "claude-code", "asserts are the test style")),
      // The other rule never steered here, so no session names its agent.
      expect.stringMatching(entry(OTHER_RULE, "unknown agent", "exec runs a fixed template")),
      expect.stringMatching(entry(RULE, "claude-code", "still the test style")),
    ]);
    expect(blocks(one)).toEqual([blocks(all)[0], blocks(all)[2]]);
    expect(all).not.toMatch(/steered|clone:/u);
  });

  it("lists nothing without creating a database where there is none", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-feedback-empty-"));
    directories.push(root);
    const databasePath = join(root, "rerail.sqlite");
    let out = "";

    await runFeedbackCommand(["list", "--rule", RULE], () => "usage", { packageVersion: "9.9.9", databasePath, stdout: { write: text => (out += text) } });

    expect(out).toBe(`Rerail has no feedback notes on ${RULE}.\n`);
    expect(existsSync(databasePath)).toBe(false);
  });
});
