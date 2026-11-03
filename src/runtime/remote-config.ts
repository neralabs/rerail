/**
 * Internal testing: the remote sink's configuration in the Rerail home. Nothing leaves the machine until a remote is
 * added. Each remote is an HTTPS URL and a bearer token (docs/remote-sink.md):
 *
 * - `<home>/remote.json` (`0600`): the remotes (name, URL, whether event content is sent) and the repositories opted
 *   out of every remote.
 * - `<home>/remote-tokens/<name>` (`0600`, folder `0700`): each remote's token, alone in its file, never in the
 *   database (support bundles and `rerail scrub` read the database) and never printed.
 * - `<home>/machine-id` (`0600`): a random UUID made once, which names this machine to every remote.
 */
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { join } from "node:path";
import { keepPrivate, makeRerailHome } from "../engine/storage/private-files.js";
import { defaultRerailHome, MACHINE_ID_FILE, REMOTE_CONFIG_FILE, REMOTE_TOKENS_FOLDER } from "../engine/storage/rerail-home.js";

export interface RemoteEntry {
  /** Unique in the home; names the token file and the outbox's cursors. */
  readonly name: string;
  readonly url: string;
  /** False: events go without their payloads (`"content":"omitted"`). */
  readonly content: boolean;
  readonly addedAt: number;
}

export interface RemoteConfig {
  readonly remotes: readonly RemoteEntry[];
  /** Repository keys no remote receives anything of. */
  readonly excludedRepositories: readonly string[];
}

const EMPTY: RemoteConfig = { remotes: [], excludedRepositories: [] };
const NAME = /^[a-z0-9][a-z0-9._-]{0,62}$/u;
const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** `content` as the whole of `file`, owner-only, through a temporary file renamed over it. */
function writePrivate(file: string, content: string): void {
  const temporary = `${file}.${process.pid}.tmp`;
  try {
    writeFileSync(temporary, content, { mode: 0o600 });
    renameSync(temporary, file);
  } finally {
    rmSync(temporary, { force: true });
  }
}

/** The remote's URL, checked: HTTPS, or HTTP to this machine only (a local test server); no credentials in it. */
export function remoteUrl(value: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`Not a URL: ${value}`);
  }
  if (url.username !== "" || url.password !== "") throw new Error("Put the token in --token-file or --token-stdin, not in the URL");
  if (url.protocol !== "https:" && !(url.protocol === "http:" && LOOPBACK.has(url.hostname)))
    throw new Error(`A remote must use https (http only to this machine): ${value}`);
  return url.toString().replace(/\/+$/u, "");
}

export function readRemoteConfig(home: string = defaultRerailHome()): RemoteConfig {
  let text: string;
  try {
    text = readFileSync(join(home, REMOTE_CONFIG_FILE), "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return EMPTY;
    throw error;
  }
  const parsed = JSON.parse(text) as Partial<RemoteConfig>;
  return {
    remotes: Array.isArray(parsed.remotes) ? parsed.remotes.filter(remote => typeof remote?.name === "string" && typeof remote.url === "string") : [],
    excludedRepositories: Array.isArray(parsed.excludedRepositories) ? parsed.excludedRepositories.filter(key => typeof key === "string") : [],
  };
}

function writeRemoteConfig(home: string, config: RemoteConfig): void {
  makeRerailHome(home);
  writePrivate(join(home, REMOTE_CONFIG_FILE), `${JSON.stringify(config, null, 2)}\n`);
}

function tokenFile(home: string, name: string): string {
  return join(home, REMOTE_TOKENS_FOLDER, name);
}

/** The remote's token, or undefined when its file is gone. */
export function readRemoteToken(name: string, home: string = defaultRerailHome()): string | undefined {
  try {
    return readFileSync(tokenFile(home, name), "utf8").trim() || undefined;
  } catch {
    return undefined;
  }
}

/** A name for `url` when none is given: its host, made a valid name. */
export function defaultRemoteName(url: string): string {
  const name = new URL(url).hostname.toLowerCase().replace(/[^a-z0-9._-]+/gu, "-").replace(/^[^a-z0-9]+/u, "").slice(0, 63);
  return name === "" ? "remote" : name;
}

export function addRemote(
  options: { url: string; token: string; name?: string; content: boolean },
  home: string = defaultRerailHome(),
  now: number = Date.now(),
): RemoteEntry {
  const url = remoteUrl(options.url);
  const name = options.name ?? defaultRemoteName(url);
  if (!NAME.test(name)) throw new Error(`A remote's name is lower-case letters, digits, '.', '_' and '-': ${name}`);
  const token = options.token.trim();
  if (token === "" || /\s/u.test(token)) throw new Error("The token is empty or has whitespace in it");
  const config = readRemoteConfig(home);
  if (config.remotes.some(remote => remote.name === name)) throw new Error(`A remote named ${name} exists; remove it first, or pass --name`);
  makeRerailHome(home);
  const folder = join(home, REMOTE_TOKENS_FOLDER);
  mkdirSync(folder, { recursive: true, mode: 0o700 });
  keepPrivate(folder);
  writePrivate(tokenFile(home, name), `${token}\n`);
  const entry: RemoteEntry = { name, url, content: options.content, addedAt: now };
  writeRemoteConfig(home, { ...config, remotes: [...config.remotes, entry] });
  return entry;
}

/** Removes the remote and its token; false when there is none of that name or URL. */
export function removeRemote(nameOrUrl: string, home: string = defaultRerailHome()): RemoteEntry | undefined {
  const config = readRemoteConfig(home);
  const removed = config.remotes.find(remote => remote.name === nameOrUrl || remote.url === nameOrUrl.replace(/\/+$/u, ""));
  if (removed === undefined) return undefined;
  writeRemoteConfig(home, { ...config, remotes: config.remotes.filter(remote => remote !== removed) });
  rmSync(tokenFile(home, removed.name), { force: true });
  return removed;
}

/** Opts `repositoryKey` out of every remote (`excluded`), or back in. */
export function setRepositoryExcluded(repositoryKey: string, excluded: boolean, home: string = defaultRerailHome()): void {
  const config = readRemoteConfig(home);
  const others = config.excludedRepositories.filter(key => key !== repositoryKey);
  writeRemoteConfig(home, { ...config, excludedRepositories: excluded ? [...others, repositoryKey].sort() : others });
}

/** This machine's id for remotes, made once and kept in the home. */
export function machineId(home: string = defaultRerailHome()): string {
  const file = join(home, MACHINE_ID_FILE);
  try {
    const existing = readFileSync(file, "utf8").trim();
    if (/^[0-9a-f-]{36}$/u.test(existing)) return existing;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  makeRerailHome(home);
  const id = randomUUID();
  try {
    // Exclusive create: two processes making it at once agree on the first one's id.
    writeFileSync(file, `${id}\n`, { mode: 0o600, flag: "wx" });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    return readFileSync(file, "utf8").trim();
  }
  return id;
}

export function machineLabel(): string {
  return hostname();
}
