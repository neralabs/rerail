/**
 * Merge Assist's reads from GitHub, through the developer's own `gh` sign-in: whether `gh` is signed in, the open pull
 * requests, the candidate files at a pull request's head commit, and an author's permission on the repository.
 *
 * `gh` only ever runs `auth status`, `pr list` and `api` with GET, with no shell, prompts off, a timeout and an output
 * cap. Everything it prints is untrusted: each response is validated, and every value placed in an API path is checked
 * for shape first. See docs/merge-assist-prd.md §7.
 */
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { z } from "zod";
import {
  CANDIDATE_DIRECTORY,
  type Candidate,
  isCandidateFileName,
  MAX_CANDIDATE_BYTES,
} from "../../engine/merge-assist/candidate.js";
import {
  CANDIDATE_NOT_REGULAR,
  CANDIDATE_TOO_LARGE,
  candidateFromText,
  capCandidateNames,
  type CandidateSet,
} from "../../engine/merge-assist/candidate-files.js";
import type { GithubReadiness } from "../../engine/merge-assist/model.js";
import { resolveExecutable } from "../npm.js";

export interface GithubRepository { readonly host: string; readonly owner: string; readonly name: string }

export interface PullRequestSummary {
  number: number;
  title: string;
  url: string;
  headRefName: string;
  headCommit: string;
  baseRefName: string;
  author: string;
  isDraft: boolean;
  updatedAt: string;
}

export type AuthorPermission = "admin" | "maintain" | "write" | "triage" | "read" | "none" | "unknown";

export interface GithubClient {
  /** `not-github` without a repository; `missing` without `gh`; `signed-out` when `gh auth status` fails for the host. */
  readiness(repository: GithubRepository | null): Promise<GithubReadiness>;
  /** Up to 50 open pull requests; entries that fail validation are left out. Throws `GithubError` when `gh` fails. */
  openPullRequests(repository: GithubRepository): Promise<PullRequestSummary[]>;
  /**
   * The candidate files in `.rerail/candidates/` at `commit`; none when the folder is not there. A file that is too
   * large or does not parse is a problem, not an error. Throws `GithubError` when `gh` fails, so a failed read is never
   * mistaken for a branch without candidates.
   */
  candidatesAt(repository: GithubRepository, commit: string): Promise<CandidateSet>;
  /** The author's permission on the repository: `none` when GitHub does not know them, `unknown` when it will not say. */
  authorPermission(repository: GithubRepository, login: string): Promise<AuthorPermission>;
}

export interface GithubClientOptions {
  readonly env?: NodeJS.ProcessEnv;
  /** Test seam. */
  readonly spawn?: typeof spawn;
  readonly timeoutMs?: number;
}

export const GITHUB_ERROR_REASONS = ["invalid-input", "missing", "timeout", "output", "failed", "invalid-response"] as const;
export type GithubErrorReason = (typeof GITHUB_ERROR_REASONS)[number];

/** A `gh` read that did not happen or did not answer usably; the message says why for a person. */
export class GithubError extends Error {
  constructor(readonly reason: GithubErrorReason, message: string, readonly status: number | null = null) {
    super(message);
    this.name = "GithubError";
  }
}

const GH_TIMEOUT_MS = 20_000;
const GH_OUTPUT_LIMIT_BYTES = 4 * 1024 * 1024;
const KILL_GRACE_MS = 2_000;
const STDERR_TAIL_CHARS = 2_000;
const DETAIL_CHARS = 300;
const PULL_REQUEST_LIMIT = 50;
const MAX_TITLE_LENGTH = 256;
/** How many candidate files are fetched at once. */
const FETCH_CONCURRENCY = 4;
const ACCEPT_HEADER = "Accept: application/vnd.github+json";

