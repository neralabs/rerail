import type { ChildProcess, spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  createGithubClient,
  GithubError,
  PULL_REQUEST_LIMIT,
  type GithubRepository,
} from "../../../src/runtime/merge-assist/github.js";

const REPOSITORY: GithubRepository = { host: "github.com", owner: "neralabs", name: "rerail" };
const COMMIT = "ed0e9fcf09b181eb3440042af2cb24e5cc63b722";
const ACCEPT = "Accept: application/vnd.github+json";
const PR_FIELDS = "number,title,url,headRefName,headRefOid,headRepositoryOwner,isCrossRepository,baseRefName,author,isDraft,updatedAt";

/** What the fake `gh` does for one invocation. */
interface Reply {
  readonly code?: number;
  readonly stdout?: string | Buffer;
  readonly stderr?: string;
  /** Never exits on its own; only `kill` ends it. */
  readonly hang?: boolean;
  readonly error?: NodeJS.ErrnoException;
}

interface Call {
  readonly command: string;
  readonly args: readonly string[];
  readonly options: { env?: NodeJS.ProcessEnv; shell?: boolean; stdio?: unknown };
  readonly kills: string[];
}

interface FakeGh { readonly spawn: typeof spawn; readonly calls: Call[] }

function fakeGh(reply: (args: readonly string[]) => Reply): FakeGh {
  const calls: Call[] = [];
  const fake = (command: string, args: readonly string[], options: Call["options"]): ChildProcess => {
    const call: Call = { command, args, options, kills: [] };
    calls.push(call);
    const child = new EventEmitter() as EventEmitter & {
      stdout: PassThrough; stderr: PassThrough; exitCode: number | null; signalCode: string | null; kill: (signal: string) => boolean;
    };
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    child.signalCode = null;
    const close = (code: number | null, signal: string | null): void => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.exitCode = code;
      child.signalCode = signal;
      child.stdout.end();
      child.stderr.end();
      setImmediate(() => child.emit("close", code, signal));
    };
    child.kill = (signal: string) => {
      call.kills.push(signal);
      close(null, signal);
      return true;
    };
    const response = reply(args);
    setImmediate(() => {
      if (response.error !== undefined) {
        child.emit("error", response.error);
        return;
      }
      if (response.stdout !== undefined) child.stdout.write(response.stdout);
      if (response.stderr !== undefined) child.stderr.write(response.stderr);
      if (response.hang !== true) close(response.code ?? 0, null);
    });
    return child as unknown as ChildProcess;
  };
  return { spawn: fake as unknown as typeof spawn, calls };
}

let home: string;
let env: NodeJS.ProcessEnv;

beforeEach(() => {
  home = mkdtempSync(join(tmpdir(), "rerail-github-"));
  const gh = join(home, "gh");
  writeFileSync(gh, "#!/bin/sh\nexit 0\n");
  chmodSync(gh, 0o755);
  env = { PATH: home, HOME: home, GH_FORCE_TTY: "1", GH_TOKEN_SAMPLE: "kept", GH_ENTERPRISE_TOKEN: "ghe-secret", GITHUB_ENTERPRISE_TOKEN: "ghe-secret-2" };
});

afterEach(() => {
  rmSync(home, { recursive: true, force: true });
});

