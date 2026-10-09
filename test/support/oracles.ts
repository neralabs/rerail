import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

/**
 * The conformance fixtures directory: `RERAIL_FIXTURES_DIR` if set (relative to the repository root),
 * else the repository's `fixtures/`.
 */
export function rerailOraclesDirectory(): URL {
  const configured = process.env.RERAIL_FIXTURES_DIR;
  const directory = resolve(repositoryRoot, configured || "fixtures");
  if (!isCheckout(directory)) {
    throw new Error(
      configured
        ? `RERAIL_FIXTURES_DIR=${configured} is not a fixtures directory (resolved to ${directory})`
        : `Rerail fixtures not found in ${directory}: check out the repository's fixtures/ directory, or set RERAIL_FIXTURES_DIR`,
    );
  }
  return pathToFileURL(`${directory}/`);
}

/** A directory with something in it: a sparse checkout can leave `fixtures/` empty. */
function isCheckout(directory: string): boolean {
  try {
    return readdirSync(directory).length > 0;
  } catch {
    return false;
  }
}
