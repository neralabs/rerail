import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

/** A path in the repository, from its root: test data outside `fixtures/` (packs' upstream recordings, rules). */
export function repositoryPath(...segments: string[]): string {
  return join(repositoryRoot, ...segments);
}

/**
 * A path in the fixtures directory, the one place TypeScript resolves it (Rust's is `native/build.rs`):
 * `RERAIL_FIXTURES_DIR` if set (relative to the repository root), else the repository's `fixtures/`.
 */
export function fixturesPath(...segments: string[]): string {
  const configured = process.env.RERAIL_FIXTURES_DIR;
  const directory = resolve(repositoryRoot, configured || "fixtures");
  if (!isCheckout(directory)) {
    throw new Error(
      configured
        ? `RERAIL_FIXTURES_DIR=${configured} is not a fixtures directory (resolved to ${directory})`
        : `Rerail fixtures not found in ${directory}: check out the repository's fixtures/ directory, or set RERAIL_FIXTURES_DIR`,
    );
  }
  return join(directory, ...segments);
}

/** A directory with something in it: a sparse checkout can leave `fixtures/` empty. */
function isCheckout(directory: string): boolean {
  try {
    return readdirSync(directory).length > 0;
  } catch {
    return false;
  }
}
