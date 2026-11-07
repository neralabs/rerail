import { randomUUID } from "node:crypto";
import { execFile, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  realpath,
  rename,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { dirname, isAbsolute, join, relative, resolve, sep, win32 as windowsPath } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../runtime/event-protocol.js";
import { runNpm } from "../runtime/npm.js";
import { ensureSqlite, type SqliteRepairResult } from "../runtime/sqlite-repair.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { ENTRY_POINTS, RERAIL_PACKAGE_NAME } from "../engine/package-root.js";

export const DEFAULT_UPDATE_CHANNEL = "latest";

const UPDATE_STATE_VERSION = 1;
const LOCK_LEASE_MS = 15 * 60 * 1_000;
const LOCK_RENEWAL_MS = LOCK_LEASE_MS / 3;
const EVENT_HEALTH_TIMEOUT_MS = 15_000;
const STAGING_RENAME_RETRY_MS = 10_000;

export interface Installation {
  version: string;
  packageRoot: string;
}

export interface UpdateState {
  schemaVersion: 1;
  channel: string;
  current: Installation;
  previous?: Installation;
  lastCheckAt?: string;
}

export interface UpdateStatus {
  channel: string;
  current: Installation;
  previous?: Installation;
  lastCheckAt?: string;
}

export interface UpdateCheck extends UpdateStatus {
  availableVersion: string;
  updateAvailable: boolean;
}

export interface UpdateResult extends UpdateStatus {
  changed: boolean;
}

export interface PackageRegistry {
  resolveVersion(channel: string): Promise<string>;
  installVersion(version: string, prefix: string): Promise<void>;
}

export interface ManagedUpdaterOptions {
  home?: string;
  packageRoot: string;
  packageVersion: string;
  registry?: PackageRegistry;
  now?: () => Date;
  platform?: NodeJS.Platform;
  validateInstallation?: (installation: Installation) => Promise<void>;
  /** Rebuilds the SQLite binding below a package root that fails validation; tests substitute it. */
  repairInstallation?: (packageRoot: string) => Promise<SqliteRepairResult>;
}

interface UpdateLockLease {
  owner: string;
  expiresAt: string;
}

function assertChannel(channel: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(channel)) {
    throw new Error(`Invalid npm update channel: ${channel}`);
  }
}

interface ParsedSemver {
  major: number;
  minor: number;
  patch: number;
  prerelease: Array<number | string>;
}

function parseSemver(version: string): ParsedSemver {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/u.exec(version);
  if (match === null) throw new Error(`Registry returned an invalid semantic version: ${version}`);
  const prerelease =
    match[4] === undefined
      ? []
      : match[4].split(".").map((identifier) => (/^(0|[1-9]\d*)$/u.test(identifier) ? Number(identifier) : identifier));
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease,
  };
}

function compareCoreSemver(left: ParsedSemver, right: ParsedSemver): number {
  for (const key of ["major", "minor", "patch"] as const) {
    if (left[key] !== right[key]) return left[key] < right[key] ? -1 : 1;
  }
  return 0;
}

function comparePrereleaseIdentifier(left: number | string | undefined, right: number | string | undefined): number {
  if (left === undefined) return -1;
  if (right === undefined) return 1;
  if (left === right) return 0;
  if (typeof left === "number" && typeof right === "string") return -1;
  if (typeof left === "string" && typeof right === "number") return 1;
  return left < right ? -1 : 1;
}

function comparePrerelease(left: ParsedSemver["prerelease"], right: ParsedSemver["prerelease"]): number {
  if (left.length === 0 && right.length === 0) return 0;
  if (left.length === 0) return 1;
  if (right.length === 0) return -1;
  const count = Math.max(left.length, right.length);
  for (let index = 0; index < count; index += 1) {
    const comparison = comparePrereleaseIdentifier(left[index], right[index]);
    if (comparison !== 0) return comparison;
  }
  return 0;
}

export function compareSemver(left: string, right: string): number {
  const a = parseSemver(left);
  const b = parseSemver(right);
  const coreComparison = compareCoreSemver(a, b);
  return coreComparison === 0 ? comparePrerelease(a.prerelease, b.prerelease) : coreComparison;
}

