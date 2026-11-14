#!/bin/sh
# Builds simdjson_check against Homebrew's simdjson (brew install simdjson).
set -e
cd "$(dirname "$0")"
clang++ -O3 -std=c++17 -I/opt/homebrew/include simdjson_check.cpp /opt/homebrew/lib/libsimdjson_static.a -o "${1:-/tmp/simdjson_check}"
