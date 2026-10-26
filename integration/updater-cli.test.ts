import { afterEach, describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { pathToFileURL } from "node:url";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

interface PackageManifest {
  name: string;
  version: string;
}

interface CommandResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

async function packageManifest(root = "."): Promise<PackageManifest> {
  const value = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as Record<string, unknown>;
  if (typeof value.name !== "string" || typeof value.version !== "string") throw new Error("Invalid package manifest");
  return { name: value.name, version: value.version };
}

function laterPatch(version: string, steps: number): string {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/u.exec(version);
  if (match === null) throw new Error(`Expected a semver package version, got ${version}`);
  const [, major, minor, patch] = match;
  return `${major}.${minor}.${Number(patch) + steps}`;
}

async function run(command: readonly string[], environment: NodeJS.ProcessEnv, cwd?: string): Promise<CommandResult> {
  const [executable, ...args] = command;
  if (executable === undefined) throw new Error("Expected a command to run");
  const child = spawn(executable, args, {
    env: environment,
    ...(cwd === undefined ? {} : { cwd }),
    stdio: ["ignore", "pipe", "pipe"],
  });
  const [stdout, stderr, exitCode] = await Promise.all([
    text(child.stdout),
    text(child.stderr),
    new Promise<number>((resolve, reject) => {
      child.once("error", reject);
      child.once("close", code => resolve(code ?? 1));
    }),
  ]);
  return { exitCode, stdout, stderr };
}

async function createPackageFixture(source: string, root: string, version: string, valid: boolean): Promise<string> {
  const packageRoot = join(root, version);
  await mkdir(packageRoot, { recursive: true });
  await writeFile(join(packageRoot, "package.json"), `${JSON.stringify({
    name: valid ? "@neralabs/rerail" : "@fixture/rejected-rerail",
    version,
    type: "module",
  })}\n`);
  if (!valid) return packageRoot;

  const dist = join(packageRoot, "dist");
  await mkdir(join(dist, "cli"), { recursive: true });
  await mkdir(join(dist, "harness"));
  const sourceCli = pathToFileURL(join(source, "dist", "cli", "cli.js")).href;
  await Promise.all([
    writeFile(join(dist, "index.js"), "export function nativeHealth() { return { ok: true }; }\n"),
    writeFile(join(dist, "harness", "event-stream.js"), [
      'import { mkdirSync, readFileSync } from "node:fs";',
      'import { join } from "node:path";',
      'const lines = readFileSync(0, "utf8").trimEnd().split("\\n");',
      "const request = JSON.parse(lines.at(-1));",
      "// The health check requires the session to have opened under the storage root.",
      'mkdirSync(join(process.env.RERAIL_STORAGE_ROOT, "session"), { recursive: true });',
      'process.stdout.write(JSON.stringify({ eventId: request.event.eventId, deliveries: [] }) + "\\n");',
      "",
    ].join("\n")),
    writeFile(join(dist, "cli", "cli.js"), [
      'import { writeFileSync } from "node:fs";',
      'const managedChildArgument = "--rerail-managed-child";',
      "if (process.argv[2] !== managedChildArgument) process.argv.splice(2, 0, managedChildArgument);",
      'if (process.env.RERAIL_LAUNCH_MARKER !== undefined) {',
      `  writeFileSync(process.env.RERAIL_LAUNCH_MARKER, ${JSON.stringify(version)});`,
      "}",
      "// The packaged CLI path is selected when the fixture is created.",
      `await import(${JSON.stringify(sourceCli)});`,
      "",
    ].join("\n")),
  ]);
  return packageRoot;
}

async function createLocalRegistry(root: string): Promise<string> {
  const bin = join(root, "bin");
  await mkdir(bin, { recursive: true });
  const fixture = join(bin, "npm-fixture.mjs");
  await writeFile(fixture, `import { cpSync, readFileSync } from "node:fs";
import { join } from "node:path";
// Rerail puts npm's own options (fetch retries) ahead of the command, as npm accepts them: the command is the first
// argument that is not an option.
const argv = process.argv.slice(2);
const args = argv.slice(argv.findIndex(argument => !argument.startsWith("-")));
if (args[0] === "view") {
  process.stdout.write(JSON.stringify(args[1].endsWith("@rejected") ? process.env.RERAIL_REJECTED_VERSION : process.env.RERAIL_ACCEPTED_VERSION) + "\\n");
  process.exit(0);
}
if (args[0] === "install") {
  const prefix = args[args.indexOf("--prefix") + 1];
  const version = args.at(-1).split("@").at(-1);
  const source = version === process.env.RERAIL_ACCEPTED_VERSION ? process.env.RERAIL_ACCEPTED_PACKAGE : process.env.RERAIL_REJECTED_PACKAGE;
  // Both packages install where the accepted package's name says, so a rejected one is caught by its identity.
  const { name } = JSON.parse(readFileSync(join(process.env.RERAIL_ACCEPTED_PACKAGE, "package.json"), "utf8"));
  cpSync(source, join(prefix, "node_modules", ...name.split("/")), { recursive: true });
  process.exit(0);
}
process.exit(1);
`);
  const npm = join(bin, "npm");
  // The POSIX shim deliberately exercises launching a runtime-selected fixture path.
  await writeFile(npm, `#!/usr/bin/env node\nawait import(${JSON.stringify(fixture)});\n`);
  await chmod(npm, 0o755);
  await writeFile(
    join(bin, "npm.cmd"),
    `@echo off\r\n"${process.execPath.replaceAll("%", "%%")}" "${fixture.replaceAll("%", "%%")}" %*\r\n`,
  );
  return bin;
}

describe("Rerail managed launcher", () => {
  test("installs the packaged launcher and accepts then rejects deterministic local updates", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-launcher-"));
    temporaryDirectories.push(root);
    const manifest = await packageManifest();
    const acceptedVersion = laterPatch(manifest.version, 1);
    const rejectedVersion = laterPatch(manifest.version, 2);
    const packaged = await run(["npm", "pack", "--pack-destination", root], process.env);
    expect(packaged.exitCode).toBe(0);
    const archive = join(root, `${manifest.name.slice(1).replace("/", "-")}-${manifest.version}.tgz`);
    const consumer = join(root, "consumer");
    const installed = await run(["npm", "install", "--prefix", consumer, "--no-audit", "--no-fund", archive], process.env);
    expect(installed.exitCode).toBe(0);
    const launcherRoot = join(consumer, "node_modules", ...manifest.name.split("/"));
    const accepted = await createPackageFixture(launcherRoot, join(root, "registry"), acceptedVersion, true);
    const rejected = await createPackageFixture(launcherRoot, join(root, "registry"), rejectedVersion, false);
    const bin = await createLocalRegistry(root);
    const home = join(root, "home");
    const userHome = join(root, "user");
    await mkdir(join(userHome, ".claude"), { recursive: true });
    await mkdir(join(userHome, ".cursor"));
    const marker = join(root, "launched-version");
    const environment = {
      ...process.env,
      PATH: `${bin}${delimiter}${process.env.PATH ?? ""}`,
      // The coding agents' config: Claude Code connected below, Cursor present but never connected.
      HOME: userHome,
      RERAIL_DEVICE_SECRET_STORE: "file",
      RERAIL_HOME: home,
      RERAIL_ACCEPTED_VERSION: acceptedVersion,
      RERAIL_ACCEPTED_PACKAGE: accepted,
      RERAIL_REJECTED_PACKAGE: rejected,
      RERAIL_LAUNCH_MARKER: marker,
      RERAIL_REJECTED_VERSION: rejectedVersion,
    };
    const entrypoint = join(launcherRoot, "dist", "cli", "cli.js");
    const connected = await run(["node", entrypoint, "install", "--root", launcherRoot, "--agents", "claude-code"], environment, root);
    expect(connected.exitCode).toBe(0);
    // The installed version, for a terminal, without a daemon to ask: `rerail version`, or the flags people try first.
    for (const argument of ["version", "--version", "-v"]) {
      expect(await run(["node", entrypoint, argument], environment)).toEqual({ exitCode: 0, stdout: `rerail ${manifest.version}\n`, stderr: "" });
    }
    const commands = join(userHome, ".claude", "commands");
    const showMe = join(commands, "rerail-show-me.md");
    // A command this version adds that the connected Claude Code lacks, as after updating from one without it.
    await rm(showMe);

    const update = await run(["node", entrypoint, "update"], environment);
    // Installing a version says what it downloads, on stderr; nothing else is written there.
    expect(update).toMatchObject({ exitCode: 0, stderr: `Downloading ${manifest.name}@${acceptedVersion} from npm…\n` });
    expect(update.stdout).toContain(`${acceptedVersion}; new harness sessions will use it.`);
    expect(update.stdout).toContain(`Refreshed Rerail in Claude Code:`);
    expect(await readFile(showMe, "utf8")).toContain(join(home, "current", "dist", "cli", "cli.js"));
    expect(await readdir(join(userHome, ".cursor"))).toEqual([]);

    // With nothing newer, update still refreshes the connected coding agents.
    await rm(showMe);
    const current = await run(["node", entrypoint, "update"], environment);
    expect(current).toMatchObject({ exitCode: 0, stderr: "" });
    expect(await readdir(commands)).toContain("rerail-show-me.md");
    const acceptedState = JSON.parse(await readFile(join(home, "update-state.json"), "utf8")) as Record<string, unknown>;
    expect(acceptedState).toMatchObject({
      current: { version: acceptedVersion },
      previous: { version: manifest.version },
    });

    const dispatched = await run(["node", entrypoint, "update", "check"], environment);
    expect(dispatched.exitCode).toBe(0);
    expect(await readFile(marker, "utf8")).toBe(acceptedVersion);

    const launcher = join(home, "bin", process.platform === "win32" ? "rerail.cmd" : "rerail");
    const launchCommand = process.platform === "win32"
      ? ["cmd.exe", "/d", "/s", "/c", launcher, "update", "check"]
      : [launcher, "update", "check"];
    const launched = await run(launchCommand, environment);
    expect(launched.exitCode).toBe(0);
    expect(await readFile(marker, "utf8")).toBe(acceptedVersion);
    const rejectedUpdate = await run(["node", entrypoint, "update", "--channel", "rejected"], environment);
    expect(rejectedUpdate.exitCode).toBe(1);
    expect(rejectedUpdate.stderr).toContain("Installed package identity mismatch");
    const rejectedState = JSON.parse(await readFile(join(home, "update-state.json"), "utf8")) as Record<string, unknown>;
    expect(rejectedState).toMatchObject({
      current: { version: acceptedVersion },
      previous: { version: manifest.version },
    });

    // The rejected update above left its channel selected; back on latest, nothing is newer and only the refresh runs.
    // A coding agent never connected whose configuration cannot be read is named and left alone: the update succeeds.
    await writeFile(join(userHome, ".cursor", "hooks.json"), "{ not json");
    const unconnected = await run(["node", entrypoint, "update", "--channel", "latest"], environment);
    expect(unconnected).toMatchObject({ exitCode: 0, stderr: "" });
    expect(unconnected.stdout).toContain("left Cursor alone");
    expect(await readFile(join(userHome, ".cursor", "hooks.json"), "utf8")).toBe("{ not json");

    // A connected coding agent registration cannot write fails the update, which keeps the version it activated.
    const settingsPath = join(userHome, ".claude", "settings.json");
    const settings = JSON.parse(await readFile(settingsPath, "utf8")) as { hooks: Record<string, unknown> };
    settings.hooks.SessionStart = "not a list";
    await writeFile(settingsPath, JSON.stringify(settings));
    const unwritable = await run(["node", entrypoint, "update"], environment);
    expect(unwritable.exitCode).toBe(1);
    expect(unwritable.stderr).toContain(settingsPath);
    expect(unwritable.stderr).toContain("rerail install --refresh");
    expect(JSON.parse(await readFile(join(home, "update-state.json"), "utf8"))).toMatchObject({ current: { version: acceptedVersion } });
  }, 60_000);

  test("keeps status and rollback available when the active managed CLI is missing", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-launcher-"));
    temporaryDirectories.push(root);
    const manifest = await packageManifest();
    const missingVersion = laterPatch(manifest.version, 1);
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    await writeFile(join(home, "update-state.json"), `${JSON.stringify({
      schemaVersion: 1,
      channel: "latest",
      current: { version: missingVersion, packageRoot: join(root, "missing") },
      previous: { version: manifest.version, packageRoot: resolve(".") },
    })}\n`);
    const environment = { ...process.env, RERAIL_HOME: home };

    const status = await run(["node", "dist/cli/cli.js", "update", "status"], environment);
    expect(status).toMatchObject({ exitCode: 0, stderr: "" });
    expect(status.stdout).toContain(`Current: ${missingVersion}`);
    const rollback = await run(["node", "dist/cli/cli.js", "update", "rollback"], environment);
    expect(rollback).toMatchObject({ exitCode: 0, stderr: "" });
    expect(rollback.stdout).toContain(`${manifest.version}; new harness sessions will use it.`);
  }, 30_000); // Rollback health-checks this checkout: a real start, then a real session.

});
