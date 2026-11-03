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
  type GithubRepository,
} from "../../../src/runtime/merge-assist/github.js";

/** The limits and format docs/merge-assist-v1-build.md §6–7 fixes: 64 candidates per branch, 16 KiB per file. */
const CANDIDATE_FORMAT = "rerail.candidate/v1";
const MAX_CANDIDATE_BYTES = 16 * 1024;
const MAX_CANDIDATES_PER_BRANCH = 64;

const REPOSITORY: GithubRepository = { host: "github.com", owner: "neralabs", name: "rerail" };
const COMMIT = "ed0e9fcf09b181eb3440042af2cb24e5cc63b722";
const ACCEPT = "Accept: application/vnd.github+json";
const PR_FIELDS = "number,title,url,headRefName,headRefOid,baseRefName,author,isDraft,updatedAt";

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

/** The REST path a `gh api` invocation reads. */
function apiPath(args: readonly string[]): string | null {
  return args[0] === "api" ? args.at(-1) ?? null : null;
}

function notFound(): Reply {
  return {
    code: 1,
    stdout: JSON.stringify({ message: "Not Found", documentation_url: "https://docs.github.com/rest", status: "404" }),
    stderr: "gh: Not Found (HTTP 404)\n",
  };
}

function candidateText(id: string, overrides: Record<string, unknown> = {}): string {
  return `${JSON.stringify({
    format: CANDIDATE_FORMAT,
    id: `candidate:${id}`,
    decision: "Retries live in the transport layer; callers don't wrap requests in retry loops.",
    generality: "repository",
    scope: ["src/api/**"],
    author: "jbarnes",
    createdAt: "2026-11-03T14:02:11Z",
    state: "active",
    supersedes: null,
    ...overrides,
  }, null, 2)}\n`;
}

function sha(seed: number): string {
  return seed.toString(16).padStart(40, "0");
}

/** One entry of the contents API listing a directory, as GitHub returns it (trimmed to the fields Rerail reads). */
function listed(name: string, seed: number, size: number, type = "file"): Record<string, unknown> {
  return { name, path: `.rerail/candidates/${name}`, sha: sha(seed), size, type, url: "https://api.github.com/..." };
}

