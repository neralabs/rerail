import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  engineLoadFailure,
  NATIVE_ENGINE_SYSTEMS,
  nativeEnginePackageName,
  nativePlatformSuffix,
  resolveNativeBindingPath,
  type NativeEngineSite,
} from "../../../src/engine/native/native.js";
import { removeFolderSync } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

const VERSION = "1.2.0";
type System = readonly [platform: NodeJS.Platform, architecture: string];
const LINUX: System = ["linux", "x64"];
const MAC: System = ["darwin", "arm64"];

const engineFile = (system: System): string => `rerail_engine.${nativePlatformSuffix(...system)}.node`;

/**
 * A folder npm installed Rerail into: `<prefix>/node_modules/@neralabs/rerail`, named by the prefix's `package.json`.
 * With `folder`, the prefix is that folder inside a new one, as an update's staging folder is inside `versions/`.
 */
function installedRerail(dependencies: Record<string, string> = { "@neralabs/rerail": VERSION }, folder?: string): { prefix: string; packageRoot: string } {
  // Node resolves a package to its real path, and macOS keeps its temporary folder behind a symlink.
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-engine-package-")));
  roots.push(root);
  const prefix = folder === undefined ? root : join(root, folder);
  const packageRoot = join(prefix, "node_modules", "@neralabs", "rerail");
  mkdirSync(packageRoot, { recursive: true });
  writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: VERSION }));
  writeFileSync(join(prefix, "package.json"), JSON.stringify({ dependencies }));
  return { prefix, packageRoot };
}

/** The engine package of `system` in the `node_modules` of `folder`, at `version`; returns its engine file. */
function enginePackage(folder: string, system: System, version = VERSION): string {
  const root = join(folder, "node_modules", ...nativeEnginePackageName(...system).split("/"));
  mkdirSync(root, { recursive: true });
  writeFileSync(join(root, "package.json"), JSON.stringify({ name: nativeEnginePackageName(...system), version }));
  writeFileSync(join(root, engineFile(system)), "");
  return join(root, engineFile(system));
}

const on = (packageRoot: string, [platform, architecture]: System, musl = false): NativeEngineSite => ({ packageRoot, platform, architecture, musl });
/** The engine the Rerail at `packageRoot` loads on `system`, with no `RERAIL_ENGINE_PATH` set. */
const engineOf = (packageRoot: string, system: System, musl = false): string => resolveNativeBindingPath(undefined, on(packageRoot, system, musl));

describe("the engine Rerail loads", () => {
  it("is the file RERAIL_ENGINE_PATH names, which must exist, whatever is installed", () => {
    const { prefix, packageRoot } = installedRerail();
    const configured = enginePackage(prefix, LINUX, "9.9.9");
    expect(resolveNativeBindingPath(configured, on(packageRoot, LINUX))).toBe(configured);
    expect(() => resolveNativeBindingPath(join(prefix, "absent.node"), on(packageRoot, LINUX))).toThrow(`RERAIL_ENGINE_PATH does not exist: ${join(prefix, "absent.node")}`);
  });

  it("is the one in the package's own native/ before any engine package, as a checkout's build is", () => {
    const { prefix, packageRoot } = installedRerail();
    // Never read: an engine package of another version would be refused.
    enginePackage(prefix, LINUX, "9.9.9");
    mkdirSync(join(packageRoot, "native"));
    writeFileSync(join(packageRoot, "native", engineFile(LINUX)), "");
    expect(engineOf(packageRoot, LINUX)).toBe(join(packageRoot, "native", engineFile(LINUX)));
  });

  it("is the one in the system's engine package beside Rerail, where an install that names the package puts it", () => {
    const { prefix, packageRoot } = installedRerail();
    const engine = enginePackage(prefix, LINUX);
    enginePackage(prefix, MAC);
    expect(engineOf(packageRoot, LINUX)).toBe(engine);
  });

  it("is the one in the engine package inside Rerail's own node_modules, where npm puts its optional dependencies, before one beside it", () => {
    const { prefix, packageRoot } = installedRerail();
    for (const system of NATIVE_ENGINE_SYSTEMS) enginePackage(packageRoot, system);
    enginePackage(prefix, LINUX, "9.9.9");
    expect(engineOf(packageRoot, LINUX)).toBe(join(packageRoot, "node_modules", ...nativeEnginePackageName(...LINUX).split("/"), engineFile(LINUX)));
  });

  it("is never one in a node_modules above the install, where Node's own lookup would go on to find it", () => {
    const { prefix } = installedRerail();
    // An install nested in a folder that has an engine package of its own: `<prefix>/inner/node_modules/@neralabs/rerail`.
    const inner = join(prefix, "inner");
    const packageRoot = join(inner, "node_modules", "@neralabs", "rerail");
    mkdirSync(packageRoot, { recursive: true });
    writeFileSync(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: VERSION }));
    enginePackage(prefix, LINUX);
    expect(() => engineOf(packageRoot, LINUX)).toThrow("Rerail's engine for Linux on x64 is not installed");
    const engine = enginePackage(inner, LINUX);
    expect(engineOf(packageRoot, LINUX)).toBe(engine);
  });

  it("is found once per process: later calls neither resolve the package nor read its version again", () => {
    const { prefix, packageRoot } = installedRerail();
    const engine = enginePackage(prefix, LINUX);
    expect(engineOf(packageRoot, LINUX)).toBe(engine);
    enginePackage(prefix, LINUX, "9.9.9");
    expect(engineOf(packageRoot, LINUX)).toBe(engine);
  });
});

