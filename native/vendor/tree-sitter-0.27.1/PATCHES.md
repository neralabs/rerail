# tree-sitter 0.27.1, patched

crates.io's tree-sitter 0.27.1, with one change in `binding_rust/lib.rs`: `Language` is `Send` and `Sync` on
every target. Upstream drops both impls on `target_family = "wasm"` (its wasm store can hold a language
loaded from a wasm module, which is not thread-safe). Rerail never loads wasm languages, and its wasm32-wasip1
build (the browser explorer) runs on one thread, while the engine keeps `Language` in shared statics and rayon
jobs. Native targets are unchanged: the impls were already there.