/** The git blobs API response: base64 content wrapped at 60 columns, as GitHub sends it. */
function blob(seed: number, text: string): string {
  const content = Buffer.from(text, "utf8").toString("base64").replace(/(.{60})/gu, "$1\n");
  return JSON.stringify({ sha: sha(seed), node_id: "B_x", size: Buffer.byteLength(text), url: "https://api.github.com/...", content, encoding: "base64" });
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
      "pr", "list", "--repo=github.com/neralabs/rerail", "--state", "open", "--limit", "50", "--json", PR_FIELDS,
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
    const gh = fakeGh(() => ({ stdout: JSON.stringify({ permission: "write", role_name: "write" }) }));
    await createGithubClient({ env, spawn: gh.spawn }).authorPermission({ ...REPOSITORY, host: "github.example.com" }, "jbarnes");
    expect(gh.calls.map(call => call.args)).toEqual([
      ["auth", "status", "--hostname", "github.example.com"],
      ["api", "--method", "GET", "--hostname", "github.example.com", "-H", ACCEPT, "repos/neralabs/rerail/collaborators/jbarnes/permission"],
    ]);
  });

  it("never passes an Enterprise token, and reads an Enterprise host only through a stored gh login for it", async () => {
    // A remote of github.evil.example makes it a GitHub Enterprise host to Rerail; gh would send GH_ENTERPRISE_TOKEN to it.
    const lookalike = { ...REPOSITORY, host: "github.evil.example" };
    const gh = fakeGh(args => (args[0] === "auth" ? { code: 1, stderr: "You are not logged into any accounts on github.evil.example\n" } : { stdout: "[]" }));
    const client = createGithubClient({ env, spawn: gh.spawn });
    await expect(client.openPullRequests(lookalike)).rejects.toMatchObject({ reason: "failed", message: expect.stringContaining("not signed in to github.evil.example") });
    await expect(client.candidatesAt(lookalike, COMMIT)).rejects.toMatchObject({ reason: "failed" });
    await expect(client.authorPermission(lookalike, "jbarnes")).resolves.toBe("unknown");
    expect(gh.calls.map(call => call.args[0])).toEqual(["auth", "auth", "auth"]);
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

  it("stops gh that prints more than 4 MiB", async () => {
    const gh = fakeGh(() => ({ stdout: Buffer.alloc(4 * 1024 * 1024 + 1, 0x20), hang: true }));
    const error = await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY).catch((caught: unknown) => caught);
    expect(error).toMatchObject({ name: "GithubError", reason: "output" });
    expect(gh.calls[0]?.kills).toEqual(["SIGTERM"]);
  });

  it("refuses an owner, name, host, commit or login that could change the API path, without running gh", async () => {
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
      await expect(client.candidatesAt(repository, COMMIT)).rejects.toMatchObject({ reason: "invalid-input" });
      await expect(client.authorPermission(repository, "jbarnes")).rejects.toMatchObject({ reason: "invalid-input" });
    }
    for (const commit of ["main", "../../user", `${COMMIT}&ref=main`, COMMIT.toUpperCase(), `${COMMIT}0`]) {
      await expect(client.candidatesAt(REPOSITORY, commit)).rejects.toMatchObject({ reason: "invalid-input" });
    }
    for (const login of ["../admin", "a/b", "jbarnes?x=1", "jbarnes%2F", "", "a".repeat(40), "bot[bot][bot]"]) {
      await expect(client.authorPermission(REPOSITORY, login)).rejects.toMatchObject({ reason: "invalid-input" });
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
  // The shape `gh pr list --json number,title,url,headRefName,headRefOid,baseRefName,author,isDraft,updatedAt` prints (gh 2.89).
  const real = {
    author: { id: "U_kgDOBnypnA", is_bot: false, login: "Moti-Velz", name: "Timo" },
    baseRefName: "main",
    headRefName: "Moti-Velz/changelog-entries",
    headRefOid: COMMIT,
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
    ];
    const gh = fakeGh(() => ({ stdout: JSON.stringify(entries) }));
    const pullRequests = await createGithubClient({ env, spawn: gh.spawn }).openPullRequests(REPOSITORY);
    expect(pullRequests).toEqual([
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
      },
      expect.objectContaining({ number: 390, author: "dependabot[bot]", isDraft: true, title: "Bump zod" }),
    ]);
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

describe("candidatesAt", () => {
  const listingPath = `repos/neralabs/rerail/contents/.rerail/candidates?ref=${COMMIT}`;

  function contents(entries: unknown, blobs: Record<string, Reply> = {}): FakeGh {
    return fakeGh(args => {
      const path = apiPath(args);
      if (path === listingPath) return { stdout: JSON.stringify(entries) };
      const blobSha = /\/git\/blobs\/([0-9a-f]+)$/u.exec(path ?? "")?.[1];
      if (blobSha !== undefined && blobs[blobSha] !== undefined) return blobs[blobSha];
      return notFound();
    });
  }

  it("is an empty set when the folder is not at the commit", async () => {
    const gh = fakeGh(() => notFound());
    await expect(createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT))
      .resolves.toEqual({ candidates: [], problems: [], truncated: false });
    expect(gh.calls[0]?.args).toEqual(["api", "--method", "GET", "--hostname", "github.com", "-H", ACCEPT, listingPath]);
  });

  it("throws when the listing fails for any other reason", async () => {
    const gh = fakeGh(() => ({ code: 1, stderr: "gh: Server Error (HTTP 502)\n" }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT))
      .rejects.toMatchObject({ reason: "failed", status: 502 });
  });

  it("decodes each file's base64 blob and parses it", async () => {
    const text = candidateText("retries-in-transport-3f9a1c");
    const gh = contents([listed("retries-in-transport-3f9a1c.json", 1, Buffer.byteLength(text))], { [sha(1)]: { stdout: blob(1, text) } });
    const set = await createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT);
    expect(set.problems).toEqual([]);
    expect(set.truncated).toBe(false);
    expect(set.candidates.map(candidate => [candidate.id, candidate.author])).toEqual([["candidate:retries-in-transport-3f9a1c", "jbarnes"]]);
    expect(gh.calls[1]?.args.at(-1)).toBe(`repos/neralabs/rerail/git/blobs/${sha(1)}`);
  });

  it("ignores names Rerail does not read, and reports matching names that are not regular files", async () => {
    const gh = contents([
      listed("README.md", 1, 10),
      listed("Upper.json", 2, 10),
      listed("-dash.json", 3, 10),
      listed("nested", 4, 0, "dir"),
      listed("linked.json", 5, 30, "symlink"),
      { ...listed("moved.json", 6, 10), path: "elsewhere/moved.json" },
    ]);
    const set = await createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT);
    expect(set).toEqual({
      candidates: [],
      problems: ["linked.json: not a regular file", "moved.json: GitHub listed it in a shape Rerail does not read"],
      truncated: false,
    });
    expect(gh.calls).toHaveLength(1);
  });

  it("skips a file listed larger than the limit without fetching it", async () => {
    const text = candidateText("small-one-aaaaaa");
    const gh = contents(
      [listed("huge-one-bbbbbb.json", 2, MAX_CANDIDATE_BYTES + 1), listed("small-one-aaaaaa.json", 1, Buffer.byteLength(text))],
      { [sha(1)]: { stdout: blob(1, text) }, [sha(2)]: { stdout: blob(2, text) } },
    );
    const set = await createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT);
    expect(set.problems).toEqual([`huge-one-bbbbbb.json: larger than ${MAX_CANDIDATE_BYTES} bytes`]);
    expect(set.candidates.map(candidate => candidate.id)).toEqual(["candidate:small-one-aaaaaa"]);
    expect(gh.calls.map(call => apiPath(call.args))).not.toContain(`repos/neralabs/rerail/git/blobs/${sha(2)}`);
  });

  it("reads at most the per-branch limit, in name order, and says the rest was left", async () => {
    const count = MAX_CANDIDATES_PER_BRANCH + 6;
    const entries = Array.from({ length: count }, (_, index) => listed(`c${String(count - index).padStart(3, "0")}.json`, index + 1, 100));
    const blobs = Object.fromEntries(entries.map((entry, index) => {
      const id = String(entry.name).replace(/\.json$/u, "");
      return [sha(index + 1), { stdout: blob(index + 1, candidateText(id)) }];
    }));
    const gh = contents(entries, blobs);
    const set = await createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT);
    expect(set.truncated).toBe(true);
    expect(set.problems).toEqual([`.rerail/candidates: 6 more candidate files not read; a branch holds at most ${MAX_CANDIDATES_PER_BRANCH}`]);
    expect(set.candidates).toHaveLength(MAX_CANDIDATES_PER_BRANCH);
    expect(set.candidates[0]?.id).toBe("candidate:c001");
    expect(set.candidates.at(-1)?.id).toBe(`candidate:c${String(MAX_CANDIDATES_PER_BRANCH).padStart(3, "0")}`);
    expect(gh.calls).toHaveLength(1 + MAX_CANDIDATES_PER_BRANCH);
  });

  it("surfaces files that do not decode or parse as problems", async () => {
    const wrongId = candidateText("good-one-cccccc", { id: "candidate:Not Valid" });
    const gh = contents(
      [listed("a-json.json", 1, 9), listed("b-schema.json", 2, 200), listed("c-binary.json", 3, 2), listed("d-encoding.json", 4, 2), listed("e-renamed.json", 5, 300)],
      {
        [sha(1)]: { stdout: blob(1, "not json") },
        [sha(2)]: { stdout: blob(2, wrongId) },
        [sha(3)]: { stdout: JSON.stringify({ sha: sha(3), size: 2, encoding: "base64", content: Buffer.from([0xff, 0xfe]).toString("base64") }) },
        [sha(4)]: { stdout: JSON.stringify({ sha: sha(4), size: 2, encoding: "utf-8", content: "{}" }) },
        [sha(5)]: { stdout: blob(5, candidateText("someone-else-eeeeee")) },
      },
    );
    const set = await createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT);
    expect(set.candidates).toEqual([]);
    expect(set.problems).toEqual([
      "a-json.json: not JSON",
      expect.stringMatching(/^b-schema\.json: id: a candidate id is/u),
      "c-binary.json: not UTF-8 text",
      "d-encoding.json: GitHub did not return its content",
      "e-renamed.json: its id candidate:someone-else-eeeeee belongs in someone-else-eeeeee.json",
    ]);
  });

  it("reports a candidates path that is not a folder", async () => {
    const gh = fakeGh(() => ({ stdout: JSON.stringify({ name: "candidates", path: ".rerail/candidates", type: "symlink", target: "/etc", size: 4, sha: sha(1) }) }));
    await expect(createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT))
      .resolves.toEqual({ candidates: [], problems: [".rerail/candidates: not a folder"], truncated: false });
  });

  it("throws when a listed file cannot be fetched, rather than dropping it", async () => {
    const gh = contents([listed("gone-one-dddddd.json", 1, 100)], { [sha(1)]: { code: 1, stderr: "gh: Server Error (HTTP 500)\n" } });
    await expect(createGithubClient({ env, spawn: gh.spawn }).candidatesAt(REPOSITORY, COMMIT)).rejects.toMatchObject({ reason: "failed", status: 500 });
  });
});

