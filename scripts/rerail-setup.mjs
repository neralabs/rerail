#!/usr/bin/env node
// `just rerail`: give npm a read credential for the private Azure Artifacts feed, then hand off to `npx @neralabs/rerail setup`.
// Everything after the handoff (versions under ~/.rerail, host hooks, the daemon, updates) is owned by `rerail setup`.
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { constants as osConstants, homedir, hostname, platform } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { RestoreError, parseRestoreArguments, restoreInstalled, selectBackup } from "./rerail-restore.mjs";

export const DEFAULT_REGISTRY = "https://pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/registry/";
export const SCOPE = "@neralabs";
export const PACKAGE = `${SCOPE}/rerail`;
export const MIN_NODE_MAJOR = 24;
export const AZURE_ORG = "nera-software";
export const ENTRA_TENANT = "nera.software";
export const AZURE_DEVOPS_RESOURCE = "499b84ac-1321-427f-aa17-267ca6975798";
export const PAT_URL = `https://vssps.dev.azure.com/${AZURE_ORG}/_apis/tokens/pats?api-version=7.1-preview.1`;
export const TOKENS_PAGE = `https://dev.azure.com/${AZURE_ORG}/_usersSettings/tokens`;
export const NPMRC_USERNAME = AZURE_ORG;
export const NPMRC_EMAIL = "npm requires email to be set but doesn't use the value";
export const NOT_IN_ORG_MESSAGE =
  "You are not in the nera-software Azure DevOps org yet. Ask Jeremy (jbarnes@nera.software) to add you to the "
  + "'Nera Developers' Entra group, then re-run just rerail.";
const BLOCK_MARKER = "# @neralabs registry credentials, managed by `just rerail`";
const NPM_PROBE_FLAGS = ["--fetch-timeout=10000", "--fetch-retries=0"];
const AZ_TIMEOUT_MS = 60_000;
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]"]);
const SCOPE_LINE = `${SCOPE}:registry=`;

export function normalizeRegistry(url) {
  const trimmed = url.trim();
  let end = trimmed.length;
  while (end > 0 && trimmed[end - 1] === "/") end -= 1;
  return `${trimmed.slice(0, end)}/`;
}

function hasControlCharacter(text) {
  return [...text].some(char => char < " " || char === "\u007f");
}

/** RERAIL_REGISTRY ends up inside ~/.npmrc and receives the PAT, so only a clean https URL (or loopback http) is accepted. */
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

/** Azure token acquisition targets the Nera feed only; a PAT minted for it must never be sent to another registry. */
export function isNeraFeed(registry) {
  return normalizeRegistry(registry) === DEFAULT_REGISTRY;
}

export function foreignRegistryMessage(registry) {
  return `RERAIL_REGISTRY points at ${registry}, which is not the Nera feed; set NERA_NPM_TOKEN to a token for that registry.`;
}

export function unreachableMessage(registry, reason) {
  return `Could not reach ${registry} (${reason}). Check your network or VPN and re-run just rerail; your saved credential was not changed.`;
}

/** npmrc credential keys for a registry; Azure feeds are also addressed by their `/npm/` parent. */
export function registryAuthKeys(registry) {
  const url = new URL(normalizeRegistry(registry));
  const key = `//${url.host}${url.pathname}`;
  return key.endsWith("/npm/registry/") ? [key, key.slice(0, -"registry/".length)] : [key];
}

export function encodePassword(pat) {
  return Buffer.from(pat, "utf8").toString("base64");
}

function lineEnding(text) {
  return text.includes("\r\n") ? "\r\n" : "\n";
}

function splitLines(text) {
  const lines = text.split(/\r?\n/u);
  while (lines.length > 0 && lines.at(-1).trim() === "") lines.pop();
  return lines;
}

function joinLines(lines, eol) {
  return `${lines.join(eol)}${eol}`;
}

const CREDENTIAL_LINE = /^(\/\/\S+?):(username|_password|email)=(.*)$/u;
/** Credential fields in the order the managed block writes them; the block cycles back to `username` per registry key. */
const CREDENTIAL_FIELDS = ["username", "_password", "email"];

function nextField(field) {
  return CREDENTIAL_FIELDS[(CREDENTIAL_FIELDS.indexOf(field) + 1) % CREDENTIAL_FIELDS.length];
}

function isOwnedCredential(field, value) {
  if (field === "_password") return true;
  return value === (field === "username" ? NPMRC_USERNAME : NPMRC_EMAIL);
}