export class NpmPackageRegistry implements PackageRegistry {
  async resolveVersion(channel: string): Promise<string> {
    const result = await runNpm(["view", `${RERAIL_PACKAGE_NAME}@${channel}`, "version", "--json"]);
    const parsed = JSON.parse(result.stdout) as unknown;
    if (typeof parsed !== "string") {
      throw new TypeError(`npm returned no single version for ${RERAIL_PACKAGE_NAME}@${channel}`);
    }
    parseSemver(parsed);
    return parsed;
  }

  async installVersion(version: string, prefix: string): Promise<void> {
    await runNpm([
      "install",
      "--prefix",
      prefix,
      "--omit=dev",
      "--no-audit",
      "--no-fund",
      `${RERAIL_PACKAGE_NAME}@${version}`,
    ]);
  }
}

function isInstallation(value: unknown): value is Installation {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.version === "string" && typeof candidate.packageRoot === "string";
}

function parseState(text: string): UpdateState {
  const value = JSON.parse(text) as unknown;
  if (typeof value !== "object" || value === null) throw new Error("Rerail update state is not an object");
  const state = value as Record<string, unknown>;
  if (
    state.schemaVersion !== UPDATE_STATE_VERSION ||
    typeof state.channel !== "string" ||
    !isInstallation(state.current)
  ) {
    throw new Error("Rerail update state has an unsupported shape");
  }
  if (state.previous !== undefined && !isInstallation(state.previous)) {
    throw new Error("Rerail update state has an invalid previous installation");
  }
  if (state.lastCheckAt !== undefined && typeof state.lastCheckAt !== "string") {
    throw new Error("Rerail update state has an invalid last check time");
  }
  assertChannel(state.channel);
  parseSemver(state.current.version);
  if (state.previous !== undefined) parseSemver(state.previous.version);
  return {
    schemaVersion: 1,
    channel: state.channel,
    current: state.current,
    ...(state.previous === undefined ? {} : { previous: state.previous }),
    ...(state.lastCheckAt === undefined ? {} : { lastCheckAt: state.lastCheckAt }),
  };
}

const LAUNCHER_HEADER = "Generated by rerail setup; edits are overwritten on the next update.";

/** The stable launcher written below `<home>/bin` for the host command shell. */
export function launcherShim(
  currentLink: string,
  nodePath: string,
  platform: NodeJS.Platform = process.platform,
): string {
  const cliPath =
    platform === "win32" ? windowsPath.join(currentLink, ENTRY_POINTS.cli) : join(currentLink, ENTRY_POINTS.cli);
  if (platform === "win32") {
    const escapedNodePath = nodePath.replaceAll("%", "%%");
    const escapedCliPath = cliPath.replaceAll("%", "%%");
    return [
      "@echo off",
      `rem ${LAUNCHER_HEADER}`,
      "setlocal",
      `set "RERAIL_NODE=${escapedNodePath}"`,
      'if not exist "%RERAIL_NODE%" set "RERAIL_NODE=node"',
      `"%RERAIL_NODE%" "${escapedCliPath}" %*`,
      "exit /b %ERRORLEVEL%",
      "",
    ].join("\r\n");
  }
  const escapedSingleQuote = String.raw`'\''`;
  const quotedNodePath = `'${nodePath.replaceAll("'", escapedSingleQuote)}'`;
  const quotedCliPath = `'${cliPath.replaceAll("'", escapedSingleQuote)}'`;
  return [
    "#!/bin/sh",
    `# ${LAUNCHER_HEADER}`,
    `NODE=${quotedNodePath}`,
    '[ -x "$NODE" ] || NODE=node',
    `exec "$NODE" ${quotedCliPath} "$@"`,
    "",
  ].join("\n");
}

async function repairManagedInstallation(packageRoot: string): Promise<SqliteRepairResult> {
  const result = await ensureSqlite(packageRoot, { log: line => process.stderr.write(`${line}\n`) });
  if (result.rebuilt) process.stderr.write(`Rebuilt better-sqlite3 in ${result.prefix}\n`);
  return result;
}

const REPOSITORY_START_STATES: ReadonlySet<string> = new Set(["started", "stopped", "not-set-up", "never"]);

