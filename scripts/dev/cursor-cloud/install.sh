#!/usr/bin/env bash
# The `install` step of a Cursor cloud test repository's .cursor/environment.json (docs/cursor-cloud-agents-prd.md,
# "Pre-merge test plan"). Run from the repository's root. It sets Rerail up in one command, as a customer's platform
# engineer would, then writes the Cursor cloud hooks with `rerail start --cursor-cloud`, so the Build Cursor captures
# already holds them when an agent boots. The environment's `start` runs `~/.rerail/bin/rerail start --cursor-cloud`.
#
#   RERAIL_PACKS=complexity@0.1.9 scripts/dev/cursor-cloud/install.sh
#
# RERAIL_PACKS     the packs to start with, as `--packs` takes them (required).
# RERAIL_PR_BUNDLE a folder of the tarballs of a build under test: env-matrix.yml's `rerail-package` artifact, core and
#                  at least this machine's engine package. Served from a registry on this VM for set-up only, as
#                  scripts/host/azure/guest.sh does. Without it, Rerail comes from npmjs.
# RERAIL_AGENTS    the coding agents set-up connects (default: cursor).
#
# Safe to run again: Cursor runs `install` for every Build, on the disk state an earlier one left.
set -euo pipefail

: "${RERAIL_PACKS:?set RERAIL_PACKS to the packs to start with, e.g. complexity@0.1.9}"
REPO="$(pwd)"
WORK="$(mktemp -d)"
REGISTRY_PID=""
NPMRC_LINE=""

cleanup() {
  # Builds keep disk state only, so a registry left running or named in ~/.npmrc would be gone, or worse, half there,
  # when an agent boots: every later npm call then fails.
  if [ -n "$REGISTRY_PID" ]; then kill -- "-$REGISTRY_PID" 2>/dev/null || kill "$REGISTRY_PID" 2>/dev/null || true; fi
  if [ -n "$NPMRC_LINE" ] && [ -f "$HOME/.npmrc" ]; then
    grep -vxF "$NPMRC_LINE" "$HOME/.npmrc" >"$WORK/npmrc" || true
    cp "$WORK/npmrc" "$HOME/.npmrc"
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# npmjs marks a version whose tarball ships an npm-shrinkwrap.json with `_hasShrinkwrap: true`, and npm reads a
# dependency's shrinkwrap only when its registry marks it so; Verdaccio stores no such mark (guest.sh, mark_shrinkwrap).
mark_shrinkwrap() { # <tarball> <storage>
  tar -tzf "$1" package/npm-shrinkwrap.json >/dev/null 2>&1 || return 0
  tar -xzOf "$1" package/package.json | node -e '
    const fs = require("node:fs");
    let text = "";
    process.stdin.on("data", (d) => text += d).on("end", () => {
      const { name, version } = JSON.parse(text);
      const file = `${process.argv[1]}/${name}/package.json`;
      const doc = JSON.parse(fs.readFileSync(file, "utf8"));
      doc.versions[version]._hasShrinkwrap = true;
      fs.writeFileSync(file, JSON.stringify(doc, null, 2));
    });
  ' "$2"
}

# The build under test from a registry on this VM; rule packs and everything else still come from npmjs.
serve_bundle() { # <bundle folder>
  local port registry token
  port="$(node -e 'const s=require("net").createServer();s.listen(0,"127.0.0.1",()=>{process.stdout.write(String(s.address().port));s.close()})')"
  registry="http://127.0.0.1:$port"
  mkdir -p "$WORK/verdaccio"
  cat >"$WORK/verdaccio/config.yaml" <<EOF
storage: $WORK/verdaccio/storage
auth:
  htpasswd:
    file: $WORK/verdaccio/htpasswd
    max_users: 1
uplinks:
  npmjs:
    url: https://registry.npmjs.org/
packages:
  "@neralabs/rerail-pack-*":
    access: \$all
    proxy: npmjs
  "@neralabs/*":
    access: \$all
    publish: \$authenticated
  "**":
    access: \$all
    proxy: npmjs
max_body_size: 1gb
log: { type: stdout, level: warn }
EOF
  # Its own session, so one signal stops npx and the server it starts (cleanup).
  local detach=""
  command -v setsid >/dev/null && detach=setsid
  $detach npx --yes verdaccio@6 --config "$WORK/verdaccio/config.yaml" --listen "127.0.0.1:$port" >"$WORK/verdaccio.log" 2>&1 &
  REGISTRY_PID=$!
  for _ in $(seq 1 180); do curl -fsS "$registry/-/ping" >/dev/null 2>&1 && break; sleep 1; done
  curl -fsS "$registry/-/ping" >/dev/null || { cat "$WORK/verdaccio.log"; return 1; }
  token="$(curl -fsS -X PUT -H 'content-type: application/json' \
    -d "{\"name\":\"publisher\",\"password\":\"$(node -p 'require("crypto").randomBytes(18).toString("hex")')\"}" \
    "$registry/-/user/org.couchdb.user:publisher" \
    | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).token))')"
  printf '//127.0.0.1:%s/:_authToken=%s\n' "$port" "$token" >"$WORK/publisher.npmrc"
  local tarball
  for tarball in "$1"/*.tgz; do
    npm publish "$tarball" --tag latest --registry "$registry" --userconfig "$WORK/publisher.npmrc" --loglevel=warn
    mark_shrinkwrap "$tarball" "$WORK/verdaccio/storage"
  done
  NPMRC_LINE="@neralabs:registry=$registry/"
  printf '%s\n' "$NPMRC_LINE" >>"$HOME/.npmrc"
  echo "serving the build under test from $registry: $(cd "$1" && ls ./*.tgz | tr '\n' ' ')"
}

if [ -n "${RERAIL_PR_BUNDLE:-}" ]; then serve_bundle "$(cd "$RERAIL_PR_BUNDLE" && pwd)"; fi

npx -y @neralabs/rerail@latest setup --agents "${RERAIL_AGENTS:-cursor}" --repo "$REPO" --allow-sessions \
  --answer pick-myself --packs "$RERAIL_PACKS"
"$HOME/.rerail/bin/rerail" start --cursor-cloud --repo "$REPO"
