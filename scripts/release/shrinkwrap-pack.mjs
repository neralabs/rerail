#!/usr/bin/env node
// Writes, removes, or checks what packing adds to core's tarball: the npm-shrinkwrap.json it ships (shrinkwrap.mjs) and
// its pins on the engine packages (engine/packages.mjs). npm runs `write` as prepack and `remove` as postpack, so the
// tarball carries both while the checkout never does. A pack that fails after prepack skips postpack and leaves them
// behind: npm would then prefer the shrinkwrap to package-lock.json, and package.json would name packages no registry
// has yet. `node scripts/release/shrinkwrap-pack.mjs remove` (idempotent) cleans both up.
//
// RERAIL_ENGINE_PACKAGES names the folder `engine/packages.mjs pack` wrote. `write` pins every engine package its
// manifest lists: an exact-version optional dependency in package.json, and an entry in the shrinkwrap. A release sets
// it to a folder holding all six. A checkout that packs without it gets a tarball with no pin, whose install has no
// engine (a development flow then points RERAIL_ENGINE_PATH at one); packed against a folder holding some of the six,
// a tarball whose install has an engine on those platforms only. `write` says so in both cases.
//
// release.yml runs `check` on the tarball before publishing. It fails the release unless the tarball ships the
// shrinkwrap package-lock.json gives and pins all six engine packages, each as the tarball in the folder was packed,
// so neither tarball above can be published.
//
//   node scripts/release/shrinkwrap-pack.mjs write [package-root]
//   node scripts/release/shrinkwrap-pack.mjs remove [package-root]
//   node scripts/release/shrinkwrap-pack.mjs check <tarball> [package-root]
import { execFileSync } from "node:child_process";
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { absentEnginePackages, ENGINE_PACKAGES, ENGINE_PACKAGES_ENV, engineTarballsProblem, readEngineManifest, withEnginePins, withoutEnginePins } from "./engine/packages.mjs";
import { packedProblem, SHRINKWRAP_FILE, shrinkwrapText } from "./shrinkwrap.mjs";

/**
 * The text of `file` packed in `tarball`, or undefined when it has none. tar runs in the tarball's folder and gets its
 * bare name: GNU tar, first on PATH in Git Bash on Windows, reads the `C:` of an absolute path as a remote host and
 * finds no archive, and the tars of Windows and macOS have no `--force-local` to turn that off.
 */
function packedFile(tarball, file) {
  const path = resolve(tarball);
  try {
    return execFileSync("tar", ["-xzOf", basename(path), `package/${file}`], {
      cwd: dirname(path),
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    return undefined;
  }
}

/** The folder RERAIL_ENGINE_PACKAGES names and its manifest, or undefined when it names none. */
function packedEngines() {
  const named = process.env[ENGINE_PACKAGES_ENV] ?? "";
  if (named === "") return undefined;
  const folder = resolve(named);
  return { folder, manifest: readEngineManifest(folder) };
}

/** What a tarball packed with `engines` lacks for a release, for whoever packs a checkout; undefined when nothing. */
function developmentNote(engines) {
  const refused = "and the release's check refuses it";
  if (engines === undefined) return `Packing core with no engine package (${ENGINE_PACKAGES_ENV} names no folder): its install has no engine, ${refused}.`;
  const absent = absentEnginePackages(engines.manifest);
  if (absent.length === 0) return undefined;
  return `Packing core with ${engines.manifest.packages.length} of ${ENGINE_PACKAGES.length} engine packages (${engines.folder} lacks ${absent.map(({ suffix }) => suffix).join(", ")}): its install has no engine on those platforms, ${refused}.`;
}

/** Writes `root`'s shrinkwrap and stamps its package.json, both from the engine packages the environment names. */
function write(root) {
  const path = join(root, "package.json");
  const packageJson = JSON.parse(readFileSync(path, "utf8"));
  const engines = packedEngines();
  if (engines !== undefined && engines.manifest.version !== packageJson.version) {
    throw new Error(`${engines.folder} holds the engine packages of ${engines.manifest.version}, and package.json is ${packageJson.version}: pack them again`);
  }
  // Both texts before either file: a lockfile or manifest that refuses leaves the checkout as it was.
  const shrinkwrap = shrinkwrapText(root, engines?.manifest);
  const stamped = engines === undefined ? withoutEnginePins(packageJson) : withEnginePins(packageJson, engines.manifest);
  writeFileSync(join(root, SHRINKWRAP_FILE), shrinkwrap);
  if (stamped !== packageJson) writeFileSync(path, `${JSON.stringify(stamped, null, 2)}\n`);
  const note = developmentNote(engines);
  if (note !== undefined) process.stderr.write(`${note}\n`);
}

/** Removes `root`'s shrinkwrap and the engine pins from its package.json. */
function remove(root) {
  rmSync(join(root, SHRINKWRAP_FILE), { force: true });
  const path = join(root, "package.json");
  const packageJson = JSON.parse(readFileSync(path, "utf8"));
  const restored = withoutEnginePins(packageJson);
  if (restored !== packageJson) writeFileSync(path, `${JSON.stringify(restored, null, 2)}\n`);
}

/**
 * Why `tarball` is not a release's core, packed from `root` against the engine packages the environment names, or
 * undefined. A folder with no manifest, or one `pack` did not write, is a reason like any other.
 */
function check(tarball, root) {
  let engines;
  try {
    engines = packedEngines();
  } catch (error) {
    return error.message;
  }
  if (engines === undefined) return `${ENGINE_PACKAGES_ENV} names no folder: the check needs the engine packages core was packed against (node scripts/release/engine/packages.mjs pack --out <folder> --all)`;
  const packed = { shrinkwrap: packedFile(tarball, SHRINKWRAP_FILE), packageJson: packedFile(tarball, "package.json") };
  return engineTarballsProblem(engines.folder, engines.manifest) ?? packedProblem(packed, shrinkwrapText(root, engines.manifest), engines.manifest);
}

const [mode, ...rest] = process.argv.slice(2);
if (mode === "write") {
  write(resolve(rest[0] ?? process.cwd()));
} else if (mode === "remove") {
  remove(resolve(rest[0] ?? process.cwd()));
} else if (mode === "check" && rest[0] !== undefined) {
  const [tarball, root = process.cwd()] = rest;
  const problem = check(tarball, resolve(root));
  if (problem !== undefined) {
    process.stderr.write(`${tarball}: ${problem}\n`);
    process.exit(1);
  }
  console.log(`${tarball} ships the ${SHRINKWRAP_FILE} package-lock.json gives, and pins the ${ENGINE_PACKAGES.length} engine packages in ${resolve(process.env[ENGINE_PACKAGES_ENV])}.`);
} else {
  process.stderr.write("usage: shrinkwrap-pack.mjs write|remove [package-root] | check <tarball> [package-root]\n");
  process.exit(2);
}
