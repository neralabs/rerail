#!/usr/bin/env bash
#
# `just rerail-smoke`: prove the installer end to end without the real npm registry.
#
# Publishes this checkout's package to a throwaway Verdaccio on localhost that, like
# @neralabs/rerail on npm before launch, serves @neralabs/* only to a logged-in user.
# Runs the real `just rerail` installer (scripts/dev/rerail-setup.mjs) against it inside a
# sandbox HOME that carries fake coding-agent directories, first logged out (it must stop
# with a request to npm login) and then logged in, exercises the installed rerail launcher
# (status, daemon stop/start, update check, update to a newer published version, a
# just nuke-rerail -> just rerail --restore round trip, install --remove and its database
# backup), and tears everything down. The real ~/.rerail, ~/.npmrc, and coding-agent
# configs are never read or written; the package's own dependencies still come from
# npmjs. KEEP=1 leaves the sandbox for inspection.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MIN_NODE_MAJOR=24
PACKAGE="@neralabs/rerail"
VERDACCIO="verdaccio@6"

info() { printf '\033[34m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[31m!!\033[0m %s\n' "$*" >&2; exit 1; }

# Every coding-agent plugin manifest in a tarball must carry the package version stamped at pack time.
assert_plugin_versions() {
  local tarball="$1" expected="$2" harness actual
  for harness in claude codex cursor; do
    actual="$(tar -xzOf "$tarball" "package/.$harness-plugin/plugin.json" | "$NODE" -e 'let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => console.log(JSON.parse(s).version));')"
    [ "$actual" = "$expected" ] || fail "$tarball .$harness-plugin/plugin.json version is $actual, not $expected"
  done
}

free_port() {
  "$NODE" -e 'const s = require("node:net").createServer(); s.listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port)); s.close(); });'
}

NODE="$(command -v node)" || fail "node not found. Install Node.js $MIN_NODE_MAJOR or newer."
NPM="$(command -v npm)" || fail "npm not found."
NPX="$(command -v npx)" || fail "npx not found."
CURL="$(command -v curl)" || fail "curl not found."
node_major="$("$NODE" -p 'process.versions.node.split(".")[0]')"
[ "$node_major" -ge "$MIN_NODE_MAJOR" ] || fail "Node $node_major is too old; Rerail needs $MIN_NODE_MAJOR+."