/** Block state after `trimmed`, or undefined when the line ends the managed block. */
function advanceBlock(trimmed, block) {
  if (trimmed.startsWith(SCOPE_LINE) && block.expected === "username") return block;
  const match = CREDENTIAL_LINE.exec(trimmed);
  if (match === null) return undefined;
  const [, lineKey, field, value] = match;
  if (field !== block.expected) return undefined;
  if (field !== "username" && lineKey !== block.key) return undefined;
  if (!isOwnedCredential(field, value)) return undefined;
  return { expected: nextField(field), key: lineKey };
}

/** Fields the managed block writes or supersedes; other settings under the same key (certfile, always-auth) are the user's. */
const SUPERSEDED_FIELDS = new Set([...CREDENTIAL_FIELDS, "_authToken", "_auth"]);

function isSupersededCredential(trimmed, keys) {
  const key = keys.find(candidate => trimmed.startsWith(`${candidate}:`));
  if (key === undefined) return false;
  const rest = trimmed.slice(key.length + 1);
  const equals = rest.indexOf("=");
  return equals !== -1 && SUPERSEDED_FIELDS.has(rest.slice(0, equals).trim());
}

/**
 * Lines outside this script's managed block. The block is the marker plus the scope mapping and credential lines
 * that follow it, whatever registry they name; stray scope mappings and superseded credentials under `keys` go too.
 */
function withoutOwnedLines(lines, keys) {
  const kept = [];
  let block;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed === BLOCK_MARKER) {
      block = { expected: "username", key: "" };
      continue;
    }
    block = block === undefined ? undefined : advanceBlock(trimmed, block);
    if (block !== undefined) continue;
    if (trimmed.startsWith(SCOPE_LINE) || isSupersededCredential(trimmed, keys)) continue;
    kept.push(line);
  }
  while (kept.length > 0 && kept.at(-1).trim() === "") kept.pop();
  return kept;
}

/** Point the scope at the registry, leaving a file that already does exactly that byte-for-byte alone. */
export function ensureScopeMapping(text, registry) {
  const wanted = `${SCOPE_LINE}${normalizeRegistry(registry)}`;
  const lines = splitLines(text);
  const mappings = lines.filter(line => line.trim().startsWith(SCOPE_LINE));
  if (mappings.length === 1 && mappings[0].trim() === wanted) return text;
  const kept = lines.filter(line => !line.trim().startsWith(SCOPE_LINE));
  const marker = kept.findIndex(line => line.trim() === BLOCK_MARKER);
  if (marker === -1) kept.push(wanted);
  else kept.splice(marker + 1, 0, wanted);
  return joinLines(kept, lineEnding(text));
}

/** Replace the managed block and the credential fields for the registry's keys; leave everything else alone. */
export function mergeNpmrc(text, registry, pat) {
  const keys = registryAuthKeys(registry);
  const kept = withoutOwnedLines(splitLines(text), keys);
  const password = encodePassword(pat);
  const block = [BLOCK_MARKER, `${SCOPE_LINE}${normalizeRegistry(registry)}`];
  for (const key of keys) {
    block.push(`${key}:username=${NPMRC_USERNAME}`, `${key}:_password=${password}`, `${key}:email=${NPMRC_EMAIL}`);
  }
  return joinLines([...kept, ...block], lineEnding(text));
}

/**
 * Drop the managed block, every scope mapping, and the credential fields for each registry's keys, so the next
 * `just rerail` acquires a credential from scratch. Unrelated lines stay byte-for-byte; an unchanged file is returned as is.
 */
export function removeManagedNpmrc(text, registries) {
  const keys = registries.flatMap(registry => registryAuthKeys(registry));
  const lines = splitLines(text);
  const kept = withoutOwnedLines(lines, keys);
  if (kept.length === lines.length) return text;
  return kept.length === 0 ? "" : joinLines(kept, lineEnding(text));
}

/** A new credential is acquired only after the registry answered and rejected auth; an outage must not replace a working one. */
export function chooseCredentialSource({ credentialWorks, kind, envToken, interactive, neraFeed }) {
  if (credentialWorks) return "existing";
  if (envToken !== undefined && envToken !== "") return "env";
  if (!neraFeed) return "foreign-registry";
  if (kind !== "auth") return "unreachable";
  return interactive ? "az" : "non-interactive";
}

const PAT_LIFETIME = /lifetime|lifespan|expiration|validTo|maximum.*(days|duration)/iu;

