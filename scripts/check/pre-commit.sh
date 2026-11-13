#!/bin/sh
# The checks of ci.yml's "Lint and type-check" job, run before each commit: the Justfile's format, the release notes
# this branch adds against the local `origin/main` (CI fetches the base first; this never touches the network), and
# `npm run check` with the UI token and DSL reference checks. They read the working tree, not only what is staged.
# `scripts/dev/install-git-hooks.mjs` installs it; `git commit --no-verify` skips it.
set -eu
cd "$(git rev-parse --show-toplevel)"

if command -v just >/dev/null 2>&1; then
  just --fmt --check --unstable
else
  echo "pre-commit: just is not installed, so the Justfile's format is not checked" >&2
fi
if git rev-parse --verify --quiet origin/main >/dev/null; then
  node scripts/release/core-changelog-check.mjs origin/main
else
  echo "pre-commit: there is no origin/main, so the release notes are not checked" >&2
fi
npm run --silent check
npm run --silent check:ui-tokens
npm run --silent check:reference
