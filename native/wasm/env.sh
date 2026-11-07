# Source me: the C and C++ toolchain for `cargo … --target wasm32-wasip1` (wasm/README.md).
# Needs Homebrew's llvm, lld, wasi-libc and wasi-runtimes, and `rustup target add wasm32-wasip1 --toolchain 1.95.0`.
# SQLite, tree-sitter's grammars and zstd are C; tokenizers' suffix arrays (esaxx) are C++. Homebrew keeps libc
# and libc++ in two sysroots, so this joins them once under target/wasi-sysroot.
_root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)
_brew=$(brew --prefix)
_sys=$_root/target/wasi-sysroot
if [ ! -d "$_sys/include/c++" ]; then
  mkdir -p "$_sys/include" "$_sys/lib"
  cp -R "$_brew/opt/wasi-libc/share/wasi-sysroot/include/wasm32-wasip1" "$_sys/include/"
  cp -R "$_brew/opt/wasi-libc/share/wasi-sysroot/lib/wasm32-wasip1" "$_sys/lib/"
  cp -R "$_brew/opt/wasi-runtimes/share/wasi-sysroot/include/wasm32-wasip1/c++" "$_sys/include/wasm32-wasip1/"
  cp -R "$_brew/opt/wasi-runtimes/share/wasi-sysroot/include/c++" "$_sys/include/"
  cp "$_brew/opt/wasi-runtimes/share/wasi-sysroot/lib/wasm32-wasip1/"*.a "$_sys/lib/wasm32-wasip1/"
fi
export CC_wasm32_wasip1=$_brew/opt/llvm/bin/clang
export CXX_wasm32_wasip1=$_brew/opt/llvm/bin/clang++
export AR_wasm32_wasip1=$_brew/opt/llvm/bin/llvm-ar
export CFLAGS_wasm32_wasip1="--target=wasm32-wasip1 --sysroot=$_sys"
export CXXFLAGS_wasm32_wasip1="--target=wasm32-wasip1 --sysroot=$_sys -fwasm-exceptions"
export WASI_SYSROOT=$_sys
