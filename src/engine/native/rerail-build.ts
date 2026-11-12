import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { loadNativeBinding } from "./native.js";
import { defaultRerailHome, VERSIONS_DIR } from "../storage/rerail-home.js";
import { PACKAGE_ROOT, RERAIL_PACKAGE_NAME } from "../package-root.js";

/**
 * Which Rerail is running. `main` is the managed installation, loaded from
 * `<home>/versions/<version>/node_modules/@neralabs/rerail` (usually through `<home>/current`); it alone owns
 * the machine-wide content store. `dev` is any build running from a Git checkout (source, `dist/`, tests,
 * benchmarks, worktrees); it keeps a content store private to that checkout.
 */
export type RerailBuild =
  | { kind: "main"; home: string; version: string; packageRoot: string }
  | { kind: "dev"; packageRoot: string; checkout: string };

/** The managed installation whose real package root is `realPackageRoot`, if it is one. */
function managedBuild(
  realPackageRoot: string,
  environment: NodeJS.ProcessEnv,
): Extract<RerailBuild, { kind: "main" }> | undefined {
  const home = defaultRerailHome(environment);
  const versionsRoot = join(home, VERSIONS_DIR);
  if (!existsSync(versionsRoot)) return undefined;
  const [version, ...packagePath] = relative(realpathSync(versionsRoot), realPackageRoot).split(sep);
  // A dot prefix is a staging directory (`.<version>.<uuid>.staging`) or a path outside `versions/`.
  if (version === undefined || version === "" || version.startsWith(".")) return undefined;
  if (packagePath.join("/") !== `node_modules/${RERAIL_PACKAGE_NAME}`) return undefined;
  return { kind: "main", home, version, packageRoot: realPackageRoot };
}

/**
 * Classifies the package at `packageRoot`. Main iff its real path is a version directory of the managed
 * Rerail home (`RERAIL_HOME`, default `~/.rerail`); otherwise dev iff it lives in a Git checkout (the nearest
 * ancestor holding `.git`, a directory or a worktree's file). Anything else has no content store of its own.
 */
export function resolveRerailBuild(packageRoot: string, environment: NodeJS.ProcessEnv = process.env): RerailBuild {
  const realPackageRoot = realpathSync(packageRoot);
  const main = managedBuild(realPackageRoot, environment);
  if (main !== undefined) return main;
  for (let directory = realPackageRoot; ; directory = dirname(directory)) {
    if (existsSync(join(directory, ".git"))) return { kind: "dev", packageRoot: realPackageRoot, checkout: directory };
    if (dirname(directory) === directory) break;
  }
  throw new Error(
    `Rerail at ${realPackageRoot} is neither the managed installation under ` +
      `${join(defaultRerailHome(environment), VERSIONS_DIR)} nor in a Git checkout, so it has no content store`,
  );
}

/**
 * The running build, or why it has none: resolved once per process, a failure as much as a build. Every code rule
 * workspace asks for it, so a failure resolved again would repeat the realpath and the walk to the root each time.
 */
let current: { build: RerailBuild } | { problem: string } | undefined;

function currentResolution(): { build: RerailBuild } | { problem: string } {
  if (current === undefined) {
    try {
      current = { build: resolveRerailBuild(PACKAGE_ROOT) };
    } catch (error) {
      current = { problem: error instanceof Error ? error.message : String(error) };
    }
  }
  return current;
}

/** The running build, resolved once per process; a package with no content store throws the same reason every time. */
export function currentRerailBuild(): RerailBuild {
  const resolution = currentResolution();
  if ("problem" in resolution) throw new Error(resolution.problem);
  return resolution.build;
}

/**
 * Why this process has no content store, so no code rule can run; undefined when it has one (its build's, or the one
 * `RERAIL_CONTENT_STORE_PATH` names). Never throws: the daemon reports it as its engine's status.
 */
export function contentStoreProblem(): string | undefined {
  if (contentStoreOverride() !== undefined) return undefined;
  const resolution = currentResolution();
  return "problem" in resolution ? resolution.problem : undefined;
}

/**
 * Whether this process runs the managed installation. Unlike `currentRerailBuild`, it never throws: a package that
 * is neither main nor in a checkout (the updater's staging health check) is simply not main.
 */
export function isMainRerailBuild(): boolean {
  return managedBuild(realpathSync(PACKAGE_ROOT), process.env) !== undefined;
}

/** The directory holding the machine-wide content store that only the main build opens. */
export function mainContentStoreDirectory(): string {
  return join(homedir(), ".cache", "rerail");
}

/** The content store `RERAIL_CONTENT_STORE_PATH` names, absolute, or undefined when it names none. */
export function contentStoreOverride(): string | undefined {
  const override = process.env.RERAIL_CONTENT_STORE_PATH;
  return override === undefined || override === "" ? undefined : resolve(override);
}

/**
 * The content store `build` (default: this process's) opens: the machine-wide one for main, `<checkout>/.rerail/`
 * for dev, so no two checkouts and no checkout and the managed installation share a store. The file name is the
 * native binary's (`content-v1.sqlite` at store floor 1), so a floor raise renames every store at once.
 * `RERAIL_CONTENT_STORE_PATH` overrides both (tests give each run its own file), and then no build is resolved.
 */
export function contentStorePath(build?: RerailBuild): string {
  const override = contentStoreOverride();
  if (override !== undefined) return override;
  const resolved = build ?? currentRerailBuild();
  const directory = resolved.kind === "main" ? mainContentStoreDirectory() : join(resolved.checkout, ".rerail");
  return join(directory, loadNativeBinding().contentStoreFileName());
}