describe("gh invocation", () => {
  it("runs the resolved gh with an args array, no shell, and prompts, pagers and colour off", async () => {
    const gh = fakeGh(() => ({ stdout: "[]" }));
    await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY);
    expect(gh.calls).toHaveLength(1);
    const [call] = gh.calls;
    expect(call?.command).toBe(join(home, "gh"));
    expect(call?.args).toEqual([
      "pr", "list", "--repo=github.com/neralabs/rerail", "--state", "open", "--limit", "200", "--json", PR_FIELDS,
    ]);
    expect(call?.options.shell).toBe(false);
    expect(call?.options.stdio).toEqual(["ignore", "pipe", "pipe"]);
    expect(call?.options.env).toMatchObject({
      GH_PROMPT_DISABLED: "1",
      GH_NO_UPDATE_NOTIFIER: "1",
      NO_COLOR: "1",
      GH_PAGER: "cat",
      GH_TOKEN_SAMPLE: "kept",
      PATH: home,
    });
    expect(call?.options.env?.GH_FORCE_TTY).toBeUndefined();
    expect(call?.options.env?.GH_ENTERPRISE_TOKEN).toBeUndefined();
    expect(call?.options.env?.GITHUB_ENTERPRISE_TOKEN).toBeUndefined();
  });

  it("reads the API only with GET, an explicit host and the GitHub media type", async () => {
    const gh = fakeGh(() => ({ stdout: JSON.stringify({ login: "jbarnes" }) }));
    await createGithubClient({ env, spawn: gh.spawn }).readiness({ ...REPOSITORY, host: "github.example.com" });
    expect(gh.calls.map(call => call.args)).toEqual([
      ["auth", "status", "--hostname", "github.example.com"],
      ["api", "--method", "GET", "--hostname", "github.example.com", "-H", ACCEPT, "user"],
    ]);
  });

  it("never passes an Enterprise token, and reads an Enterprise host only through a stored gh login for it", async () => {
    // A remote of github.evil.example makes it a GitHub Enterprise host to Rerail; gh would send GH_ENTERPRISE_TOKEN to it.
    const lookalike = { ...REPOSITORY, host: "github.evil.example" };
    const gh = fakeGh(args => (args[0] === "auth" ? { code: 1, stderr: "You are not logged into any accounts on github.evil.example\n" } : { stdout: "[]" }));
    const client = createGithubClient({ env, spawn: gh.spawn });
    await expect(client.openPullRequests(lookalike)).rejects.toMatchObject({ reason: "failed", message: expect.stringContaining("not signed in to github.evil.example") });
    await expect(client.readiness(lookalike)).resolves.toEqual({ state: "signed-out", login: null, repository: "neralabs/rerail" });
    expect(gh.calls.map(call => call.args[0])).toEqual(["auth", "auth"]);
    for (const call of gh.calls) {
      expect(call.options.env?.GH_ENTERPRISE_TOKEN).toBeUndefined();
      expect(call.options.env?.GITHUB_ENTERPRISE_TOKEN).toBeUndefined();
    }
  });

  it("asks gh auth status once per Enterprise host it reads, and never for github.com", async () => {
    const gh = fakeGh(args => (args[0] === "auth" ? {} : { stdout: "[]" }));
    const client = createGithubClient({ env, spawn: gh.spawn });
    const enterprise = { ...REPOSITORY, host: "github.example.com" };
    await client.openPullRequests(enterprise);
    await client.openPullRequests(enterprise);
    await client.openPullRequests(REPOSITORY);
    expect(gh.calls.map(call => call.args[0])).toEqual(["auth", "pr", "pr", "pr"]);
  });

  it("stops gh that does not answer in time and says so", async () => {
    const gh = fakeGh(() => ({ hang: true }));
    const client = createGithubClient({ env, spawn: gh.spawn, timeoutMs: 20 });
    const error = await client.openPullRequests(REPOSITORY).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(GithubError);
    expect(error).toMatchObject({ reason: "timeout" });
    expect(gh.calls[0]?.kills).toEqual(["SIGTERM"]);
  });

  it("answers at its timeout even when a process gh started still holds its output open", async () => {
    // A gh wrapper that does not exec: the shell is stopped, the process it started keeps stdout open for 5 s.
    writeFileSync(join(home, "gh"), "#!/bin/sh\n/bin/sleep 5 &\nwait\n");
    const started = Date.now();
    const error = await createGithubClient({ env, timeoutMs: 300 }).openPullRequests(REPOSITORY).catch((caught: unknown) => caught);
    expect(error).toMatchObject({ reason: "timeout" });
    expect(Date.now() - started).toBeLessThan(3_000);
  });

  it("stops gh that prints more than 4 MiB", async () => {
    const gh = fakeGh(() => ({ stdout: Buffer.alloc(4 * 1024 * 1024 + 1, 0x20), hang: true }));
    const error = await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY).catch((caught: unknown) => caught);
    expect(error).toMatchObject({ name: "GithubError", reason: "output" });
    expect(gh.calls[0]?.kills).toEqual(["SIGTERM"]);
  });

  it("refuses an owner, name or host that could change the API path, without running gh", async () => {
    const gh = fakeGh(() => ({ stdout: "[]" }));
    const client = createGithubClient({ env, spawn: gh.spawn });
    const repositories: GithubRepository[] = [
      { ...REPOSITORY, owner: "../neralabs" },
      { ...REPOSITORY, owner: "neralabs/rerail" },
      { ...REPOSITORY, owner: "-neralabs" },
      { ...REPOSITORY, name: ".." },
      { ...REPOSITORY, name: "rerail/contents" },
      { ...REPOSITORY, name: "rerail?ref=main" },
      { ...REPOSITORY, name: "rerail#x" },
      { ...REPOSITORY, host: "github.com/evil" },
      { ...REPOSITORY, host: "-github.com" },
    ];
    for (const repository of repositories) {
      await expect(client.openPullRequests(repository)).rejects.toMatchObject({ reason: "invalid-input" });
      await expect(client.readiness(repository)).resolves.toEqual({ state: "not-github", login: null, repository: null });
    }
    expect(gh.calls).toEqual([]);
  });
});