const HOST_PATTERN = /^(?=.{1,253}$)[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*$/iu;
const OWNER_PATTERN = /^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/u;
const NAME_PATTERN = /^[A-Za-z0-9._-]{1,100}$/u;
const COMMIT_PATTERN = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u;
const LOGIN_PATTERN = /^[A-Za-z0-9-]{1,39}(?:\[bot\])?$/u;
const BASE64_PATTERN = /^[A-Za-z0-9+/=\r\n]*$/u;

function validRepository(repository: GithubRepository): boolean {
  return HOST_PATTERN.test(repository.host)
    && OWNER_PATTERN.test(repository.owner)
    && NAME_PATTERN.test(repository.name)
    && repository.name !== "."
    && repository.name !== "..";
}

function checkRepository(repository: GithubRepository): void {
  if (!validRepository(repository)) throw new GithubError("invalid-input", "The repository is not a GitHub owner/name Rerail can read.");
}

/** Whether a branch name has the shape `git check-ref-format --branch` accepts. */
function validRefName(name: string): boolean {
  return name.length > 0
    && name.length <= 255
    && !/[\u0000-\u0020\u007f~^:?*[\\]/u.test(name)
    && !name.includes("..")
    && !name.includes("@{")
    && !name.includes("//")
    && !name.includes("/.")
    && !/^[-/.]/u.test(name)
    && !/(?:[/.]|\.lock)$/u.test(name)
    && name !== "@";
}

/** `gh pr list` names an app author `app/<slug>`; the REST API and candidate files call it `<slug>[bot]`. */
function loginOf(author: { login: string; is_bot?: boolean | undefined }): string | null {
  const login = author.login.startsWith("app/") ? `${author.login.slice("app/".length)}[bot]` : author.login;
  return LOGIN_PATTERN.test(login) ? login : null;
}

function cleanTitle(title: string): string {
  return title.replace(/[\p{Cc}\p{Cf}]+/gu, " ").replace(/\s+/gu, " ").trim().slice(0, MAX_TITLE_LENGTH);
}

function pullRequestUrl(url: string, host: string, number: number): string | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "https:" || parsed.hostname !== host.toLowerCase() || parsed.port !== "") return null;
  if (parsed.username !== "" || parsed.password !== "" || parsed.search !== "" || parsed.hash !== "") return null;
  const match = /^\/[A-Za-z0-9-]+\/[A-Za-z0-9._-]+\/pull\/(\d+)$/u.exec(parsed.pathname);
  return match !== null && Number(match[1]) === number ? parsed.href : null;
}

/** Error text from `gh` fit to show a person: one line, short, with anything token-shaped removed. */
function cleanDetail(text: string): string {
  return text
    .replace(/\b(?:gh[oprsu]_|github_pat_)[A-Za-z0-9_]+/gu, "[REDACTED]")
    .replace(/[\p{Cc}\p{Cf}]+/gu, " ")
    .replace(/\s+/gu, " ")
    .trim()
    .slice(-DETAIL_CHARS);
}

/**
 * The environment `gh` runs with: the caller's, so it finds its sign-in, with prompts, pagers and colour off.
 * `GH_ENTERPRISE_TOKEN` and `GITHUB_ENTERPRISE_TOKEN` are removed: `gh` sends them to any host other than github.com
 * (and `*.ghe.com`), and a repository's remote names the host, so a look-alike such as `github.evil.example` would
 * receive the token. Without them `gh` authenticates an Enterprise host only with a login stored by
 * `gh auth login --hostname <host>`, which the person made for that host.
 */
function ghEnvironment(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const result: NodeJS.ProcessEnv = { ...env };
  delete result.GH_FORCE_TTY;
  delete result.CLICOLOR_FORCE;
  delete result.GH_ENTERPRISE_TOKEN;
  delete result.GITHUB_ENTERPRISE_TOKEN;
  return {
    ...result,
    GH_PROMPT_DISABLED: "1",
    GH_NO_UPDATE_NOTIFIER: "1",
    GH_NO_EXTENSION_UPDATE_NOTIFIER: "1",
    GH_SPINNER_DISABLED: "1",
    NO_COLOR: "1",
    GH_PAGER: "cat",
  };
}

interface GhRun {
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}

type ApiResult =
  | { readonly ok: true; readonly value: unknown }
  | { readonly ok: false; readonly status: number | null; readonly detail: string };

const pullRequestEntrySchema = z.object({
  number: z.number().int().positive(),
  title: z.string(),
  url: z.string(),
  headRefName: z.string(),
  headRefOid: z.string().regex(COMMIT_PATTERN),
  baseRefName: z.string(),
  author: z.object({ login: z.string(), is_bot: z.boolean().optional() }),
  isDraft: z.boolean(),
  updatedAt: z.iso.datetime(),
});

const listingEntrySchema = z.object({
  name: z.string(),
  path: z.string(),
  type: z.string(),
  size: z.number().int().nonnegative(),
  sha: z.string().regex(COMMIT_PATTERN),
});

const blobSchema = z.object({
  sha: z.string(),
  size: z.number().int().nonnegative(),
  encoding: z.string(),
  content: z.string(),
});

