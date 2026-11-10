/**
 * The fixtures and recorders test/cli/repository-setup*.test.ts share: a temporary Git repository with a database
 * beside it, the coding agent's and the terminal's lanes, and readers of what set-up left in the database. Each test
 * file removes the fixtures after each test (`removeFixtures`).
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect } from "vitest";
import { lookupCheckout, offerSetupReview, openRerailDatabase, recordCheckout, repositoryIdentityLocation } from "../../src/engine/index.js";
import type { SetupRequest } from "../../src/cli/index.js";
import { onRuleCount, type RepositorySetupIo } from "../../src/runtime/index.js";
import { attachedPacks } from "./attached-packs.js";
import { repositoryPackVersions } from "./directory-pack-store.js";
import type { FakeSetupEnvironment } from "./fake-setup-environment.js";

const temporaryDirectories: string[] = [];

export const TS_PACKS = ["typescript"];

export interface Fixture {
  root: string;
  databasePath: string;
  /** A TypeScript repository: the TypeScript pack fits it. */
  repo: string;
}

export async function fixture(): Promise<Fixture> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-setup-")));
  temporaryDirectories.push(root);
  return { root, databasePath: join(root, "home", "rerail.sqlite"), repo: gitRepository(join(root, "acme-api")) };
}

export function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "app.ts"), "export const hi = 1;\n");
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: path });
  execFileSync("git", ["add", "--", "app.ts"], { cwd: path });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base"], { cwd: path });
  return path;
}

export const stripAnsi = (text: string): string => text.replace(/\u001b\[[0-9;]*m/gu, "");

export type Recorded = RepositorySetupIo & { said: string[]; text(): string };

/** The coding agent's side: no terminal, so set-up stops at each question. */
export function agentIo(): Recorded {
  const said: string[] = [];
  return { lane: "agent", said, say: text => { said.push(text); }, text: () => said.join("\n") };
}

/** A person at a terminal typing `answers` in order; input ends after the last one. */
export function terminalIo(answers: readonly string[]): Recorded & { left: string[] } {
  const said: string[] = [];
  const left = [...answers];
  return {
    lane: "terminal", said, left,
    say: text => { said.push(text); },
    ask: async () => left.shift(),
    text: () => said.join("\n"),
  };
}

export function request(fixture: Fixture, environment: FakeSetupEnvironment, extra: Partial<SetupRequest> = {}): SetupRequest {
  return { cwd: fixture.repo, databasePath: fixture.databasePath, environment, ...extra };
}

export function selected(fixture: Fixture, repo = fixture.repo): readonly string[] {
  return attachedPacks(repo, fixture.databasePath);
}

export function enabledRules(fixture: Fixture, repo = fixture.repo): number {
  const database = openRerailDatabase(fixture.databasePath, { readonly: true });
  try {
    return onRuleCount(database, lookupCheckout(database, repo)!);
  } finally {
    database.$client.close();
  }
}

/** Every start and set-up value Rerail holds, with when each last changed. */
export function startRows(fixture: Fixture): unknown {
  const database = openRerailDatabase(fixture.databasePath, { readonly: true });
  try {
    return {
      repositories: database.$client.prepare("SELECT repository_key, started, configured_at, updated_at FROM repositories").all(),
      checkouts: database.$client.prepare("SELECT checkout_path, started FROM checkouts").all(),
    };
  } finally {
    database.$client.close();
  }
}

/** Nothing of Rerail's in the repository, its Git metadata, or the database. */
export function expectUntouched(fixture: Fixture, repo = fixture.repo): void {
  expect(existsSync(repositoryIdentityLocation(repo).identityPath)).toBe(false);
  expect(existsSync(join(repo, ".rerail"))).toBe(false);
  expect(existsSync(join(repo, ".claude"))).toBe(false);
  expect(execFileSync("git", ["status", "--porcelain", "--ignored"], { cwd: repo, encoding: "utf8" })).toBe("");
}

/** Every start of a repository fails from now on, as a write that fails partway through set-up's would. */
export function failStarts(databasePath: string): void {
  const database = openRerailDatabase(databasePath);
  try {
    database.$client.exec("CREATE TRIGGER fail_start BEFORE UPDATE OF started ON repositories WHEN NEW.started = 1 BEGIN SELECT RAISE(ABORT, 'disk full'); END");
  } finally {
    database.$client.close();
  }
}

/** Each test sets real Git repositories up against SQLite; a loaded full-suite run needs more than the default 5 s. */
export const SETUP_TIMEOUT_MS = 30_000;
/** The pack set-up offers the TypeScript fixture, at the version its "Yes" answer carries (`--packs`). */
export const TYPESCRIPT = repositoryPackVersions("typescript");

/**
 * A repository an earlier version's set-up offered the dashboard's review in: its checkout recorded and the offer kept,
 * as `offerSetupReview` left it. Set-up no longer offers the review, but goes straight to its confirm question there.
 */
export function offerReview(fixture: Fixture, repo = fixture.repo): { repositoryKey: string; checkoutPath: string } {
  const database = openRerailDatabase(fixture.databasePath);
  try {
    const { repositoryKey, checkoutPath } = recordCheckout(database, repo);
    offerSetupReview(database, repositoryKey);
    return { repositoryKey, checkoutPath };
  } finally {
    database.$client.close();
  }
}

/** Removes every fixture made since the last call. */
export async function removeFixtures(): Promise<void> {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
}
