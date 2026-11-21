# accel

`rerail-accel`: the logic engine's relational kernels on every compute lane, and the executor that batches their
launches (decision 0098, docs/final-shape/decisions/0098-acceleration-lanes.md).

| Directory | What it holds |
|---|---|
| `src/cpu/` | the portable CPU kernels, the reference the other lanes are checked against |
| `src/sme/` | the kernels on the CPU's SME matrix units |
| `src/gpu/` | the Metal kernels and their launch code |
| `src/ane/` | the Neural Engine lane: the Core ML model packages it writes, and the coremltools outputs its writer is checked against (`fixtures/`) |
| `build/`, `build.rs` | each lane's build step, run only when its feature is on; `build/ane.rs` compiles the ahead-of-time models `models/aot.list` names |
| `tests/` | the lanes against each other, and the cost model that picks one |

Each lane is a Cargo feature (`sme`, `gpu`, `ane`), so one lane builds alone with `--no-default-features --features
<lane>`.
