//! Executable representation experiments. These are excluded from the addon, not selectable
//! compiler alternatives. Each module states its supported subset and checks an independent oracle.

mod artifacts;
mod frontier;
mod packed;
mod sharing;
mod views;

use std::time::Instant;

fn measured<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let result = f();
    (result, started.elapsed().as_secs_f64() * 1000.0)
}