describe("readiness", () => {
  it("is not-github without a GitHub repository, and runs nothing", async () => {
    const gh = fakeGh(() => ({}));
    await expect(createGithubClient({ env, spawn: gh.spawn }).readiness(null))
      .resolves.toEqual({ state: "not-github", login: null, repository: null });
    expect(gh.calls).toEqual([]);
  });

  it("is missing when gh is not on PATH", async () => {
    const gh = fakeGh(() => ({}));
    const client = createGithubClient({ env: { ...env, PATH: join(home, "nowhere") }, spawn: gh.spawn });
    await expect(client.readiness(REPOSITORY)).resolves.toEqual({ state: "missing", login: null, repository: "neralabs/rerail" });
    await expect(client.openPullRequests(REPOSITORY)).rejects.toMatchObject({ reason: "missing" });
    expect(gh.calls).toEqual([]);
  });

  it("is missing when gh cannot be started", async () => {
    const enoent = Object.assign(new Error("spawn gh ENOENT"), { code: "ENOENT" });
    const gh = fakeGh(() => ({ error: enoent }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).readiness(REPOSITORY))
      .resolves.toMatchObject({ state: "missing" });
  });

  it("is signed-out when gh auth status fails for the host", async () => {
    const gh = fakeGh(() => ({ code: 1, stderr: "You are not logged into any accounts on github.example.com\n" }));
    const repository = { ...REPOSITORY, host: "github.example.com" };
    await expect(createGithubClient({ env, spawn: gh.spawn }).readiness(repository))
      .resolves.toEqual({ state: "signed-out", login: null, repository: "neralabs/rerail" });
    expect(gh.calls.map(call => call.args)).toEqual([["auth", "status", "--hostname", "github.example.com"]]);
  });

  it("is unavailable, not signed-out, when gh auth status does not answer in time", async () => {
    const gh = fakeGh(() => ({ hang: true }));
    await expect(createGithubClient({ env, spawn: gh.spawn, timeoutMs: 20 }).readiness(REPOSITORY))
      .resolves.toEqual({ state: "unavailable", login: null, repository: "neralabs/rerail" });
  });

  it("is signed-in with the login GitHub reports for the user", async () => {
    // `gh api --method GET user --jq '{login}'` → {"login":"Moti-Velz"}
    const gh = fakeGh(args => (args[0] === "auth" ? { stdout: "github.com\n  ✓ Logged in to github.com account Moti-Velz (keyring)\n" } : { stdout: JSON.stringify({ login: "Moti-Velz", id: 1, type: "User" }) }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).readiness(REPOSITORY))
      .resolves.toEqual({ state: "signed-in", login: "Moti-Velz", repository: "neralabs/rerail" });
    expect(gh.calls[1]?.args).toEqual(["api", "--method", "GET", "--hostname", "github.com", "-H", ACCEPT, "user"]);
  });

  it("stays signed-in without a login when the login is not one", async () => {
    const gh = fakeGh(args => (args[0] === "auth" ? {} : { stdout: JSON.stringify({ login: "evil\u001b[2J" }) }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).readiness(REPOSITORY))
      .resolves.toEqual({ state: "signed-in", login: null, repository: "neralabs/rerail" });
  });
});