# 1. A publishable build: the native engine plus dist/ (runtime, hook bundles, UI). An existing
#    complete build is packed as it is; run `npm run build` first to pack fresh sources.
if [ ! -f "$REPO_ROOT/dist/cli/cli.js" ] || ! ls "$REPO_ROOT"/native/*.node >/dev/null 2>&1; then
  info "No complete build in this checkout; running npm run build"
  (cd "$REPO_ROOT" && "$NPM" run build)
fi

# 2. Scratch space. Everything below lives here and dies with it.
SMOKE="$(mktemp -d "${TMPDIR:-/tmp}/rerail-smoke.XXXXXX")"
SANDBOX_HOME="$SMOKE/home"
PROJECT="$SANDBOX_HOME/proj"
RERAIL_DIR="$SANDBOX_HOME/.rerail"
LAUNCHER="$RERAIL_DIR/bin/rerail"
VERDACCIO_PID=""

# Sandbox commands see only the scratch HOME: coding-agent detection, ~/.npmrc, npm's cache, the
# checkout dev-daemon registry (XDG_STATE_HOME, read by nuke-rerail), and Rerail's home (RERAIL_HOME)
# and with it the one database, rerail.sqlite, all resolve there. Inherited Rerail and npm settings that
# could point the sandbox elsewhere are dropped; RERAIL_STORAGE_ROOT is pinned so no inherited value can
# reach a real project's session files, and the device keystore never touches the system keychain.
sandbox() {
  env -u RERAIL_CONTENT_STORE_PATH -u RERAIL_DEV -u RERAIL_MODEL_PATH -u RERAIL_ENGINE_SOCKET \
    -u RERAIL_ENGINE_PATH -u RERAIL_TAG -u RERAIL_REGISTRY -u RERAIL_AUTOSTART \
    -u NPM_CONFIG_REGISTRY -u npm_config_registry -u NPM_CONFIG_GLOBALCONFIG -u npm_config_globalconfig \
    HOME="$SANDBOX_HOME" RERAIL_HOME="$RERAIL_DIR" XDG_STATE_HOME="$SMOKE/state" \
    RERAIL_STORAGE_ROOT="$PROJECT/.rerail/sessions" RERAIL_DEVICE_SECRET_STORE=file \
    NPM_CONFIG_USERCONFIG="$SANDBOX_HOME/.npmrc" NPM_CONFIG_CACHE="$SMOKE/npm-cache" "$@"
}

cleanup() {
  local rc=$?
  set +e
  [ -x "$LAUNCHER" ] && sandbox "$LAUNCHER" daemon stop >/dev/null 2>&1
  if [ -n "$VERDACCIO_PID" ]; then
    # npx wraps the server in a child; the job ran in its own process group so one signal reaches both.
    kill -- "-$VERDACCIO_PID" 2>/dev/null || kill "$VERDACCIO_PID" 2>/dev/null
    wait "$VERDACCIO_PID" 2>/dev/null
  fi
  if [ "$rc" -ne 0 ] && [ "${KEEP:-0}" != "1" ]; then
    for log in "$SMOKE/verdaccio.log" "$RERAIL_DIR/daemon.log"; do
      [ -s "$log" ] && { printf '\n--- %s (last 20 lines) ---\n' "$log" >&2; tail -n 20 "$log" >&2; }
    done
    printf '\nRe-run with KEEP=1 to keep the sandbox.\n' >&2
  fi
  if [ "${KEEP:-0}" = "1" ]; then info "KEEP=1: leaving $SMOKE in place"; else rm -rf "$SMOKE"; fi
  exit "$rc"
}
trap cleanup EXIT

# 3. A local registry. The port is whatever the kernel hands out for 127.0.0.1:0. One user exists,
#    registration is closed, and @neralabs/* is readable only when logged in, as on npm before launch.
PORT="$(free_port)"
REGISTRY="http://127.0.0.1:$PORT"
REGISTRY_USER="rerail-smoke"
REGISTRY_PASSWORD="$("$NODE" -e 'process.stdout.write(require("node:crypto").randomBytes(18).toString("hex"))')"
REGISTRY_PASSWORD="$REGISTRY_PASSWORD" REGISTRY_USER="$REGISTRY_USER" "$NODE" -e '
  const sha = require("node:crypto").createHash("sha1").update(process.env.REGISTRY_PASSWORD).digest("base64");
  require("node:fs").writeFileSync(process.argv[1], `${process.env.REGISTRY_USER}:{SHA}${sha}\n`);
' "$SMOKE/htpasswd"
cat > "$SMOKE/verdaccio.yaml" <<EOF
storage: $SMOKE/storage
auth:
  htpasswd:
    file: $SMOKE/htpasswd
    max_users: -1
uplinks: {}
packages:
  "@neralabs/*":
    access: \$authenticated
    publish: \$authenticated
  "**":
    access: \$all
    publish: \$all
max_body_size: 200mb
log: { type: stdout, level: warn }
EOF

info "Starting Verdaccio on $REGISTRY"
set -m
"$NPX" --yes "$VERDACCIO" --config "$SMOKE/verdaccio.yaml" --listen "127.0.0.1:$PORT" > "$SMOKE/verdaccio.log" 2>&1 &
VERDACCIO_PID=$!
set +m
for ((attempt = 0; attempt < 120; attempt++)); do
  "$CURL" -sf "$REGISTRY/-/ping" >/dev/null 2>&1 && break
  kill -0 "$VERDACCIO_PID" 2>/dev/null || fail "Verdaccio exited before answering; see $SMOKE/verdaccio.log"
  sleep 1
done
"$CURL" -sf "$REGISTRY/-/ping" >/dev/null 2>&1 || fail "Verdaccio did not answer $REGISTRY/-/ping within 120s"

# An auth token for the user, exactly what `npm login` obtains and stores (an existing user logs in with Basic auth).
REGISTRY_TOKEN="$(REGISTRY_USER="$REGISTRY_USER" REGISTRY_PASSWORD="$REGISTRY_PASSWORD" "$NODE" --input-type=module -e '
  const { REGISTRY_USER: name, REGISTRY_PASSWORD: password } = process.env;
  const response = await fetch(`${process.argv[1]}/-/user/org.couchdb.user:${name}`, {
    method: "PUT",
    headers: { "content-type": "application/json", authorization: `Basic ${Buffer.from(`${name}:${password}`).toString("base64")}` },
    body: JSON.stringify({ name: process.env.REGISTRY_USER, password: process.env.REGISTRY_PASSWORD }),
  });
  const body = await response.json();
  if (!response.ok || typeof body.token !== "string") throw new Error(`login failed: ${response.status} ${JSON.stringify(body)}`);
  process.stdout.write(body.token);
' "$REGISTRY")"
# The publisher's own throwaway userconfig; the sandbox .npmrc is the installing user's.
printf '//127.0.0.1:%s/:_authToken=%s\n' "$PORT" "$REGISTRY_TOKEN" > "$SMOKE/npmrc"

# 4. Publish this checkout's package there. The tarball is exactly what the release
#    workflow would ship: package.json "files" decides what goes in.
info "Packing and publishing $PACKAGE to $REGISTRY"
manifests_before="$(cd "$REPO_ROOT" && cat .claude-plugin/plugin.json .codex-plugin/plugin.json .cursor-plugin/plugin.json)"
# --json keeps the prepack/postpack banners out of the filename npm reports on stdout.
TARBALL="$(cd "$REPO_ROOT" && "$NPM" pack --json --pack-destination "$SMOKE" 2>/dev/null \
  | "$NODE" -e 'let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => console.log(JSON.parse(s)[0].filename));')"
[ -f "$SMOKE/$TARBALL" ] || fail "npm pack produced no tarball"
[ "$(cd "$REPO_ROOT" && cat .claude-plugin/plugin.json .codex-plugin/plugin.json .cursor-plugin/plugin.json)" = "$manifests_before" ] \
  || fail "npm pack left the plugin manifests stamped"
FIRST_VERSION="$("$NODE" -p 'require(process.argv[1]).version' "$REPO_ROOT/package.json")"
assert_plugin_versions "$SMOKE/$TARBALL" "$FIRST_VERSION"
# A prerelease package.json version needs an explicit dist-tag. The first release is on latest (the
# updater's channel) and next (which the installer is asked for, to exercise --tag).
"$NPM" publish "$SMOKE/$TARBALL" --tag latest --registry "$REGISTRY" --userconfig "$SMOKE/npmrc" --loglevel=warn >/dev/null
"$NPM" dist-tag add "$PACKAGE@$FIRST_VERSION" next --registry "$REGISTRY" --userconfig "$SMOKE/npmrc" --loglevel=warn >/dev/null

# 5. A sandbox HOME: every coding agent Rerail registers with looks present. Its .npmrc starts empty,
#    a machine whose npm is not logged in.
mkdir -p "$SANDBOX_HOME/.omp/agent" "$SANDBOX_HOME/.pi/agent" "$SANDBOX_HOME/.claude" "$SANDBOX_HOME/.codex" "$SANDBOX_HOME/.cursor" \
  "$PROJECT" "$SMOKE/npm-cache" "$SMOKE/state"
# A real pre-existing user status must survive registration and removal.
printf '%s\n' '{"statusLine":{"type":"command","command":"printf user-status","padding":3}}' > "$SANDBOX_HOME/.claude/settings.json"
: > "$SANDBOX_HOME/.npmrc"
cd "$PROJECT"

info "just rerail while npm is not logged in stops with a request to npm login"
if sandbox env RERAIL_REGISTRY="$REGISTRY/" "$NODE" "$REPO_ROOT/scripts/rerail-setup.mjs" --tag next </dev/null >"$SMOKE/logged-out.log" 2>&1; then
  fail "just rerail installed $PACKAGE without an npm login"
fi
cat "$SMOKE/logged-out.log"
grep -qF 'Run `npm login`' "$SMOKE/logged-out.log" || fail "the logged-out installer did not ask for npm login"
[ ! -e "$RERAIL_DIR" ] || fail "the logged-out installer created $RERAIL_DIR"

# `npm login --scope=@neralabs --registry=$REGISTRY` writes exactly these two lines. A global registry
# would send the package's own dependencies to the throwaway registry, which has no uplink.
printf '@neralabs:registry=%s/\n//127.0.0.1:%s/:_authToken=%s\n' "$REGISTRY" "$PORT" "$REGISTRY_TOKEN" > "$SANDBOX_HOME/.npmrc"

# Setup starts the daemon from ~/.rerail/current the way it does on a clean machine, but on a
# sandbox-only port: the default port belongs to whatever real Rerail is running here. cwd is
# $PROJECT, inside the sandbox HOME, so its repository identity and session files stay in the sandbox.
DAEMON_PORT="$(free_port)"
info "Running the just rerail installer (--tag next) against the local registry (daemon on port $DAEMON_PORT)"
sandbox env RERAIL_REGISTRY="$REGISTRY/" \
  "$NODE" "$REPO_ROOT/scripts/rerail-setup.mjs" --tag next --project "$PROJECT" --port "$DAEMON_PORT" </dev/null \
  | tee "$SMOKE/setup.log"
grep -qF "$PACKAGE@next is $FIRST_VERSION" "$SMOKE/setup.log" || fail "the installer did not resolve $PACKAGE@next to $FIRST_VERSION"

[ -L "$RERAIL_DIR/current" ] || fail "setup did not create $RERAIL_DIR/current"
[ -x "$LAUNCHER" ] || fail "setup did not write the launcher $LAUNCHER"
CLAUDE_COMMAND="$SANDBOX_HOME/.claude/commands/rerail.md"
[ -f "$CLAUDE_COMMAND" ] || fail "setup did not create $CLAUDE_COMMAND"

grep -qF "Daemon started at http://127.0.0.1:$DAEMON_PORT" "$SMOKE/setup.log" || fail "setup did not start the daemon on port $DAEMON_PORT"
info "rerail status (setup-started daemon)"
status_output="$(sandbox "$LAUNCHER" status)"
printf '%s\n' "$status_output"
case "$status_output" in *"running at http://127.0.0.1:$DAEMON_PORT"*) ;; *) fail "rerail status does not show the setup-started daemon running" ;; esac
case "$status_output" in *"Native kernel: ok"*) ;; *) fail "the setup-started daemon has no working native kernel" ;; esac
case "$status_output" in *"Database: "*"${SMOKE##*/}/home/.rerail/rerail.sqlite"*) ;; *) fail "the setup-started daemon reads a database outside the sandbox Rerail home" ;; esac
info "rerail daemon stop (setup-started daemon)"
sandbox "$LAUNCHER" daemon stop