describe("an engine Rerail cannot load", () => {
  it("refuses an engine package of another version than Rerail's, naming both", () => {
    const { prefix, packageRoot } = installedRerail();
    enginePackage(prefix, LINUX, "1.1.9");
    const name = nativeEnginePackageName(...LINUX);
    expect(() => engineOf(packageRoot, LINUX)).toThrow(
      `${name} at ${join(prefix, "node_modules", ...name.split("/"))} is version 1.1.9, and the Rerail at ${packageRoot} is version 1.2.0: Rerail loads only the engine released with it. Install it with \`npm install --prefix "${prefix}" ${name}@1.2.0\`.`,
    );
  });

  it.each<[string, System, boolean, string]>([
    ["an operating system", ["freebsd", "x64"], false, "Rerail has no engine for freebsd on x64: it runs on macOS, Windows, and Linux with glibc, each on arm64 or x64."],
    ["a processor", ["linux", "riscv64"], false, "Rerail has no engine for Linux on riscv64: it runs on macOS, Windows, and Linux with glibc, each on arm64 or x64."],
    ["a Linux on musl", LINUX, true, "Rerail has no engine for Linux with musl (Alpine, for example): on Linux it needs glibc."],
  ])("says no engine exists for %s Rerail does not run on", (_what, system, musl, message) => {
    const { packageRoot } = installedRerail();
    expect(() => engineOf(packageRoot, system, musl)).toThrow(new Error(message));
  });

  // npm 11.6.2 and later install the glibc engine packages on musl too, and the system's loader then refuses the file.
  it("says no engine exists for a Linux on musl where the engine npm installed there does not load", () => {
    const refused = new Error("Error loading shared library ld-linux-x86-64.so.2: No such file or directory");
    expect(engineLoadFailure(refused, { platform: "linux", musl: true }, undefined)).toEqual(
      new Error("Rerail has no engine for Linux with musl (Alpine, for example): on Linux it needs glibc.", { cause: refused }),
    );
    expect(engineLoadFailure(refused, { platform: "linux", musl: false }, undefined)).toBe(refused);
    expect(engineLoadFailure(refused, { platform: "darwin", musl: true }, undefined)).toBe(refused);
    // An engine RERAIL_ENGINE_PATH names is the person's own build: its failure is theirs to read.
    expect(engineLoadFailure(refused, { platform: "linux", musl: true }, "/opt/engine/rerail_engine.node")).toBe(refused);
  });

  it("says the engine package is not installed, why npm leaves one out, and the command that installs it", () => {
    const { prefix, packageRoot } = installedRerail();
    const spec = `${nativeEnginePackageName(...LINUX)}@1.2.0`;
    expect(() => engineOf(packageRoot, LINUX)).toThrow(new Error(
      `Rerail's engine for Linux on x64 is not installed: the Rerail at ${packageRoot} has no ${spec}, an optional dependency of @neralabs/rerail. ` +
      "npm leaves optional dependencies out under --omit=optional (omit=optional or optional=false in an .npmrc), and when the registry or mirror it installs from lacks the package or serves another file for it. " +
      `Install it with \`npm install --prefix "${prefix}" ${spec}\`. ` +
      `Or set RERAIL_ENGINE_PATH to an engine file for this system (${engineFile(LINUX)}).`,
    ));
  });

  // `npm install --prefix` removes what the folder's package.json does not name: under a global install, every package.
  it("names no folder in the command where npm did not install Rerail into one as a dependency, as under a global install", () => {
    const { packageRoot } = installedRerail({});
    const spec = `${nativeEnginePackageName(...LINUX)}@1.2.0`;
    expect(() => engineOf(packageRoot, LINUX)).toThrow(
      `Install it with \`npm install ${spec}\`, run where @neralabs/rerail is installed (with \`-g\` for a global install). Or set RERAIL_ENGINE_PATH`,
    );
  });

  it("says Rerail was installed for another system where only that system's engine package is installed", () => {
    const { prefix, packageRoot } = installedRerail();
    enginePackage(prefix, ["darwin", "x64"]);
    const spec = `${nativeEnginePackageName(...MAC)}@1.2.0`;
    expect(() => engineOf(packageRoot, MAC)).toThrow(new Error(
      `The Rerail at ${packageRoot} was installed for macOS on x64, and this is macOS on arm64: it has ${nativeEnginePackageName("darwin", "x64")} and no ${spec}. ` +
      "Its folder came from another computer, or this Node is built for another processor than the Node that installed it. " +
      "Run Rerail with that Node, or add this system's engine. " +
      `Install it with \`npm install --prefix "${prefix}" --force ${spec}\`. \`--force\` has npm keep the other system's engine. ` +
      `Or set RERAIL_ENGINE_PATH to an engine file for this system (${engineFile(MAC)}).`,
    ));
  });

  // npm 11.6.2 and later install every system's engine package from core's shrinkwrap, on any system, and skip one
  // they cannot fetch or verify with exit code 0: the five that remain are no install made for another system.
  it("says the engine package is not installed where npm installed every other system's and not this one's", () => {
    const { packageRoot } = installedRerail();
    for (const system of NATIVE_ENGINE_SYSTEMS.filter(([platform, architecture]) => platform !== MAC[0] || architecture !== MAC[1])) enginePackage(packageRoot, system);
    expect(() => engineOf(packageRoot, MAC)).toThrow(`Rerail's engine for macOS on arm64 is not installed: the Rerail at ${packageRoot} has no ${nativeEnginePackageName(...MAC)}@1.2.0`);
    expect(() => engineOf(packageRoot, MAC)).not.toThrow("was installed for");
  });

  // An update loads the version it staged to check its engine, and removes the staging folder when that fails: a
  // command that installs into the folder would be given for one already gone. The updater of a version from before
  // the engine packages names no engine package, so its npm must be told to install core's optional dependencies.
  it("gives no command for a Rerail an update staged, whose folder the update removes, and says to run the update again", () => {
    const { prefix, packageRoot } = installedRerail(undefined, ".1.2.0.0b0f1d6e-5d1c-4f0e-9a43-6f7f0f3f2a10.staging");
    const spec = `${nativeEnginePackageName(...LINUX)}@1.2.0`;
    const again = "An update or set-up staged this Rerail, and removes its folder when it fails: run the update or set-up again once npm can install the package. " +
      "Where npm's settings leave optional dependencies out, set the environment variable npm_config_include=optional for that run.";
    expect(() => engineOf(packageRoot, LINUX)).toThrow(new Error(
      `Rerail's engine for Linux on x64 is not installed: the Rerail at ${packageRoot} has no ${spec}, an optional dependency of @neralabs/rerail. ` +
      "npm leaves optional dependencies out under --omit=optional (omit=optional or optional=false in an .npmrc), and when the registry or mirror it installs from lacks the package or serves another file for it. " +
      `${again} Or set RERAIL_ENGINE_PATH to an engine file for this system (${engineFile(LINUX)}).`,
    ));
    // The one other system's engine an npm on another processor's Node staged: no `--force` command either.
    enginePackage(prefix, ["linux", "arm64"]);
    expect(() => engineOf(packageRoot, LINUX)).toThrow(`Run Rerail with that Node, or add this system's engine. ${again} Or set RERAIL_ENGINE_PATH`);
  });

  it("says a checkout's engine is not built, and how to build or fetch one", () => {
    const { packageRoot } = installedRerail();
    mkdirSync(join(packageRoot, "native"));
    writeFileSync(join(packageRoot, "native", "Cargo.toml"), "");
    expect(() => engineOf(packageRoot, MAC)).toThrow(new Error(
      `Rerail's engine for macOS on arm64 is not built: ${join(packageRoot, "native")} has no ${engineFile(MAC)}. ` +
      "Build it with `npm run build:native`, or fetch a released one with `npm run engine:pull`. " +
      `Or set RERAIL_ENGINE_PATH to an engine file for this system (${engineFile(MAC)}).`,
    ));
  });
});
