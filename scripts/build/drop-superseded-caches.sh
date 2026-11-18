#!/usr/bin/env bash
# Deletes this ref's Actions cache entries whose key starts with $PREFIX, except $KEEP: the cargo targets an engine
# build in ci.yml saved before the one it just saved. Each engine change saves a new target (a gigabyte or so), the
# repository's cache holds 10 GB, and an older target is never restored again once a newer one exists for the ref, so
# leaving them would evict the caches other jobs and branches still read. Deletes nothing unless $KEEP was saved (a
# save that could not reserve its key only warns). Needs GH_TOKEN with actions: write.
set -euo pipefail
: "${PREFIX:?}" "${KEEP:?}" "${GH_TOKEN:?}"
entries="$(gh cache list --repo "$GITHUB_REPOSITORY" --ref "$GITHUB_REF" --key "$PREFIX" --limit 100 --json id,key \
  --jq '.[] | "\(.id) \(.key)"')"
if ! grep -qxF "$KEEP" <<< "$(cut -d' ' -f2- <<< "$entries")"; then
  echo "$KEEP is not in the cache; keeping the older targets"
  exit 0
fi
while read -r id key; do
  [ -z "$id" ] || [ "$key" = "$KEEP" ] && continue
  echo "Deleting $key"
  gh cache delete "$id" --repo "$GITHUB_REPOSITORY"
done <<< "$entries"
