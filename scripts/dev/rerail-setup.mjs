#!/usr/bin/env node
// `just rerail`: check that npm can read @neralabs/rerail, then hand off to `npx @neralabs/rerail setup`.
// Everything after the handoff (versions under ~/.rerail, coding-agent hooks, the daemon, updates) is owned by
// `rerail setup`. npm's own login is the only credential: this script never writes ~/.npmrc or a token.
import { spawn, spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { constants as osConstants, homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { RestoreError, parseRestoreArguments, restoreInstalled, selectBackup } from "./rerail-restore.mjs";

export const SCOPE = "@neralabs";
export const PACKAGE = `${SCOPE}/rerail`;
export const MIN_NODE_MAJOR = 24;
export const DEFAULT_TAG = "latest";
const NPM_PROBE_FLAGS = ["--fetch-timeout=10000", "--fetch-retries=0"];
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]"]);
/** npm reads `npm_config_<key>` from the environment, so every npm the installer starts sees the same scope registry. */
const SCOPE_REGISTRY_ENV = `npm_config_${SCOPE}:registry`;

export const USAGE = [
  "usage: just rerail [--tag <tag>] [--restore [backup-dir]] [rerail setup options]",
  "",
  "Installs @neralabs/rerail from npm (`npx @neralabs/rerail@<tag> setup`) and wires it into every coding agent.",
  "",
  "  --tag <tag>            npm dist-tag or version to install (default: $RERAIL_TAG, else latest; e.g. --tag next)",
  "  --restore [dir]        after setup, restore the databases of a just nuke-rerail backup",
  "                         (default: the newest ~/.rerail-clean-*)",
  "  -h, --help             show this help",
  "",
  "rerail setup options, passed through:",
  "  [--answer <answer>] [--repo <dir>] [--agents omp,pi,claude-code,codex,cursor] [--port <n>] [--json]",
  "",
  "Rerail is not public yet: npm must be logged in (`npm login`) with an account that can read the @neralabs scope.",
].join("\n");

export class SetupError extends Error {}

export function normalizeRegistry(url) {
  const trimmed = url.trim();
  let end = trimmed.length;
  while (end > 0 && trimmed[end - 1] === "/") end -= 1;
  return `${trimmed.slice(0, end)}/`;
}

function hasControlCharacter(text) {
  return [...text].some(char => char < " " || char === "\u007f");
}

/** RERAIL_REGISTRY (a local test registry) must be a clean https URL, or http on loopback. */
export function validateRegistry(raw) {
  const invalid = reason => ({ error: `RERAIL_REGISTRY ${JSON.stringify(raw)} is not usable: ${reason}` });
  const trimmed = raw.trim();
  if (trimmed === "") return invalid("it is empty.");
  if (/\s/u.test(trimmed) || hasControlCharacter(trimmed)) return invalid("it contains whitespace or control characters.");
  let url;
  try {
    url = new URL(trimmed);
  } catch {
    return invalid("it is not a URL.");
  }
  if (url.username !== "" || url.password !== "") return invalid("it must not carry credentials.");
  if (url.search !== "" || url.hash !== "") return invalid("it must not have a query or fragment.");
  if (url.protocol === "https:") return { registry: normalizeRegistry(trimmed) };
  if (url.protocol === "http:" && LOOPBACK_HOSTS.has(url.hostname)) return { registry: normalizeRegistry(trimmed) };
  return invalid("use https://, or http:// only for 127.0.0.1, localhost, or [::1].");
}

/** An npm dist-tag or version: what `npx <package>@<tag>` accepts without shell or path meaning. */
export function validateTag(raw) {
  if (/^[A-Za-z0-9][\w.+-]{0,63}$/u.test(raw)) return { tag: raw };
  return { error: `${JSON.stringify(raw)} is not an npm dist-tag or version.` };
}

/**
 * Split argv into this script's options (`--tag`, `--restore`, `--help`) and the arguments forwarded to
 * `rerail setup`. The tag comes from `--tag`, then RERAIL_TAG, then latest.
 */
export function parseSetupArguments(argv, { env = process.env, userHome = env.HOME ?? homedir() } = {}) {
  const { restore, args: rest } = parseRestoreArguments(argv);
  const args = [];
  let tag;
  let help = false;
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index];
    if (argument === "--help" || argument === "-h") {
      help = true;
    } else if (argument === "--tag" || argument.startsWith("--tag=")) {
      if (tag !== undefined) throw new SetupError("--tag may be given once");
      tag = argument === "--tag" ? rest[(index += 1)] : argument.slice("--tag=".length);
      if (tag === undefined || tag === "") throw new SetupError("--tag needs a dist-tag or version, e.g. --tag next");
    } else {
      args.push(argument);
    }
  }
  if (help) return { help, args, tag: undefined, backupDir: undefined };
  const checked = validateTag(tag ?? (env.RERAIL_TAG || DEFAULT_TAG));
  if ("error" in checked) throw new SetupError(`${tag === undefined ? "RERAIL_TAG" : "--tag"}: ${checked.error}`);
  const backupDir = restore === undefined ? undefined : selectBackup({ userHome, backupDir: restore.backupDir });
  return { help, args, tag: checked.tag, backupDir };
}