describe("authorPermission", () => {
  async function permission(reply: Reply): Promise<string> {
    const gh = fakeGh(() => reply);
    return await createGithubClient({ env, spawn: gh.spawn }).authorPermission(REPOSITORY, "jbarnes");
  }

  it("reads a built-in role before the base permission", async () => {
    // `gh api repos/{o}/{r}/collaborators/{login}/permission --jq '{permission, role_name}'` → {"permission":"admin","role_name":"admin"}
    await expect(permission({ stdout: JSON.stringify({ permission: "admin", role_name: "admin", user: { login: "jbarnes" } }) })).resolves.toBe("admin");
    await expect(permission({ stdout: JSON.stringify({ permission: "write", role_name: "maintain" }) })).resolves.toBe("maintain");
    await expect(permission({ stdout: JSON.stringify({ permission: "read", role_name: "triage" }) })).resolves.toBe("triage");
    await expect(permission({ stdout: JSON.stringify({ permission: "write", role_name: "release-manager" }) })).resolves.toBe("write");
    await expect(permission({ stdout: JSON.stringify({ permission: "read", role_name: "read" }) })).resolves.toBe("read");
    await expect(permission({ stdout: JSON.stringify({ permission: "none" }) })).resolves.toBe("none");
  });

  it("is none when GitHub does not know the author, and unknown when it will not say", async () => {
    await expect(permission(notFound())).resolves.toBe("none");
    await expect(permission({ code: 1, stderr: "gh: Must have push access to view collaborator permission. (HTTP 403)\n" })).resolves.toBe("unknown");
    await expect(permission({ stdout: JSON.stringify({ permission: "constructor" }) })).resolves.toBe("unknown");
    await expect(permission({ stdout: "<html>" })).resolves.toBe("unknown");
  });

  it("encodes a bot login in the path", async () => {
    const gh = fakeGh(() => ({ stdout: JSON.stringify({ permission: "write", role_name: "write" }) }));
    await createGithubClient({ env, spawn: gh.spawn }).authorPermission(REPOSITORY, "dependabot[bot]");
    expect(gh.calls[0]?.args.at(-1)).toBe("repos/neralabs/rerail/collaborators/dependabot%5Bbot%5D/permission");
  });
});