info "rerail daemon start --port $DAEMON_PORT"
sandbox "$LAUNCHER" daemon start --port "$DAEMON_PORT"

info "rerail status"
sandbox "$LAUNCHER" status

info "Rerail is off until started: the hook does nothing and the HUD says not on here"
sandbox "$NODE" -e 'process.stdout.write(JSON.stringify({hook_event_name:"SessionStart",session_id:"installer-smoke-off",cwd:process.cwd(),source:"startup"}))' \
  | sandbox "$NODE" "$RERAIL_DIR/current/dist/hosts/claude/claude-hook.js" >/dev/null
status_output="$(sandbox "$NODE" -e 'process.stdout.write(JSON.stringify({session_id:"installer-smoke-off",cwd:process.cwd()}))' \
  | sandbox "$NODE" "$RERAIL_DIR/current/dist/hosts/claude/claude-status.js")"
case "$status_output" in *"not on here"*) ;; *) fail "the HUD in a repository that was never started does not say not on here" ;; esac

info "rerail start"
start_output="$(sandbox "$LAUNCHER" start)"
printf '%s\n' "$start_output"
case "$start_output" in *"Rerail is on in proj."*" rules are on."*) ;; *) fail "rerail start did not set up and start the project" ;; esac

info "Exercising the installed Claude hook, composed HUD, and exact-session command"
sandbox "$NODE" -e 'process.stdout.write(JSON.stringify({hook_event_name:"SessionStart",session_id:"installer-smoke",cwd:process.cwd(),source:"startup"}))' \
  | sandbox "$NODE" "$RERAIL_DIR/current/dist/hosts/claude/claude-hook.js" >/dev/null