export function classifyPatFailure(status, body) {
  if (status === 401 || status === 403 || /TF400813|not authorized|userIsNotAuthorized|not a member/iu.test(body)) return "not-in-org";
  if (PAT_LIFETIME.test(body)) return "lifetime";
  return "other";
}

/** A 200 is only a success when it carries a token and no `patTokenError` other than "none". */
export function interpretPatResponse(status, body) {
  if (status < 200 || status >= 300) return { failure: classifyPatFailure(status, body) };
  let parsed;
  try {
    parsed = JSON.parse(body);
  } catch {
    return { failure: "no-token", detail: undefined };
  }
  const error = typeof parsed === "object" && parsed !== null ? parsed.patTokenError : undefined;
  const detail = error === undefined || error === null || error === "none" ? undefined : String(error);
  const token = readPatToken(parsed);
  if (detail === undefined && token !== undefined) return { token };
  if (detail !== undefined && PAT_LIFETIME.test(detail)) return { failure: "lifetime" };
  return { failure: "no-token", detail };
}

export function patFailureMessage(detail) {
  const reason = detail === undefined ? "" : ` (${detail})`;
  return `Azure DevOps did not issue a token${reason}. If you are not in the ${AZURE_ORG} org yet, ask Jeremy `
    + "(jbarnes@nera.software) to add you to the 'Nera Developers' Entra group and re-run just rerail; "
    + "otherwise create the token by hand below.";
}

export function readPatToken(value) {
  if (typeof value !== "object" || value === null) return undefined;
  const token = value.patToken?.token;
  return typeof token === "string" && token !== "" ? token : undefined;
}

export function patExpiry(now, days) {
  return new Date(now.getTime() + days * 86_400_000).toISOString();
}

export function npmErrorReason(output) {
  const code = /npm error code (\S+)/u.exec(output);
  if (code !== null) return code[1];
  const first = output.split("\n").map(line => line.trim()).find(line => line !== "");
  return first === undefined ? "no response" : first.slice(0, 120);
}

export function classifyNpmError(output) {
  if (/\b(E401|E403|ENEEDAUTH)\b|\b401 Unauthorized\b/u.test(output)) return "auth";
  if (/\bE404\b/u.test(output)) return "not-found";
  if (/\b(ECONNREFUSED|ENOTFOUND|ETIMEDOUT|ECONNRESET|EAI_AGAIN)\b/u.test(output)) return "network";
  return "other";
}

export function azureCliInstallHint(os) {
  if (os === "darwin") return "brew install azure-cli";
  if (os === "win32") return "winget install -e --id Microsoft.AzureCLI";
  return "see https://learn.microsoft.com/cli/azure/install-azure-cli-linux";
}

export function manualPatSteps() {
  return [
    `1. Open ${TOKENS_PAGE}`,
    "2. Click New Token.",
    `3. Organization: ${AZURE_ORG}.`,
    "4. Scopes: Custom defined, then Packaging: Read.",
    "5. Expiration: the longest allowed.",
    "6. Create, then copy the token.",
  ];
}

let stepNumber = 0;
function step(title) {
  stepNumber += 1;
  process.stdout.write(`\n\x1b[34m[${stepNumber}]\x1b[0m ${title}\n`);
}
function note(message) {
  process.stdout.write(`    ${message}\n`);
}
class SetupError extends Error {}
function fail(message) {
  throw new SetupError(message);
}

function findOnPath(name, env, os) {
  const pathName = Object.keys(env).find(key => key.toUpperCase() === "PATH") ?? "PATH";
  const directories = (env[pathName] ?? "").split(os === "win32" ? ";" : ":").filter(directory => directory !== "");
  const extensions = os === "win32" ? (env.PATHEXT ?? ".EXE;.CMD;.BAT;.COM").split(";").filter(extension => extension !== "") : [""];
  for (const directory of directories) {
    for (const extension of extensions) {
      const candidate = join(directory, `${name}${extension.toLowerCase()}`);
      if (existsSync(candidate)) return candidate;
    }
  }
  return undefined;
}

function windowsNpmBin(execPath, env) {
  const inherited = env.npm_execpath;
  if (inherited !== undefined && /(?:^|[\\/])npm-cli\.js$/iu.test(inherited)) return dirname(inherited);
  return join(dirname(execPath), "node_modules", "npm", "bin");
}

/**
 * How to spawn npm, npx, or az without a shell. Windows ships npm and npx as .cmd shims, so they run as
 * `node npm-cli.js` / `node npx-cli.js`; az is only az.cmd, so it goes through cmd.exe with constant arguments.
 */
