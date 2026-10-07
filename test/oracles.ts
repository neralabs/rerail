import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

/**
 * The conformance fixtures checkout (`rerail-fixtures`): `RERAIL_FIXTURES_DIR` if set (relative to
 * the repository root), else the repository's `fixtures/` submodule.
 */
export function rerailOraclesDirectory(): URL {
  const configured = process.env.RERAIL_FIXTURES_DIR;
  const directory = resolve(repositoryRoot, configured || "fixtures");
  if (!isCheckout(directory)) {
    throw new Error(
      configured
        ? `RERAIL_FIXTURES_DIR=${configured} is not a rerail-fixtures checkout (resolved to ${directory})`
        : `Rerail fixtures not found in ${directory}: run \`git submodule update --init\`, or set RERAIL_FIXTURES_DIR`,
    );
  }
  return pathToFileURL(`${directory}/`);
}

/** A directory with something in it: an uninitialised submodule is an empty directory. */
function isCheckout(directory: string): boolean {
  try {
    return readdirSync(directory).length > 0;
  } catch {
    return false;
  }
}
