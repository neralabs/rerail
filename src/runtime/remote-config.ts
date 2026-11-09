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
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { renameWithRetrySync } from "../engine/file-replace.js";
import { hostname } from "node:os";
import { join } from "node:path";
import { execFileSync } from "../engine/child-process.js";
import { keepPrivate, makeRerailHome } from "../engine/storage/private-files.js";
import { defaultRerailHome, MACHINE_ID_FILE, REMOTE_CONFIG_FILE, REMOTE_TOKENS_FOLDER } from "../engine/storage/rerail-home.js";

export interface RemoteEntry {
  /** Unique in the home; names the token file. */
  readonly name: string;
  /**
   * Made new by every `add`: the outbox keys its cursors and sent hashes by it ({@link remoteStateKey}), so a remote
   * added again under a removed one's name starts from nothing, and a sync still running for the removed one cannot
   * write state the new one would read. Absent in a configuration written before it existed.
   */
  readonly id?: string;
  readonly url: string;
  /** False: events go without their payloads (`"content":"omitted"`). */
  readonly content: boolean;
  /** What this machine is called to this remote (`--label`); the computer's name when absent. */
  readonly label?: string;
  readonly addedAt: number;
}

/**
 * A repository opted out of every remote, until `include` (`to`, null while it still is). What the repository
 * recorded before `to` and no remote had yet is never sent, whenever the sender meets it, so `include` sends only what
 * comes after it. `from` is 0: an exclusion also holds back what was recorded before it and not sent yet.
 */
export interface RepositoryExclusion {
  readonly repositoryKey: string;
  readonly from: number;
  readonly to: number | null;
}

export interface RemoteConfig {
  readonly remotes: readonly RemoteEntry[];
  /** Repository keys opted out now: those with an exclusion still open. */
  readonly excludedRepositories: readonly string[];
  /** Every exclusion, open or closed, kept so what was recorded while excluded stays unsent. */
  readonly exclusions: readonly RepositoryExclusion[];
}

const EMPTY: RemoteConfig = { remotes: [], excludedRepositories: [], exclusions: [] };

/** The key the outbox keeps a remote's state under: its per-add id, or its name for one added before ids. */
export function remoteStateKey(remote: Pick<RemoteEntry, "name" | "id">): string {
  return remote.id ?? remote.name;
}

/** Whether something of `repositoryKey` recorded at `at` falls in one of its exclusions. */
export function excludedAt(exclusions: readonly RepositoryExclusion[], repositoryKey: string, at: number): boolean {
  return exclusions.some(exclusion => exclusion.repositoryKey === repositoryKey && exclusion.from <= at && (exclusion.to === null || at < exclusion.to));
}
const NAME = /^[a-z0-9][a-z0-9._-]{0,62}$/u;
const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** `content` as the whole of `file`, owner-only, through a temporary file renamed over it. */
function writePrivate(file: string, content: string): void {
  const temporary = `${file}.${process.pid}.tmp`;
  try {
    writeFileSync(temporary, content, { mode: 0o600 });
    renameWithRetrySync(temporary, file);
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
  const listed = Array.isArray(parsed.exclusions)
    ? parsed.exclusions.filter(exclusion => typeof exclusion?.repositoryKey === "string" && typeof exclusion.from === "number"
      && (exclusion.to === null || typeof exclusion.to === "number"))
    : undefined;
  // A configuration from before exclusions had times: each excluded repository counts as excluded since always.
  const exclusions = listed ?? (Array.isArray(parsed.excludedRepositories) ? parsed.excludedRepositories : [])
    .filter(key => typeof key === "string").map(repositoryKey => ({ repositoryKey, from: 0, to: null }));
  return {
    remotes: Array.isArray(parsed.remotes) ? parsed.remotes.filter(remote => typeof remote?.name === "string" && typeof remote.url === "string") : [],
    excludedRepositories: [...new Set(exclusions.filter(exclusion => exclusion.to === null).map(exclusion => exclusion.repositoryKey))].sort(),
    exclusions,
  };
}

function writeRemoteConfig(home: string, config: Omit<RemoteConfig, "excludedRepositories">): void {
  makeRerailHome(home);
  const excludedRepositories = [...new Set(config.exclusions.filter(exclusion => exclusion.to === null).map(exclusion => exclusion.repositoryKey))].sort();
  writePrivate(join(home, REMOTE_CONFIG_FILE), `${JSON.stringify({ remotes: config.remotes, excludedRepositories, exclusions: config.exclusions }, null, 2)}\n`);
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
  options: { url: string; token: string; name?: string; content: boolean; label?: string },
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
  const label = options.label?.trim();
  if (label !== undefined && (label === "" || label.length > 100 || /[\p{Cc}]/u.test(label))) throw new Error("A label is 1 to 100 characters, with no control characters");
  const entry: RemoteEntry = { name, id: randomUUID(), url, content: options.content, addedAt: now, ...(label === undefined ? {} : { label }) };
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

/**
 * Opts `repositoryKey` out of every remote (`excluded`): nothing of it not sent yet is sent. Or back in from `now`:
 * only what it records from then on is sent. The exclusion is kept, closed, not deleted, so what it held back stays
 * unsent.
 */
export function setRepositoryExcluded(repositoryKey: string, excluded: boolean, home: string = defaultRerailHome(), now: number = Date.now()): void {
  const config = readRemoteConfig(home);
  const open = config.exclusions.some(exclusion => exclusion.repositoryKey === repositoryKey && exclusion.to === null);
  if (excluded === open) return;
  const exclusions = excluded
    ? [...config.exclusions, { repositoryKey, from: 0, to: null }]
    : config.exclusions.map(exclusion => (exclusion.repositoryKey === repositoryKey && exclusion.to === null ? { ...exclusion, to: now } : exclusion));
  writeRemoteConfig(home, { ...config, exclusions });
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

let computerName: string | undefined;

/**
 * What a person calls this machine: on macOS its Computer Name (System Settings › General › Sharing), since its host
 * name is often one the network handed out; elsewhere, and when that cannot be read, its host name.
 */
export function machineLabel(): string {
  if (computerName !== undefined) return computerName;
  computerName = hostname();
  if (process.platform === "darwin") {
    try {
      const name = execFileSync("/usr/sbin/scutil", ["--get", "ComputerName"], { encoding: "utf8", timeout: 2_000, stdio: ["ignore", "pipe", "ignore"] }).trim();
      if (name !== "") computerName = name;
    } catch { /* the host name stands */ }
  }
  return computerName;
}