export function platformCommand(tool, args, { platform: os = process.platform, execPath = process.execPath, env = process.env } = {}) {
  if (os !== "win32") return { command: tool, args };
  if (tool === "npm" || tool === "npx") {
    return { command: execPath, args: [join(windowsNpmBin(execPath, env), `${tool}-cli.js`), ...args] };
  }
  const unsafe = args.find(arg => !/^[\w.:@/-]+$/u.test(arg));
  if (unsafe !== undefined) throw new Error(`unsafe argument for cmd.exe: ${unsafe}`);
  const azCmd = findOnPath("az", env, os) ?? "az.cmd";
  return {
    command: env.ComSpec ?? env.COMSPEC ?? "cmd.exe",
    args: ["/d", "/s", "/c", `""${azCmd}" ${args.join(" ")}"`],
    options: { windowsVerbatimArguments: true },
  };
}

const MACHINE_NAME_MAX = 40;

/** A PAT display-name suffix: [A-Za-z0-9._-] only, whitespace runs become '-', at most 40 characters. */
export function sanitizeMachineName(raw) {
  if (typeof raw !== "string") return undefined;
  const cleaned = raw.trim().replace(/\s+/gu, "-").replace(/[^\w.-]/gu, "").slice(0, MACHINE_NAME_MAX);
  let end = cleaned.length;
  while (end > 0 && cleaned[end - 1] === "-") end -= 1;
  return end === 0 ? undefined : cleaned.slice(0, end);
}

function scutilName(key) {
  const result = spawnSync("/usr/sbin/scutil", ["--get", key], { encoding: "utf8", timeout: 5_000, stdio: ["ignore", "pipe", "ignore"] });
  return result.status === 0 ? result.stdout.trim() : undefined;
}

/** macOS hostname() can be a MAC address; the ComputerName the user set is what they recognise in the token list. */
export function machineName({ platform: os = process.platform, scutil = scutilName, hostname: host = hostname } = {}) {
  const candidates = os === "darwin" ? [() => scutil("ComputerName"), () => scutil("LocalHostName"), host] : [host];
  for (const candidate of candidates) {
    const name = sanitizeMachineName(candidate());
    if (name !== undefined) return name;
  }
  return "machine";
}

function spawnTool(tool, args, options) {
  const resolved = platformCommand(tool, args);
  return spawnSync(resolved.command, resolved.args, { ...options, ...resolved.options });
}

function npm(args, timeout = 15_000) {
  const result = spawnTool("npm", args, { encoding: "utf8", timeout });
  return { ok: result.status === 0, output: `${result.stdout ?? ""}${result.stderr ?? ""}` };
}

function probeRegistry(registry) {
  const scoped = [`--registry=${registry}`, `--${SCOPE}:registry=${registry}`, ...NPM_PROBE_FLAGS];
  const whoami = npm(["whoami", ...scoped]);
  const view = npm(["view", `${PACKAGE}@latest`, "version", ...scoped]);
  if (view.ok) return { credentialWorks: true, published: true, version: view.output.trim() };
  const kind = classifyNpmError(view.output);
  return { credentialWorks: whoami.ok || kind === "not-found", published: false, kind, reason: npmErrorReason(view.output) };
}

function hasCommand(command) {
  const finder = process.platform === "win32" ? "where" : "which";
  return spawnSync(finder, [command], { stdio: "ignore" }).status === 0;
}

function az(args) {
  const result = spawnTool("az", args, { encoding: "utf8", timeout: AZ_TIMEOUT_MS, stdio: ["ignore", "pipe", "pipe"] });
  const timedOut = result.error?.code === "ETIMEDOUT";
  if (timedOut) note(`az ${args.slice(0, 2).join(" ")} did not answer within ${AZ_TIMEOUT_MS / 1000}s.`);
  return { ok: result.status === 0, timedOut, stdout: (result.stdout ?? "").trim() };
}

function azureAccessToken() {
  const args = ["account", "get-access-token", "--tenant", ENTRA_TENANT, "--resource", AZURE_DEVOPS_RESOURCE, "--query", "accessToken", "-o", "tsv"];
  const first = az(args);
  if (first.ok && first.stdout !== "") return first.stdout;
  if (first.timedOut) return undefined;
  note(`Signing in to Azure (tenant ${ENTRA_TENANT}); a browser window will open.`);
  const login = spawnTool("az", ["login", "--tenant", ENTRA_TENANT, "--allow-no-subscriptions"], { stdio: "inherit" });
  if (login.status !== 0) return undefined;
  const second = az(args);
  return second.ok && second.stdout !== "" ? second.stdout : undefined;
}