status_output="$(sandbox "$NODE" -e 'process.stdout.write(JSON.stringify({session_id:"installer-smoke",cwd:process.cwd()}))' \
  | sandbox "$NODE" "$RERAIL_DIR/current/dist/hosts/claude/claude-status.js")"
case "$status_output" in
  *user-status*" rules"*"steered 0×"*) ;;
  *) fail "installed HUD did not preserve the user status and render actual session counters" ;;
esac
session_output="$(sandbox "$LAUNCHER" session claude-code installer-smoke)"
case "$session_output" in
  *"sessionId=installer-smoke&host=claude-code#sessions"*) ;;
  *) fail "installed session command did not navigate to the originating session" ;;
esac

info "rerail update check"
update_output="$(sandbox "$LAUNCHER" update check)"
printf '%s\n' "$update_output"
case "$update_output" in *"Update available: no"*) ;; *) fail "update check did not report 'Update available: no'" ;; esac

# A newer release on latest: the same tarball with its patch version bumped.
NEXT_VERSION="$("$NODE" -p 'const [a, b, c] = process.argv[1].split("-")[0].split(".").map(Number); `${a}.${b}.${c + 1}`' "$FIRST_VERSION")"
info "Publishing $PACKAGE@$NEXT_VERSION to $REGISTRY"
mkdir -p "$SMOKE/next"
tar -xzf "$SMOKE/$TARBALL" -C "$SMOKE/next"
"$NODE" -e 'const fs = require("node:fs"); const p = process.argv[1]; const pkg = JSON.parse(fs.readFileSync(p, "utf8")); pkg.version = process.argv[2]; fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + "\n");' \
  "$SMOKE/next/package/package.json" "$NEXT_VERSION"
