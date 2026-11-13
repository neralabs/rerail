/**
 * The repository's files, for the readers whose linter takes a config per folder (Ruff's nearest config, oxlint's
 * nearest `.oxlintrc.json`, SQLFluff's nested `.sqlfluff`, an eslintrc cascade, Biome's nested configs): a config
 * below the root decides its folder otherwise than the root's. The listing is set-up's pack matcher's
 * (`listRepositoryFiles` in `rule-pack-matcher.ts`): Git's tracked and untracked-but-not-ignored files, or its bounded
 * walk outside Git, leaving out the same folders.
 */
import { listRepositoryFiles } from "../rule-pack-matcher.js";

/** The listing kept while `withRepositoryFiles` runs, so every reader of one reading lists the repository once. */
let kept: { readonly root: string; readonly files: readonly string[] } | undefined;
let keeping = false;

/** The files under `root`, repository-relative with `/`; listed once per root inside `withRepositoryFiles`, afresh outside it. */
export function repositoryFiles(root: string): readonly string[] {
  if (keeping && kept?.root === root) return kept.files;
  const { files } = listRepositoryFiles(root);
  if (keeping) kept = { root, files };
  return files;
}

/** Runs `read`, `repositoryFiles` listing each root once for all of it; the listing is dropped when it returns. */
export function withRepositoryFiles<T>(read: () => T): T {
  if (keeping) return read();
  keeping = true;
  try {
    return read();
  } finally {
    keeping = false;
    kept = undefined;
  }
}
