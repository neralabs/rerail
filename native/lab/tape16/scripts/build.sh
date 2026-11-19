#!/bin/sh
# The lab's build, niced, with the Homebrew rustc workaround (its LLVM 23 upgrade broke it; 22.1.8 and z3 4.16.0 work).
cd "$(dirname "$0")/.." && exec nice -n 19 env DYLD_LIBRARY_PATH=/opt/homebrew/Cellar/llvm/22.1.8/lib:/opt/homebrew/Cellar/z3/4.16.0/lib cargo build "$@"