"$NODE" "$REPO_ROOT/scripts/stamp-plugin-versions.mjs" stamp "$SMOKE/next/package"
COPYFILE_DISABLE=1 tar -czf "$SMOKE/next.tgz" -C "$SMOKE/next" package
assert_plugin_versions "$SMOKE/next.tgz" "$NEXT_VERSION"
"$NPM" publish "$SMOKE/next.tgz" --registry "$REGISTRY" --userconfig "$SMOKE/npmrc" --loglevel=warn >/dev/null

info "rerail update check (expects $NEXT_VERSION)"
update_output="$(sandbox "$LAUNCHER" update check)"
printf '%s\n' "$update_output"
case "$update_output" in *"Available: $NEXT_VERSION"*"Update available: yes"*) ;; *) fail "update check did not offer $NEXT_VERSION" ;; esac

info "rerail update"
update_output="$(sandbox "$LAUNCHER" update)"
printf '%s\n' "$update_output"
case "$update_output" in *"Activated Rerail $NEXT_VERSION"*) ;; *) fail "rerail update did not activate $NEXT_VERSION" ;; esac
status_output="$(sandbox "$LAUNCHER" update status)"
printf '%s\n' "$status_output"
case "$status_output" in *"Current: $NEXT_VERSION"*"Previous: $FIRST_VERSION"*) ;; *) fail "update status does not show $NEXT_VERSION current over $FIRST_VERSION" ;; esac
active_version="$(sandbox "$NODE" -p 'require(process.argv[1]).version' "$RERAIL_DIR/current/package.json")"
[ "$active_version" = "$NEXT_VERSION" ] || fail "$RERAIL_DIR/current is $active_version, not $NEXT_VERSION"

# A just nuke-rerail -> just rerail --restore round trip: the sandbox machine loses every Rerail trace into a
# dated backup, reinstalls from the registry, and gets its database back.
MOCK_RULE="mock-cosine-prefer-generated-client"
DATABASE="$RERAIL_DIR/rerail.sqlite"
rule_count() {
  sandbox "$NODE" --no-warnings -e '
    const { DatabaseSync } = require("node:sqlite");
    const database = new DatabaseSync(process.argv[1], { readOnly: true });
    const all = database.prepare("SELECT count(*) AS n FROM library_rules").get().n;
    const mock = database.prepare("SELECT count(*) AS n FROM library_rules WHERE rule_id = ?").get(process.argv[2]).n;
    console.log(`${all} ${mock}`);
  ' "$DATABASE" "$MOCK_RULE"
}
info "rerail seed (a rule the round trip must keep)"
sandbox "$LAUNCHER" seed >/dev/null
read -r RULES_BEFORE MOCK_BEFORE <<<"$(rule_count)"
[ "$MOCK_BEFORE" -gt 0 ] || fail "rerail seed did not write $MOCK_RULE to $DATABASE"

info "just nuke-rerail --dry-run changes nothing"
sandbox "$NODE" "$REPO_ROOT/scripts/rerail-nuke.mjs" --dry-run </dev/null | tee "$SMOKE/nuke-dry-run.log"
[ -x "$LAUNCHER" ] || fail "nuke-rerail --dry-run removed $LAUNCHER"
if compgen -G "$SANDBOX_HOME/.rerail-clean-*" >/dev/null; then fail "nuke-rerail --dry-run created a backup"; fi
if grep -qF "Stop the dev daemon of" "$SMOKE/nuke-dry-run.log"; then fail "the sandboxed nuke-rerail planned to stop a real checkout's dev daemon"; fi

