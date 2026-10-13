# Operational commands for working on Rerail. Every recipe wraps a script under scripts/ (or playground/ and
# conformance/), so `npm` users can run the same thing without `just`.

# List the recipes.
default:
    @just --list --unsorted

# Install Rerail on this machine from npm and run its set-up (`npx @neralabs/rerail setup`); arguments go to `rerail setup`; --restore [dir] then restores a nuke-rerail backup's databases.
rerail *args:
    @node scripts/dev/rerail-setup.mjs {{ args }}

# Remove Rerail (npm install and dev bridge) from this machine, backing everything up to ~/.rerail-clean-YYYYMMDD[-HHMMSS], and stop this machine's checkout dev daemons (their state is kept; just rerail-dev restarts them): --dry-run | --yes.
nuke-rerail *args:
    @node scripts/dev/rerail-nuke.mjs {{ args }}

# Build Rerail's native rules engine for this checkout (installs rustup and the pinned toolchain if needed).
rerail-native:
    @bash scripts/dev/rerail-native.sh

# Resolve this checkout's dev identity, home and port without starting the daemon.
rerail-setup:
    @node scripts/dev/rerail-dev.mjs setup

# Build and (re)start this checkout's isolated dev daemon; --install-bridge registers this checkout's build with every coding agent; --restore [dir] restores a nuke-rerail backup's database into it.
rerail-dev *args:
    @node scripts/dev/rerail-dev.mjs up {{ args }}

# Run the development rule playground beside this checkout's dev daemon (attended macOS only).
rerail-playground *args:
    @node playground/scripts/playground-dev.mjs {{ args }}

# Delete this repository's session history, across all its worktrees, from this checkout's dev database; rules are kept.
rerail-reset:
    @node scripts/dev/rerail-dev.mjs reset

# Stop this checkout's dev daemon while preserving its state.
rerail-down:
    @node scripts/dev/rerail-dev.mjs down

# Show this checkout's dev daemon, home and endpoint.
rerail-status:
    @node scripts/dev/rerail-dev.mjs status

# Run the rerail CLI against this checkout's dev home.
[private]
rerail-command *args:
    @node scripts/dev/rerail-dev.mjs command {{ args }}

# Prove the installer end to end against a throwaway local registry (no real ~/.rerail or ~/.npmrc).
rerail-smoke:
    @bash scripts/dev/rerail-smoke.sh

# Language-pack rule conformance: --pack <name> [--rule <id>] [--fixtures] [--corpus <name>] [--write] | --check.
rerail-language-conformance *args:
    @node --import tsx conformance/languages/cli.ts {{ args }}

# Compare a reference native query-engine addon with this checkout's addon (RERAIL_QUERY_PARITY_REFERENCE_ADDON=/absolute/reference.node, or pass both sides).
rerail-query-parity *args:
    @node --import tsx benchmark/query-parity.ts {{ args }}
