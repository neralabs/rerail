#!/usr/bin/env bash
#
# Runs on a throwaway Azure Linux VM, or a GitHub-hosted Linux or macOS runner, for scripts/host/azure/matrix.mjs,
# which prepends the RERAIL_ENV_* settings and sends this file through Azure Run Command, or runs it with sudo on the
# runner (`matrix.mjs local`: the matrix's macOS environments, and the release workflow's Linux and macOS installs).
# As root it provisions what a user brings (Node from nodejs.org; Git from the distribution, or the Command Line Tools
# on a Mac) and the package under test, then runs itself again as the machine's ordinary user through a login shell,
# the way a person opens a terminal. That run installs Rerail with `npx @neralabs/rerail setup`, answers set-up's
# questions, and checks the managed runtime: launcher, daemon and native engine, PATH in new shells, the Claude Code
# and Codex hooks as those agents spawn them, daemon restart, and removal. It uploads results.json and a log to the
# blob URLs it was given, or leaves them in $WORK when it was given none; the driver judges them against the
# environment's expectation. Bash 3.2, which is macOS's /bin/bash.
set -uo pipefail

: "${RERAIL_ENV_ID:?}" "${RERAIL_ENV_USER:?}" "${RERAIL_ENV_NODE:?}"
RERAIL_ENV_RESULT_URL="${RERAIL_ENV_RESULT_URL:-}"
RERAIL_ENV_LOG_URL="${RERAIL_ENV_LOG_URL:-}"
WORK=/opt/rerail-env

upload() { # <file> <sas url>; nothing without a URL
  [ -n "$2" ] || return 0
  curl -fsS --retry 5 --retry-all-errors -X PUT -H 'x-ms-blob-type: BlockBlob' --data-binary "@$1" "$2" >/dev/null
}

# A download, retried: a new VM's first minutes on the network see failures a person would simply retry.
fetch() { # <url> [curl options...]
  curl -fsS --retry 5 --retry-all-errors --retry-delay 5 "$@"
}

# macOS has no `timeout`; perl's alarm ends the command the same way (the exit code then reports SIGALRM).
command -v timeout >/dev/null || timeout() { perl -e 'alarm shift; exec @ARGV' "$@"; }

# A Mac as a developer has it: a zsh account (the runner image's account uses bash), Git from the Command Line Tools
# (which the image carries), and Node from nodejs.org's installer, which puts it in /usr/local/bin.
provision_macos() {
  dscl . -create "/Users/$RERAIL_ENV_USER" UserShell /bin/zsh
  git --version
  local version
  fetch "https://nodejs.org/dist/latest-v${RERAIL_ENV_NODE}.x/SHASUMS256.txt" -o "$WORK/SHASUMS256.txt"
  version="$(sed -n "s/.*node-\(v[0-9.]*\)\.pkg$/\1/p" "$WORK/SHASUMS256.txt")"
  [ -n "$version" ] || { echo "no Node ${RERAIL_ENV_NODE}.x installer for macOS" >&2; exit 1; }
  fetch "https://nodejs.org/dist/${version}/node-${version}.pkg" -o "$WORK/node.pkg"
  installer -pkg "$WORK/node.pkg" -target /
}

provision() {
  set -e
  mkdir -p "$WORK"
  cp "$0" "$WORK/guest.sh"
  chmod 0755 "$WORK" "$WORK/guest.sh"
  exec >"$WORK/provision.log" 2>&1
  if [ "$(uname -s)" = Darwin ]; then
    provision_macos
  else
    provision_linux
  fi
  if [ -n "${RERAIL_ENV_BUNDLE_URL:-}" ]; then
    fetch "$RERAIL_ENV_BUNDLE_URL" -o "$WORK/bundle.tar"
    mkdir -p "$WORK/bundle"
    tar -xf "$WORK/bundle.tar" -C "$WORK/bundle"
    chmod -R a+rX "$WORK/bundle"
  fi
}

