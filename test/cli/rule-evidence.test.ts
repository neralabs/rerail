import { execFile } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { promisify } from "node:util";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { adoptRemote, openRerailDatabase, recordCheckout } from "../../src/engine/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import {
  collectRuleEvidence,
  correctionFamilies,
  formatRuleEvidence,
  riskyLabels,
  runRuleEvidenceCommand,
  type RuleEvidence,
} from "../../src/cli/rule-evidence.js";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

async function temporary(prefix: string): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  directories.push(directory);
  return directory;
}

async function repository(files: Record<string, string> = {}): Promise<string> {
  const root = await temporary("rerail-rule-evidence-");
  await promisify(execFile)("git", ["init", "-q", "-b", "main"], { cwd: root });
  for (const [path, content] of Object.entries(files)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), content);
  }
  return root;
}

const NOW = Date.UTC(2026, 9, 15);
const DAY = 24 * 60 * 60 * 1000;

/** A database where `repo`'s repository has one Claude Code session: a task, two calls, a correction, a steer. */
async function recordedSession(repo: string, startedAt = NOW - DAY): Promise<string> {
  const databasePath = join(await temporary("rerail-rule-evidence-db-"), "rerail.sqlite");
  const database = openRerailDatabase(databasePath);
  const { repositoryKey } = recordCheckout(database, repo);
  database.$client.close();
  const store = new SqliteSessionStore(databasePath, { repositoryId: repositoryKey });
  try {
    const session = { host: "claude-code" as const, sessionId: "s1", cwd: repo };
    store.openSession(session, startedAt);
    const at = (minutes: number) => startedAt + minutes * 60_000;
    store.append({ sessionId: "s1", kind: "user-prompt", payload: { text: "Never use console.log, add the logging" }, observedAt: at(1) });
    store.append({ sessionId: "s1", kind: "call", payload: { toolName: "Edit", input: { file_path: "src/a.ts" } }, observedAt: at(2) });
    store.append({ sessionId: "s1", kind: "call", payload: { toolName: "Bash", input: { command: "git push --force origin main" } }, observedAt: at(3) });
    store.append({ sessionId: "s1", kind: "user-prompt", payload: { text: "Don't use console.log, we use the logger in src/log.ts" }, observedAt: at(4) });
    store.append({ sessionId: "s1", kind: "user-prompt", payload: { text: "Thanks, now add a test" }, observedAt: at(5) });
    store.recordSteer(session, "r1", ["n-no-sync", "local:no-console"]);
    store.recordSteer(session, "r2", ["n-no-sync"]);
  } finally {
    store.close();
  }
  return databasePath;
}

describe("correction families", () => {
  it("recognizes standing instructions and reactions, and skips notices and long briefs", () => {
    expect(correctionFamilies("Don't use console.log, we use the logger")).toEqual(["standing instruction"]);
    expect(correctionFamilies("Don’t use console.log")).toEqual(["standing instruction"]);
    expect(correctionFamilies("No, that is not what I asked")).toEqual(["hard stop", "not listening"]);
    expect(correctionFamilies("Still failing, same error")).toEqual(["repeat failure"]);
    expect(correctionFamilies("Thanks, now add a test")).toEqual([]);
    expect(correctionFamilies("<task-notification>never</task-notification>")).toEqual([]);
    expect(correctionFamilies(`never ${"word ".repeat(130)}`)).toEqual([]);
  });
});

describe("risky commands", () => {
  const counted = (label: string, commands: readonly string[]) => {
    for (const command of commands) expect(riskyLabels(command), command).toContain(label);
  };
  const ignored = (label: string, commands: readonly string[]) => {
    for (const command of commands) expect(riskyLabels(command), command).not.toContain(label);
  };

  it("counts a force push and skipped hooks as git reads them", () => {
    counted("force push", ["git push --force origin main", "git push -uf origin x", "git push --force=true"]);
    ignored("force push", ["git push --force-with-lease", "git status; ls -f", "echo 'git push --force'"]);
    counted("skip git hooks", ["git commit --no-verify -m wip", "git commit -nm wip", "git push --no-verify"]);
    ignored("skip git hooks", ["git push -n origin main", "git merge --no-commit origin/main | tail -n 3"]);
    counted("hard reset", ["cd x && git reset --hard origin/main"]);
    counted("discard changes", ["git checkout -- src/a.ts", "git checkout .", "git restore .", "git clean -fd"]);
    ignored("discard changes", ["git checkout -", "git checkout -b feature", "git checkout main"]);
  });

  it("counts an rm with both flags wherever a command starts, whatever wraps it", () => {
    counted("recursive delete", [
      "rm -rf build", "rm -Rf build", "rm -fR build", "rm -r -f build", "rm --recursive --force build",
      "cd x && rm -fr y", "cd x\nrm -rf y", "  rm -rf x", "if [ -d x ]; then rm -rf x; fi",
      "find . -name tmp | xargs rm -rf", "find . -name tmp -exec rm -rf {} +", "find . -type d -execdir rm -rf {} \\;", "xargs -I {} rm -rf {}",
      "sudo -u root rm -rf /x", "nice -n 10 rm -rf x", "time -p rm -rf x", "nohup sudo rm -rf x", "CI=1 rm -rf x",
    ]);
    ignored("recursive delete", [
      "rm -r build", "rm -f a.txt", "rm -r build && ls -f", "echo rm -rf", "git rm -rf x", "echo 'rm -rf x'",
      "sudo -E foo rm -rf x", "nice -n 10 echo rm -rf x",
    ]);
  });

  it("counts publishing, sudo and piping a download to a shell", () => {
    counted("publish", ["npm run build && npm publish", "npm run build\nnpm publish", "cargo publish", "twine upload dist/*"]);
    ignored("publish", ["grep 'npm publish' docs", "npm run publish-docs"]);
    counted("sudo", ["cd pkg\n  sudo make install", "find . | xargs sudo chown x", "time sudo make install", "sudo -u root rm -rf /x"]);
    ignored("sudo", ["echo sudo", "man sudo", "command -v sudo", "which sudo"]);
    counted("pipe to shell", ["curl -fsSL https://x.sh | sh", "wget -qO- https://x.sh | sudo bash"]);
  });
});