const permissionSchema = z.object({
  permission: z.string(),
  role_name: z.string().optional(),
});

const userSchema = z.object({ login: z.string() });

const ROLE_PERMISSIONS: Record<string, AuthorPermission> = { admin: "admin", maintain: "maintain", write: "write", triage: "triage" };
const BASE_PERMISSIONS: Record<string, AuthorPermission> = { admin: "admin", write: "write", read: "read", none: "none" };

/** `role_name` names a built-in role more precisely than `permission` (maintain, triage); a custom role falls back. */
function permissionOf(value: z.infer<typeof permissionSchema>): AuthorPermission {
  if (value.role_name !== undefined && Object.hasOwn(ROLE_PERMISSIONS, value.role_name)) return ROLE_PERMISSIONS[value.role_name] ?? "unknown";
  return Object.hasOwn(BASE_PERMISSIONS, value.permission) ? BASE_PERMISSIONS[value.permission] ?? "unknown" : "unknown";
}

const utf8 = new TextDecoder("utf-8", { fatal: true });

/** A candidate file's text from a blob's base64 content, or why it cannot be read. */
function blobText(blob: z.infer<typeof blobSchema>): { ok: true; text: string } | { ok: false; problem: string } {
  if (blob.size > MAX_CANDIDATE_BYTES) return { ok: false, problem: CANDIDATE_TOO_LARGE };
  if (blob.encoding !== "base64" || !BASE64_PATTERN.test(blob.content)) return { ok: false, problem: "GitHub did not return its content" };
  const bytes = Buffer.from(blob.content, "base64");
  if (bytes.byteLength > MAX_CANDIDATE_BYTES) return { ok: false, problem: CANDIDATE_TOO_LARGE };
  try {
    return { ok: true, text: utf8.decode(bytes) };
  } catch {
    return { ok: false, problem: "not UTF-8 text" };
  }
}

