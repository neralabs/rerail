# gpu

The GPU device layer (decision 0440): one interface over Metal (macOS), CUDA (Linux), OpenCL and the CPU, with
kernels written once in UKL and launched through MLDB's grid runtime.

- `device/` is `rerail-device`, the layer below the UKL preprocessor. It holds each backend, MLDB's compute-kernel
  runtime and the process's shared device. The blocks (`native/crates/blocks`) bind their memory through it.
  `device/src/mldb/` is copied from mldbai/mldb under the paths it has there, under MLDB's licence (Apache-2.0,
  `device/src/mldb/LICENSE`). `device/examples/` holds `clc` and `nvc`, the OpenCL and NVRTC drivers alone, which
  measure kernel compile costs (decision 0395 §6).
- `src/` is `rerail-gpu`. It re-exports the device layer and adds each device's UKL preprocessing, the kernels
  compiled from UKL at run time with the UKL library carried in the binary (`kernels.rs`), and the library's
  device-wide calls (`library.rs`). `library_tests.rs` and `library_tests.uglykl` run each library primitive on the
  device against its Rust reference.
- `examples/ukl_check.rs` compiles a UKL source for every backend the machine has, so a port is checked before
  anything launches it.

The forest's trainer (`forest/`) runs on this layer. The engine's GPU lane is moving onto it (decision 0440 step 3).
