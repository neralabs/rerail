# simdjson 5.0.5, amalgamated

`simdjson.h` and `simdjson.cpp` are the release's single-header amalgamation, unchanged, from
<https://github.com/simdjson/simdjson/tree/v5.0.5/singleheader> (the header is byte for byte
Homebrew's `simdjson` 5.0.5). Licence: Apache-2.0 or MIT (`LICENSE`, `LICENSE-MIT`).

`rerail-simdjson` (`native/crates/simdjson`) compiles them with its shim; JSON's tape is read from
simdjson's structural index (`transducer/packed/json_parser/simdjson.rs`).

SHA-256:

```
97d622441079fae86c7c342508880168bd7bf9e3342b1f4c565077da0155ca79  simdjson.cpp
869e8a4ff285cec36bc79d4a671a433158391a4f1898bb1c3b1f6e654b9547ae  simdjson.h
```