provision_linux() {
  if command -v apt-get >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    # On first boot unattended-upgrades holds the dpkg lock for minutes; wait for it rather than fail.
    apt-get -o DPkg::Lock::Timeout=600 update -q
    apt-get -o DPkg::Lock::Timeout=600 install -y -q git curl ca-certificates xz-utils
  elif command -v dnf >/dev/null; then
    dnf install -y -q git curl tar xz
  else
    echo "no supported package manager" >&2
    exit 1
  fi
  # RHEL's Azure image gives /home its own 1 GB logical volume, too small for an install (a version is ~300 MB, plus
  # npm's cache); a developer's machine has room. Grow it from the volume group's free space when it is that small.
  local home_device
  home_device="$(findmnt -n -o SOURCE /home || true)"
  if [ -n "$home_device" ] && command -v lvextend >/dev/null && [ "$(df --output=avail -k /home | tail -n 1)" -lt 4000000 ]; then
    lvextend --resizefs -L +8G "$home_device"
  fi
  local arch version
  case "$(uname -m)" in
    x86_64) arch=x64 ;;
    aarch64) arch=arm64 ;;
    *) echo "unsupported machine $(uname -m)" >&2; exit 1 ;;
  esac
  # The newest release of the requested major, as a user downloading Node from nodejs.org gets it. Every download goes
  # to a file first: a retried download piped onwards would hand the reader the failed attempt's bytes too.
  fetch "https://nodejs.org/dist/latest-v${RERAIL_ENV_NODE}.x/SHASUMS256.txt" -o "$WORK/SHASUMS256.txt"
  version="$(sed -n "s/.*node-\(v[0-9.]*\)-linux-${arch}\.tar\.xz$/\1/p" "$WORK/SHASUMS256.txt")"
  [ -n "$version" ] || { echo "no Node ${RERAIL_ENV_NODE}.x for linux-${arch}" >&2; exit 1; }
  fetch "https://nodejs.org/dist/${version}/node-${version}-linux-${arch}.tar.xz" -o "$WORK/node.tar.xz"
  # A Node already in /usr/local (a GitHub runner's) leaves its npm's files behind under the new one's, and the mix fails
  # with "Class extends value undefined"; a fresh VM has none to remove.
  rm -rf /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/corepack
  tar -xJf "$WORK/node.tar.xz" -C /usr/local --strip-components=1
}

if [ "$(id -u)" = 0 ] && [ "${1:-}" != test ]; then
  started=$SECONDS
  if ! (provision); then
    printf '{"env":"%s","provisioned":false,"steps":[],"facts":{}}\n' "$RERAIL_ENV_ID" >"$WORK/results.json"
    upload "$WORK/results.json" "$RERAIL_ENV_RESULT_URL"
    upload "$WORK/provision.log" "$RERAIL_ENV_LOG_URL"
    echo "provisioning failed after $((SECONDS - started)) s; see the uploaded log"
    exit 1
  fi
  # A login shell as the user: HOME, SHELL and PATH come from the account, as in a new terminal.
  su - "$RERAIL_ENV_USER" -c "bash $WORK/guest.sh test"
  status=$?
  user_out="$(su - "$RERAIL_ENV_USER" -c 'printf %s "$HOME"')/rerail-env"
  cat "$WORK/provision.log" "$user_out/test.log" >"$WORK/all.log" 2>/dev/null
  cp "$user_out/results.json" "$WORK/results.json" 2>/dev/null
  upload "$WORK/all.log" "$RERAIL_ENV_LOG_URL"
  exit "$status"
fi

# ---- As the user ------------------------------------------------------------------------------------------------
OUT="$HOME/rerail-env"
LOG="$OUT/test.log"
PROJECT="$HOME/proj"
LAUNCHER="$HOME/.rerail/bin/rerail"
mkdir -p "$OUT"
: >"$LOG"
export BROWSER=none NPM_CONFIG_UPDATE_NOTIFIER=false NO_COLOR=1
steps="$OUT/steps.jsonl"
: >"$steps"
facts="$OUT/facts.json"
setup_ok=0
daemon_port=""

json() { node -e 'process.stdout.write(JSON.stringify(process.argv[1]))' "$1"; }

# step <name> <function>: runs the function in this shell (so it can set state for later steps), records ok and
# seconds, and keeps the tail of what it printed.
step() {
  local name="$1" fn="$2" rc start=$SECONDS detail
  printf '\n==== %s\n' "$name" >>"$LOG"
  "$fn" </dev/null >"$OUT/step.out" 2>&1
  rc=$?
  cat "$OUT/step.out" >>"$LOG"
  detail="$(tail -c 1500 "$OUT/step.out")"
  printf '{"name":"%s","ok":%s,"seconds":%s,"detail":%s}\n' "$name" "$([ "$rc" = 0 ] && echo true || echo false)" \
    "$((SECONDS - start))" "$(json "$detail")" >>"$steps"
  return "$rc"
}