describe("openPullRequests", () => {
  // The shape `gh pr list --json <PR_FIELDS>` prints (gh 2.89).
  const real = {
    author: { id: "U_kgDOBnypnA", is_bot: false, login: "Moti-Velz", name: "Timo" },
    baseRefName: "main",
    headRefName: "Moti-Velz/changelog-entries",
    headRefOid: COMMIT,
    headRepositoryOwner: { id: "O_kgDOCHyD1g", login: "neralabs" },
    isCrossRepository: false,
    isDraft: false,
    number: 389,
    title: "Release notes: changelog entries wait in .changelog/",
    updatedAt: "2026-11-02T20:09:43Z",
    url: "https://github.com/neralabs/rerail/pull/389",
  };

  it("parses gh's list and leaves out entries that fail validation", async () => {
    const entries = [
      real,
      { ...real, number: 390, url: "https://github.com/neralabs/rerail/pull/390", author: { id: "BOT_x", is_bot: true, login: "app/dependabot" }, isDraft: true, title: "Bump\u0007 zod\u202e" },
      { ...real, number: 391, url: "https://github.com/neralabs/rerail/pull/391", headRefOid: "main" },
      { ...real, number: 392, url: "javascript:alert(1)" },
      { ...real, number: 393, url: "https://evil.example/neralabs/rerail/pull/393" },
      { ...real, number: 394, url: "https://github.com/neralabs/rerail/pull/1" },
      { ...real, number: 395, url: "https://github.com/neralabs/rerail/pull/395", headRefName: "../../etc" },
      { ...real, number: 396, url: "https://github.com/neralabs/rerail/pull/396", baseRefName: "-x" },
      { ...real, number: 397, url: "https://github.com/neralabs/rerail/pull/397", author: { login: "a/b/c" } },
      { ...real, number: 398, url: "https://github.com/neralabs/rerail/pull/398", updatedAt: "yesterday" },
      "not an object",
      // A fork's pull request names its head's owner; a deleted fork's names none.
      { ...real, number: 399, url: "https://github.com/neralabs/rerail/pull/399", isCrossRepository: true, headRepositoryOwner: { id: "U_x", login: "Moti-Velz" } },
      { ...real, number: 400, url: "https://github.com/neralabs/rerail/pull/400", isCrossRepository: true, headRepositoryOwner: null },
    ];
    const gh = fakeGh(() => ({ stdout: JSON.stringify(entries) }));
    const { pulls, truncated } = await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY);
    expect(truncated).toBe(false);
    expect(pulls).toEqual([
      {
        number: 389,
        title: "Release notes: changelog entries wait in .changelog/",
        url: "https://github.com/neralabs/rerail/pull/389",
        headRefName: "Moti-Velz/changelog-entries",
        headCommit: COMMIT,
        baseRefName: "main",
        author: "Moti-Velz",
        isDraft: false,
        updatedAt: "2026-11-02T20:09:43Z",
        crossRepository: false,
        headOwner: "neralabs",
      },
      expect.objectContaining({ number: 390, author: "dependabot[bot]", isDraft: true, title: "Bump zod" }),
      expect.objectContaining({ number: 399, crossRepository: true, headOwner: "Moti-Velz" }),
      expect.objectContaining({ number: 400, crossRepository: true, headOwner: null }),
    ]);
  });

  it("says when the listing stopped at its limit, invalid entries counted", async () => {
    const entries = Array.from({ length: PULL_REQUEST_LIMIT }, (_, index) => (index === 0 ? "not an object" : { ...real, number: index, url: `https://github.com/neralabs/rerail/pull/${index}` }));
    const gh = fakeGh(() => ({ stdout: JSON.stringify(entries) }));
    const listing = await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY);
    expect(listing).toMatchObject({ truncated: true, pulls: expect.any(Array) });
    expect(listing.pulls).toHaveLength(PULL_REQUEST_LIMIT - 1);
  });

  it("throws when gh fails, rather than reporting no pull requests", async () => {
    const gh = fakeGh(() => ({ code: 1, stderr: "HTTP 401: Bad credentials (https://api.github.com/graphql)\n" }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY))
      .rejects.toMatchObject({ reason: "failed", message: expect.stringContaining("Bad credentials") });
  });

  it("throws on output that is not a list", async () => {
    const gh = fakeGh(() => ({ stdout: "{\"message\":\"nope\"}" }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY)).rejects.toMatchObject({ reason: "invalid-response" });
  });
});