async function mapLimited<T, R>(items: readonly T[], limit: number, map: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array<R>(items.length);
  let next = 0;
  const worker = async (): Promise<void> => {
    while (next < items.length) {
      const index = next;
      next += 1;
      results[index] = await map(items[index] as T);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return results;
}

export function createGithubClient(options: GithubClientOptions = {}): GithubClient {
  const spawnProcess = options.spawn ?? spawn;
  const timeoutMs = options.timeoutMs ?? GH_TIMEOUT_MS;
  /** Enterprise hosts `gh auth status` has confirmed a stored login for. */
  const signedInHosts = new Set<string>();

  /** Runs `gh` once to completion. Throws `GithubError` when it cannot start, times out, or prints too much. */
  async function run(args: readonly string[]): Promise<GhRun> {
    const env = ghEnvironment(options.env ?? process.env);
    const command = resolveExecutable("gh", env);
    if (command === undefined) throw new GithubError("missing", "gh is not installed or not on PATH.");
    const { promise, resolve } = Promise.withResolvers<GhRun | GithubError>();
    const child = spawnProcess(command, [...args], {
      cwd: tmpdir(),
      env,
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const chunks: Buffer[] = [];
    let bytes = 0;
    let stderr = "";
    let stopped: "timeout" | "output" | undefined;
    let settled = false;
    let killer: NodeJS.Timeout | undefined;
    const settle = (result: GhRun | GithubError): void => {
      if (settled) return;
      settled = true;
      resolve(result);
    };
    const stop = (reason: "timeout" | "output"): void => {
      if (stopped !== undefined) return;
      stopped = reason;
      child.kill("SIGTERM");
      killer = setTimeout(() => {
        if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
      }, KILL_GRACE_MS);
      killer.unref();
    };
    const timer = setTimeout(() => stop("timeout"), timeoutMs);
    child.stdout?.on("data", (chunk: Buffer) => {
      if (stopped !== undefined) return;
      bytes += chunk.byteLength;
      if (bytes > GH_OUTPUT_LIMIT_BYTES) {
        stop("output");
        return;
      }
      chunks.push(chunk);
    });
    child.stderr?.setEncoding("utf8").on("data", (chunk: string) => {
      stderr = (stderr + chunk).slice(-STDERR_TAIL_CHARS);
    });
    child.on("error", (error: NodeJS.ErrnoException) => {
      settle(error.code === "ENOENT"
        ? new GithubError("missing", "gh is not installed or not on PATH.")
        : new GithubError("failed", `gh could not be started: ${cleanDetail(error.message)}`));
    });
    child.on("close", code => {
      if (stopped === "timeout") settle(new GithubError("timeout", `gh did not answer within ${timeoutMs / 1000} s and was stopped.`));
      else if (stopped === "output") settle(new GithubError("output", `gh printed more than ${GH_OUTPUT_LIMIT_BYTES / (1024 * 1024)} MiB and was stopped.`));
      else settle({ code: code ?? 1, stdout: Buffer.concat(chunks).toString("utf8"), stderr });
    });
    const result = await promise;
    clearTimeout(timer);
    if (result instanceof GithubError) {
      clearTimeout(killer);
      throw result;
    }
    return result;
  }

  function json(stdout: string): unknown {
    try {
      return JSON.parse(stdout) as unknown;
    } catch {
      throw new GithubError("invalid-response", "gh printed something other than JSON.");
    }
  }

  /** `gh api` with GET for one REST path. A non-zero exit is a result with GitHub's HTTP status when gh reports one. */
  async function apiGet(host: string, path: string): Promise<ApiResult> {
    const result = await run(["api", "--method", "GET", "--hostname", host, "-H", ACCEPT_HEADER, path]);
    if (result.code === 0) return { ok: true, value: json(result.stdout) };
    const status = /\(HTTP (\d{3})\)/u.exec(result.stderr)?.[1];
    return { ok: false, status: status === undefined ? null : Number(status), detail: cleanDetail(result.stderr) };
  }

  function failed(what: string, result: { status: number | null; detail: string }): GithubError {
    return new GithubError("failed", `gh could not ${what}${result.detail === "" ? "." : `: ${result.detail}`}`, result.status);
  }

  function repositoryPath(repository: GithubRepository): string {
    return `repos/${encodeURIComponent(repository.owner)}/${encodeURIComponent(repository.name)}`;
  }

  async function readiness(repository: GithubRepository | null): Promise<GithubReadiness> {
    if (repository === null || !validRepository(repository)) return { state: "not-github", login: null, repository: null };
    const name = `${repository.owner}/${repository.name}`;
    try {
      const status = await run(["auth", "status", "--hostname", repository.host]);
      if (status.code !== 0) return { state: "signed-out", login: null, repository: name };
      signedInHosts.add(repository.host);
    } catch (error) {
      if (!(error instanceof GithubError)) throw error;
      return { state: error.reason === "missing" ? "missing" : "signed-out", login: null, repository: name };
    }
    let login: string | null = null;
    try {
      const user = await apiGet(repository.host, "user");
      const parsed = user.ok ? userSchema.safeParse(user.value) : null;
      if (parsed?.success === true && LOGIN_PATTERN.test(parsed.data.login)) login = parsed.data.login;
    } catch (error) {
      // Signed in, but the login could not be read this time; the state stands without it.
      if (!(error instanceof GithubError)) throw error;
    }
    return { state: "signed-in", login, repository: name };
  }

  /**
   * Refuses to read a host other than github.com unless `gh` holds a stored login for it, so a repository whose remote
   * names a host the person never signed in to is not contacted at all.
   */
  async function requireSignedIn(host: string): Promise<void> {
    if (host === "github.com" || signedInHosts.has(host)) return;
    const status = await run(["auth", "status", "--hostname", host]);
    if (status.code !== 0) throw new GithubError("failed", `gh is not signed in to ${host}; Rerail reads a GitHub Enterprise host only after gh auth login --hostname ${host}.`);
    signedInHosts.add(host);
  }

  async function openPullRequests(repository: GithubRepository): Promise<PullRequestSummary[]> {
    checkRepository(repository);
    await requireSignedIn(repository.host);
    const result = await run([
      "pr", "list",
      `--repo=${repository.host}/${repository.owner}/${repository.name}`,
      "--state", "open",
      "--limit", String(PULL_REQUEST_LIMIT),
      "--json", "number,title,url,headRefName,headRefOid,baseRefName,author,isDraft,updatedAt",
    ]);
    if (result.code !== 0) throw failed("list open pull requests", { status: null, detail: cleanDetail(result.stderr) });
    const entries = z.array(z.unknown()).safeParse(json(result.stdout));
    if (!entries.success) throw new GithubError("invalid-response", "gh pr list did not print a list.");
    const pullRequests: PullRequestSummary[] = [];
    for (const entry of entries.data) {
      const parsed = pullRequestEntrySchema.safeParse(entry);
      if (!parsed.success) continue;
      const value = parsed.data;
      const author = loginOf(value.author);
      const url = pullRequestUrl(value.url, repository.host, value.number);
      if (author === null || url === null || !validRefName(value.headRefName) || !validRefName(value.baseRefName)) continue;
      pullRequests.push({
        number: value.number,
        title: cleanTitle(value.title),
        url,
        headRefName: value.headRefName,
        headCommit: value.headRefOid,
        baseRefName: value.baseRefName,
        author,
        isDraft: value.isDraft,
        updatedAt: value.updatedAt,
      });
    }
    return pullRequests;
  }

  async function candidatesAt(repository: GithubRepository, commit: string): Promise<CandidateSet> {
    checkRepository(repository);
    if (!COMMIT_PATTERN.test(commit)) throw new GithubError("invalid-input", "The commit is not a full Git object id.");
    await requireSignedIn(repository.host);
    const base = repositoryPath(repository);
    const listing = await apiGet(repository.host, `${base}/contents/${CANDIDATE_DIRECTORY}?ref=${commit}`);
    if (!listing.ok && listing.status === 404) return { candidates: [], problems: [], truncated: false };
    if (!listing.ok) throw failed(`list ${CANDIDATE_DIRECTORY} at ${commit.slice(0, 12)}`, listing);
    if (!Array.isArray(listing.value)) return { candidates: [], problems: [`${CANDIDATE_DIRECTORY}: not a folder`], truncated: false };

    const problems: string[] = [];
    const files = new Map<string, z.infer<typeof listingEntrySchema>>();
    for (const entry of listing.value) {
      const name = z.object({ name: z.string() }).safeParse(entry);
      if (!name.success || !isCandidateFileName(name.data.name)) continue;
      const parsed = listingEntrySchema.safeParse(entry);
      if (!parsed.success || parsed.data.path !== `${CANDIDATE_DIRECTORY}/${parsed.data.name}`) {
        problems.push(`${name.data.name}: GitHub listed it in a shape Rerail does not read`);
      } else if (parsed.data.type !== "file") {
        problems.push(`${name.data.name}: ${CANDIDATE_NOT_REGULAR}`);
      } else {
        files.set(parsed.data.name, parsed.data);
      }
    }
    const capped = capCandidateNames([...files.keys()]);
    problems.push(...capped.problems);

    const read = capped.read.flatMap(name => files.get(name) ?? []);
    const outcomes = await mapLimited(read, FETCH_CONCURRENCY, async (file): Promise<{ candidate: Candidate } | { problem: string }> => {
      if (file.size > MAX_CANDIDATE_BYTES) return { problem: `${file.name}: ${CANDIDATE_TOO_LARGE}` };
      const blob = await apiGet(repository.host, `${base}/git/blobs/${file.sha}`);
      if (!blob.ok) throw failed(`read ${CANDIDATE_DIRECTORY}/${file.name} at ${commit.slice(0, 12)}`, blob);
      const parsed = blobSchema.safeParse(blob.value);
      if (!parsed.success || parsed.data.sha !== file.sha) throw new GithubError("invalid-response", `GitHub returned something other than ${CANDIDATE_DIRECTORY}/${file.name}.`);
      const text = blobText(parsed.data);
      return text.ok ? candidateFromText(file.name, text.text) : { problem: `${file.name}: ${text.problem}` };
    });
    const candidates: Candidate[] = [];
    for (const outcome of outcomes) {
      if ("candidate" in outcome) candidates.push(outcome.candidate);
      else problems.push(outcome.problem);
    }
    return { candidates, problems, truncated: capped.truncated };
  }

  async function authorPermission(repository: GithubRepository, login: string): Promise<AuthorPermission> {
    checkRepository(repository);
    if (!LOGIN_PATTERN.test(login)) throw new GithubError("invalid-input", "The author is not a GitHub login.");
    let result: ApiResult;
    try {
      await requireSignedIn(repository.host);
      result = await apiGet(repository.host, `${repositoryPath(repository)}/collaborators/${encodeURIComponent(login)}/permission`);
    } catch (error) {
      if (error instanceof GithubError) return "unknown";
      throw error;
    }
    if (!result.ok) return result.status === 404 ? "none" : "unknown";
    const parsed = permissionSchema.safeParse(result.value);
    return parsed.success ? permissionOf(parsed.data) : "unknown";
  }

  return { readiness, openPullRequests, candidatesAt, authorPermission };
}
