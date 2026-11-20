# Source me: the C and C++ toolchain for `cargo … --target wasm32-wasip1` (wasm/README.md).
# Needs Homebrew's llvm, lld, wasi-libc and wasi-runtimes, or a wasi-sdk named by WASI_SDK_PATH (Linux), and
# `rustup target add wasm32-wasip1 --toolchain 1.95.0`.
# SQLite, tree-sitter's grammars and zstd are C; tokenizers' suffix arrays (esaxx) are C++. Homebrew keeps libc
# and libc++ in two sysroots, so this joins them once under target/wasi-sysroot; wasi-sdk keeps both in one.
_root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)
if [ -n "${WASI_SDK_PATH:-}" ]; then
  _llvm=$WASI_SDK_PATH/bin
  _sys=$WASI_SDK_PATH/share/wasi-sysroot
  # wasi-sdk keeps the libc++ built for wasm exceptions in a folder of its own, beside its unwinder.
  _eh="-L native=$_sys/lib/wasm32-wasip1/eh -l static=unwind"
else
  _eh=
  _brew=$(brew --prefix)
  _llvm=$_brew/opt/llvm/bin
  _sys=$_root/target/wasi-sysroot
  if [ ! -d "$_sys/include/c++" ]; then
    mkdir -p "$_sys/include" "$_sys/lib"
    cp -R "$_brew/opt/wasi-libc/share/wasi-sysroot/include/wasm32-wasip1" "$_sys/include/"
    cp -R "$_brew/opt/wasi-libc/share/wasi-sysroot/lib/wasm32-wasip1" "$_sys/lib/"
    cp -R "$_brew/opt/wasi-runtimes/share/wasi-sysroot/include/wasm32-wasip1/c++" "$_sys/include/wasm32-wasip1/"
    cp -R "$_brew/opt/wasi-runtimes/share/wasi-sysroot/include/c++" "$_sys/include/"
    cp "$_brew/opt/wasi-runtimes/share/wasi-sysroot/lib/wasm32-wasip1/"*.a "$_sys/lib/wasm32-wasip1/"
  fi
fi
export CC_wasm32_wasip1=$_llvm/clang
export CXX_wasm32_wasip1=$_llvm/clang++
export AR_wasm32_wasip1=$_llvm/llvm-ar
export CFLAGS_wasm32_wasip1="--target=wasm32-wasip1 --sysroot=$_sys"
export CXXFLAGS_wasm32_wasip1="--target=wasm32-wasip1 --sysroot=$_sys -fwasm-exceptions"
export WASI_SYSROOT=$_sys
# Link the C and C++ objects' own libc (the sysroot's wasi-libc, newer than the copy Rust bundles: its TLS start
# `__wasi_init_tp` is not in Rust's), and libc++ for esaxx.
export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="-C link-self-contained=no -C link-arg=$_sys/lib/wasm32-wasip1/crt1-command.o -L native=$_sys/lib/wasm32-wasip1 -l static=c++ -l static=c++abi${_eh:+ $_eh}"
# SQLite without the extensions the engine never uses (its SQL has no FTS3 or R-tree table; FTS5, JSON and
# dbstat stay). `-U` after the bundled build's `-D`s; the engine uses neither on any target, so a host build in
# this shell loses nothing.
export LIBSQLITE3_FLAGS="-USQLITE_ENABLE_FTS3 -USQLITE_ENABLE_FTS3_PARENTHESIS -USQLITE_ENABLE_RTREE"