export function npmErrorReason(output) {
  const code = /npm error code (\S+)/u.exec(output);
  if (code !== null) return code[1];
  const first = output.split("\n").map(line => line.trim()).find(line => line !== "");
  return first === undefined ? "no response" : first.slice(0, 120);
}

export function classifyNpmError(output) {
  if (/\b(E401|E403|ENEEDAUTH)\b|\b40[13] (Unauthorized|Forbidden)\b/u.test(output)) return "auth";
  if (/\bE404\b|\b404 Not Found\b/u.test(output)) return "not-found";
  if (/\b(ECONNREFUSED|ENOTFOUND|ETIMEDOUT|ECONNRESET|EAI_AGAIN)\b/u.test(output)) return "network";
  return "other";
}

/**
 * What to tell the user when `npm view @neralabs/rerail@<tag>` failed. The package is public, so a 404 means the tag
 * names nothing published, or npm asks another registry for the scope; a failure while logged in may be the token.
 */
export function accessFailureMessage({ kind, reason, user, registry, tag }) {
  const spec = `${PACKAGE}@${tag}`;
  if (kind === "network") return `Could not reach ${registry} (${reason}). Check your network and re-run just rerail.`;
  if (kind === "other") return `npm could not read ${spec} from ${registry} (${reason}).`;
  if (user === undefined) {
    return `${spec} is not on ${registry} (${reason}). Try --tag next, or check which registry your npm configuration `
      + `names for the ${SCOPE} scope, then re-run just rerail.`;
  }
  return `npm is logged in to ${registry} as ${user}, but cannot read ${spec} (${reason}). ${PACKAGE} is public, so `
    + `check the token in ~/.npmrc or run \`npm logout\`, then re-run just rerail. (${spec} may not be published: `
    + `try --tag next.)`;
}

function windowsNpmBin(execPath, env) {
  const inherited = env.npm_execpath;
  if (inherited !== undefined && /(?:^|[\\/])npm-cli\.js$/iu.test(inherited)) return dirname(inherited);
  return join(dirname(execPath), "node_modules", "npm", "bin");
}

/** How to spawn npm or npx without a shell. Windows ships them as .cmd shims, so they run as `node npm-cli.js`. */
export function platformCommand(tool, args, { platform = process.platform, execPath = process.execPath, env = process.env } = {}) {
  if (platform !== "win32") return { command: tool, args };
  return { command: execPath, args: [join(windowsNpmBin(execPath, env), `${tool}-cli.js`), ...args] };
}

export function exitCodeFor(code, signal) {
  if (code !== null) return code;
  if (signal === null) return 1;
  return 128 + (osConstants.signals[signal] ?? 0);
}

let stepNumber = 0;
function step(title) {
  stepNumber += 1;
  process.stdout.write(`\n\x1b[34m[${stepNumber}]\x1b[0m ${title}\n`);
}
function note(message) {
  process.stdout.write(`    ${message}\n`);
}

/** The environment every npm/npx child gets: RERAIL_REGISTRY, when set, becomes the @neralabs scope registry. */
export function npmEnvironment(env, registry) {
  return registry === undefined ? { ...env } : { ...env, [SCOPE_REGISTRY_ENV]: registry };
}

function npm(args, env, timeout = 20_000) {
  const resolved = platformCommand("npm", args, { env });
  const result = spawnSync(resolved.command, resolved.args, { encoding: "utf8", timeout, env });
  return { ok: result.status === 0, stdout: (result.stdout ?? "").trim(), output: `${result.stdout ?? ""}${result.stderr ?? ""}` };
}