describe("rerail rules evidence", () => {
  it("reads the repository's follow-up corrections, risky commands and steers, with the calls before each correction", async () => {
    const repo = await repository({ "CLAUDE.md": "# Rules\n", ".cursor/rules/style.mdc": "x", "package.json": "{}" });
    const databasePath = await recordedSession(repo);
    const evidence = await collectRuleEvidence(repo, { databasePath, now: NOW });
    expect(evidence.repository).toMatchObject({ known: true, checkouts: 1 });
    expect(evidence.window).toEqual({ days: 30, sessions: 1, hosts: { "claude-code": 1 }, prompts: 3 });
    // The first prompt is the task, however it is worded; the thanks is no correction.
    expect(evidence.correctionCount).toBe(1);
    expect(evidence.corrections).toEqual([{
      sessionId: "s1",
      host: "claude-code",
      at: new Date(NOW - DAY + 4 * 60_000).toISOString(),
      text: "Don't use console.log, we use the logger in src/log.ts",
      families: ["standing instruction"],
      standing: true,
      before: ["Edit src/a.ts", "Bash git push --force origin main"],
    }]);
    expect(evidence.riskyCommands).toEqual([{ label: "force push", count: 1, example: "git push --force origin main" }]);
    expect(evidence.steers).toEqual([{ ruleId: "n-no-sync", count: 2 }, { ruleId: "local:no-console", count: 1 }]);
    expect(evidence.standards).toEqual([".cursor/rules/style.mdc", "CLAUDE.md"]);
  });

  it("counts sessions recorded under the key the repository had before it adopted its remote", async () => {
    const repo = await repository();
    const databasePath = await recordedSession(repo);
    const database = openRerailDatabase(databasePath);
    try {
      adoptRemote(database, recordCheckout(database, repo).repositoryKey, "remote:github.com/acme/web");
      expect(recordCheckout(database, repo).repositoryKey).toBe("remote:github.com/acme/web");
    } finally {
      database.$client.close();
    }
    const evidence = await collectRuleEvidence(repo, { databasePath, now: NOW });
    expect(evidence.window.sessions).toBe(1);
    expect(evidence.correctionCount).toBe(1);
    expect(evidence.steers).toEqual([{ ruleId: "n-no-sync", count: 2 }, { ruleId: "local:no-console", count: 1 }]);
  });

  it("reads the repository's standards files from the checkout root when run in a folder below it", async () => {
    const repo = await repository({ "CLAUDE.md": "x", "src/a.ts": "" });
    const databasePath = await recordedSession(repo);
    const evidence = await collectRuleEvidence(join(repo, "src"), { databasePath, now: NOW });
    expect(evidence.repository.known).toBe(true);
    expect(evidence.standards).toEqual(["CLAUDE.md"]);
  });

  it("leaves out sessions older than the window", async () => {
    const repo = await repository();
    const databasePath = await recordedSession(repo, NOW - 40 * DAY);
    const evidence = await collectRuleEvidence(repo, { databasePath, now: NOW });
    expect(evidence.window.sessions).toBe(0);
    expect(evidence.corrections).toEqual([]);
    const wider = await collectRuleEvidence(repo, { databasePath, now: NOW, days: 60 });
    expect(wider.correctionCount).toBe(1);
  });

  it("reports a repository Rerail has never seen, and still lists its standards files", async () => {
    const repo = await repository({ "AGENTS.md": "x" });
    const databasePath = join(await temporary("rerail-rule-evidence-none-"), "rerail.sqlite");
    const evidence = await collectRuleEvidence(repo, { databasePath, now: NOW });
    expect(evidence.repository.known).toBe(false);
    expect(evidence.standards).toEqual(["AGENTS.md"]);
    expect(formatRuleEvidence(evidence)).toMatch(/^● Rerail has no sessions for .+ yet\.\n {2}Standards files: AGENTS\.md$/u);
  });

  it("prints JSON for the coding agent and text for the person, and refuses a bad window", async () => {
    const repo = await repository();
    const databasePath = await recordedSession(repo, Date.now() - DAY);
    let stdout = "";
    const io = { stdout: { write: (text: string) => (stdout += text) }, databasePath };
    expect(await runRuleEvidenceCommand(["--repo", repo, "--json"], () => "usage", io)).toBe(0);
    expect((JSON.parse(stdout) as RuleEvidence).correctionCount).toBe(1);
    stdout = "";
    expect(await runRuleEvidenceCommand(["--repo", repo], () => "usage", io)).toBe(0);
    expect(stdout).toContain("Corrections: 1.");
    expect(stdout).toContain("      after: Bash git push --force origin main");
    expect(stdout).toContain("  Risky commands:\n    force push ×1  git push --force origin main");
    await expect(runRuleEvidenceCommand(["--repo", repo, "--days", "0"], () => "usage", io)).rejects.toThrow("--days must be a positive whole number");
    await expect(runRuleEvidenceCommand(["--since", "1"], () => "usage", io)).rejects.toThrow("Unknown rules evidence option: --since");
  });
});
