#!/usr/bin/env bash
set -euo pipefail

home=$(mktemp -d)
trap 'rm -rf "$home"' EXIT

env -u RERAIL_CONTENT_STORE_PATH HOME="$home" npx vitest run test/semantic-acceptance.test.ts -t 'A16' >/dev/null
test ! -e "$home/.cache/nera"
