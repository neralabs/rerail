#!/bin/sh
# E10/E11 on another machine (t38 "Still to measure": M1, M3, x86): packs the lab, the one engine crate it
# needs, the toolchain pin and the named captures into one directory with its own run.sh. Copy that directory
# to the machine (it needs rustup and a C compiler), then run `./run.sh` there; results land in results/.
#
#   scripts/bench-pack.sh <out-dir> <capture>...
#
# The captures hold source text (capture.rs), so a pack carries the code of the repositories captured.
set -eu
[ $# -ge 2 ] || { echo "usage: $0 <out-dir> <capture>..." >&2; exit 2; }
lab=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$lab/../../.." && pwd)
out=$1
shift
[ ! -e "$out" ] || { echo "$out exists" >&2; exit 1; }
mkdir -p "$out/native/lab" "$out/native/crates" "$out/cap"
cp "$root/rust-toolchain.toml" "$out/"
# The lab without its build outputs; `lean` (the engine's .rr parser) is left out, so its path need not exist.
mkdir "$out/native/lab/tape16"
cp -R "$lab/src" "$lab/Cargo.lock" "$lab/kinds.tsv" "$out/native/lab/tape16/"
sed -e '/^rerail-rr-syntax/d' -e '/^lean = /d' -e '/^# E6: the lean/d' "$lab/Cargo.toml" > "$out/native/lab/tape16/Cargo.toml"
cp -R "$root/native/crates/tape_format" "$out/native/crates/"
rm -rf "$out/native/crates/tape_format/target"
for c in "$@"; do
    cp "$c" "$out/cap/"
done
cat > "$out/run.sh" <<'EOF'
#!/bin/sh
# Two builds: the architecture's baseline, and this CPU's own features (`target-cpu=native`: on x86 with BMI2
# the Elias–Fano select is PDEP). Each runs `tape16 <command>` (default `inplace`; `prim` for the primitives)
# on every capture, niced; `--native` runs the native build only.
set -eu
cmd=${1:-inplace}
builds="base native"
[ "${2:-}" = --native ] && builds=native
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/native/lab/tape16"
# Results are named by the CPU model (never the hostname); T16_LABEL overrides it, for two machines of one model.
if [ -z "${T16_LABEL:-}" ]; then
    if [ "$(uname -s)" = Darwin ]; then T16_LABEL=$(sysctl -n machdep.cpu.brand_string)
    else T16_LABEL=$(sed -n 's/^model name[^:]*: *//p' /proc/cpuinfo | head -n 1); fi
fi
host=$(printf '%s' "${T16_LABEL:-cpu}" | tr -cs 'A-Za-z0-9' '-' | sed 's/^-//; s/-$//')
mkdir -p "$here/results"
for build in $builds; do
    # The base build is the farm's, shared with every worktree; the native one differs by design (the
    # flag is what is measured), so it says so (`RERAIL_FARM_ANY_ENV`, scripts/build/farm/README.md).
    if [ "$build" = native ]; then
        RERAIL_FARM_ANY_ENV=1 RUSTFLAGS="-C target-cpu=native" nice -n 19 cargo build --target-dir "target-$build" --bin tape16
    else
        nice -n 19 cargo build --target-dir "target-$build" --bin tape16
    fi
    for c in "$here"/cap/*.cap; do
        name=$(basename "$c" .cap)
        res="$here/results/$host-$cmd-$build-$name.md"
        nice -n 19 "target-$build/debug/tape16" "$cmd" "$c" > "$res" 2>&1
        echo "$host $cmd $build $name: $(grep '^| answers wrong' "$res")"
    done
done
# On a Mac, the questions as Metal kernels too (one build is enough: the kernels are the GPU's code).
if [ "$(uname -s)" = Darwin ] && [ "$cmd" = inplace ]; then
    for c in "$here"/cap/*.cap; do
        name=$(basename "$c" .cap)
        nice -n 19 target-native/debug/tape16 inplace-gpu "$c" > "$here/results/$host-metal-$name.md" 2>&1
        echo "$host metal $name: $(grep -c WRONG "$here/results/$host-metal-$name.md") wrong"
    done
fi
EOF
chmod +x "$out/run.sh"
du -sh "$out"
