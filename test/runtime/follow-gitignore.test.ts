import { execFileSync } from "node:child_process";
import { appendFile, mkdir, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { eq } from "drizzle-orm";
import { describe, expect, it, vi } from "vitest";
import { codeRuleSchema, NativeCodeRuleEnforcer, openRerailDatabase, repositories, SqliteAuditStore } from "../../src/engine/index.js";
import { RerailRuntime, SqliteSessionReader } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";
import { startTestRepository } from "../support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-follow-gitignore-");

function needleCatalog() {
  return testCodeRuleCatalog([codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{
      id: "needle",
      violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" },
      forbid: { regex: { pattern: "NEEDLE", source: "file.content" } },
    }],
  })]);
}

interface GitignoreSession {
  root: string;
  sessionId: string;
  runtime: RerailRuntime;
  audits: SqliteAuditStore;
  reader: SqliteSessionReader;
  /** The payloads of the session's events of `kind`, in order. */
  events(kind: string): unknown[];
  /** The paths the session's completed ai-output audits found the needle in. */
  findings(): string[];
}

/**
 * One session in a Git checkout whose repository follows `.gitignore` or not, past its baseline walk; the checkout's
 * `.gitignore` leaves out `gen/`, and its `.git/info/exclude` holds `exclude`.
 */
async function withSession(
  { followGitignore, exclude = "" }: { followGitignore: boolean; exclude?: string },
  run: (session: GitignoreSession) => Promise<void>,
): Promise<void> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  // Git reads the test's home, so the developer's own global excludes file plays no part.
  vi.stubEnv("HOME", join(root, "home"));
  vi.stubEnv("XDG_CONFIG_HOME", "");
  vi.stubEnv("GIT_CONFIG_GLOBAL", join(root, "home", "gitconfig"));
  const databasePath = join(root, "rerail.sqlite");
  await writeFile(join(root, ".gitignore"), "gen/\nsessions/\nhome/\nrerail.sqlite*\n");
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  await writeFile(join(root, ".git", "info", "exclude"), exclude);
  const repositoryKey = startTestRepository(root, databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    database.update(repositories).set({ followGitignore }).where(eq(repositories.repositoryKey, repositoryKey)).run();
  } finally {
    database.$client.close();
  }
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder,
    createAuditor: options => new NativeCodeRuleEnforcer(needleCatalog(), options),
  });
  const audits = new SqliteAuditStore(databasePath);
  const reader = new SqliteSessionReader(databasePath);
  const sessionId = "follow-gitignore";
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "codex", sessionId, cwd: root } });
    await runtime.codeRulesOpened(sessionId);
    // The baseline walk has run before the session writes anything.
    await runtime.ingest({ kind: "turn-end", eventId: "baseline", sessionId, text: "" });
    await run({
      root, sessionId, runtime, audits, reader,
      events: kind => reader.events(sessionId).filter(event => event.kind === kind).map(event => event.payload),
      findings: () => audits.list({ sessionId }).audits
        .filter(audit => audit.scope === "ai-output" && audit.status === "completed")
        .flatMap(audit => audits.get(audit.auditId)?.violations ?? [])
        .map(violation => violation.path ?? "")
        .toSorted(),
    });
  } finally {
    await runtime.dispose();
    audits.close();
    reader.close();
    vi.unstubAllEnvs();
  }
}

/** One tool call of `toolName`, with `act` run between its call and its result, as the tool would. */
async function toolCall(
  { runtime, sessionId }: GitignoreSession,
  id: string,
  toolName: string,
  input: unknown,
  act: () => Promise<void>,
): Promise<void> {
  await runtime.ingest({ kind: "call", eventId: `${id}-call`, sessionId, toolCallId: id, toolName, input });
  await act();
  await runtime.ingest({ kind: "result", eventId: `${id}-result`, sessionId, toolCallId: id, toolName, output: "" });
}

/** The paths the session's audits found the needle in, after one shell command writes a source in `gen/` and one beside it. */
async function shellWriteFindings(followGitignore: boolean): Promise<string[]> {
  let findings: string[] = [];
  await withSession({ followGitignore }, async session => {
    await toolCall(session, "shell-1", "shell", { command: ["bash", "-lc", "python3 generate.py"] }, async () => {
      await mkdir(join(session.root, "gen"));
      await writeFile(join(session.root, "gen", "client.py"), "NEEDLE\n");
      await writeFile(join(session.root, "kept.py"), "NEEDLE\n");
    });
    await vi.waitFor(() => expect(session.findings()).toContain("kept.py"), { timeout: 10_000 });
    findings = session.findings();
  });
  return findings;
}