function patDisplayName() {
  return `rerail-npm-read-${machineName()}`;
}

async function createPat(accessToken, days) {
  const response = await fetch(PAT_URL, {
    method: "POST",
    headers: { Authorization: `Bearer ${accessToken}`, "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify({
      displayName: patDisplayName(),
      scope: "vso.packaging",
      validTo: patExpiry(new Date(), days),
      allOrgs: false,
    }),
    signal: AbortSignal.timeout(30_000),
  });
  return interpretPatResponse(response.status, await response.text());
}

async function patFromAzureCli() {
  step("Creating a read-only Azure Artifacts token with the Azure CLI");
  if (!hasCommand("az")) {
    note("The Azure CLI (az) is not installed. Install it with:");
    note(`  ${azureCliInstallHint(platform())}`);
    note("or create the token by hand below.");
    return undefined;
  }
  const accessToken = azureAccessToken();
  if (accessToken === undefined) {
    note("Could not get an Azure DevOps access token from the Azure CLI.");
    return undefined;
  }
  for (const days of [365, 90]) {
    let result;
    try {
      result = await createPat(accessToken, days);
    } catch (error) {
      note(`Token request failed: ${error.message}`);
      return undefined;
    }
    if (result.token !== undefined) {
      note(`Created token "${patDisplayName()}", valid ${days} days.`);
      return result.token;
    }
    if (result.failure === "not-in-org") fail(NOT_IN_ORG_MESSAGE);
    if (result.failure !== "lifetime") {
      note(patFailureMessage(result.detail));
      return undefined;
    }
    note(`The org caps token lifetime below ${days} days; trying a shorter one.`);
  }
  return undefined;
}

function readHidden(prompt) {
  const { promise, resolve, reject } = Promise.withResolvers();
  const input = process.stdin;
  process.stdout.write(prompt);
  input.setRawMode(true);
  input.resume();
  input.setEncoding("utf8");
  let value = "";
  const finish = (error) => {
    input.setRawMode(false);
    input.pause();
    input.removeListener("data", onData);
    process.stdout.write("\n");
    if (error === undefined) resolve(value.trim());
    else reject(error);
  };
  const onData = (chunk) => {
    for (const char of chunk) {
      if (char === "\r" || char === "\n" || char === "\u0004") return finish();
      if (char === "\u0003") return finish(new SetupError("Cancelled."));
      if (char === "\u007f" || char === "\b") value = value.slice(0, -1);
      else if (char >= " ") value += char;
    }
  };
  input.on("data", onData);
  return promise;
}

async function patByHand() {
  step("Create a token by hand");
  for (const line of manualPatSteps()) note(line);
  const pat = await readHidden("    Paste PAT (input hidden): ");
  if (pat === "") fail("No token entered; nothing was written. Re-run just rerail when you have one.");
  return pat;
}

/** The mode option only applies when the file is created, so an existing .npmrc is narrowed after every write. */
function writePrivate(path, text) {
  writeFileSync(path, text, { mode: 0o600 });
  if (process.platform !== "win32") chmodSync(path, 0o600);
}

export function writeNpmrc(path, registry, pat) {
  const existing = existsSync(path) ? readFileSync(path, "utf8") : "";
  writePrivate(path, mergeNpmrc(existing, registry, pat));
}

export function exitCodeFor(code, signal) {
  if (code !== null) return code;
  if (signal === null) return 1;
  return 128 + (osConstants.signals[signal] ?? 0);
}

function runSetup(args) {
  const { promise, resolve } = Promise.withResolvers();
  const resolved = platformCommand("npx", ["--yes", `${PACKAGE}@latest`, "setup", ...args]);
  const child = spawn(resolved.command, resolved.args, { stdio: "inherit" });
  child.once("error", error => {
    process.stderr.write(`npx failed to start: ${error.message}\n`);
    resolve(1);
  });
  child.once("exit", (code, signal) => resolve(exitCodeFor(code, signal)));
  return promise;
}

function checkNode() {
  step("Checking Node.js");
  const nodeMajor = Number(process.versions.node.split(".")[0]);
  if (nodeMajor < MIN_NODE_MAJOR) fail(`Node ${nodeMajor} is too old; Rerail needs ${MIN_NODE_MAJOR}+.`);
  note(`Node ${process.versions.node}`);
}