/**
 * Starts Rerail in the health-check repository; any failure is the staged package's and is thrown. The repository
 * is not set up, so start runs set-up, which asks first: "I'll pick rule packs myself" turns it on reading no code,
 * and the scratch `HOME` gives it no coding agent to connect. A release from before the set-up (rollback validates
 * the previous release) takes no `--answer` and starts at once.
 */
async function startHealthRepository(cli: string, workspace: string, env: NodeJS.ProcessEnv): Promise<void> {
  const run = (args: readonly string[]) => promisify(execFile)(
    process.execPath, [cli, ...args], { cwd: workspace, env, timeout: EVENT_HEALTH_TIMEOUT_MS },
  );
  const state = (await run(["state", "--repo", workspace])).stdout.trim();
  if (!REPOSITORY_START_STATES.has(state)) throw new Error(`rerail state answered ${JSON.stringify(state)}`);
  try {
    await run(["start", "--repo", workspace, "--answer", "pick-myself"]);
  } catch (error) {
    const stderr = error instanceof Error && "stderr" in error && typeof error.stderr === "string" ? error.stderr : "";
    if (!stderr.includes("Unknown start option: --answer")) throw error;
    await run(["start", "--repo", workspace]);
  }
}

export async function validateManagedInstallation(installation: Installation): Promise<void> {
  const manifestPath = join(installation.packageRoot, "package.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as Record<string, unknown>;
  if (manifest.name !== RERAIL_PACKAGE_NAME || manifest.version !== installation.version) {
    throw new Error(`Installed package identity mismatch at ${manifestPath}`);
  }

  const entrypoint = join(installation.packageRoot, ENTRY_POINTS.index);
  // A child loads the staged engine: on Windows a process with the engine mapped cannot rename the staging
  // directory into place or remove it, so an in-process check failed every install there with EPERM.
  const healthScript = [
    `const loaded = await import(${JSON.stringify(pathToFileURL(entrypoint).href)});`,
    `if (typeof loaded.nativeHealth !== "function") throw new Error("Installed package does not export nativeHealth");`,
    `process.stdout.write("\\n" + JSON.stringify(loaded.nativeHealth()));`,
  ].join("\n");
  let health: unknown;
  try {
    const { stdout } = await promisify(execFile)(process.execPath, ["--input-type=module", "--eval", healthScript], {
      env: { ...process.env, RERAIL_AUTOSTART: "0" },
    });
    health = JSON.parse(stdout.slice(stdout.lastIndexOf("\n") + 1));
  } catch (error) {
    throw new Error(`Installed native runtime health check failed: ${error instanceof Error ? error.message : String(error)}`);
  }
  if (typeof health !== "object" || health === null || (health as Record<string, unknown>).ok !== true) {
    throw new Error("Installed native runtime health check failed");
  }

  // The round-trip runs in a scratch repository with its own Rerail home and user home, so it neither autostarts
  // a daemon from a staging directory, connects a coding agent, nor writes into a database or user file shared with
  // anything else; a candidate that does not speak this build's protocol fails here without touching the real home.
  const root = await mkdtemp(join(tmpdir(), "rerail-update-health-"));
  const workspace = join(root, "workspace");
  const storageRoot = join(root, "sessions");
  const userHome = join(root, "user");
  const env = {
    ...process.env,
    HOME: userHome,
    USERPROFILE: userHome,
    BROWSER: "none",
    RERAIL_HOME: join(root, "home"),
    RERAIL_STORAGE_ROOT: storageRoot,
    RERAIL_AUTOSTART: "0",
  };
  try {
    await mkdir(workspace);
    await mkdir(userHome);
    await startHealthRepository(join(installation.packageRoot, ENTRY_POINTS.cli), workspace, env);
  } catch (error) {
    await rm(root, { recursive: true, force: true });
    throw new Error(`Installed CLI could not start a health-check repository: ${error instanceof Error ? error.message : String(error)}`);
  }
  const child = spawn(process.execPath, [join(installation.packageRoot, ENTRY_POINTS.eventStream)], {
    cwd: workspace,
    env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const response = Promise.withResolvers<string>();
  const exited = Promise.withResolvers<void>();
  let output = "";
  let errors = "";
  child.stdout.on("data", (chunk) => {
    output += Buffer.from(chunk).toString("utf8");
    const newline = output.indexOf("\n");
    if (newline >= 0) response.resolve(output.slice(0, newline));
  });
  child.stderr.on("data", (chunk) => {
    errors += Buffer.from(chunk).toString("utf8");
  });
  child.once("error", (error) => {
    response.reject(error);
    exited.reject(error);
  });
  child.once("exit", (code) => {
    if (code === 0) exited.resolve();
    else exited.reject(new Error(`Installed event stream exited ${String(code)}: ${errors}`));
  });
  const timeoutResult = Promise.withResolvers<never>();
  const timeout = setTimeout(
    () => timeoutResult.reject(new Error("Installed event stream health check timed out")),
    EVENT_HEALTH_TIMEOUT_MS,
  );
  const eventId = `health:${randomUUID()}`;
  child.stdin.end(
    `${RERAIL_EVENT_PROTOCOL_PREFACE}\n${JSON.stringify({
      event: {
        kind: "session-start",
        eventId,
        session: { host: "claude-code", sessionId: eventId, cwd: workspace },
      },
    })}\n`,
  );
  try {
    const [line] = await Promise.race([Promise.all([response.promise, exited.promise]), timeoutResult.promise]);
    const result = JSON.parse(line) as Record<string, unknown>;
    if (result.eventId !== eventId || !Array.isArray(result.deliveries)) {
      throw new Error("Installed event stream returned an invalid health response");
    }
    // A reply alone proves little: a stopped repository answers with no deliveries too.
    if ((await readdir(storageRoot).catch(() => [])).length === 0) {
      throw new Error("Installed event stream answered without opening a session");
    }
  } finally {
    clearTimeout(timeout);
    if (child.exitCode === null) child.kill();
    await rm(root, { recursive: true, force: true });
  }
}
async function pathEntryExists(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}

async function unlinkIfPresent(path: string): Promise<void> {
  try {
    await unlink(path);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
}

export class ManagedUpdater {
  readonly home: string;
  readonly statePath: string;
  readonly versionsRoot: string;
  private readonly packageRoot: string;
  private readonly packageVersion: string;
  private readonly registry: PackageRegistry;
  private readonly now: () => Date;
  private readonly validate: (installation: Installation) => Promise<void>;
  private readonly repair: (packageRoot: string) => Promise<SqliteRepairResult>;
  private readonly platform: NodeJS.Platform;

  constructor(options: ManagedUpdaterOptions) {
    this.home = options.home ?? defaultRerailHome();
    this.statePath = join(this.home, "update-state.json");
    this.versionsRoot = join(this.home, "versions");
    this.packageRoot = resolve(options.packageRoot);
    this.packageVersion = options.packageVersion;
    parseSemver(this.packageVersion);
    this.registry = options.registry ?? new NpmPackageRegistry();
    this.now = options.now ?? (() => new Date());
    this.platform = options.platform ?? process.platform;
    this.validate = options.validateInstallation ?? validateManagedInstallation;
    this.repair = options.repairInstallation ?? repairManagedInstallation;
  }

  /** The version of the package this updater runs from; what `install()` activates by default. */
  get packageVersionLabel(): string {
    return this.packageVersion;
  }

  async readState(): Promise<UpdateState> {
    try {
      return parseState(await readFile(this.statePath, "utf8"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      return {
        schemaVersion: 1,
        channel: DEFAULT_UPDATE_CHANNEL,
        current: { version: this.packageVersion, packageRoot: this.packageRoot },
      };
    }
  }

  /** `<home>/current` always points at the active package root; harness registrations reference it. */
  get currentLink(): string {
    return join(this.home, "current");
  }

  /** Stable launcher every hook and shell alias runs (`rerail.cmd` on Windows). */
  get launcherPath(): string {
    return join(this.home, "bin", this.platform === "win32" ? "rerail.cmd" : "rerail");
  }

  private async writeState(state: UpdateState): Promise<void> {
    await mkdir(this.home, { recursive: true, mode: 0o700 });
    const temporaryPath = `${this.statePath}.${process.pid}.${randomUUID()}.tmp`;
    await writeFile(temporaryPath, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
    await rename(temporaryPath, this.statePath);
    await this.syncLauncher(state.current);
  }

  private async syncCurrentLink(target: string): Promise<void> {
    const linkTarget = resolve(target);
    const canonicalTarget = await realpath(linkTarget).catch(() => linkTarget);
    const canonicalCurrent = await realpath(this.currentLink).catch(() => undefined);
    const equal =
      this.platform === "win32"
        ? canonicalCurrent?.toLowerCase() === canonicalTarget.toLowerCase()
        : canonicalCurrent === canonicalTarget;
    if (equal) return;

    const temporaryLink = `${this.currentLink}.${process.pid}.${randomUUID()}.tmp`;
    await symlink(linkTarget, temporaryLink, this.platform === "win32" ? "junction" : undefined);
    if (this.platform !== "win32") {
      await rename(temporaryLink, this.currentLink);
      return;
    }

    const previousLink = `${this.currentLink}.${process.pid}.${randomUUID()}.old`;
    const hadCurrent = await pathEntryExists(this.currentLink);
    try {
      if (hadCurrent) await rename(this.currentLink, previousLink);
      await rename(temporaryLink, this.currentLink);
      if (hadCurrent) await unlink(previousLink);
    } catch (error) {
      await unlinkIfPresent(temporaryLink);
      if (hadCurrent && !(await pathEntryExists(this.currentLink))) await rename(previousLink, this.currentLink);
      throw error;
    }
  }

  /** Re-point `current` and refresh the platform launcher without changing registrations. */
  private async syncLauncher(installation: Installation): Promise<void> {
    await this.syncCurrentLink(resolve(installation.packageRoot));
    const shim = launcherShim(this.currentLink, process.execPath, this.platform);
    const current = await readFile(this.launcherPath, "utf8").catch(() => undefined);
    if (current === shim) return;
    await mkdir(dirname(this.launcherPath), { recursive: true, mode: 0o700 });
    const temporaryPath = `${this.launcherPath}.${process.pid}.${randomUUID()}.tmp`;
    await writeFile(temporaryPath, shim, { mode: 0o755 });
    if (this.platform !== "win32") await chmod(temporaryPath, 0o755);
    await rename(temporaryPath, this.launcherPath);
  }

  private async readLockLease(lockPath: string): Promise<UpdateLockLease | undefined> {
    try {
      const value = JSON.parse(await readFile(join(lockPath, "lease.json"), "utf8")) as unknown;
      if (typeof value !== "object" || value === null) return undefined;
      const lease = value as Record<string, unknown>;
      if (typeof lease.owner !== "string" || typeof lease.expiresAt !== "string") return undefined;
      if (!Number.isFinite(Date.parse(lease.expiresAt))) return undefined;
      return { owner: lease.owner, expiresAt: lease.expiresAt };
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  private async renewLockLease(lockPath: string, owner: string): Promise<void> {
    const lease = await this.readLockLease(lockPath);
    if (lease?.owner !== owner) throw new Error("Rerail update lock ownership was lost");
    const next: UpdateLockLease = {
      owner,
      expiresAt: new Date(this.now().getTime() + LOCK_LEASE_MS).toISOString(),
    };
    await writeFile(join(lockPath, "lease.json"), `${JSON.stringify(next)}\n`, { mode: 0o600 });
  }

  private async releaseLock(lockPath: string, owner: string): Promise<void> {
    const lease = await this.readLockLease(lockPath);
    if (lease?.owner === owner) await rm(lockPath, { recursive: true, force: true });
  }

  private async withLock<T>(operation: () => Promise<T>): Promise<T> {
    const lockPath = join(this.home, "update.lock");
    const owner = randomUUID();
    await mkdir(this.home, { recursive: true, mode: 0o700 });
    try {
      await mkdir(lockPath, { mode: 0o700 });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      const lease = await this.readLockLease(lockPath);
      if (lease === undefined || Date.parse(lease.expiresAt) > this.now().getTime()) {
        throw new Error("Another Rerail update is already running");
      }
      await rm(lockPath, { recursive: true, force: true });
      await mkdir(lockPath, { mode: 0o700 });
    }
    await writeFile(
      join(lockPath, "lease.json"),
      `${JSON.stringify({
        owner,
        expiresAt: new Date(this.now().getTime() + LOCK_LEASE_MS).toISOString(),
      } satisfies UpdateLockLease)}\n`,
      { mode: 0o600 },
    );
    let renewalFailure: Error | undefined;
    const renewal = setInterval(() => {
      void this.renewLockLease(lockPath, owner).catch((error) => {
        renewalFailure ??= error as Error;
      });
    }, LOCK_RENEWAL_MS);
    renewal.unref();
    try {
      const result = await operation();
      if (renewalFailure !== undefined) throw renewalFailure;
      return result;
    } finally {
      clearInterval(renewal);
      await this.releaseLock(lockPath, owner);
    }
  }

  async status(): Promise<UpdateStatus> {
    const state = await this.readState();
    return {
      channel: state.channel,
      current: state.current,
      ...(state.previous === undefined ? {} : { previous: state.previous }),
      ...(state.lastCheckAt === undefined ? {} : { lastCheckAt: state.lastCheckAt }),
    };
  }

  async check(channel?: string): Promise<UpdateCheck> {
    const state = await this.readState();
    const selectedChannel = channel ?? state.channel;
    assertChannel(selectedChannel);
    const availableVersion = await this.registry.resolveVersion(selectedChannel);
    return {
      channel: selectedChannel,
      current: state.current,
      ...(state.previous === undefined ? {} : { previous: state.previous }),
      ...(state.lastCheckAt === undefined ? {} : { lastCheckAt: state.lastCheckAt }),
      availableVersion,
      updateAvailable: compareSemver(availableVersion, state.current.version) > 0,
    };
  }

  /**
   * Install `version` below `versions/` unless it is already there, validating
   * either way, and return its package root. Staging keeps a failed install
   * out of the versions directory.
   */
  private async materializeVersion(version: string): Promise<Installation> {
    await mkdir(this.versionsRoot, { recursive: true, mode: 0o700 });
    const finalPrefix = join(this.versionsRoot, version);
    const installation: Installation = {
      version,
      packageRoot: join(finalPrefix, "node_modules", "@neralabs", "rerail"),
    };
    if (existsSync(installation.packageRoot)) {
      await this.validateOrRepair(installation);
      return installation;
    }
    const stagingPrefix = join(this.versionsRoot, `.${version}.${randomUUID()}.staging`);
    try {
      await this.registry.installVersion(version, stagingPrefix);
      await this.validate({ version, packageRoot: join(stagingPrefix, "node_modules", "@neralabs", "rerail") });
      await this.renameStaging(stagingPrefix, finalPrefix);
    } catch (error) {
      await rm(stagingPrefix, { recursive: true, force: true, maxRetries: 10 });
      throw error;
    }
    return installation;
  }

  /**
   * Windows denies renaming a directory for a moment after its files were loaded or written: the health-check
   * children have exited but their handles, or an antivirus scan of the new native modules, linger. Retry for a
   * few seconds, as npm does, instead of failing the install.
   */
  private async renameStaging(stagingPrefix: string, finalPrefix: string): Promise<void> {
    const deadline = Date.now() + STAGING_RENAME_RETRY_MS;
    for (;;) {
      try {
        await rename(stagingPrefix, finalPrefix);
        return;
      } catch (error) {
        const code = (error as NodeJS.ErrnoException).code;
        const transient = code === "EPERM" || code === "EACCES" || code === "EBUSY";
        if (this.platform !== "win32" || !transient || Date.now() >= deadline) throw error;
        await sleep(100);
      }
    }
  }

  /**
   * An installed version fails validation after a Node upgrade because its SQLite binding was built for the old
   * ABI. Rebuild it once and validate again; when the binding was never the problem, the original failure stands.
   */
  private async validateOrRepair(installation: Installation): Promise<void> {
    try {
      await this.validate(installation);
    } catch (error) {
      const { rebuilt } = await this.repair(installation.packageRoot);
      if (!rebuilt) throw error;
      await this.validate(installation);
    }
  }

  /** Whether an installation lives in the managed versions directory (as opposed to an npx cache or a checkout). */
  private isManaged(installation: Installation): boolean {
    const path = relative(resolve(this.versionsRoot), resolve(installation.packageRoot));
    return path !== "" && path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path);
  }

  async update(channel?: string): Promise<UpdateResult> {
    return await this.withLock(async () => {
      const state = await this.readState();
      const selectedChannel = channel ?? state.channel;
      assertChannel(selectedChannel);
      const checkedAt = this.now().toISOString();
      let next: Installation;
      try {
        const availableVersion = await this.registry.resolveVersion(selectedChannel);
        if (compareSemver(availableVersion, state.current.version) <= 0) {
          // The installed version stays current while Node's ABI may not have: repair it in place rather than skip it.
          await this.validateOrRepair(state.current);
          await this.writeState({ ...state, channel: selectedChannel, lastCheckAt: checkedAt });
          return { ...(await this.status()), changed: false };
        }
        next = await this.materializeVersion(availableVersion);
      } catch (error) {
        await this.writeState({ ...state, channel: selectedChannel, lastCheckAt: checkedAt }).catch(() => undefined);
        throw error;
      }
      await this.writeState({
        schemaVersion: 1,
        channel: selectedChannel,
        current: next,
        previous: state.current,
        lastCheckAt: checkedAt,
      });
      return { ...(await this.status()), changed: true };
    });
  }

  /**
   * Record `next` as the active installation, re-point `current`, and refresh
   * the launcher. A previous managed version is kept for rollback; an unmanaged
   * one (npx cache, checkout) is not, since it may vanish at any time. Callers
   * hold the update lock.
   */
  private async adopt(next: Installation): Promise<UpdateResult> {
    const state = await this.readState();
    const unchanged = resolve(state.current.packageRoot) === resolve(next.packageRoot);
    let previous: Installation | undefined;
    if (unchanged) {
      previous = state.previous;
    } else if (this.isManaged(state.current)) {
      previous = state.current;
    }
    await this.writeState({
      schemaVersion: 1,
      channel: state.channel,
      current: next,
      ...(previous === undefined ? {} : { previous }),
      ...(state.lastCheckAt === undefined ? {} : { lastCheckAt: state.lastCheckAt }),
    });
    return { ...(await this.status()), changed: !unchanged };
  }

  /**
   * Bootstrap: make `version` the managed, active installation regardless of
   * what is active now. This is how `npx @neralabs/rerail setup` turns the
   * ephemeral npx copy it runs from into a durable install under `versions/`.
   */
  async install(version = this.packageVersion): Promise<UpdateResult> {
    parseSemver(version);
    return await this.withLock(async () => await this.adopt(await this.materializeVersion(version)));
  }

  /**
   * Point `current` and the launcher at a package root that stays where it is —
   * a checkout, as `rerail install --root` uses. Recording it as the current
   * installation keeps `status`, the daemon, and the symlink telling the same
   * story; `isManaged` still reports it as unmanaged, so `update` replaces it
   * rather than offering it as a rollback target.
   */
  async activateRoot(packageRoot: string): Promise<UpdateResult> {
    const root = resolve(packageRoot);
    const manifestPath = join(root, "package.json");
    const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as Record<string, unknown>;
    const { version } = manifest;
    if (typeof version !== "string") throw new Error(`No package version in ${manifestPath}`);
    parseSemver(version);
    return await this.withLock(async () => await this.adopt({ version, packageRoot: root }));
  }

  async rollback(): Promise<UpdateResult> {
    return await this.withLock(async () => {
      const state = await this.readState();
      if (state.previous === undefined) throw new Error("No previous Rerail version is available for rollback");
      if (!existsSync(state.previous.packageRoot)) {
        throw new Error(`Previous Rerail installation is missing: ${state.previous.packageRoot}`);
      }
      await this.validate(state.previous);
      const rolledBack: UpdateState = {
        ...state,
        current: state.previous,
        previous: state.current,
      };
      await this.writeState(rolledBack);
      return { ...(await this.status()), changed: true };
    });
  }
}
