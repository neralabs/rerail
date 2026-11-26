# forest

`rerail-forest`: MLDB's random forests, trained and predicted on the GPU (Metal), and the learned cost model built on
them (`src/cost_model.rs`). macOS only; elsewhere the crate is empty.

`src/mldb/` is copied from mldbai/mldb (commit e3caed6f, branch opencl-random-forests) under the paths it has there:
the host sources, translated to Rust where they sit, and the kernels, one C source written against UKL's shims that
`build.rs` preprocesses into Metal source through `rerail-ukl`. The shims and UKL's common headers (`ukl`, `ukl_shim.h`,
`ukl_macros.h`, `ukl_auto_macros.h` and each backend's `ukl_*_shim.h`) are carried by that crate, under the same
paths in `native/crates/ukl/mldb/`, so the engine compiles only from `native/`. `ukl_auto_macros.h` is upstream's
generated file, copied as it is; its generator (`src/mldb/block/ukl/ukl_auto_macro_gen.py`) stays here. The licence is
MLDB's (Apache-2.0), beside both copies.
