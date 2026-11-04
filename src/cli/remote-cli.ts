/**
 * Internal testing: `rerail remote`, the remote sink's commands (docs/remote-sink.md). With no remote added, nothing
 * leaves this machine.
 */
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { lookupCheckout } from "../engine/catalog/repository-registry.js";
import { readCloneId, readRemoteAddress, repositoryIdentityLocation, repositoryKeyOf } from "../engine/catalog/repository-identity.js";
import { openRerailDatabase, RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import { addRemote, readRemoteConfig, remoteStateKey, removeRemote, setRepositoryExcluded } from "../runtime/remote-config.js";
import { RemoteOutbox, type RemoteStatus } from "../runtime/remote-outbox.js";

export interface RemoteCliOptions {
  home: string;
  stdout?: Pick<NodeJS.WriteStream, "write">;
  /** The token `--token-stdin` reads; standard input by default. */
  readStdin?: () => Promise<string>;
  outbox?: RemoteOutbox;
}

const USAGE = "rerail remote add <url> [--name <name>] [--label <machine label>] (--token-stdin | --token-file <file>) [--no-content] | list | remove <name|url> | status | push | exclude [--repo <dir>] | include [--repo <dir>]";

async function readAll(stream: NodeJS.ReadableStream): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(typeof chunk === "string" ? Buffer.from(chunk) : chunk);
  return Buffer.concat(chunks).toString("utf8");
}

function ago(at: number | null, now: number): string {
  if (at === null) return "never";
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 120) return `${seconds}s ago`;
  if (seconds < 7200) return `${Math.round(seconds / 60)}m ago`;
  return `${Math.round(seconds / 3600)}h ago`;
}

/** One line per remote: where it is, when it last took a batch, what waits, and the last error. */
export function describeRemoteStatus(status: RemoteStatus, now: number = Date.now()): string {
  const retry = status.nextAttemptAt !== null && status.nextAttemptAt > now ? `, retrying in ${Math.ceil((status.nextAttemptAt - now) / 1000)}s` : "";
  const error = status.lastError === null ? "" : `; last error: ${status.lastError}`;
  return `Remote ${status.name} (${status.url}${status.content ? "" : ", no content"}): last sent ${ago(status.lastSuccessAt, now)}, ${status.pendingEvents} events and ${status.queuedScans} scans waiting${retry}${error}`;
}

/** The repository key of the checkout at `dir`, read without writing anything. */
function repositoryKeyAt(home: string, dir: string): string {
  const databasePath = join(home, RERAIL_DATABASE_FILE);
  if (existsSync(databasePath)) {
    const database = openRerailDatabase(databasePath, { readonly: true });
    try {
      const checkout = lookupCheckout(database, dir);
      if (checkout !== undefined) return checkout.repositoryKey;
    } finally {
      database.$client.close();
    }
  }
  const { workspaceRoot } = repositoryIdentityLocation(dir);
  const remote = readRemoteAddress(workspaceRoot);
  const cloneId = readCloneId(workspaceRoot);
  if (remote === undefined && cloneId === undefined) throw new Error(`Rerail has not seen the repository at ${dir} yet, and it has no Git remote to name it by`);
  return repositoryKeyOf(remote, cloneId ?? "");
}