info "just nuke-rerail --yes"
sandbox "$NODE" "$REPO_ROOT/scripts/rerail-nuke.mjs" --yes </dev/null | tee "$SMOKE/nuke.log"
[ ! -e "$RERAIL_DIR" ] || fail "nuke-rerail left $RERAIL_DIR behind"
[ ! -e "$CLAUDE_COMMAND" ] || fail "nuke-rerail left $CLAUDE_COMMAND behind"
[ ! -e "$SANDBOX_HOME/.omp/agent/extensions/rerail" ] || fail "nuke-rerail left the OMP extension link behind"
backups=("$SANDBOX_HOME"/.rerail-clean-*)
BACKUP="${backups[${#backups[@]}-1]}"
for backed_up in rerail-home/rerail.sqlite rerail-home/harness.sqlite configs/.claude_settings.json claude-commands/rerail.md; do
  [ -e "$BACKUP/$backed_up" ] || fail "nuke-rerail did not back up $backed_up in $BACKUP"
done
sandbox "$NODE" -e '
  const fs = require("node:fs"), assert = require("node:assert/strict"), home = process.env.HOME;
  const settings = JSON.parse(fs.readFileSync(`${home}/.claude/settings.json`, "utf8"));
  assert.deepEqual(settings.statusLine, { type: "command", command: "printf user-status", padding: 3 });
  for (const file of [".claude/settings.json", ".codex/hooks.json", ".cursor/hooks.json", ".omp/agent/mcp.json", ".pi/agent/settings.json"]) {
    const path = `${home}/${file}`;
    if (fs.existsSync(path)) assert.ok(!fs.readFileSync(path, "utf8").includes("/.rerail/"), `${file} still names the Rerail home`);
  }'

info "just rerail --restore"
sandbox env RERAIL_REGISTRY="$REGISTRY/" \
  "$NODE" "$REPO_ROOT/scripts/rerail-setup.mjs" --restore --project "$PROJECT" --port "$DAEMON_PORT" </dev/null \
  | tee "$SMOKE/restore.log"
grep -qF "Daemon healthy at http://127.0.0.1:$DAEMON_PORT" "$SMOKE/restore.log" || fail "the restored daemon is not healthy on port $DAEMON_PORT"
[ -f "$CLAUDE_COMMAND" ] || fail "just rerail --restore did not reinstall $CLAUDE_COMMAND"
read -r RULES_AFTER MOCK_AFTER <<<"$(rule_count)"
[ "$MOCK_AFTER" -gt 0 ] || fail "$MOCK_RULE did not survive nuke-rerail -> rerail --restore"
[ "$RULES_AFTER" -ge "$RULES_BEFORE" ] || fail "rerail --restore brought back $RULES_AFTER catalog rules, not $RULES_BEFORE"
status_output="$(sandbox "$LAUNCHER" status)"
case "$status_output" in *"running at http://127.0.0.1:$DAEMON_PORT"*) ;; *) fail "rerail status does not show the restored daemon running" ;; esac

info "rerail install --remove"
backups_before=$(compgen -G "$SANDBOX_HOME/.rerail-clean-*" | wc -l)
sandbox "$LAUNCHER" install --remove
if [ -f "$CLAUDE_COMMAND" ]; then fail "install --remove left $CLAUDE_COMMAND behind"; fi
sandbox "$NODE" -e 'const fs=require("node:fs"), assert=require("node:assert/strict"); assert.deepEqual(JSON.parse(fs.readFileSync(process.env.HOME+"/.claude/settings.json","utf8")).statusLine,{type:"command",command:"printf user-status",padding:3});'
[ ! -e "$DATABASE" ] || fail "install --remove left $DATABASE in place"
backups_after=$(compgen -G "$SANDBOX_HOME/.rerail-clean-*" | wc -l)
[ "$backups_after" -gt "$backups_before" ] || fail "install --remove did not back up the database into a new .rerail-clean-* directory"
compgen -G "$SANDBOX_HOME/.rerail-clean-*/rerail-home/rerail.sqlite" >/dev/null || fail "install --remove backed up no rerail.sqlite"

info "rerail-smoke: PASS"
