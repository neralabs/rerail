#!/bin/sh
# The checks of ci.yml's "Lint and type-check" job, run before each commit: the Justfile's format, the release notes
# this branch adds against the local `origin/main` (CI fetches the base first; this never touches the network), and
# `npm run check` with the UI token and DSL reference checks. They read the working tree, not only what is staged.
# CI runs them on the pull request merged into main, so a branch behind the local `origin/main` is checked as that
# merge, in a throwaway worktree (scripts/check/merged-checkout.mjs); a merge that would conflict, or that Git cannot
# make, is said, and the branch is checked alone. `scripts/dev/install-git-hooks.mjs` installs it; `git commit
# --no-verify` skips it.
set -eu
root="$(git rev-parse --show-toplevel)"
cd "$root"

# The throwaway worktree's folder is made here, so a Ctrl-C at any point leaves the clean-up something to remove.
scratch=""
cleanup() {
  [ -z "$scratch" ] || { cd "$root" && node scripts/check/merged-checkout.mjs remove "$scratch/checkout"; }
}
trap cleanup EXIT
trap 'exit 130' INT TERM
tmp="${TMPDIR:-/tmp}"
scratch="$(mktemp -d "${tmp%/}/rerail-pre-commit-XXXXXX")"
checkout="$(node scripts/check/merged-checkout.mjs prepare --scratch "$scratch")"
# The index and repository Git hands the hook belong to this checkout, not the throwaway one.
if [ "$checkout" != "$root" ]; then
  unset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE GIT_PREFIX
fi
cd "$checkout"

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
# `npm run check` and `check:reference` are off on this branch (the user, 2026-11-17): they fail on what the branch
# carried before the hook was installed, not on a commit's change. `check` fails `check:structure` (15 of the
# repository-layout scorecard's targets) and `check:internal-refs` (476 `nera-` names outside the public list);
# `check:reference` finds 3 productions where the DSL reference and the zod schema differ (8 languages in
# `codeRuleDefineSchema` and `codeRuleQuerySchema.ast`, and `codeRuleParseSchema`'s `filePath`, not in zod).
# Run them by hand: `npm run check`, `npm run check:reference`.
npm run --silent check:ui-tokens
# The engine's `test-hooks` feature carries what its dev-dependencies turn on, so the CLI and the tests build
# their dependencies once (native/Cargo.toml).
python3 scripts/check/check-dev-features.py
# Blocks are never copied into another data structure (decision 0353): a commit that adds a copy is refused.
python3 scripts/check/check-no-block-copies.py