skip() { printf '{"name":"%s","ok":null,"seconds":0,"detail":"skipped: set-up did not finish"}\n' "$1" >>"$steps"; }

collect_facts() {
  node -e '
    const { execSync } = require("node:child_process");
    const run = (cmd) => { try { return execSync(cmd, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim(); } catch (e) { return `unavailable (${(e.stderr || e.message).trim().split("\n")[0]})`; } };
    const linux = process.platform === "linux";
    const os = linux
      ? Object.fromEntries(require("node:fs").readFileSync("/etc/os-release", "utf8").split("\n").filter(Boolean).map((l) => { const [k, ...v] = l.split("="); return [k, v.join("=").replace(/^"|"$/g, "")]; })).PRETTY_NAME
      : `macOS ${run("sw_vers -productVersion")}`;
    process.stdout.write(JSON.stringify({ os, arch: process.arch, ...(linux ? { glibc: run("ldd --version").split("\n")[0] } : {}), node: process.version, nodePath: process.execPath, npm: run("npm --version"), git: run("git --version"), shell: process.env.SHELL }));
  ' >"$facts"
}

# Packages under test (core and every rule pack) from a registry on this VM, as npmjs will serve them after launch;
# everything else still comes from npmjs.
start_registry() {
  local port password token
  port="$(node -e 'const s=require("net").createServer();s.listen(0,"127.0.0.1",()=>{process.stdout.write(String(s.address().port));s.close()})')"
  REGISTRY="http://127.0.0.1:$port"
  mkdir -p "$OUT/verdaccio"
  cat >"$OUT/verdaccio/config.yaml" <<EOF
storage: $OUT/verdaccio/storage
auth:
  htpasswd:
    file: $OUT/verdaccio/htpasswd
    max_users: 1
uplinks:
  npmjs:
    url: https://registry.npmjs.org/
packages:
  "@neralabs/*":
    access: \$all
    publish: \$authenticated
  "**":
    access: \$all
    proxy: npmjs
max_body_size: 200mb
log: { type: stdout, level: warn }
EOF
  # Its own session where there is setsid (Linux); macOS has none, and a background job is enough there.
  local detach=""
  command -v setsid >/dev/null && detach=setsid
  $detach npx --yes verdaccio@6 --config "$OUT/verdaccio/config.yaml" --listen "127.0.0.1:$port" >"$OUT/verdaccio.log" 2>&1 &
  for _ in $(seq 1 180); do curl -fsS "$REGISTRY/-/ping" >/dev/null 2>&1 && break; sleep 1; done
  curl -fsS "$REGISTRY/-/ping" >/dev/null || { cat "$OUT/verdaccio.log"; return 1; }
  password="$(node -p 'require("crypto").randomBytes(18).toString("hex")')"
  token="$(curl -fsS -X PUT -H 'content-type: application/json' \
    -d "{\"name\":\"publisher\",\"password\":\"$password\"}" "$REGISTRY/-/user/org.couchdb.user:publisher" \
    | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).token))')"
  printf '//127.0.0.1:%s/:_authToken=%s\n' "$port" "$token" >"$OUT/publisher.npmrc"
  local tarball
  for tarball in "$WORK"/bundle/*.tgz; do
    npm publish "$tarball" --tag latest --registry "$REGISTRY" --userconfig "$OUT/publisher.npmrc" --loglevel=warn || return 1
  done
  printf '@neralabs:registry=%s/\n' "$REGISTRY" >"$HOME/.npmrc"
  echo "registry $REGISTRY serves: $(ls "$WORK/bundle" | tr '\n' ' ')"
}

use_npm() {
  [ -z "${RERAIL_ENV_NPM_TOKEN:-}" ] || printf '//registry.npmjs.org/:_authToken=%s\n' "$RERAIL_ENV_NPM_TOKEN" >"$HOME/.npmrc"
  echo "installing @neralabs/rerail@$RERAIL_ENV_NPM_SPEC from npmjs"
}

make_project() {
  mkdir -p "$PROJECT/src" "$HOME/.claude" "$HOME/.codex" "$HOME/.cursor"
  cd "$PROJECT" || return 1
  git init -q
  printf '{ "name": "proj", "private": true, "dependencies": { "react": "^19.0.0" }, "devDependencies": { "typescript": "^5.0.0" } }\n' >package.json
  printf 'export const answer = 42;\n' >src/index.ts
  git add -A
  git -c user.name=rerail -c user.email=rerail@example.invalid commit -qm init
  echo "project at $PROJECT; coding-agent folders: .claude .codex .cursor"
}

# Set-up's machine-readable lane: each step is a JSON object whose options carry the exact launcher arguments for
# each answer. Answer as a person accepting the defaults would: allow sessions, read this repo, start with the packs
# shown. The first run is npx (it installs); later answers go to the installed launcher.
run_setup() {
  cd "$PROJECT" || return 1
  local spec="${RERAIL_ENV_NPM_SPEC:-latest}" step_json next i
  step_json="$(timeout 900 npx --yes "@neralabs/rerail@$spec" setup --json --repo "$PROJECT")"
  for i in 1 2 3 4 5 6; do
    printf '%s\n' "$step_json" >"$OUT/setup-$i.json"
    next="$(node -e '
      const step = JSON.parse(process.argv[1]);
      console.error(`step ${step.step}: ${step.screen}`);
      if (step.step === "done") { process.stdout.write("DONE\n" + step.screen); process.exit(0); }
      if (step.question === null) { process.stdout.write("STOP\n" + step.screen); process.exit(0); }
      const want = { sessions: "allow", "read-repo": "yes", "confirm-packs": "start" }[step.question.id];
      const option = step.question.options.find((o) => o.answer === want);
      if (option === undefined) { process.stdout.write(`STOP\nno "${want}" answer for question ${step.question.id}`); process.exit(0); }
      process.stdout.write("NEXT\n" + JSON.stringify(option.next.args));
    ' "$step_json")" || { echo "set-up printed no JSON step: $step_json"; return 1; }
    case "$next" in
      DONE*) printf '%s\n' "${next#DONE$'\n'}"; grep -q "Rerail is on" <<<"$next" || return 1; setup_ok=1; return 0 ;;
      STOP*) printf '%s\n' "${next#STOP$'\n'}"; return 1 ;;
    esac
    local -a args=()
    local arg
    while IFS= read -r arg; do args+=("$arg"); done < <(node -e 'for (const a of JSON.parse(process.argv[1])) console.log(a)' "${next#NEXT$'\n'}")
    step_json="$(timeout 900 "$LAUNCHER" "${args[@]}")"
  done
  echo "set-up did not finish in 6 answers"
  return 1
}

check_status() {
  local status
  status="$("$LAUNCHER" status)"
  printf '%s\n' "$status"
  [ -L "$HOME/.rerail/current" ] || { echo "$HOME/.rerail/current is not a symlink"; return 1; }
  grep -q "Engine: ok" <<<"$status" || return 1
  daemon_port="$(sed -n 's#.*running at http://127\.0\.0\.1:\([0-9]*\)/.*#\1#p' <<<"$status" | head -n 1)"
  [ -n "$daemon_port" ] || { echo "no daemon URL in rerail status"; return 1; }
  echo "$daemon_port" >"$OUT/daemon-port"
}

# A new terminal's shell, in the account's own shell: bash on the Linux VMs, zsh on a Mac. Startup files may print
# (Ubuntu's sudo hint), so only the last line is the answer. Terminal.app opens a login shell that is interactive too;
# a login zsh that is not interactive never reads .zshrc, which set-up's documentation already says of `zsh -c`.
check_path_login() {
  local found shell flags=-lc
  shell="$(basename "$SHELL")"
  [ "$shell" = zsh ] && flags=-lic
  found="$("$SHELL" "$flags" 'command -v rerail' 2>/dev/null | tail -n 1)"
  [ "$found" = "$LAUNCHER" ] || { echo "a login $shell finds '${found}', not $LAUNCHER"; return 1; }
  echo "login $shell finds $found"
}
check_path_interactive() {
  local found shell
  shell="$(basename "$SHELL")"
  found="$("$SHELL" -ic 'command -v rerail' 2>/dev/null | tail -n 1)"
  [ "$found" = "$LAUNCHER" ] || { echo "an interactive $shell finds '${found}', not $LAUNCHER"; return 1; }
  echo "interactive $shell finds $found"
}

# Runs each agent's registered SessionStart command the way that agent does (through a shell, payload on stdin),
# then asks the daemon which sessions it recorded: an exit code alone proves nothing, since a hook exits 0 where
# Rerail is off too.
check_hooks() {
  cd "$PROJECT" || return 1
  local agent command
  for agent in claude codex; do
    case "$agent" in
      claude) command="$(node -p 'require(process.argv[1]).hooks.SessionStart[0].hooks[0].command' "$HOME/.claude/settings.json")" ;;
      codex) command="$(node -p 'require(process.argv[1]).hooks.SessionStart[0].hooks[0].command' "$HOME/.codex/hooks.json")" ;;
    esac
    printf '{"session_id":"env-%s","cwd":"%s","hook_event_name":"SessionStart","source":"startup"}' "$agent" "$PROJECT" \
      | timeout 60 sh -c "$command" >"$OUT/hook-$agent.out" 2>&1
    echo "$agent hook exit $?: $(head -c 300 "$OUT/hook-$agent.out")"
  done
  grep -q rerail "$HOME/.cursor/hooks.json" || { echo "Cursor hooks were not registered"; return 1; }
  node -e '
    fetch(`http://127.0.0.1:${process.argv[1]}/api/sessions`).then((r) => r.json()).then(({ sessions }) => {
      const ids = sessions.map((s) => s.sessionId);
      const missing = ["env-claude", "env-codex"].filter((id) => !ids.includes(id));
      console.log(`daemon recorded: ${ids.join(", ")}`);
      process.exitCode = missing.length === 0 ? 0 : 1;
    });
  ' "$daemon_port"
}

check_daemon_restart() {
  timeout 60 "$LAUNCHER" daemon stop || { echo "rerail daemon stop did not return within 60 s"; return 1; }
  if "$LAUNCHER" status >/dev/null 2>&1; then echo "status still reports a running daemon after stop"; return 1; fi
  "$LAUNCHER" daemon start || return 1
  "$LAUNCHER" status | grep "Engine: ok"
}

check_uninstall() {
  "$LAUNCHER" install --remove || return 1
  # No agent config may still point into the Rerail home, or the agent would run a deleted launcher.
  local file
  for file in "$HOME/.claude/settings.json" "$HOME/.codex/hooks.json" "$HOME/.cursor/hooks.json"; do
    if [ -f "$file" ] && grep -q '/\.rerail/' "$file"; then echo "Rerail hooks left in $file"; return 1; fi
  done
  for file in "$HOME/.bashrc" "$HOME/.profile" "$HOME/.bash_profile" "$HOME/.zshrc" "$HOME/.zprofile" "$HOME/.zshenv"; do
    if [ -f "$file" ] && grep -q '\.rerail/bin' "$file"; then echo "PATH block left in $file"; return 1; fi
  done
  echo "Claude Code, Codex and Cursor unhooked, and the PATH block removed"
}

collect_facts
if [ -n "${RERAIL_ENV_BUNDLE_URL:-}" ]; then step registry start_registry; else step registry use_npm; fi
step project make_project
step setup run_setup
if [ "$setup_ok" = 1 ]; then
  step status check_status
  step path-login-shell check_path_login
  step path-interactive-shell check_path_interactive
  step hooks check_hooks
  step daemon-restart check_daemon_restart
  step uninstall check_uninstall
else
  for name in status path-login-shell path-interactive-shell hooks daemon-restart uninstall; do skip "$name"; done
fi
[ -x "$LAUNCHER" ] && timeout 60 "$LAUNCHER" daemon stop >/dev/null 2>&1
pkill -u "$(id -u)" -f verdaccio >/dev/null 2>&1

node -e '
  const fs = require("node:fs");
  const [id, factsFile, stepsFile, out] = process.argv.slice(1);
  const steps = fs.readFileSync(stepsFile, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
  fs.writeFileSync(out, JSON.stringify({ env: id, provisioned: true, facts: JSON.parse(fs.readFileSync(factsFile, "utf8")), steps }, null, 2));
' "$RERAIL_ENV_ID" "$facts" "$steps" "$OUT/results.json"
upload "$OUT/results.json" "$RERAIL_ENV_RESULT_URL"
echo "results written"