function configuredRegistry() {
  const checked = validateRegistry(process.env.RERAIL_REGISTRY || DEFAULT_REGISTRY);
  if ("error" in checked) fail(checked.error);
  return checked.registry;
}

function keepExistingCredential(npmrc, registry) {
  note("Your existing credential works.");
  const current = existsSync(npmrc) ? readFileSync(npmrc, "utf8") : "";
  const mapped = ensureScopeMapping(current, registry);
  if (mapped !== current) writePrivate(npmrc, mapped);
}

async function obtainPat(source, envToken) {
  if (source === "env") {
    note("Using the token in NERA_NPM_TOKEN.");
    return envToken;
  }
  if (source === "non-interactive") {
    process.stdout.write("\nThis terminal is not interactive and NERA_NPM_TOKEN is not set. To get a token:\n");
    for (const line of manualPatSteps()) note(line);
    fail("Run `just rerail` in an interactive terminal, or set NERA_NPM_TOKEN to that token and re-run.");
  }
  return (await patFromAzureCli()) ?? (await patByHand());
}

async function saveNewCredential({ npmrc, registry, probe, source, envToken }) {
  if (probe.kind === "auth") note("No working credential yet.");
  else note(`The registry did not answer (${probe.reason}); saving the token from NERA_NPM_TOKEN anyway.`);
  const pat = await obtainPat(source, envToken);
  step(`Saving the credential to ${npmrc}`);
  writeNpmrc(npmrc, registry, pat);
  note(`Wrote the ${SCOPE} scope mapping and credential for ${registry}`);
}

function requirePublished(verified, npmrc, registry) {
  if (verified.published) return;
  if (verified.kind === "auth") {
    fail(`The registry rejected the token (${npmrc}). It may be wrong, expired, or lack Packaging: Read; re-run just rerail to replace it.`);
  }
  if (verified.kind === "not-found" || verified.credentialWorks) {
    fail(`${PACKAGE} is not published yet; install from this checkout with: just rerail-dev`);
  }
  fail(`Could not reach ${registry}. Check your network and re-run just rerail.`);
}

/** `--restore [dir]` belongs to this script; every other argument is forwarded to `rerail setup`. */
export function splitSetupArguments(argv, home = homedir()) {
  const { restore, args } = parseRestoreArguments(argv);
  return { args, backupDir: restore === undefined ? undefined : selectBackup({ home, backupDir: restore.backupDir }) };
}

async function restoreAfterSetup(backupDir) {
  step(`Restoring Rerail databases from ${backupDir}`);
  try {
    await restoreInstalled({ backupDir, log: note });
  } catch (error) {
    if (error instanceof RestoreError) fail(`${error.message}. The fresh install is intact; the backup was not modified.`);
    throw error;
  }
}

export async function main(argv) {
  let split;
  try {
    split = splitSetupArguments(argv, process.env.HOME ?? homedir());
  } catch (error) {
    if (error instanceof RestoreError) fail(error.message);
    throw error;
  }
  const { args, backupDir } = split;
  checkNode();
  if (backupDir !== undefined) note(`Will restore databases from ${backupDir} after setup`);
  const registry = configuredRegistry();
  const npmrc = process.env.NPM_CONFIG_USERCONFIG || join(homedir(), ".npmrc");
  step("Checking your npm credential for the Rerail registry");
  note(`Registry: ${registry}`);
  note(`npm config: ${npmrc}`);
  const probe = probeRegistry(registry);
  const envToken = process.env.NERA_NPM_TOKEN;
  const source = chooseCredentialSource({
    credentialWorks: probe.credentialWorks,
    kind: probe.kind,
    envToken,
    interactive: Boolean(process.stdin.isTTY && process.stdout.isTTY),
    neraFeed: isNeraFeed(registry),
  });
  if (source === "foreign-registry") fail(foreignRegistryMessage(registry));
  if (source === "unreachable") fail(unreachableMessage(registry, probe.reason));

  if (source === "existing") keepExistingCredential(npmrc, registry);
  else await saveNewCredential({ npmrc, registry, probe, source, envToken });

  step("Verifying access to the Rerail package");
  const verified = source === "existing" ? probe : probeRegistry(registry);
  requirePublished(verified, npmrc, registry);
  note(`${PACKAGE}@latest is ${verified.version}`);

  step(`Running npx ${PACKAGE}@latest setup ${args.join(" ")}`.trimEnd());
  const status = await runSetup(args);
  if (status !== 0 || backupDir === undefined) return status;
  await restoreAfterSetup(backupDir);
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
