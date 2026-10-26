import { execFileSync } from "node:child_process";
import { mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { RerailRuntime } from "../../src/runtime/index.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";

const temporaryDirectory = temporaryDirectories("rerail-session-records-gitignore-");

/** A Git checkout with no ignores of its own, and the session storage root where hooks put it by default. */
async function checkoutWithSessionStorage(): Promise<{ checkout: string; storageRoot: string; databasePath: string }> {
  const root = await realpath(await temporaryDirectory({ started: false }));
  const checkout = join(root, "checkout");
  const databasePath = join(root, "rerail.sqlite");
  await mkdir(checkout);
  execFileSync("git", ["init", "--quiet"], { cwd: checkout });
  startTestRepository(checkout, databasePath);
  return { checkout, storageRoot: join(checkout, ".rerail", "sessions"), databasePath };
}

async function openOneSession(checkout: string, storageRoot: string, databasePath: string): Promise<void> {
  const runtime = new RerailRuntime({ storageRoot, databasePath, workspaceRoot: checkout, autostart: false, embedder: testEmbedder });
  try {
    await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "gitignore", cwd: checkout } });
    await runtime.ingest({ kind: "user-prompt", eventId: "prompt", sessionId: "gitignore", text: "Add a priority score to each todo" });
  } finally {
    await runtime.dispose();
  }
}

describe("session records in a checkout", () => {
  it("never show in git status, so git add -A cannot commit them", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await openOneSession(checkout, storageRoot, databasePath);
    expect(execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: checkout, encoding: "utf8" })).toBe("");
  });

  it("leave a .gitignore already in the storage root as it is", async () => {
    const { checkout, storageRoot, databasePath } = await checkoutWithSessionStorage();
    await mkdir(storageRoot, { recursive: true });
    await writeFile(join(storageRoot, ".gitignore"), "events.jsonl\n");
    await openOneSession(checkout, storageRoot, databasePath);
    expect(await readFile(join(storageRoot, ".gitignore"), "utf8")).toBe("events.jsonl\n");
  });
});
