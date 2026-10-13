import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { loadNativeBinding } from "./native.js";
import { defaultRerailHome } from "../storage/rerail-home.js";
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
  const versionsRoot = join(home, "versions");
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
      `${join(defaultRerailHome(environment), "versions")} nor in a Git checkout, so it has no content store`,
  );
}

let current: RerailBuild | undefined;

/** The running build, resolved once per process. */
export function currentRerailBuild(): RerailBuild {
  current ??= resolveRerailBuild(PACKAGE_ROOT);
  return current;
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

/**
 * The content store `build` (default: this process's) opens: the machine-wide one for main, `<checkout>/.rerail/`
 * for dev, so no two checkouts and no checkout and the managed installation share a store. The file name is the
 * native binary's (`content-v1.sqlite` at store floor 1), so a floor raise renames every store at once.
 * `RERAIL_CONTENT_STORE_PATH` overrides both (tests give each run its own file), and then no build is resolved.
 */
export function contentStorePath(build?: RerailBuild): string {
  const override = process.env.RERAIL_CONTENT_STORE_PATH;
  if (override !== undefined && override !== "") return resolve(override);
  const resolved = build ?? currentRerailBuild();
  const directory = resolved.kind === "main" ? mainContentStoreDirectory() : join(resolved.checkout, ".rerail");
  return join(directory, loadNativeBinding().contentStoreFileName());
}