/** The registry npm resolves the scope against: the scope mapping, else npm's default registry. */
function scopeRegistry(env, run) {
  const scoped = run(["config", "get", `${SCOPE}:registry`], env);
  if (scoped.ok && scoped.stdout !== "" && scoped.stdout !== "undefined") return normalizeRegistry(scoped.stdout);
  const fallback = run(["config", "get", "registry"], env);
  return normalizeRegistry(fallback.ok && fallback.stdout !== "" ? fallback.stdout : "https://registry.npmjs.org/");
}

/** `npm view <package>@<tag> version`, and on failure who npm is logged in as there. */
export function probeAccess({ tag, env, run = npm }) {
  const view = run(["view", `${PACKAGE}@${tag}`, "version", ...NPM_PROBE_FLAGS], env);
  if (view.ok && view.stdout !== "") return { ok: true, version: view.stdout.split("\n").at(-1).replaceAll(/['"]/gu, "").trim() };
  if (view.ok) return { ok: false, kind: "no-tag" };
  const kind = classifyNpmError(view.output);
  const result = { ok: false, kind, reason: npmErrorReason(view.output) };
  if (kind !== "auth" && kind !== "not-found") return result;
  const registry = scopeRegistry(env, run);
  const whoami = run(["whoami", `--registry=${registry}`, ...NPM_PROBE_FLAGS], env);
  return { ...result, registry, user: whoami.ok && whoami.stdout !== "" ? whoami.stdout : undefined };
}

function checkNode() {
  step("Checking Node.js");
  const nodeMajor = Number(process.versions.node.split(".")[0]);
  if (nodeMajor < MIN_NODE_MAJOR) throw new SetupError(`Node ${nodeMajor} is too old; Rerail needs ${MIN_NODE_MAJOR}+.`);
  note(`Node ${process.versions.node}`);
}

function configuredRegistry(env) {
  if (!env.RERAIL_REGISTRY) return undefined;
  const checked = validateRegistry(env.RERAIL_REGISTRY);
  if ("error" in checked) throw new SetupError(checked.error);
  return checked.registry;
}

function runSetup(tag, args, env) {
  const { promise, resolve } = Promise.withResolvers();
  const resolved = platformCommand("npx", ["--yes", `${PACKAGE}@${tag}`, "setup", ...args], { env });
  const child = spawn(resolved.command, resolved.args, { stdio: "inherit", env });
  child.once("error", error => {
    process.stderr.write(`npx failed to start: ${error.message}\n`);
    resolve(1);
  });
  child.once("exit", (code, signal) => resolve(exitCodeFor(code, signal)));
  return promise;
}

async function restoreAfterSetup(backupDir, env) {
  step(`Restoring Rerail databases from ${backupDir}`);
  try {
    await restoreInstalled({ backupDir, env, log: note });
  } catch (error) {
    if (error instanceof RestoreError) throw new SetupError(`${error.message}. The fresh install is intact; the backup was not modified.`);
    throw error;
  }
}

export async function main(argv, env = process.env) {
  let parsed;
  try {
    parsed = parseSetupArguments(argv, { env });
  } catch (error) {
    if (error instanceof RestoreError) throw new SetupError(error.message);
    throw error;
  }
  if (parsed.help) {
    process.stdout.write(`${USAGE}\n`);
    return 0;
  }
  const { args, tag, backupDir } = parsed;
  const registry = configuredRegistry(env);
  checkNode();
  if (backupDir !== undefined) note(`Will restore databases from ${backupDir} after setup`);
  const npmEnv = npmEnvironment(env, registry);

  step(`Checking npm access to ${PACKAGE}@${tag}`);
  if (registry !== undefined) note(`Registry for ${SCOPE}: ${registry} (RERAIL_REGISTRY)`);
  const access = probeAccess({ tag, env: npmEnv });
  if (!access.ok && access.kind === "no-tag") throw new SetupError(`${PACKAGE} has no version tagged ${tag}.`);
  if (!access.ok) throw new SetupError(accessFailureMessage({ ...access, registry: access.registry ?? registry ?? "the npm registry", tag }));
  note(`${PACKAGE}@${tag} is ${access.version}`);

  step(`Running npx ${PACKAGE}@${tag} setup ${args.join(" ")}`.trimEnd());
  const status = await runSetup(tag, args, npmEnv);
  if (status !== 0 || backupDir === undefined) return status;
  await restoreAfterSetup(backupDir, npmEnv);
  return 0;
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    if (!(error instanceof SetupError)) throw error;
    process.stderr.write(`\n\x1b[31m!!\x1b[0m ${error.message}\n`);
    process.exitCode = 1;
  }
}
