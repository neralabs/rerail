/**
 * The folders no repository walk enters, by name: version control's, Rerail's own, and dependency, cache and build
 * output. A module of its own so that set-up's file listing can take it without loading the code rule enforcer. The
 * engine reads the same list (`npm run generate:ignored-directories` writes its copy), so a `.rerail` run's corpus is
 * the files this walk reads.
 */
export const IGNORED_DIRECTORIES = new Set([
  ".git",
  ".hg",
  ".svn",
  ".rerail",
  ".venv",
  "__pycache__",
  "node_modules",
  "dist",
  "build",
  ".next",
  ".turbo",
  "coverage",
  "target",
  "vendor",
]);