describe("following .gitignore", () => {
  it("never audits a shell write under a gitignored folder while the repository follows .gitignore", async () => {
    expect(await shellWriteFindings(true)).toEqual(["kept.py"]);
  }, 30_000);

  it("audits it like any other source once the repository stops following .gitignore", async () => {
    expect(await shellWriteFindings(false)).toEqual(["gen/client.py", "kept.py"]);
  }, 30_000);

  it("records a tool edit Git's rules left unchecked, naming the ignore file, line and pattern", async () => {
    await withSession({ followGitignore: true, exclude: "# local\nscratch/\n" }, async session => {
      const write = (path: string) => toolCall(session, path, "Write", { file_path: join(session.root, path), content: "NEEDLE\n" }, async () => {
        await mkdir(join(session.root, "scratch"), { recursive: true });
        await writeFile(join(session.root, path), "NEEDLE\n");
      });
      await write("scratch/evil.py");
      await write("kept.py");
      await write("scratch/notes.txt");
      await vi.waitFor(() => expect(session.findings()).toEqual(["kept.py"]), { timeout: 10_000 });
      // `notes.txt` is no rule's file, so Git's rules took nothing from what was checked.
      expect(session.events("ignored-edit")).toEqual([{ path: "scratch/evil.py", source: ".git/info/exclude", line: 2, pattern: "scratch/" }]);
      expect(session.reader.gaps(session.sessionId)).toMatchObject({ ignoredEdits: 1 });
    });
  }, 30_000);

  it("records what makes Git's ignore rules ignore more: an exclude file a command wrote, a .gitignore a tool edit wrote", async () => {
    await withSession({ followGitignore: true, exclude: "node_modules\n" }, async session => {
      await toolCall(session, "shell-1", "Bash", { command: "echo 'scratch/' >> .git/info/exclude" }, async () => {
        await appendFile(join(session.root, ".git", "info", "exclude"), "scratch/\n");
      });
      // A line the file already holds, as a tool outside the repository appends it again: nothing more is ignored.
      await toolCall(session, "shell-2", "Bash", { command: "echo node_modules >> .git/info/exclude" }, async () => {
        await appendFile(join(session.root, ".git", "info", "exclude"), "node_modules\n");
      });
      await toolCall(session, "edit-1", "Edit", { file_path: join(session.root, ".gitignore") }, async () => {
        await appendFile(join(session.root, ".gitignore"), "out/\n");
      });
      const patch = "*** Begin Patch\n*** Update File: .gitignore\n@@\n+dist/\n*** End Patch\n";
      await toolCall(session, "patch-1", "apply_patch", { input: patch }, async () => {
        await appendFile(join(session.root, ".gitignore"), "dist/\n");
      });
      expect(session.events("ignore-rules-changed")).toEqual([
        { path: ".git/info/exclude", via: "observed", ignoresMore: ["scratch/"] },
        { path: ".gitignore", via: "tool", ignoresMore: ["out/"] },
        { path: ".gitignore", via: "tool", ignoresMore: ["dist/"] },
      ]);
    });
  }, 30_000);

  it("records a .gitignore a command wrote, though it ignores itself and no diff shows it", async () => {
    await withSession({ followGitignore: true }, async session => {
      await toolCall(session, "shell-1", "Bash", { command: "mkdir hidden && printf '*\\n' > hidden/.gitignore && cp a.py hidden/" }, async () => {
        await mkdir(join(session.root, "hidden"));
        await writeFile(join(session.root, "hidden", ".gitignore"), "*\n");
        await writeFile(join(session.root, "hidden", "evil.py"), "NEEDLE\n");
      });
      expect(session.events("ignore-rules-changed")).toEqual([{ path: "hidden/.gitignore", via: "observed", ignoresMore: ["*"] }]);
      expect(session.findings()).toEqual([]);
    });
  }, 30_000);

  it("neither watches Git's ignore rules nor reports what they leave out once the repository stops following .gitignore", async () => {
    await withSession({ followGitignore: false, exclude: "scratch/\n" }, async session => {
      await toolCall(session, "shell-1", "Bash", { command: "echo 'gen/' >> .git/info/exclude" }, async () => {
        await appendFile(join(session.root, ".git", "info", "exclude"), "gen/\n");
      });
      await toolCall(session, "write-1", "Write", { file_path: join(session.root, "scratch", "evil.py") }, async () => {
        await mkdir(join(session.root, "scratch"));
        await writeFile(join(session.root, "scratch", "evil.py"), "NEEDLE\n");
      });
      await vi.waitFor(() => expect(session.findings()).toEqual(["scratch/evil.py"]), { timeout: 10_000 });
      expect(session.reader.gaps(session.sessionId)).toMatchObject({ ignoredEdits: 0, ignoreRuleChanges: 0 });
    });
  }, 30_000);
});