export async function runRemoteCommand(args: readonly string[], options: RemoteCliOptions): Promise<number> {
  const out = options.stdout ?? process.stdout;
  const [action, ...rest] = args;
  const outbox = options.outbox ?? new RemoteOutbox({ home: options.home });
  const unknown = (argument: string): string => `Unknown option: ${argument}\nUsage: ${USAGE}`;

  if (action === "add") {
    const [url, ...flagArgs] = rest;
    if (url === undefined || url.startsWith("--")) throw new Error(`rerail remote add needs a URL\nUsage: ${USAGE}`);
    const parsed = parseArguments(flagArgs, { flags: ["--token-stdin", "--no-content"], options: ["--name", "--token-file", "--label"] }, unknown);
    const tokenFile = parsed.options.get("--token-file");
    if ((tokenFile === undefined) === !parsed.flags.has("--token-stdin")) throw new Error("Give the token with exactly one of --token-stdin or --token-file <file>");
    const token = tokenFile !== undefined ? readFileSync(resolve(tokenFile), "utf8") : await (options.readStdin ?? (() => readAll(process.stdin)))();
    const name = parsed.options.get("--name");
    const label = parsed.options.get("--label");
    const entry = addRemote({
      url, token, content: !parsed.flags.has("--no-content"), ...(name === undefined ? {} : { name }), ...(label === undefined ? {} : { label }),
    }, options.home);
    out.write(`Added remote ${entry.name} (${entry.url})${entry.content ? "" : ", without event content"}. Rerail sends this machine's sessions, events, audits and scans to it from now on, starting with what it already holds.\n`);
    return 0;
  }
  if (action === "list") {
    const parsed = parseArguments(rest, { flags: ["--json"], options: [] }, unknown);
    const config = readRemoteConfig(options.home);
    if (parsed.flags.has("--json")) {
      out.write(`${JSON.stringify(config)}\n`);
      return 0;
    }
    if (config.remotes.length === 0) out.write("No remotes: nothing leaves this machine.\n");
    for (const remote of config.remotes) out.write(`${remote.name}\t${remote.url}${remote.content ? "" : "\t(no content)"}\n`);
    for (const key of config.excludedRepositories) out.write(`excluded\t${key}\n`);
    return 0;
  }
  if (action === "remove") {
    const [target, ...extra] = rest;
    if (target === undefined || extra.length > 0) throw new Error(`rerail remote remove takes one name or URL\nUsage: ${USAGE}`);
    const removed = removeRemote(target, options.home);
    if (removed === undefined) throw new Error(`No remote named ${target}`);
    outbox.forget(remoteStateKey(removed));
    out.write(`Removed remote ${removed.name}; its token is deleted.\n`);
    return 0;
  }
  if (action === "status") {
    const parsed = parseArguments(rest, { flags: ["--json"], options: [] }, unknown);
    const statuses = outbox.status();
    if (parsed.flags.has("--json")) out.write(`${JSON.stringify({ remotes: statuses })}\n`);
    else if (statuses.length === 0) out.write("No remotes: nothing leaves this machine.\n");
    else for (const status of statuses) out.write(`${describeRemoteStatus(status)}\n`);
    return statuses.some(status => status.lastError !== null && status.consecutiveFailures > 0) ? 1 : 0;
  }
  if (action === "push") {
    const parsed = parseArguments(rest, { flags: ["--json"], options: [] }, unknown);
    const results = await outbox.sync({ force: true, maxBatches: Number.MAX_SAFE_INTEGER });
    if (parsed.flags.has("--json")) out.write(`${JSON.stringify({ results })}\n`);
    else if (results.length === 0) out.write("No remotes: nothing leaves this machine.\n");
    else for (const result of results) {
      const outcome = result.skipped === "locked" ? "skipped: another Rerail process is sending to it"
        : result.skipped === "no-token" ? "skipped: its token file is missing; remove and add it again"
          : result.skipped === "removed" ? "stopped: it was removed while sending"
          : `${result.batches} batches, ${result.records} records sent${result.error === undefined ? "" : `; failed: ${result.error}`}`;
      out.write(`Remote ${result.remote}: ${outcome}\n`);
    }
    return results.some(result => result.error !== undefined || result.skipped === "no-token") ? 1 : 0;
  }
  if (action === "exclude" || action === "include") {
    const parsed = parseArguments(rest, { flags: [], options: ["--repo"] }, unknown);
    const key = repositoryKeyAt(options.home, resolve(parsed.options.get("--repo") ?? process.cwd()));
    setRepositoryExcluded(key, action === "exclude", options.home);
    out.write(action === "exclude"
      ? `No remote receives anything of ${key} from now on; what was sent already stays sent.\n`
      : `Remotes receive ${key} again from now on; events recorded while it was excluded are not sent.\n`);
    return 0;
  }
  throw new Error(`Usage: ${USAGE}`);
}
