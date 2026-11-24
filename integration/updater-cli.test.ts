import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { execFile, spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { nativePlatformSuffix } from "../src/engine/index.js";
import { runNpm } from "../src/runtime/index.js";
import { thirdPartyPack } from "../test/support/installed-packs.js";
import { removeFolder } from "../test/support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
/** Each command the test ran and how long it took, so a slow run says where the time went, not just that it ran out. */
const commandTimes: Array<[string, number]> = [];
/** Two thirds of the 60 s a test gets for its CLI runs; the pack and install have their own budget and are not counted. */
const SLOW_RUN_MS = 40_000;
afterEach(async () => {
  // A test that timed out keeps running, and its CLI children may still be writing under the directory: removeFolder
  // then names each one still running there.
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
  const total = commandTimes.reduce((sum, [, ms]) => sum + ms, 0);
  if (total > SLOW_RUN_MS) {
    const lines = commandTimes.map(([command, ms]) => `  ${String(ms).padStart(6)} ms  ${command}`);
    process.stderr.write(`updater-cli: ${total} ms spent in ${commandTimes.length} commands:\n${lines.join("\n")}\n`);
  }
  commandTimes.length = 0;
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
  const started = Date.now();
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
  // The launcher's path is the same in every command; its arguments are what tell them apart.
  commandTimes.push([[executable, ...args].map(part => part.replace(/^.*\/(?=[^/]+$)/u, "")).join(" "), Date.now() - started]);
  return { exitCode, stdout, stderr };
}

/**
 * A core package as a registry holds it. Like a released core, it pins `enginePackage`, the engine package of this
 * machine, in the shrinkwrap it ships: the updater holds the engine npm installed to that integrity.
 */
async function createPackageFixture(source: string, root: string, version: string, valid: boolean, enginePackage: string): Promise<string> {
  const packageRoot = join(root, version);
  await mkdir(packageRoot, { recursive: true });
  await writeFile(join(packageRoot, "package.json"), `${JSON.stringify({
    name: valid ? "@neralabs/rerail" : "@fixture/rejected-rerail",
    version,
    type: "module",
  })}\n`);
  await writeFile(join(packageRoot, "npm-shrinkwrap.json"), `${JSON.stringify({
    packages: { [`node_modules/${enginePackage}`]: { version, integrity: `sha512-engine-of-${version}` } },
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

/**
 * Installs pack packages (a `package.json` each) into `home`'s packs as npm records them from the registry; with `keep`,
 * beside the packs already recorded there.
 */
async function installRegistryPacks(home: string, packs: Record<string, string>, { keep = false } = {}): Promise<void> {
  const prefix = join(home, "packs");
  for (const [packageName, version] of Object.entries(packs)) {
    await mkdir(join(prefix, "node_modules", ...packageName.split("/")), { recursive: true });
    await writeFile(join(prefix, "node_modules", ...packageName.split("/"), "package.json"), JSON.stringify({ name: packageName, version }));
  }
  const recorded = keep ? (JSON.parse(await readFile(join(prefix, "package.json"), "utf8")) as { dependencies: Record<string, string> }).dependencies : {};
  await writeFile(join(prefix, "package.json"), JSON.stringify({ dependencies: { ...recorded, ...packs } }));
}

/** Pack versions npm serves in the local registry: their dist-tags, core ranges and folders (`RERAIL_PACK_REGISTRY`). */
function packRegistry(root: string): string {
  const fits = ">=0.0.0-0";
  const at = (packageName: string, id: string, version: string, range = fits) => ({ range, folder: thirdPartyPack(join(root, "pack-registry"), packageName, id, version, range) });
  return JSON.stringify({
    "@acme/rerail-pack-sql": { tags: { latest: "0.9.0" }, versions: { "0.2.0": { range: fits }, "0.9.0": at("@acme/rerail-pack-sql", "sql", "0.9.0") } },
    "@neralabs/rerail-pack-typescript": {
      tags: { latest: "1.0.0" },
      versions: { "0.1.0": { range: fits }, "0.9.0": at("@neralabs/rerail-pack-typescript", "typescript", "0.9.0"), "1.0.0": { range: ">=99.0.0" } },
    },
    // Each version npm has fits; 1.1.0 as published names a core range none of these cores is in.
    "@acme/rerail-pack-lint": {
      tags: { latest: "1.1.0" },
      versions: { "1.0.0": at("@acme/rerail-pack-lint", "lint", "1.0.0"), "1.1.0": { range: fits, folder: thirdPartyPack(join(root, "pack-registry"), "@acme/rerail-pack-lint", "lint", "1.1.0", ">=99.0.0") } },
    },
    "@acme/rerail-pack-fmt": { tags: { latest: "1.1.0" }, versions: { "1.0.0": { range: fits }, "1.1.0": at("@acme/rerail-pack-fmt", "fmt", "1.1.0") } },
    // Published under next only, as set-up installs it.
    "@neralabs/rerail-pack-java": {
      tags: { next: "0.1.0-next.3" },
      versions: { "0.1.0-next.1": { range: fits }, "0.1.0-next.3": at("@neralabs/rerail-pack-java", "java", "0.1.0-next.3") },
    },
  });
}

async function createLocalRegistry(root: string): Promise<string> {
  const bin = join(root, "bin");
  await mkdir(bin, { recursive: true });
  const fixture = join(bin, "npm-fixture.mjs");
  await writeFile(fixture, `import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
// Rerail puts npm's own options (fetch retries) ahead of the command, as npm accepts them: the command is the first
// argument that is not an option.
const argv = process.argv.slice(2);
const command = argv.findIndex(argument => !argument.startsWith("-"));
const args = command === -1 ? [] : argv.slice(command);
// Rule packs, from RERAIL_PACK_REGISTRY: each package's dist-tags, and each version's folder and core range.
const packRegistry = JSON.parse(process.env.RERAIL_PACK_REGISTRY ?? "{}");
const packOf = spec => spec.slice(0, spec.lastIndexOf("@"));
if (args[0] === "view" && args[1].includes("rerail-pack-")) {
  const listing = args[2] === "dist-tags";
  const found = packRegistry[listing ? args[1] : packOf(args[1])];
  if (found === undefined) {
    process.stderr.write("npm error code E404\\nnpm error 404 Not Found - " + args[1] + "\\n");
    process.exit(1);
  }
  if (listing) {
    process.stdout.write(JSON.stringify({ "dist-tags": found.tags, versions: Object.keys(found.versions) }) + "\\n");
    process.exit(0);
  }
  const named = args[1].slice(args[1].lastIndexOf("@") + 1).split(" || ").filter(version => version in found.versions);
  const answers = named.map(version => ({ name: packOf(args[1]), version, peerDependencies: { "@neralabs/rerail": found.versions[version].range } }));
  process.stdout.write(JSON.stringify(answers.length === 1 ? answers[0] : answers) + "\\n");
  process.exit(0);
}
if (args[0] === "install" && args.at(-1).includes("rerail-pack-")) {
  // What npm leaves for a pack installed from the registry: its folder in node_modules, and its version recorded.
  const prefix = args[args.indexOf("--prefix") + 1];
  const spec = args.at(-1);
  const packageName = packOf(spec);
  const version = spec.slice(spec.lastIndexOf("@") + 1);
  const installed = join(prefix, "node_modules", ...packageName.split("/"));
  rmSync(installed, { recursive: true, force: true });
  cpSync(packRegistry[packageName].versions[version].folder, installed, { recursive: true });
  const manifest = JSON.parse(readFileSync(join(prefix, "package.json"), "utf8"));
  writeFileSync(join(prefix, "package.json"), JSON.stringify({ ...manifest, dependencies: { ...manifest.dependencies, [packageName]: version } }));
  process.exit(0);
}
if (args[0] === "view") {
  process.stdout.write(JSON.stringify(args[1].endsWith("@rejected") ? process.env.RERAIL_REJECTED_VERSION : process.env.RERAIL_ACCEPTED_VERSION) + "\\n");
  process.exit(0);
}
if (args[0] === "install") {
  const prefix = args[args.indexOf("--prefix") + 1];
  const { name } = JSON.parse(readFileSync(join(process.env.RERAIL_ACCEPTED_PACKAGE, "package.json"), "utf8"));
  // The specs: core's, and with it the engine package's of this machine (installVersion in src/setup/updater.ts).
  const specs = args.filter(argument => argument.startsWith("@"));
  const version = specs.find(spec => spec.startsWith(name + "@")).slice(name.length + 1);
  const source = version === process.env.RERAIL_ACCEPTED_VERSION ? process.env.RERAIL_ACCEPTED_PACKAGE : process.env.RERAIL_REJECTED_PACKAGE;
  // Both packages install where the accepted package's name says, so a rejected one is caught by its identity.
  cpSync(source, join(prefix, "node_modules", ...name.split("/")), { recursive: true });
  // An engine package installs beside core, and npm records it in a lockfile of its own with the integrity of the
  // tarball the registry served: here the one core pins in its shrinkwrap.
  const pinned = JSON.parse(readFileSync(join(source, "npm-shrinkwrap.json"), "utf8")).packages;
  const installed = {};
  for (const spec of specs.filter(spec => !spec.startsWith(name + "@"))) {
    const enginePackage = spec.slice(0, spec.lastIndexOf("@"));
    const location = "node_modules/" + enginePackage;
    mkdirSync(join(prefix, location), { recursive: true });
    writeFileSync(join(prefix, location, "package.json"), JSON.stringify({ name: enginePackage, version }));
    installed[location] = pinned[location];
  }
  writeFileSync(join(prefix, "node_modules", ".package-lock.json"), JSON.stringify({ packages: installed }));
  process.exit(0);
}
if (args[0] === "audit" && args[1] === "signatures") {
  // What npm 11 answers for packages from a registry that publishes no signing keys, as this local one is.
  process.stderr.write("npm error found no dependencies to audit that were installed from a supported registry\\n");
  process.exit(1);
}
process.exit(1);
`);
  const npm = join(bin, "npm");
  // The POSIX shim deliberately exercises launching a runtime-selected fixture path.
  await writeFile(npm, `#!/usr/bin/env node\nawait import(${JSON.stringify(fixture)});\n`);
  await chmod(npm, 0o755);
  // Rerail on Windows runs no npm from PATH: it runs npm's own script under Node, the npm-cli.js that npm_execpath names.
  await writeFile(join(bin, "npm-cli.js"), `await import(${JSON.stringify(pathToFileURL(fixture).href)});\n`);
  return bin;
}

describe("Rerail managed launcher", () => {
  let packageRoot: string | undefined;
  /**
   * This checkout, packed and installed into a consumer directory as `npm install @neralabs/rerail` would install it:
   * core, and beside it the engine package of this machine, which core names at its own version.
   */
  let packaged: { manifest: PackageManifest; root: string; enginePackage: string } | undefined;

  // The packs and the install take the registry's and the runner's time, not the updater's: about 13s locally and
  // twice that in CI, so they have their own budget and the tests' limits cover the CLI alone.
  beforeAll(async () => {
    packageRoot = await mkdtemp(join(tmpdir(), "rerail-launcher-package-"));
    const manifest = await packageManifest();
    // Core's tarball carries no engine. The checkout's engines (native/) are packed as engine packages, and core
    // against them, so it names each at its exact version as a release names all six.
    const engines = join(packageRoot, "engines");
    await promisify(execFile)(process.execPath, ["scripts/release/engine/packages.mjs", "pack", "--out", engines]);
    const packed = JSON.parse(await readFile(join(engines, "engine-packages.json"), "utf8")) as { packages: Array<{ suffix: string; name: string; file: string }> };
    const engine = packed.packages.find(({ suffix }) => suffix === nativePlatformSuffix());
    if (engine === undefined) throw new Error(`native/ has no engine for ${nativePlatformSuffix()}: npm run build:native builds it`);
    // npm as Rerail starts it: on Windows npm is a npm.cmd script, which only a shell runs.
    await runNpm(["pack", "--pack-destination", packageRoot], { env: { ...process.env, RERAIL_ENGINE_PACKAGES: engines } });
    const archive = join(packageRoot, `${manifest.name.slice(1).replace("/", "-")}-${manifest.version}.tgz`);
    const consumer = join(packageRoot, "consumer");
    // A clean install from the tarball's declared dependencies, so one missing from package.json fails here; packages
    // already in the npm cache (CI restores it and runs npm ci first) are taken from it instead of the registry. The
    // engine package is on no registry at this version, so its tarball is installed in the same command.
    await runNpm(["install", "--prefer-offline", "--prefix", consumer, "--no-audit", "--no-fund", archive, join(engines, engine.file)]);
    packaged = { manifest, root: join(consumer, "node_modules", ...manifest.name.split("/")), enginePackage: engine.name };
  }, 120_000);

  afterAll(async () => {
    if (packageRoot !== undefined) await removeFolder(packageRoot);
  });

  test("installs the packaged launcher and accepts then rejects deterministic local updates", async () => {
    if (packaged === undefined) throw new Error("The launcher was not packaged and installed");
    const { manifest, root: launcherRoot, enginePackage } = packaged;
    const root = await mkdtemp(join(tmpdir(), "rerail-launcher-"));
    temporaryDirectories.push(root);
    const acceptedVersion = laterPatch(manifest.version, 1);
    const rejectedVersion = laterPatch(manifest.version, 2);
    const accepted = await createPackageFixture(launcherRoot, join(root, "registry"), acceptedVersion, true, enginePackage);
    const rejected = await createPackageFixture(launcherRoot, join(root, "registry"), rejectedVersion, false, enginePackage);
    const bin = await createLocalRegistry(root);
    const home = join(root, "home");
    const userHome = join(root, "user");
    await mkdir(join(userHome, ".claude"), { recursive: true });
    await mkdir(join(userHome, ".cursor"));
    const marker = join(root, "launched-version");
    const environment = {
      // Windows reads a variable's name in any case and takes the first in sort order, so the NPM_EXECPATH npm sets
      // there would win over the npm_execpath below.
      ...Object.fromEntries(Object.entries(process.env).filter(([name]) => name.toLowerCase() !== "npm_execpath")),
      PATH: `${bin}${delimiter}${process.env.PATH ?? ""}`,
      npm_execpath: join(bin, "npm-cli.js"),
      // The coding agents' config: Claude Code connected below, Cursor present but never connected. The home is HOME on
      // POSIX and USERPROFILE on Windows.
      HOME: userHome,
      USERPROFILE: userHome,
      RERAIL_DEVICE_SECRET_STORE: "file",
      RERAIL_HOME: home,
      RERAIL_ACCEPTED_VERSION: acceptedVersion,
      RERAIL_ACCEPTED_PACKAGE: accepted,
      RERAIL_REJECTED_PACKAGE: rejected,
      RERAIL_LAUNCH_MARKER: marker,
      RERAIL_REJECTED_VERSION: rejectedVersion,
      RERAIL_PACK_REGISTRY: packRegistry(root),
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

    /** What an update with nothing newer says on stderr, with a rule pack installed: its steps, and no problem. */
    const nothingNewerSteps = [
      "Checking npm for the newest Rerail on latest…",
      `Checking that Rerail ${acceptedVersion} starts…`,
      "Checking your rule packs for newer versions…",
    ];
    const update = await run(["node", entrypoint, "update"], environment);
    // Each step says so as it starts, on stderr, a whole line each off a terminal; this local registry publishes no
    // signing keys, so npm checks no signature and the update says so there too, and goes on.
    expect(update.exitCode).toBe(0);
    expect(update.stderr.split("\n").filter(line => line !== "")).toEqual([
      "Checking npm for the newest Rerail on latest…",
      `Downloading ${manifest.name}@${acceptedVersion} from npm…`,
      `Checking the registry signatures of ${manifest.name}@${acceptedVersion}…`,
      expect.stringContaining("publishes no signing keys"),
      `Checking that Rerail ${acceptedVersion} starts…`,
      `Switching to Rerail ${acceptedVersion}…`,
    ]);
    expect(update.stdout).toContain(`Updated Rerail ${manifest.version} → ${acceptedVersion} in `);
    expect(update.stdout).toContain(" s; new harness sessions will use it.");
    expect(update.stdout).toContain(`Refreshed Rerail in Claude Code:`);
    expect(await readFile(showMe, "utf8")).toContain(join(home, "current", "dist", "cli", "cli.js"));
    expect(await readdir(join(userHome, ".cursor"))).toEqual([]);

    // With nothing newer, update still refreshes the connected coding agents, and still moves each installed rule pack
    // to the newest version that fits: packs are released without core.
    await rm(showMe);
    await installRegistryPacks(home, { "@acme/rerail-pack-sql": "0.2.0", "@neralabs/rerail-pack-typescript": "0.1.0" });
    const current = await run(["node", entrypoint, "update"], environment);
    expect(current.exitCode).toBe(0);
    expect(current.stderr.split("\n").filter(line => line !== "")).toEqual([
      ...nothingNewerSteps,
      "Installing @acme/rerail-pack-sql@0.9.0…",
      "Installing @neralabs/rerail-pack-typescript@0.9.0…",
    ]);
    // A third party's pack goes by its package name, and its new version waits for approval again. Typescript's 1.0.0
    // is for a later core, so it moves to 0.9.0.
    expect(current.stdout.endsWith([
      "Updated 2 rule packs; every repository that adds one runs its new rules:",
      "  @acme/rerail-pack-sql 0.2.0 → 0.9.0 (Rerail has not reviewed it: its rules run in Log until you approve this version, rerail rules approve @acme/rerail-pack-sql)",
      "  typescript 0.1.0 → 0.9.0",
      "",
    ].join("\n"))).toBe(true);
    const packs = JSON.parse(await readFile(join(home, "packs", "package.json"), "utf8")) as { dependencies: Record<string, string> };
    expect(packs.dependencies).toEqual({ "@acme/rerail-pack-sql": "0.9.0", "@neralabs/rerail-pack-typescript": "0.9.0" });
    // A pack the channel has no version of is taken up to its version under next, and its line says so.
    await installRegistryPacks(home, { "@neralabs/rerail-pack-java": "0.1.0-next.1" }, { keep: true });
    const held = await run(["node", entrypoint, "update"], environment);
    expect(held.exitCode).toBe(0);
    expect(held.stderr.split("\n").filter(line => line !== "")).toEqual([...nothingNewerSteps, "Installing @neralabs/rerail-pack-java@0.1.0-next.3…"]);
    expect(held.stdout.endsWith([
      "Updated a rule pack; every repository that adds it runs its new rules:",
      "  java 0.1.0-next.1 → 0.1.0-next.3 (from next)",
      "",
    ].join("\n"))).toBe(true);
    // Nothing newer for any pack: the update says so.
    const settled = await run(["node", entrypoint, "update"], environment);
    expect(settled.exitCode).toBe(0);
    expect(settled.stderr.split("\n").filter(line => line !== "")).toEqual(nothingNewerSteps);
    expect(settled.stdout.endsWith(`Your rule packs are the newest versions that fit Rerail ${acceptedVersion}.\n`)).toBe(true);
    // A pack whose new version does not load is put back alone: the other still moves, the new core stays active, and
    // the update fails, saying why and how to try again.
    const recordedPacks = await readFile(join(home, "packs", "package.json"), "utf8");
    await installRegistryPacks(home, { "@acme/rerail-pack-fmt": "1.0.0", "@acme/rerail-pack-lint": "1.0.0" }, { keep: true });
    const unloadable = await run(["node", entrypoint, "update"], environment);
    expect(unloadable.exitCode).toBe(1);
    expect(unloadable.stderr).toMatch(/rerail: could not update @acme\/rerail-pack-lint to 1\.1\.0: @acme\/rerail-pack-lint@1\.1\.0 cannot be used: .*>=99\.0\.0.*\. Nothing was changed\.\n/u);
    expect(unloadable.stderr.trimEnd().endsWith("Run `rerail packs update` to try again.")).toBe(true);
    expect(unloadable.stdout).toContain("Updated a rule pack; every repository that adds it runs its new rules:\n  @acme/rerail-pack-fmt 1.0.0 → 1.1.0");
    expect((JSON.parse(await readFile(join(home, "packs", "package.json"), "utf8")) as { dependencies: Record<string, string> }).dependencies)
      .toMatchObject({ "@acme/rerail-pack-fmt": "1.1.0", "@acme/rerail-pack-lint": "1.0.0" });
    expect(JSON.parse(await readFile(join(home, "update-state.json"), "utf8"))).toMatchObject({ current: { version: acceptedVersion } });
    // Both go again, so the later updates find every pack current.
    for (const packageName of ["rerail-pack-fmt", "rerail-pack-lint"]) await rm(join(home, "packs", "node_modules", "@acme", packageName), { recursive: true });
    await writeFile(join(home, "packs", "package.json"), recordedPacks);
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
    expect(unconnected.exitCode).toBe(0);
    expect(unconnected.stderr.split("\n").filter(line => line !== "")).toEqual(nothingNewerSteps);
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
  }, 60_000); // Fourteen CLI runs, eight of them updates: about 15 s locally, twice that in CI.

  test("keeps status available when the active managed CLI is missing", async () => {
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
    // A command the missing launcher would have run says how to get Rerail back.
    const forwarded = await run(["node", "dist/cli/cli.js", "update", "check"], environment);
    expect(forwarded.exitCode).toBe(1);
    expect(forwarded.stderr).toContain(`Active Rerail launcher is missing: ${join(root, "missing", "dist", "cli", "cli.js")}. Run npx ${manifest.name} setup`);
  });

});
