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
cp -R "$lab/src" "$lab/Cargo.lock" "$out/native/lab/tape16/"
sed -e '/^rerail-rr-syntax/d' -e '/^lean = /d' -e '/^# E6: the lean/d' "$lab/Cargo.toml" > "$out/native/lab/tape16/Cargo.toml"
cp -R "$root/native/crates/tape_format" "$out/native/crates/"
rm -rf "$out/native/crates/tape_format/target"
for c in "$@"; do
    cp "$c" "$out/cap/"
done
cat > "$out/run.sh" <<'EOF'
#!/bin/sh
# Two builds: the architecture's baseline, and this CPU's own features (`target-cpu=native`: on x86 with BMI2
# the Elias–Fano select is PDEP). Each runs `tape16 inplace` on every capture, niced.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/native/lab/tape16"
host=$(hostname -s 2>/dev/null || hostname)
mkdir -p "$here/results"
for build in base native; do
    flags=""
    [ "$build" = native ] && flags="-C target-cpu=native"
    RUSTFLAGS="$flags" CARGO_TARGET_DIR="target-$build" nice -n 19 cargo build --bin tape16
    for c in "$here"/cap/*.cap; do
        name=$(basename "$c" .cap)
        nice -n 19 "target-$build/debug/tape16" inplace "$c" > "$here/results/$host-$build-$name.md" 2>&1
        echo "$host $build $name: $(grep -c 'answers wrong | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0' "$here/results/$host-$build-$name.md") all-correct row"
    done
done
EOF
chmod +x "$out/run.sh"
du -sh "$out"
