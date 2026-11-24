//! `label_leaves` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec _kernels_rust_is` writes it again.

#![allow(unused, unused_mut, unused_parens, unused_unsafe, unreachable_code, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]

use crate::Launch;
use core::arch::aarch64::*;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.
const LANE: [[u32; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];

/// The program's constant memory, which its `__constant` variables are.
static DATA: [u8; 4133] = [16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 110, 111, 100, 101, 67, 111, 117, 110, 116, 44, 119, 111, 114, 100, 115, 44, 114, 101, 115, 117, 108, 116, 65, 114, 101, 110, 97, 44, 116, 97, 115, 107, 67, 111, 117, 110, 116, 44, 105, 116, 101, 109, 67, 111, 117, 110, 116, 44, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 44, 114, 111, 117, 110, 100, 44, 102, 105, 110, 105, 115, 104, 44, 114, 101, 99, 117, 114, 115, 105, 118, 101, 44, 108, 97, 115, 116, 76, 101, 118, 101, 108, 44, 102, 105, 108, 101, 67, 111, 117, 110, 116, 44, 115, 116, 101, 112, 115, 44, 112, 114, 111, 102, 105, 108, 101, 44, 113, 117, 101, 117, 101, 44, 98, 117, 100, 103, 101, 116, 44, 108, 97, 110, 101, 115, 44, 102, 114, 97, 109, 101, 115, 0, 0, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 79, 112, 44, 32, 111, 112, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 83, 116, 97, 116, 117, 115, 44, 32, 115, 116, 97, 116, 117, 115, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 115, 107, 67, 111, 117, 110, 116, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 114, 111, 111, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 115, 108, 111, 116, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 99, 111, 110, 116, 114, 111, 108, 44, 32, 34, 91, 53, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 105, 116, 101, 109, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 32, 42, 32, 50, 48, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 115, 116, 97, 116, 115, 44, 32, 34, 91, 51, 32, 42, 32, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 32, 44, 32, 71, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 114, 105, 100, 44, 32, 44, 32, 76, 65, 78, 69, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 114, 111, 111, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 71, 114, 111, 117, 112, 76, 101, 118, 101, 108, 44, 32, 108, 101, 118, 101, 108, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 79, 112, 44, 32, 111, 112, 115, 44, 32, 34, 91, 111, 112, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 104, 97, 110, 103, 101, 100, 44, 32, 49, 44, 32, 84, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 105, 108, 101, 44, 32, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 32, 44, 32, 76, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 105, 100, 116, 104, 44, 32, 44, 32, 76, 65, 78, 69, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 105, 109, 100, 76, 97, 110, 101, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 73, 116, 101, 109, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 105, 116, 101, 109, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 101, 99, 111, 114, 100, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 32, 42, 32, 50, 48, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 117, 114, 115, 111, 114, 44, 32, 34, 91, 50, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 97, 105, 108, 117, 114, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 32, 44, 32, 71, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 114, 105, 100, 44, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 115, 116, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 101, 97, 118, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 101, 110, 116, 115, 44, 32, 34, 91, 51, 50, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 117, 112, 44, 32, 50, 48, 52, 56, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 115, 44, 32, 50, 53, 55, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 105, 100, 44, 32, 44, 32, 84, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 98, 108, 111, 99, 107, 44, 32, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 70, 111, 114, 109, 117, 108, 97, 44, 32, 102, 111, 114, 109, 117, 108, 97, 115, 44, 32, 34, 91, 102, 111, 114, 109, 117, 108, 97, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 111, 111, 116, 115, 44, 32, 34, 91, 114, 117, 108, 101, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 102, 111, 114, 109, 117, 108, 97, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 101, 97, 118, 101, 115, 44, 32, 34, 91, 108, 101, 97, 118, 101, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 114, 111, 103, 114, 97, 109, 44, 32, 34, 91, 112, 114, 111, 103, 114, 97, 109, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 70, 111, 114, 109, 117, 108, 97, 84, 97, 114, 103, 101, 116, 44, 32, 116, 97, 114, 103, 101, 116, 115, 44, 32, 34, 91, 116, 97, 114, 103, 101, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 98, 101, 108, 83, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 117, 108, 101, 115, 44, 32, 34, 91, 114, 117, 108, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 52, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 101, 110, 116, 115, 44, 32, 34, 91, 51, 50, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 104, 97, 115, 101, 113, 117, 101, 117, 101, 44, 32, 34, 91, 54, 32, 43, 32, 51, 32, 42, 32, 102, 111, 114, 109, 117, 108, 97, 115, 32, 43, 32, 99, 97, 112, 97, 99, 105, 116, 121, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 116, 44, 32, 54, 52, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 111, 111, 107, 44, 32, 50, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0];

/// Whether a lane of mask `k` is on: a comparison of its bits, which the compiler folds where it knows `k`.
#[inline(always)]
unsafe fn any(k: &[uint32x4_t; 2]) -> bool {
    let mut x = k[0];
    for r in 1..2 {
        x = vorrq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) != 0
}

/// Whether every lane of mask `k` is on, the same way.
#[inline(always)]
unsafe fn all(k: &[uint32x4_t; 2]) -> bool {
    let mut x = k[0];
    for r in 1..2 {
        x = vandq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) == u128::MAX
}

/// Mask `k`'s lanes as bits, lane `j` bit `j`.
#[inline(always)]
unsafe fn bits(k: &[uint32x4_t; 2]) -> u32 {
    let w = vld1q_u32([1u32, 2, 4, 8].as_ptr());
    let mut b = 0u32;
    for r in 0..2 {
        b |= vaddvq_u32(vandq_u32(k[r], w)) << (4 * r);
    }
    b
}

/// The largest and the least of the lanes of `a`; the largest of those mask `k` has on (0 for one off).
#[inline(always)]
unsafe fn maxv(a: &[uint32x4_t; 2]) -> u32 {
    let mut m = a[0];
    for r in 1..2 {
        m = vmaxq_u32(m, a[r]);
    }
    vmaxvq_u32(m)
}
#[inline(always)]
unsafe fn minv(a: &[uint32x4_t; 2]) -> u32 {
    let mut m = a[0];
    for r in 1..2 {
        m = vminq_u32(m, a[r]);
    }
    vminvq_u32(m)
}
#[inline(always)]
unsafe fn maxv_on(k: &[uint32x4_t; 2], a: &[uint32x4_t; 2]) -> u32 {
    let mut m = vandq_u32(k[0], a[0]);
    for r in 1..2 {
        m = vmaxq_u32(m, vandq_u32(k[r], a[r]));
    }
    vmaxvq_u32(m)
}

/// Operations of one register NEON has as several: `!=`; a product's high half; a 64-bit value's high half of a sum
/// (less its low half's carry) and of a difference (plus its borrow); and the order and the equality of 64-bit values
/// of their halves (the high halves' order, signed or not, or where they are equal the low halves').
#[inline(always)]
unsafe fn vcneq_u32(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
	vmvnq_u32(vceqq_u32(a, b))
}
#[inline(always)]
unsafe fn vmulhiq_u32(a: uint32x4_t, b: uint32x4_t) -> uint32x4_t {
	vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(a), vget_low_u32(b))), vreinterpretq_u32_u64(vmull_high_u32(a, b)))
}
#[inline(always)]
unsafe fn vadcq_u32(a: uint32x4_t, b: uint32x4_t, c: uint32x4_t) -> uint32x4_t {
	vsubq_u32(vaddq_u32(a, b), c)
}
#[inline(always)]
unsafe fn vsbcq_u32(a: uint32x4_t, b: uint32x4_t, c: uint32x4_t) -> uint32x4_t {
	vaddq_u32(vsubq_u32(a, b), c)
}
#[inline(always)]
unsafe fn vlt64q_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vorrq_u32(vcltq_u32(ah, bh), vandq_u32(vceqq_u32(ah, bh), vcltq_u32(al, bl)))
}
#[inline(always)]
unsafe fn vlt64sq_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vorrq_u32(vcltq_s32(vreinterpretq_s32_u32(ah), vreinterpretq_s32_u32(bh)), vandq_u32(vceqq_u32(ah, bh), vcltq_u32(al, bl)))
}
#[inline(always)]
unsafe fn veq64q_u32(al: uint32x4_t, ah: uint32x4_t, bl: uint32x4_t, bh: uint32x4_t) -> uint32x4_t {
	vandq_u32(vceqq_u32(al, bl), vceqq_u32(ah, bh))
}
/// The zero register, and a value in every lane.
const Z: uint32x4_t = unsafe { core::mem::transmute::<[u32; 4], uint32x4_t>([0; 4]) };
#[inline(always)]
unsafe fn splat(x: u32) -> [uint32x4_t; 2] {
	[vdupq_n_u32(x); 2]
}
/// A 64-bit value in every lane: its halves.
#[inline(always)]
unsafe fn splat64(v: u64) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| vdupq_n_u32(if r < 2 { v as u32 } else { (v >> 32) as u32 }))
}
/// Lanes `f + j`.
#[inline(always)]
unsafe fn iota(f: u32) -> [uint32x4_t; 2] {
	core::array::from_fn(|r| vaddq_u32(vdupq_n_u32(f), vld1q_u32(LANE[r].as_ptr())))
}
/// An array's first registers, and those from its 2th (a 64-bit value's halves).
#[inline(always)]
unsafe fn lo<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r])
}
#[inline(always)]
unsafe fn hi<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r + 2])
}
/// Each register's operation, over the registers.
#[inline(always)]
unsafe fn cne_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcneq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mulhi_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmulhiq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn adc_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vadcq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn sbc_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vsbcq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn lt64_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vlt64q_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn lt64s_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vlt64sq_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn eq64_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N], a3: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| veq64q_u32(a0[r], a1[r], a2[r], a3[r]))
}
#[inline(always)]
unsafe fn and_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vandq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn orr_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vorrq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn eor_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| veorq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn bic_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vbicq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mvn_r<const N: usize>(a0: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmvnq_u32(a0[r]))
}
#[inline(always)]
unsafe fn bsl_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vbslq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn add_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vaddq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn sub_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vsubq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mul_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmulq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn mla_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N], a2: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmlaq_u32(a0[r], a1[r], a2[r]))
}
#[inline(always)]
unsafe fn max_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vmaxq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn min_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vminq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn ceq_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vceqq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn clt_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcltq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cle_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcleq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cgt_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcgtq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn cge_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vcgeq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn tst_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vtstq_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn uzp1_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vuzp1q_u32(a0[r], a1[r]))
}
#[inline(always)]
unsafe fn uzp2_r<const N: usize>(a0: [uint32x4_t; N], a1: [uint32x4_t; N]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| vuzp2q_u32(a0[r], a1[r]))
}
/// The lanes' indexes `a` where mask `k` is on, else the largest of those (a lane that does not run loads where one that does
/// does).
#[inline(always)]
unsafe fn ix_on(k: [uint32x4_t; 2], a: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let s = vdupq_n_u32(maxv_on(&k, &a));
	core::array::from_fn(|r| vbslq_u32(k[r], a[r], s))
}
#[inline]
unsafe fn ld_u8(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u8).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ld_i8(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i8).read_unaligned() as i32 as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ld_u16(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u16).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ld_i16(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i16).read_unaligned() as i32 as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ld_u32(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 2] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u32).read_unaligned() as u32); 2];
	}
	let a = words2(ix);
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ld_u64(b: *const u8, ix: [uint32x4_t; 2], s: usize) -> [uint32x4_t; 4] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		let v = (b.add(u as usize * s) as *const u64).read_unaligned();
		return core::array::from_fn(|r| vdupq_n_u32(if r < 2 { v as u32 } else { (v >> 32) as u32 }));
	}
	let a = words2(ix);
	let mut g = [0u32; 16];
	for j in 0..8 {
		let v = (b.add(a[j] as usize * s) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[8 + j] = (v >> 32) as u32;
	}
	regs4(g)
}
#[inline]
unsafe fn ldj_u8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u8(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn stjf_u8(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn ldj_i8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ldj_u16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u16(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn stjf_u16(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn ldj_i16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn ldj_u32(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 2] {
	let mut g = [0u32; 8];
	for j in 0..8 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs2(g)
}
#[inline]
unsafe fn stj_u32(k: [uint32x4_t; 2], v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words2(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn stjf_u32(v: [uint32x4_t; 2], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words2(v);
	for j in 0..8 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn ldj_u64(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 4] {
	let mut g = [0u32; 16];
	for j in 0..8 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[8 + j] = (v >> 32) as u32;
	}
	regs4(g)
}
#[inline]
unsafe fn stj_u64(k: [uint32x4_t; 2], v: [uint32x4_t; 4], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words2(k), words4(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..8 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline]
unsafe fn stjf_u64(v: [uint32x4_t; 4], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words4(v);
	for j in 0..8 {
		(at(j) as *mut u64).write_unaligned((x[8 + j] as u64) << 32 | x[j] as u64);
	}
}
/// Where a load reads when no lane of its mask runs (a block entered under a uniform branch none of its lanes
/// takes): zeros, as a store's lanes that are off write to a sink. Its loads' constant offsets lie inside it.
static NO_LANE: [u8; 4096] = [0; 4096];
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's (none on: `NO_LANE`).
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 2], hi: [uint32x4_t; 2], k: [uint32x4_t; 2]) -> [usize; 8] {
	let (l, h, b) = (words2(lo), words2(hi), bits(&k));
	if b == 0 {
		return [NO_LANE.as_ptr() as usize; 8];
	}
	let f = b.trailing_zeros() as usize % 8;
	core::array::from_fn(|j| {
		let j = if b >> j & 1 != 0 { j } else { f };
		((h[j] as u64) << 32 | l[j] as u64) as usize
	})
}
#[inline(always)]
unsafe fn bsl_w(k: [uint32x4_t; 2], a: [uint32x4_t; 4], b: [uint32x4_t; 4]) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| vbslq_u32(k[r % 2], a[r], b[r]))
}
#[inline(always)]
unsafe fn cat_w(a: [uint32x4_t; 2], b: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	core::array::from_fn(|r| if r < 2 { a[r] } else { b[r - 2] })
}
/// Registers' lanes as words, and words as registers' lanes.
#[inline(always)]
unsafe fn words2(a: [uint32x4_t; 2]) -> [u32; 8] {
    core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(a)
}
#[inline(always)]
unsafe fn regs2(a: [u32; 8]) -> [uint32x4_t; 2] {
    core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(a)
}
#[inline(always)]
unsafe fn words4(a: [uint32x4_t; 4]) -> [u32; 16] {
    core::mem::transmute::<[uint32x4_t; 4], [u32; 16]>(a)
}
#[inline(always)]
unsafe fn regs4(a: [u32; 16]) -> [uint32x4_t; 4] {
    core::mem::transmute::<[u32; 16], [uint32x4_t; 4]>(a)
}

/// `label_leaves_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sr4g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg4: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x15: [[uint32x4_t; 2]; 8] = [[vdupq_n_u32(0); 2]; 8];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut x8 = [vdupq_n_u32(0); 2];
	let mut x9: u32 = 0;
	let mut x10 = [vdupq_n_u32(0); 2];
	let mut x11 = [vdupq_n_u32(0); 2];
	let mut x12 = [vdupq_n_u32(0); 2];
	let mut x13 = [vdupq_n_u32(0); 2];
	let mut x14: u32 = 0;
	let mut x16: u32 = 0;
	let mut x17: u32 = 0;
	let mut x18 = [vdupq_n_u32(0); 2];
	let mut x19 = [vdupq_n_u32(0); 4];
	let mut x20: u32 = 0;
	let mut x21: u32 = 0;
	let mut x22: u32 = 0;
	let mut x23: u32 = 0;
	let mut x24: u32 = 0;
	let mut x25 = [vdupq_n_u32(0); 2];
	let mut x26: u32 = 0;
	let mut x27: u32 = 0;
	let mut x28 = [vdupq_n_u32(0); 2];
	let mut x29 = [vdupq_n_u32(0); 2];
	let mut x30 = [vdupq_n_u32(0); 2];
	let mut x31 = [vdupq_n_u32(0); 2];
	let mut x32: u32 = 0;
	let mut x33: u32 = 0;
	let mut live = k;
	if (((x3.wrapping_add(40u64 as usize) as *const u32).read_unaligned()) == (0u32)) {
		return;
	}
	let t0 = [vdupq_n_u32(0u32); 2];
	x6 = t0;
	let t1 = [vdupq_n_u32((x3.wrapping_add(40u64 as usize) as *const u32).read_unaligned()); 2];
	x7 = t1;
	let mut t2 = live;
	loop {
		let t3 = bits(&t2);
		if t3.count_ones() <= 2 {
			let mut t4 = t3;
			while t4 != 0 {
				let j = t4.trailing_zeros() as usize;
				t4 &= t4 - 1;
				let t5 = words2(x6);
				let mut x6_j: u32 = t5[j];
				let t6 = words2(x7);
				let mut x7_j: u32 = t6[j];
				let t7 = words2(x8);
				let mut x8_j: u32 = t7[j];
				loop {
					if !((((x7_j).wrapping_sub(x6_j)) > (1u32))) { break; }
					x8_j = ((x6_j).wrapping_add(x7_j)).checked_div(2u32).unwrap_or(0);
					if (((((arg0.wrapping_add((((x8_j) as u64)).wrapping_mul(136u64) as usize).wrapping_add(8u64 as usize) as *const u32).read_unaligned()).checked_div(32u32).unwrap_or(0)).wrapping_add((2u32).wrapping_mul(x8_j))) <= ((gid0).wrapping_add(j as u32))) {
						x6_j = x8_j;
					} else {
						x7_j = x8_j;
					}
				}
				let mut t8 = words2(x6);
				t8[j] = x6_j as u32;
				x6 = regs2(t8);
				let mut t9 = words2(x7);
				t9[j] = x7_j as u32;
				x7 = regs2(t9);
				let mut t10 = words2(x8);
				t10[j] = x8_j as u32;
				x8 = regs2(t10);
			}
			break;
		}
		// Lanes part: each its own rounds.
		let t11: [uint32x4_t; 2] = [vsubq_u32(x7[0], x6[0]), vsubq_u32(x7[1], x6[1])];
		let t12 = [vdupq_n_u32(1u32); 2];
		let t13: [uint32x4_t; 2] = [vcgtq_u32(t11[0], t12[0]), vcgtq_u32(t11[1], t12[1])];
		t2 = [vandq_u32(t2[0], t13[0]), vandq_u32(t2[1], t13[1])];
		if !(any(&t2)) { break; }
		let t14: [uint32x4_t; 2] = [vaddq_u32(x6[0], x7[0]), vaddq_u32(x6[1], x7[1])];
		let t15: [uint32x4_t; 2] = [vshlq_u32(t14[0], vdupq_n_s32(-1)), vshlq_u32(t14[1], vdupq_n_s32(-1))];
		x8 = t15;
		let t16 = vdupq_n_u32(maxv_on(&t2, &x8));
		let t17: [uint32x4_t; 2] = [vbslq_u32(t2[0], x8[0], t16), vbslq_u32(t2[1], x8[1], t16)];
		let mut t18 = [vdupq_n_u32(0); 2];
		let t19 = minv(&t17);
		if t19 == maxv(&t17) {
			t18 = [vdupq_n_u32(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(t19 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
		} else {
			t18[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t17[0]) as usize * 136) as *const u32).read_unaligned() as u32), t18[0]);
			t18[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t17[0]) as usize * 136) as *const u32).read_unaligned() as u32), t18[0]);
			t18[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t17[0]) as usize * 136) as *const u32).read_unaligned() as u32), t18[0]);
			t18[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t17[0]) as usize * 136) as *const u32).read_unaligned() as u32), t18[0]);
			t18[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t17[1]) as usize * 136) as *const u32).read_unaligned() as u32), t18[1]);
			t18[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t17[1]) as usize * 136) as *const u32).read_unaligned() as u32), t18[1]);
			t18[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t17[1]) as usize * 136) as *const u32).read_unaligned() as u32), t18[1]);
			t18[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t17[1]) as usize * 136) as *const u32).read_unaligned() as u32), t18[1]);
		}
		let t20: [uint32x4_t; 2] = [vshlq_u32(t18[0], vdupq_n_s32(-5)), vshlq_u32(t18[1], vdupq_n_s32(-5))];
		let t21 = [vdupq_n_u32(2u32); 2];
		let t22: [uint32x4_t; 2] = [vmulq_u32(t21[0], x8[0]), vmulq_u32(t21[1], x8[1])];
		let t23: [uint32x4_t; 2] = [vaddq_u32(t20[0], t22[0]), vaddq_u32(t20[1], t22[1])];
		let t24: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
		let t25: [uint32x4_t; 2] = [vcleq_u32(t23[0], t24[0]), vcleq_u32(t23[1], t24[1])];
		let t26: [uint32x4_t; 2] = [vandq_u32(t2[0], t25[0]), vandq_u32(t2[1], t25[1])];
		{
			x6 = [vbslq_u32(t26[0], x8[0], x6[0]), vbslq_u32(t26[1], x8[1], x6[1])];
		}
		let t27: [uint32x4_t; 2] = [vbicq_u32(t2[0], t25[0]), vbicq_u32(t2[1], t25[1])];
		{
			x7 = [vbslq_u32(t27[0], x8[0], x7[0]), vbslq_u32(t27[1], x8[1], x7[1])];
		}
	}
	let t28 = vdupq_n_u32(maxv_on(&live, &x6));
	let t29: [uint32x4_t; 2] = [vbslq_u32(live[0], x6[0], t28), vbslq_u32(live[1], x6[1], t28)];
	let mut t30 = [vdupq_n_u32(0); 2];
	let t31 = minv(&t29);
	if t31 == maxv(&t29) {
		t30 = [vdupq_n_u32(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(t31 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
	} else {
		t30[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t29[0]) as usize * 136) as *const u32).read_unaligned() as u32), t30[0]);
		t30[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t29[0]) as usize * 136) as *const u32).read_unaligned() as u32), t30[0]);
		t30[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t29[0]) as usize * 136) as *const u32).read_unaligned() as u32), t30[0]);
		t30[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t29[0]) as usize * 136) as *const u32).read_unaligned() as u32), t30[0]);
		t30[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t29[1]) as usize * 136) as *const u32).read_unaligned() as u32), t30[1]);
		t30[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t29[1]) as usize * 136) as *const u32).read_unaligned() as u32), t30[1]);
		t30[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t29[1]) as usize * 136) as *const u32).read_unaligned() as u32), t30[1]);
		t30[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((8u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t29[1]) as usize * 136) as *const u32).read_unaligned() as u32), t30[1]);
	}
	let t32: [uint32x4_t; 2] = [vshlq_u32(t30[0], vdupq_n_s32(-5)), vshlq_u32(t30[1], vdupq_n_s32(-5))];
	let t33 = [vdupq_n_u32(2u32); 2];
	let t34: [uint32x4_t; 2] = [vmulq_u32(t33[0], x6[0]), vmulq_u32(t33[1], x6[1])];
	let t35: [uint32x4_t; 2] = [vaddq_u32(t32[0], t34[0]), vaddq_u32(t32[1], t34[1])];
	x10 = t35;
	let t36: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
	let t37: [uint32x4_t; 2] = [vcltq_u32(t36[0], x10[0]), vcltq_u32(t36[1], x10[1])];
	let t38: [uint32x4_t; 2] = [vandq_u32(live[0], t37[0]), vandq_u32(live[1], t37[1])];
	{
		live = [vbicq_u32(live[0], t38[0]), vbicq_u32(live[1], t38[1])];
	}
	if any(&live) {
		let t39: [uint32x4_t; 2] = [vsubq_u32(t36[0], x10[0]), vsubq_u32(t36[1], x10[1])];
		x11 = t39;
		let t40 = vdupq_n_u32(maxv_on(&live, &x6));
		let t41: [uint32x4_t; 2] = [vbslq_u32(live[0], x6[0], t40), vbslq_u32(live[1], x6[1], t40)];
		let mut t42 = [vdupq_n_u32(0); 2];
		let t43 = minv(&t41);
		if t43 == maxv(&t41) {
			t42 = [vdupq_n_u32(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(t43 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
		} else {
			t42[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t41[0]) as usize * 136) as *const u32).read_unaligned() as u32), t42[0]);
			t42[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t41[0]) as usize * 136) as *const u32).read_unaligned() as u32), t42[0]);
			t42[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t41[0]) as usize * 136) as *const u32).read_unaligned() as u32), t42[0]);
			t42[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t41[0]) as usize * 136) as *const u32).read_unaligned() as u32), t42[0]);
			t42[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t41[1]) as usize * 136) as *const u32).read_unaligned() as u32), t42[1]);
			t42[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t41[1]) as usize * 136) as *const u32).read_unaligned() as u32), t42[1]);
			t42[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t41[1]) as usize * 136) as *const u32).read_unaligned() as u32), t42[1]);
			t42[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((4u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t41[1]) as usize * 136) as *const u32).read_unaligned() as u32), t42[1]);
		}
		x12 = t42;
		let t44 = [vdupq_n_u32(32u32); 2];
		let t45: [uint32x4_t; 2] = [vmulq_u32(x11[0], t44[0]), vmulq_u32(x11[1], t44[1])];
		let t46: [uint32x4_t; 2] = [vcgeq_u32(t45[0], x12[0]), vcgeq_u32(t45[1], x12[1])];
		let t47: [uint32x4_t; 2] = [vandq_u32(live[0], t46[0]), vandq_u32(live[1], t46[1])];
		{
			live = [vbicq_u32(live[0], t47[0]), vbicq_u32(live[1], t47[1])];
		}
		if any(&live) {
			let t48: [uint32x4_t; 2] = [vmulq_u32(x11[0], t44[0]), vmulq_u32(x11[1], t44[1])];
			let t49: [uint32x4_t; 2] = [vsubq_u32(x12[0], t48[0]), vsubq_u32(x12[1], t48[1])];
			let t50 = b1_sl_p(l, lid0, live, 32u32, t49);
			x13 = t50;
			x14 = 0u32;
			let mut t51 = live;
			let mut t52 = [vdupq_n_u32(0); 2];
			loop {
				// Lanes part: each its own rounds.
				let t53 = [vdupq_n_u32(if ((x14) < ((x3.wrapping_add(20u64 as usize) as *const u32).read_unaligned())) { u32::MAX } else { 0 }); 2];
				t51 = [vandq_u32(vbicq_u32(t51[0], t52[0]), t53[0]), vandq_u32(vbicq_u32(t51[1], t52[1]), t53[1])];
				if !(any(&t51)) { break; }
				x16 = 0u32;
				loop {
					// Rounds of one condition: every lane goes on.
					if !(((x16) < (8u32))) { break; }
					let t54 = ((((x16) as u64)).wrapping_mul(4u64)) as usize / 4;
					x15[t54] = [vbslq_u32(t51[0], t0[0], x15[t54][0]), vbslq_u32(t51[1], t0[1], x15[t54][1])];
					let t55 = x16;
					x16 = (x16).wrapping_add(1u32);
				}
				x17 = 0u32;
				let mut t56 = t51;
				let mut t57 = [vdupq_n_u32(0); 2];
				loop {
					// Lanes part: each its own rounds.
					let t58 = [vdupq_n_u32(x17); 2];
					let t59: [uint32x4_t; 2] = [vcltq_u32(t58[0], x13[0]), vcltq_u32(t58[1], x13[1])];
					t56 = [vandq_u32(vbicq_u32(t56[0], t57[0]), t59[0]), vandq_u32(vbicq_u32(t56[1], t57[1]), t59[1])];
					if !(any(&t56)) { break; }
					let t60: [uint32x4_t; 2] = [vmulq_u32(x11[0], t44[0]), vmulq_u32(x11[1], t44[1])];
					let t61 = [vdupq_n_u32(x17); 2];
					let t62: [uint32x4_t; 2] = [vaddq_u32(t60[0], t61[0]), vaddq_u32(t60[1], t61[1])];
					x18 = t62;
					let t63 = b2_r4al136r0l_p(l, lid0, t56, arg4, arg0, x6, x18);
					x19 = t63;
					let t64 = words2([x19[0], x19[1]]);
					let t65 = words2([x19[2], x19[3]]);
					let t66 = bits(&t56);
					let t67 = t66.trailing_zeros() as usize % 8;
					let t68: [usize; 8] = if t66 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t66 >> 0 & 1 != 0 { 0 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 1 & 1 != 0 { 1 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 2 & 1 != 0 { 2 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 3 & 1 != 0 { 3 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 4 & 1 != 0 { 4 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 5 & 1 != 0 { 5 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 6 & 1 != 0 { 6 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }, { let j = if t66 >> 7 & 1 != 0 { 7 } else { t67 }; ((t65[j] as u64) << 32 | t64[j] as u64) as usize }] };
					x20 = 0u32;
					't69: loop {
						// Rounds of one condition: every lane goes on.
						if !(((x20) < (8u32))) { break; }
						x21 = (x14).wrapping_add(x20);
						if ((x21) >= ((x3.wrapping_add(20u64 as usize) as *const u32).read_unaligned())) {
							break 't69;
						}
						let t70: [uint32x4_t; 2] = [vandq_u32(t56[0], live[0]), vandq_u32(t56[1], live[1])];
						if any(&t70) {
							x22 = (arg1.wrapping_add((((x21) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
							x23 = ((x22) & (255u32));
							x24 = (x22).wrapping_shr((8u32) as u32);
							if ((x23) == (1u32)) {
								let mut t71 = [vdupq_n_u32(0); 2];
								t71[0] = vsetq_lane_u32::<0>((t68[0] as *const u16).read_unaligned() as u32, t71[0]);
								t71[0] = vsetq_lane_u32::<1>((t68[1] as *const u16).read_unaligned() as u32, t71[0]);
								t71[0] = vsetq_lane_u32::<2>((t68[2] as *const u16).read_unaligned() as u32, t71[0]);
								t71[0] = vsetq_lane_u32::<3>((t68[3] as *const u16).read_unaligned() as u32, t71[0]);
								t71[1] = vsetq_lane_u32::<0>((t68[4] as *const u16).read_unaligned() as u32, t71[1]);
								t71[1] = vsetq_lane_u32::<1>((t68[5] as *const u16).read_unaligned() as u32, t71[1]);
								t71[1] = vsetq_lane_u32::<2>((t68[6] as *const u16).read_unaligned() as u32, t71[1]);
								t71[1] = vsetq_lane_u32::<3>((t68[7] as *const u16).read_unaligned() as u32, t71[1]);
								let t72 = [vdupq_n_u32(x24); 2];
								let t73: [uint32x4_t; 2] = [vceqq_u32(t71[0], t72[0]), vceqq_u32(t71[1], t72[1])];
								x25 = [vbslq_u32(t70[0], t73[0], x25[0]), vbslq_u32(t70[1], t73[1], x25[1])];
							} else {
								if ((x23) == (2u32)) {
									let t74: u64 = 2u64;
									let t75 = [vdupq_n_u32((t74) as u32); 2];
									let t76: [uint32x4_t; 2] = [vaddq_u32(x19[0], t75[0]), vaddq_u32(x19[1], t75[1])];
									let t77: [uint32x4_t; 2] = [vcltq_u32(t76[0], x19[0]), vcltq_u32(t76[1], x19[1])];
									let t78: [uint32x4_t; 4] = [t76[0], t76[1], vadcq_u32(x19[2], vdupq_n_u32(0), t77[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t77[1])];
									let t79: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((2u64) as u64 as usize));
									let mut t80 = [vdupq_n_u32(0); 2];
									t80[0] = vsetq_lane_u32::<0>((t79[0] as *const u16).read_unaligned() as u32, t80[0]);
									t80[0] = vsetq_lane_u32::<1>((t79[1] as *const u16).read_unaligned() as u32, t80[0]);
									t80[0] = vsetq_lane_u32::<2>((t79[2] as *const u16).read_unaligned() as u32, t80[0]);
									t80[0] = vsetq_lane_u32::<3>((t79[3] as *const u16).read_unaligned() as u32, t80[0]);
									t80[1] = vsetq_lane_u32::<0>((t79[4] as *const u16).read_unaligned() as u32, t80[1]);
									t80[1] = vsetq_lane_u32::<1>((t79[5] as *const u16).read_unaligned() as u32, t80[1]);
									t80[1] = vsetq_lane_u32::<2>((t79[6] as *const u16).read_unaligned() as u32, t80[1]);
									t80[1] = vsetq_lane_u32::<3>((t79[7] as *const u16).read_unaligned() as u32, t80[1]);
									let t81 = [vdupq_n_u32(x24); 2];
									let t82: [uint32x4_t; 2] = [vceqq_u32(t80[0], t81[0]), vceqq_u32(t80[1], t81[1])];
									x25 = [vbslq_u32(t70[0], t82[0], x25[0]), vbslq_u32(t70[1], t82[1], x25[1])];
								} else {
									if ((x23) == (3u32)) {
										x26 = ((x24) & (255u32));
										x27 = (x24).wrapping_shr((8u32) as u32);
										let t83: u64 = 4u64;
										let t84 = [vdupq_n_u32((t83) as u32); 2];
										let t85: [uint32x4_t; 2] = [vaddq_u32(x19[0], t84[0]), vaddq_u32(x19[1], t84[1])];
										let t86: [uint32x4_t; 2] = [vcltq_u32(t85[0], x19[0]), vcltq_u32(t85[1], x19[1])];
										let t87: [uint32x4_t; 4] = [t85[0], t85[1], vadcq_u32(x19[2], vdupq_n_u32(0), t86[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t86[1])];
										let t88: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((4u64) as u64 as usize));
										let mut t89 = [vdupq_n_u32(0); 2];
										t89[0] = vsetq_lane_u32::<0>((t88[0] as *const u8).read_unaligned() as u32, t89[0]);
										t89[0] = vsetq_lane_u32::<1>((t88[1] as *const u8).read_unaligned() as u32, t89[0]);
										t89[0] = vsetq_lane_u32::<2>((t88[2] as *const u8).read_unaligned() as u32, t89[0]);
										t89[0] = vsetq_lane_u32::<3>((t88[3] as *const u8).read_unaligned() as u32, t89[0]);
										t89[1] = vsetq_lane_u32::<0>((t88[4] as *const u8).read_unaligned() as u32, t89[1]);
										t89[1] = vsetq_lane_u32::<1>((t88[5] as *const u8).read_unaligned() as u32, t89[1]);
										t89[1] = vsetq_lane_u32::<2>((t88[6] as *const u8).read_unaligned() as u32, t89[1]);
										t89[1] = vsetq_lane_u32::<3>((t88[7] as *const u8).read_unaligned() as u32, t89[1]);
										let t90 = [vdupq_n_u32(x26); 2];
										let t91: [uint32x4_t; 2] = [vandq_u32(t89[0], t90[0]), vandq_u32(t89[1], t90[1])];
										let t92 = [vdupq_n_u32(x26); 2];
										let t93: [uint32x4_t; 2] = [vceqq_u32(t91[0], t92[0]), vceqq_u32(t91[1], t92[1])];
										let t94: u64 = 4u64;
										let t95 = [vdupq_n_u32((t94) as u32); 2];
										let t96: [uint32x4_t; 2] = [vaddq_u32(x19[0], t95[0]), vaddq_u32(x19[1], t95[1])];
										let t97: [uint32x4_t; 2] = [vcltq_u32(t96[0], x19[0]), vcltq_u32(t96[1], x19[1])];
										let t98: [uint32x4_t; 4] = [t96[0], t96[1], vadcq_u32(x19[2], vdupq_n_u32(0), t97[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t97[1])];
										let t99: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((4u64) as u64 as usize));
										let mut t100 = [vdupq_n_u32(0); 2];
										t100[0] = vsetq_lane_u32::<0>((t99[0] as *const u8).read_unaligned() as u32, t100[0]);
										t100[0] = vsetq_lane_u32::<1>((t99[1] as *const u8).read_unaligned() as u32, t100[0]);
										t100[0] = vsetq_lane_u32::<2>((t99[2] as *const u8).read_unaligned() as u32, t100[0]);
										t100[0] = vsetq_lane_u32::<3>((t99[3] as *const u8).read_unaligned() as u32, t100[0]);
										t100[1] = vsetq_lane_u32::<0>((t99[4] as *const u8).read_unaligned() as u32, t100[1]);
										t100[1] = vsetq_lane_u32::<1>((t99[5] as *const u8).read_unaligned() as u32, t100[1]);
										t100[1] = vsetq_lane_u32::<2>((t99[6] as *const u8).read_unaligned() as u32, t100[1]);
										t100[1] = vsetq_lane_u32::<3>((t99[7] as *const u8).read_unaligned() as u32, t100[1]);
										let t101 = [vdupq_n_u32(x27); 2];
										let t102: [uint32x4_t; 2] = [vandq_u32(t100[0], t101[0]), vandq_u32(t100[1], t101[1])];
										let t103: [uint32x4_t; 2] = [vceqq_u32(t102[0], t0[0]), vceqq_u32(t102[1], t0[1])];
										let t104: [uint32x4_t; 2] = [vandq_u32(t93[0], t103[0]), vandq_u32(t93[1], t103[1])];
										x25 = [vbslq_u32(t70[0], t104[0], x25[0]), vbslq_u32(t70[1], t104[1], x25[1])];
									} else {
										if ((x23) == (18u32)) {
											let t105 = b3_r4al136r0l_p(l, lid0, t70, arg4, arg0, x6, x18);
											let t106 = [vdupq_n_u32(4294967295u32); 2];
											let t107: [uint32x4_t; 2] = [vcneq_u32(t105[0], t106[0]), vcneq_u32(t105[1], t106[1])];
											x25 = [vbslq_u32(t70[0], t107[0], x25[0]), vbslq_u32(t70[1], t107[1], x25[1])];
										} else {
											if ((x23) == (22u32)) {
												let t108 = [vdupq_n_u32(if ((0i32) != 0) { u32::MAX } else { 0 }); 2];
												x25 = [vbslq_u32(t70[0], t108[0], x25[0]), vbslq_u32(t70[1], t108[1], x25[1])];
												let t109 = b4_r4al136r0l_p(l, lid0, t70, arg4, arg0, x6, x18);
												x28 = [vbslq_u32(t70[0], t109[0], x28[0]), vbslq_u32(t70[1], t109[1], x28[1])];
												let t110 = b5_r4al136r0ll_p(l, lid0, t70, arg4, arg0, x6, x18, x28);
												x29 = [vbslq_u32(t70[0], t110[0], x29[0]), vbslq_u32(t70[1], t110[1], x29[1])];
												let mut t111 = t70;
												let mut t112 = [vdupq_n_u32(0); 2];
												loop {
													// Lanes part: each its own rounds.
													let t113 = [vdupq_n_u32(4294967295u32); 2];
													let t114: [uint32x4_t; 2] = [vcneq_u32(x29[0], t113[0]), vcneq_u32(x29[1], t113[1])];
													t111 = [vandq_u32(vbicq_u32(t111[0], t112[0]), t114[0]), vandq_u32(vbicq_u32(t111[1], t112[1]), t114[1])];
													if !(any(&t111)) { break; }
													let t115 = b6_r4al136r0l_p(l, lid0, t111, arg4, arg0, x6, x29);
													let t116 = [vdupq_n_u32(1u32); 2];
													let t117: [uint32x4_t; 2] = [vandq_u32(t115[0], t116[0]), vandq_u32(t115[1], t116[1])];
													let t118: [uint32x4_t; 2] = [vcneq_u32(t117[0], t0[0]), vcneq_u32(t117[1], t0[1])];
													let t119: [uint32x4_t; 2] = [vandq_u32(t111[0], t118[0]), vandq_u32(t111[1], t118[1])];
													if any(&t119) {
														let t120 = [vdupq_n_u32(if ((1i32) != 0) { u32::MAX } else { 0 }); 2];
														x25 = [vbslq_u32(t119[0], t120[0], x25[0]), vbslq_u32(t119[1], t120[1], x25[1])];
														t112 = [vorrq_u32(t112[0], t119[0]), vorrq_u32(t112[1], t119[1])];
													}
													let t121: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t111[0], live[0]), t112[0]), vbicq_u32(vandq_u32(t111[1], live[1]), t112[1])];
													let t122 = b5_r4al136r0ll_p(l, lid0, t121, arg4, arg0, x6, x29, x28);
													x29 = [vbslq_u32(t121[0], t122[0], x29[0]), vbslq_u32(t121[1], t122[1], x29[1])];
												}
											} else {
												if ((x23) == (21u32)) {
													let t123 = [vdupq_n_u32(if ((0i32) != 0) { u32::MAX } else { 0 }); 2];
													x25 = [vbslq_u32(t70[0], t123[0], x25[0]), vbslq_u32(t70[1], t123[1], x25[1])];
													let t124 = b7_r4al136r0l_p(l, lid0, t70, arg4, arg0, x6, x18);
													x30 = [vbslq_u32(t70[0], t124[0], x30[0]), vbslq_u32(t70[1], t124[1], x30[1])];
													let t125 = b8_r4al136r0l_p(l, lid0, t70, arg4, arg0, x6, x18);
													x31 = [vbslq_u32(t70[0], t125[0], x31[0]), vbslq_u32(t70[1], t125[1], x31[1])];
													let mut t126 = t70;
													let mut t127 = [vdupq_n_u32(0); 2];
													loop {
														// Lanes part: each its own rounds.
														let t128 = [vdupq_n_u32(4294967295u32); 2];
														let t129: [uint32x4_t; 2] = [vcneq_u32(x31[0], t128[0]), vcneq_u32(x31[1], t128[1])];
														t126 = [vandq_u32(vbicq_u32(t126[0], t127[0]), t129[0]), vandq_u32(vbicq_u32(t126[1], t127[1]), t129[1])];
														if !(any(&t126)) { break; }
														let t130 = b6_r4al136r0l_p(l, lid0, t126, arg4, arg0, x6, x31);
														let t131 = [vdupq_n_u32(1u32); 2];
														let t132: [uint32x4_t; 2] = [vandq_u32(t130[0], t131[0]), vandq_u32(t130[1], t131[1])];
														let t133: [uint32x4_t; 2] = [vcneq_u32(t132[0], t0[0]), vcneq_u32(t132[1], t0[1])];
														let t134: [uint32x4_t; 2] = [vandq_u32(t126[0], t133[0]), vandq_u32(t126[1], t133[1])];
														if any(&t134) {
															let t135 = [vdupq_n_u32(if ((1i32) != 0) { u32::MAX } else { 0 }); 2];
															x25 = [vbslq_u32(t134[0], t135[0], x25[0]), vbslq_u32(t134[1], t135[1], x25[1])];
															t127 = [vorrq_u32(t127[0], t134[0]), vorrq_u32(t127[1], t134[1])];
														}
														let t136: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t126[0], live[0]), t127[0]), vbicq_u32(vandq_u32(t126[1], live[1]), t127[1])];
														let t137 = b5_r4al136r0ll_p(l, lid0, t136, arg4, arg0, x6, x31, x30);
														x31 = [vbslq_u32(t136[0], t137[0], x31[0]), vbslq_u32(t136[1], t137[1], x31[1])];
													}
												} else {
													if ((x23) == (19u32)) {
														let t138 = b9_r4al136r0l_p(l, lid0, t70, arg4, arg0, x6, x18);
														let t139 = [vdupq_n_u32(1u32); 2];
														let t140: [uint32x4_t; 2] = [vcgtq_u32(t138[0], t139[0]), vcgtq_u32(t138[1], t139[1])];
														x25 = [vbslq_u32(t70[0], t140[0], x25[0]), vbslq_u32(t70[1], t140[1], x25[1])];
													} else {
														let t141 = [vdupq_n_u32(if ((0i32) != 0) { u32::MAX } else { 0 }); 2];
														x25 = [vbslq_u32(t70[0], t141[0], x25[0]), vbslq_u32(t70[1], t141[1], x25[1])];
														x32 = 0u32;
														let mut t142 = t70;
														let mut t143 = [vdupq_n_u32(0); 2];
														loop {
															// Lanes part: each its own rounds.
															let t144: u64 = 5u64;
															let t145 = [vdupq_n_u32((t144) as u32); 2];
															let t146: [uint32x4_t; 2] = [vaddq_u32(x19[0], t145[0]), vaddq_u32(x19[1], t145[1])];
															let t147: [uint32x4_t; 2] = [vcltq_u32(t146[0], x19[0]), vcltq_u32(t146[1], x19[1])];
															let t148: [uint32x4_t; 4] = [t146[0], t146[1], vadcq_u32(x19[2], vdupq_n_u32(0), t147[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t147[1])];
															let t149: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((5u64) as u64 as usize));
															let mut t150 = [vdupq_n_u32(0); 2];
															t150[0] = vsetq_lane_u32::<0>((t149[0] as *const u8).read_unaligned() as u32, t150[0]);
															t150[0] = vsetq_lane_u32::<1>((t149[1] as *const u8).read_unaligned() as u32, t150[0]);
															t150[0] = vsetq_lane_u32::<2>((t149[2] as *const u8).read_unaligned() as u32, t150[0]);
															t150[0] = vsetq_lane_u32::<3>((t149[3] as *const u8).read_unaligned() as u32, t150[0]);
															t150[1] = vsetq_lane_u32::<0>((t149[4] as *const u8).read_unaligned() as u32, t150[1]);
															t150[1] = vsetq_lane_u32::<1>((t149[5] as *const u8).read_unaligned() as u32, t150[1]);
															t150[1] = vsetq_lane_u32::<2>((t149[6] as *const u8).read_unaligned() as u32, t150[1]);
															t150[1] = vsetq_lane_u32::<3>((t149[7] as *const u8).read_unaligned() as u32, t150[1]);
															let t151 = [vdupq_n_u32(x32); 2];
															let t152: [uint32x4_t; 2] = [vcltq_u32(t151[0], t150[0]), vcltq_u32(t151[1], t150[1])];
															t142 = [vandq_u32(vbicq_u32(t142[0], t143[0]), t152[0]), vandq_u32(vbicq_u32(t142[1], t143[1]), t152[1])];
															if !(any(&t142)) { break; }
															let t153: u64 = 8u64;
															let t154 = [vdupq_n_u32((t153) as u32); 2];
															let t155: [uint32x4_t; 2] = [vaddq_u32(x19[0], t154[0]), vaddq_u32(x19[1], t154[1])];
															let t156: [uint32x4_t; 2] = [vcltq_u32(t155[0], x19[0]), vcltq_u32(t155[1], x19[1])];
															let t157: [uint32x4_t; 4] = [t155[0], t155[1], vadcq_u32(x19[2], vdupq_n_u32(0), t156[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t156[1])];
															let t158: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((8u64) as u64 as usize));
															let t159: u64 = (((x32) as u64)).wrapping_mul(2u64);
															let t160 = [vdupq_n_u32((t159) as u32); 2];
															let t161 = [vdupq_n_u32(((t159) >> 32) as u32); 2];
															let t162: [uint32x4_t; 2] = [vaddq_u32(t157[0], t160[0]), vaddq_u32(t157[1], t160[1])];
															let t163: [uint32x4_t; 2] = [vcltq_u32(t162[0], t157[0]), vcltq_u32(t162[1], t157[1])];
															let t164: [uint32x4_t; 4] = [t162[0], t162[1], vadcq_u32(t157[2], t161[0], t163[0]), vadcq_u32(t157[3], t161[1], t163[1])];
															let t165: [usize; 8] = core::array::from_fn(|j| t158[j].wrapping_add(((((x32) as u64)).wrapping_mul(2u64)) as u64 as usize));
															let mut t166 = [vdupq_n_u32(0); 2];
															t166[0] = vsetq_lane_u32::<0>((t165[0] as *const u16).read_unaligned() as u32, t166[0]);
															t166[0] = vsetq_lane_u32::<1>((t165[1] as *const u16).read_unaligned() as u32, t166[0]);
															t166[0] = vsetq_lane_u32::<2>((t165[2] as *const u16).read_unaligned() as u32, t166[0]);
															t166[0] = vsetq_lane_u32::<3>((t165[3] as *const u16).read_unaligned() as u32, t166[0]);
															t166[1] = vsetq_lane_u32::<0>((t165[4] as *const u16).read_unaligned() as u32, t166[1]);
															t166[1] = vsetq_lane_u32::<1>((t165[5] as *const u16).read_unaligned() as u32, t166[1]);
															t166[1] = vsetq_lane_u32::<2>((t165[6] as *const u16).read_unaligned() as u32, t166[1]);
															t166[1] = vsetq_lane_u32::<3>((t165[7] as *const u16).read_unaligned() as u32, t166[1]);
															let t167 = [vdupq_n_u32(x24); 2];
															let t168: [uint32x4_t; 2] = [vceqq_u32(t166[0], t167[0]), vceqq_u32(t166[1], t167[1])];
															let t169: [uint32x4_t; 2] = [vandq_u32(t142[0], t168[0]), vandq_u32(t142[1], t168[1])];
															if any(&t169) {
																let t170 = [vdupq_n_u32(if ((1i32) != 0) { u32::MAX } else { 0 }); 2];
																x25 = [vbslq_u32(t169[0], t170[0], x25[0]), vbslq_u32(t169[1], t170[1], x25[1])];
																t143 = [vorrq_u32(t143[0], t169[0]), vorrq_u32(t143[1], t169[1])];
															}
															let t171: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t142[0], live[0]), t143[0]), vbicq_u32(vandq_u32(t142[1], live[1]), t143[1])];
															if any(&t171) {
																let t172: u64 = 8u64;
																let t173 = [vdupq_n_u32((t172) as u32); 2];
																let t174: [uint32x4_t; 2] = [vaddq_u32(x19[0], t173[0]), vaddq_u32(x19[1], t173[1])];
																let t175: [uint32x4_t; 2] = [vcltq_u32(t174[0], x19[0]), vcltq_u32(t174[1], x19[1])];
																let t176: [uint32x4_t; 4] = [t174[0], t174[1], vadcq_u32(x19[2], vdupq_n_u32(0), t175[0]), vadcq_u32(x19[3], vdupq_n_u32(0), t175[1])];
																let t177: [usize; 8] = core::array::from_fn(|j| t68[j].wrapping_add((8u64) as u64 as usize));
																let t178: u64 = (((x32) as u64)).wrapping_mul(2u64);
																let t179 = [vdupq_n_u32((t178) as u32); 2];
																let t180 = [vdupq_n_u32(((t178) >> 32) as u32); 2];
																let t181: [uint32x4_t; 2] = [vaddq_u32(t176[0], t179[0]), vaddq_u32(t176[1], t179[1])];
																let t182: [uint32x4_t; 2] = [vcltq_u32(t181[0], t176[0]), vcltq_u32(t181[1], t176[1])];
																let t183: [uint32x4_t; 4] = [t181[0], t181[1], vadcq_u32(t176[2], t180[0], t182[0]), vadcq_u32(t176[3], t180[1], t182[1])];
																let t184: [usize; 8] = core::array::from_fn(|j| t177[j].wrapping_add(((((x32) as u64)).wrapping_mul(2u64)) as u64 as usize));
																let mut t185 = [vdupq_n_u32(0); 2];
																t185[0] = vsetq_lane_u32::<0>((t184[0] as *const u16).read_unaligned() as u32, t185[0]);
																t185[0] = vsetq_lane_u32::<1>((t184[1] as *const u16).read_unaligned() as u32, t185[0]);
																t185[0] = vsetq_lane_u32::<2>((t184[2] as *const u16).read_unaligned() as u32, t185[0]);
																t185[0] = vsetq_lane_u32::<3>((t184[3] as *const u16).read_unaligned() as u32, t185[0]);
																t185[1] = vsetq_lane_u32::<0>((t184[4] as *const u16).read_unaligned() as u32, t185[1]);
																t185[1] = vsetq_lane_u32::<1>((t184[5] as *const u16).read_unaligned() as u32, t185[1]);
																t185[1] = vsetq_lane_u32::<2>((t184[6] as *const u16).read_unaligned() as u32, t185[1]);
																t185[1] = vsetq_lane_u32::<3>((t184[7] as *const u16).read_unaligned() as u32, t185[1]);
																let t186 = [vdupq_n_u32(x24); 2];
																let t187: [uint32x4_t; 2] = [vcgtq_u32(t185[0], t186[0]), vcgtq_u32(t185[1], t186[1])];
																let t188: [uint32x4_t; 2] = [vandq_u32(t171[0], t187[0]), vandq_u32(t171[1], t187[1])];
																if any(&t188) {
																	t143 = [vorrq_u32(t143[0], t188[0]), vorrq_u32(t143[1], t188[1])];
																}
															}
															let t189: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t142[0], live[0]), t143[0]), vbicq_u32(vandq_u32(t142[1], live[1]), t143[1])];
															let t190 = x32;
															x32 = (x32).wrapping_add(1u32);
														}
													}
												}
											}
										}
									}
								}
							}
							let t191: [uint32x4_t; 2] = [vandq_u32(t70[0], live[0]), vandq_u32(t70[1], live[1])];
							if any(&t191) {
								let t192 = ((((x20) as u64)).wrapping_mul(4u64)) as usize / 4;
								let t193: [uint32x4_t; 2] = [vshrq_n_u32::<31>(x25[0]), vshrq_n_u32::<31>(x25[1])];
								let t194: [uint32x4_t; 2] = [vshlq_u32(t193[0], vdupq_n_s32(((x17) & 31) as i32)), vshlq_u32(t193[1], vdupq_n_s32(((x17) & 31) as i32))];
								let t195: [uint32x4_t; 2] = [vorrq_u32(x15[t192][0], t194[0]), vorrq_u32(x15[t192][1], t194[1])];
								x15[t192] = [vbslq_u32(t191[0], t195[0], x15[t192][0]), vbslq_u32(t191[1], t195[1], x15[t192][1])];
							}
						}
						let t196 = x20;
						x20 = (x20).wrapping_add(1u32);
					}
					let t197: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t56[0], live[0]), t57[0]), vbicq_u32(vandq_u32(t56[1], live[1]), t57[1])];
					let t198 = x17;
					x17 = (x17).wrapping_add(1u32);
				}
				let t199: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t51[0], live[0]), t52[0]), vbicq_u32(vandq_u32(t51[1], live[1]), t52[1])];
				if any(&t199) {
					x33 = 0u32;
					loop {
						// Rounds of one condition: every lane goes on.
						let mut t200: bool = ((x33) < (8u32));
						if t200 {
							t200 = (((x14).wrapping_add(x33)) < ((x3.wrapping_add(20u64 as usize) as *const u32).read_unaligned()));
						}
						if !(t200) { break; }
						let t201 = ((((x33) as u64)).wrapping_mul(4u64)) as usize / 4;
						let t202 = [vdupq_n_u32(((x14).wrapping_add(x33)).wrapping_mul((x3.wrapping_add(32u64 as usize) as *const u32).read_unaligned())); 2];
						let t203: [uint32x4_t; 2] = [vaddq_u32(t202[0], x10[0]), vaddq_u32(t202[1], x10[1])];
						let t204: [uint32x4_t; 2] = [vaddq_u32(t203[0], x11[0]), vaddq_u32(t203[1], x11[1])];
						let mut t205w = 0u32;
						let t205 = core::hint::black_box(&raw mut t205w);
						core::hint::select_unpredictable(vgetq_lane_u32::<0>(t199[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(t204[0]) as usize), t205).write(vgetq_lane_u32::<0>(x15[t201][0]));
						core::hint::select_unpredictable(vgetq_lane_u32::<1>(t199[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(t204[0]) as usize), t205).write(vgetq_lane_u32::<1>(x15[t201][0]));
						core::hint::select_unpredictable(vgetq_lane_u32::<2>(t199[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(t204[0]) as usize), t205).write(vgetq_lane_u32::<2>(x15[t201][0]));
						core::hint::select_unpredictable(vgetq_lane_u32::<3>(t199[0]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(t204[0]) as usize), t205).write(vgetq_lane_u32::<3>(x15[t201][0]));
						core::hint::select_unpredictable(vgetq_lane_u32::<0>(t199[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<0>(t204[1]) as usize), t205).write(vgetq_lane_u32::<0>(x15[t201][1]));
						core::hint::select_unpredictable(vgetq_lane_u32::<1>(t199[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<1>(t204[1]) as usize), t205).write(vgetq_lane_u32::<1>(x15[t201][1]));
						core::hint::select_unpredictable(vgetq_lane_u32::<2>(t199[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<2>(t204[1]) as usize), t205).write(vgetq_lane_u32::<2>(x15[t201][1]));
						core::hint::select_unpredictable(vgetq_lane_u32::<3>(t199[1]) != 0, (arg2 as *mut u32).add(vgetq_lane_u32::<3>(t204[1]) as usize), t205).write(vgetq_lane_u32::<3>(x15[t201][1]));
						let t206 = x33;
						x33 = (x33).wrapping_add(1u32);
					}
				}
				let t207: [uint32x4_t; 2] = [vbicq_u32(vandq_u32(t51[0], live[0]), t52[0]), vbicq_u32(vandq_u32(t51[1], live[1]), t52[1])];
				x14 = (x14).wrapping_add(8u32);
			}
		}
	}
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b1_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x0); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(t0[0], x1[0]), vcltq_u32(t0[1], x1[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		let t5 = [vdupq_n_u32(x0); 2];
		t2 = [vbslq_u32(t3[0], t5[0], t2[0]), vbslq_u32(t3[1], t5[1], t2[1])];
	}
	{
		t2 = [vbslq_u32(t4[0], x1[0], t2[0]), vbslq_u32(t4[1], x1[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `tape_code`.
#[inline(always)]
unsafe fn b2_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	x3 = t0;
	let t1 = words2([x3[0], x3[1]]);
	let t2 = words2([x3[2], x3[3]]);
	let t3 = bits(&live);
	let t4 = t3.trailing_zeros() as usize % 8;
	let t5: [usize; 8] = if t3 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t3 >> 0 & 1 != 0 { 0 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 1 & 1 != 0 { 1 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 2 & 1 != 0 { 2 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 3 & 1 != 0 { 3 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 4 & 1 != 0 { 4 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 5 & 1 != 0 { 5 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 6 & 1 != 0 { 6 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }, { let j = if t3 >> 7 & 1 != 0 { 7 } else { t4 }; ((t2[j] as u64) << 32 | t1[j] as u64) as usize }] };
	let t6 = vdupq_n_u32(maxv_on(&live, &x1i));
	let t7: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t6), vbslq_u32(live[1], x1i[1], t6)];
	let mut t8 = [vdupq_n_u32(0); 2];
	let t9 = minv(&t7);
	if t9 == maxv(&t7) {
		t8 = [vdupq_n_u32(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(t9 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
	} else {
		t8[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t7[0]) as usize * 136) as *const u32).read_unaligned() as u32), t8[0]);
		t8[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t7[0]) as usize * 136) as *const u32).read_unaligned() as u32), t8[0]);
		t8[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t7[0]) as usize * 136) as *const u32).read_unaligned() as u32), t8[0]);
		t8[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t7[0]) as usize * 136) as *const u32).read_unaligned() as u32), t8[0]);
		t8[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t7[1]) as usize * 136) as *const u32).read_unaligned() as u32), t8[1]);
		t8[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t7[1]) as usize * 136) as *const u32).read_unaligned() as u32), t8[1]);
		t8[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t7[1]) as usize * 136) as *const u32).read_unaligned() as u32), t8[1]);
		t8[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((84u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t7[1]) as usize * 136) as *const u32).read_unaligned() as u32), t8[1]);
	}
	let t10: [uint32x4_t; 2] = [vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(t8[0]))), vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(t8[1])))];
	let t11: [uint32x4_t; 4] = [t8[0], t8[1], t10[0], t10[1]];
	let t12: [uint32x4_t; 2] = [vaddq_u32(x3[0], t11[0]), vaddq_u32(x3[1], t11[1])];
	let t13: [uint32x4_t; 2] = [vcltq_u32(t12[0], x3[0]), vcltq_u32(t12[1], x3[1])];
	let t14: [uint32x4_t; 4] = [t12[0], t12[1], vadcq_u32(x3[2], t11[2], t13[0]), vadcq_u32(x3[3], t11[3], t13[1])];
	let t15 = b11_lal136r0l_p(l, lid0, live, x3, arg0, x1i, x2);
	let t16: [uint32x4_t; 2] = [vmulq_u32(t15[0], vdupq_n_u32(24)), vmulq_u32(t15[1], vdupq_n_u32(24))];
	let t17: [uint32x4_t; 2] = [vmulhiq_u32(t15[0], vdupq_n_u32(24)), vmulhiq_u32(t15[1], vdupq_n_u32(24))];
	let t18: [uint32x4_t; 2] = [vaddq_u32(t14[0], t16[0]), vaddq_u32(t14[1], t16[1])];
	let t19: [uint32x4_t; 2] = [vcltq_u32(t18[0], t14[0]), vcltq_u32(t18[1], t14[1])];
	let t20: [uint32x4_t; 4] = [t18[0], t18[1], vadcq_u32(t14[2], t17[0], t19[0]), vadcq_u32(t14[3], t17[1], t19[1])];
	res = t20;
	return res;
}

/// `tape_next`.
#[inline(always)]
unsafe fn b3_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b12_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_siblings_end`.
#[inline(always)]
unsafe fn b4_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b13_r4al136r0l_p(l, lid0, live, arg4, arg0, x1i, x2);
	x3 = t0;
	let t1 = [vdupq_n_u32(4294967295u32); 2];
	let t2: [uint32x4_t; 2] = [vceqq_u32(x3[0], t1[0]), vceqq_u32(x3[1], t1[1])];
	let mut t3 = [vdupq_n_u32(0); 2];
	let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	let t5: [uint32x4_t; 2] = [vbicq_u32(live[0], t2[0]), vbicq_u32(live[1], t2[1])];
	{
		let t6 = [vdupq_n_u32(0u32); 2];
		t3 = [vbslq_u32(t4[0], t6[0], t3[0]), vbslq_u32(t4[1], t6[1], t3[1])];
	}
	if any(&t5) {
		let t7: [uint32x4_t; 2];
		let t8 = bits(&t5);
		if t8.count_ones() <= 2 {
			let t9 = words2(x1i);
			let t10 = words2(x3);
			let mut t11 = [0u32; 8];
			let mut t12 = t8;
			while t12 != 0 {
				let j = t12.trailing_zeros() as usize;
				t12 &= t12 - 1;
				let t13 = [vdupq_n_u32(t10[j]); 2];
				let t14 = b7_r4sl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], arg4, (arg0 as *const u8).wrapping_add(((t9[j]) as usize).wrapping_mul(136)), t13);
				t11[j] = vgetq_lane_u32::<0>(t14[0]);
			}
			t7 = regs2(t11);
		} else {
			let t15 = b7_r4al136r0l_p(l, lid0, t5, arg4, arg0, x1i, x3);
			t7 = t15;
		}
		t3 = [vbslq_u32(t5[0], t7[0], t3[0]), vbslq_u32(t5[1], t7[1], t3[1])];
	}
	res = t3;
	return res;
}

/// `tape_next_in`.
#[inline(always)]
unsafe fn b5_r4al136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b14_lal136r0ll_p(l, lid0, live, t0, arg0, x1i, x2, x3);
	res = t1;
	return res;
}

/// `tape_flags`.
#[inline(always)]
unsafe fn b6_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b2_r4al136r0l_p(l, lid0, live, arg4, arg0, x1i, x2);
	let t1: u64 = 4u64;
	let t2 = [vdupq_n_u32((t1) as u32); 2];
	let t3: [uint32x4_t; 2] = [vaddq_u32(t0[0], t2[0]), vaddq_u32(t0[1], t2[1])];
	let t4: [uint32x4_t; 2] = [vcltq_u32(t3[0], t0[0]), vcltq_u32(t3[1], t0[1])];
	let t5: [uint32x4_t; 4] = [t3[0], t3[1], vadcq_u32(t0[2], vdupq_n_u32(0), t4[0]), vadcq_u32(t0[3], vdupq_n_u32(0), t4[1])];
	let t6 = words2([t5[0], t5[1]]);
	let t7 = words2([t5[2], t5[3]]);
	let t8 = bits(&live);
	let t9 = t8.trailing_zeros() as usize % 8;
	let t10: [usize; 8] = if t8 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t8 >> 0 & 1 != 0 { 0 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 1 & 1 != 0 { 1 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 2 & 1 != 0 { 2 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 3 & 1 != 0 { 3 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 4 & 1 != 0 { 4 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 5 & 1 != 0 { 5 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 6 & 1 != 0 { 6 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }, { let j = if t8 >> 7 & 1 != 0 { 7 } else { t9 }; ((t7[j] as u64) << 32 | t6[j] as u64) as usize }] };
	let mut t11 = [vdupq_n_u32(0); 2];
	t11[0] = vsetq_lane_u32::<0>((t10[0] as *const u8).read_unaligned() as u32, t11[0]);
	t11[0] = vsetq_lane_u32::<1>((t10[1] as *const u8).read_unaligned() as u32, t11[0]);
	t11[0] = vsetq_lane_u32::<2>((t10[2] as *const u8).read_unaligned() as u32, t11[0]);
	t11[0] = vsetq_lane_u32::<3>((t10[3] as *const u8).read_unaligned() as u32, t11[0]);
	t11[1] = vsetq_lane_u32::<0>((t10[4] as *const u8).read_unaligned() as u32, t11[1]);
	t11[1] = vsetq_lane_u32::<1>((t10[5] as *const u8).read_unaligned() as u32, t11[1]);
	t11[1] = vsetq_lane_u32::<2>((t10[6] as *const u8).read_unaligned() as u32, t11[1]);
	t11[1] = vsetq_lane_u32::<3>((t10[7] as *const u8).read_unaligned() as u32, t11[1]);
	res = t11;
	return res;
}

/// `tape_end_of`.
#[inline(always)]
unsafe fn b7_r4sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, x1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4s_p(l, lid0, live, arg4, x1);
	let t1 = b15_ssl_p(l, lid0, live, t0, x1, x2);
	res = t1;
	return res;
}

/// `tape_end_of`.
#[inline(always)]
unsafe fn b7_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b15_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_first_child`.
#[inline(always)]
unsafe fn b8_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b16_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_size`.
#[inline(always)]
unsafe fn b9_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b17_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b10_r4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, x1: *const u8) -> *const u8 {
    let a0: u32 = (x1.wrapping_add(124u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x1 as *const u32).read_unaligned();
	let mut live = k;
	let mut res: *const u8 = core::ptr::null();
	res = arg4.wrapping_add((((((((((a0) as u64)).wrapping_shl((32u64) as u32)) | (((a1) as u64)))) as i64)) as u64) as usize);
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b10_r4al136r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = vdupq_n_u32(maxv_on(&live, &x1i));
	let t1: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t0), vbslq_u32(live[1], x1i[1], t0)];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3 = minv(&t1);
	if t3 == maxv(&t1) {
		t2 = [vdupq_n_u32(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(t3 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
	} else {
		t2[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1[0]) as usize * 136) as *const u32).read_unaligned() as u32), t2[0]);
		t2[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1[0]) as usize * 136) as *const u32).read_unaligned() as u32), t2[0]);
		t2[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1[0]) as usize * 136) as *const u32).read_unaligned() as u32), t2[0]);
		t2[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1[0]) as usize * 136) as *const u32).read_unaligned() as u32), t2[0]);
		t2[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<0>(t1[1]) as usize * 136) as *const u32).read_unaligned() as u32), t2[1]);
		t2[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<1>(t1[1]) as usize * 136) as *const u32).read_unaligned() as u32), t2[1]);
		t2[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<2>(t1[1]) as usize * 136) as *const u32).read_unaligned() as u32), t2[1]);
		t2[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((124u64) as usize) as *const u8).add(vgetq_lane_u32::<3>(t1[1]) as usize * 136) as *const u32).read_unaligned() as u32), t2[1]);
	}
	let t4 = [vdupq_n_u32(0), vdupq_n_u32(0), t2[0], t2[1]];
	let t5 = vdupq_n_u32(maxv_on(&live, &x1i));
	let t6: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t5), vbslq_u32(live[1], x1i[1], t5)];
	let mut t7 = [vdupq_n_u32(0); 2];
	let t8 = minv(&t6);
	if t8 == maxv(&t6) {
		t7 = [vdupq_n_u32((((arg0 as *const u8).add(t8 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
	} else {
		t7[0] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t6[0]) as usize * 136) as *const u32).read_unaligned() as u32), t7[0]);
		t7[0] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t6[0]) as usize * 136) as *const u32).read_unaligned() as u32), t7[0]);
		t7[0] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t6[0]) as usize * 136) as *const u32).read_unaligned() as u32), t7[0]);
		t7[0] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t6[0]) as usize * 136) as *const u32).read_unaligned() as u32), t7[0]);
		t7[1] = vsetq_lane_u32::<0>((((arg0 as *const u8).add(vgetq_lane_u32::<0>(t6[1]) as usize * 136) as *const u32).read_unaligned() as u32), t7[1]);
		t7[1] = vsetq_lane_u32::<1>((((arg0 as *const u8).add(vgetq_lane_u32::<1>(t6[1]) as usize * 136) as *const u32).read_unaligned() as u32), t7[1]);
		t7[1] = vsetq_lane_u32::<2>((((arg0 as *const u8).add(vgetq_lane_u32::<2>(t6[1]) as usize * 136) as *const u32).read_unaligned() as u32), t7[1]);
		t7[1] = vsetq_lane_u32::<3>((((arg0 as *const u8).add(vgetq_lane_u32::<3>(t6[1]) as usize * 136) as *const u32).read_unaligned() as u32), t7[1]);
	}
	let t9: [uint32x4_t; 4] = [vorrq_u32(t4[0], t7[0]), vorrq_u32(t4[1], t7[1]), t4[2], t4[3]];
	let t10: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
	let t11: [uint32x4_t; 4] = [vandq_u32(t9[0], t10[0]), vandq_u32(t9[1], t10[1]), vandq_u32(t9[2], t10[2]), vandq_u32(t9[3], t10[3])];
	let t12: u64 = ((arg4) as usize as u64);
	let t13 = [vdupq_n_u32((t12) as u32); 2];
	let t14 = [vdupq_n_u32(((t12) >> 32) as u32); 2];
	let t15: [uint32x4_t; 2] = [vaddq_u32(t13[0], t11[0]), vaddq_u32(t13[1], t11[1])];
	let t16: [uint32x4_t; 2] = [vcltq_u32(t15[0], t13[0]), vcltq_u32(t15[1], t13[1])];
	let t17: [uint32x4_t; 4] = [t15[0], t15[1], vadcq_u32(t14[0], t11[2], t16[0]), vadcq_u32(t14[1], t11[3], t16[1])];
	res = t17;
	return res;
}

/// `tape_number`.
#[inline(always)]
unsafe fn b11_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b18_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_next`.
#[inline(always)]
unsafe fn b12_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b19_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `tape_parent`.
#[inline(always)]
unsafe fn b13_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b20_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `clif_tape_next_in`.
#[inline(always)]
unsafe fn b14_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b21_lal136r0ll_p(l, lid0, live, x0, arg0, x1i, x2, x3);
	res = t0;
	return res;
}

/// `clif_tape_end_of`.
#[inline(always)]
unsafe fn b15_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b22_ssl_p(l, lid0, live, x0, x1, x2);
	res = t0;
	return res;
}

/// `clif_tape_end_of`.
#[inline(always)]
unsafe fn b15_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b22_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_first_child`.
#[inline(always)]
unsafe fn b16_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b23_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_size`.
#[inline(always)]
unsafe fn b17_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_number`.
#[inline(always)]
unsafe fn b18_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b25_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_f16_next`.
#[inline(always)]
unsafe fn b19_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b26_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let t1 = [vdupq_n_u32(4294967295u32); 2];
	let t2: [uint32x4_t; 2] = [vceqq_u32(x3[0], t1[0]), vceqq_u32(x3[1], t1[1])];
	let mut t3 = [vdupq_n_u32(0); 2];
	let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	let t5: [uint32x4_t; 2] = [vbicq_u32(live[0], t2[0]), vbicq_u32(live[1], t2[1])];
	{
		t3 = [vbslq_u32(t4[0], t1[0], t3[0]), vbslq_u32(t4[1], t1[1], t3[1])];
	}
	if any(&t5) {
		let t6 = b22_lal136r0l_p(l, lid0, t5, x0, arg0, x1i, x3);
		let t7: [uint32x4_t; 2];
		let t8 = bits(&t5);
		if t8.count_ones() <= 2 {
			let t9 = words4(x0);
			let t10 = words2(x1i);
			let t11 = words2(x2);
			let t12 = words2(t6);
			let mut t13 = [0u32; 8];
			let mut t14 = t8;
			while t14 != 0 {
				let j = t14.trailing_zeros() as usize;
				t14 &= t14 - 1;
				let t15 = b21_ssss_f(l, lid0, (((t9[8 + j] as u64) << 32 | t9[j] as u64) as usize as *const u8), (arg0 as *const u8).wrapping_add(((t10[j]) as usize).wrapping_mul(136)), t11[j], t12[j]);
				t13[j] = vgetq_lane_u32::<0>(t15[0]);
			}
			t7 = regs2(t13);
		} else {
			let t16 = b21_lal136r0ll_p(l, lid0, t5, x0, arg0, x1i, x2, t6);
			t7 = t16;
		}
		t3 = [vbslq_u32(t5[0], t7[0], t3[0]), vbslq_u32(t5[1], t7[1], t3[1])];
	}
	res = t3;
	return res;
}

/// `clif_tape_parent`.
#[inline(always)]
unsafe fn b20_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b26_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_f16_next_in`.
#[inline(always)]
unsafe fn b21_ssss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 2] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b22_sss_f(l, lid0, x0, x1, x2);
	x4 = t0;
	let t1 = [vdupq_n_u32(x3); 2];
	let t2: [uint32x4_t; 2] = [vcltq_u32(x4[0], t1[0]), vcltq_u32(x4[1], t1[1])];
	let mut t3 = [vdupq_n_u32(0); 2];
	let t4: [uint32x4_t; 2] = [vmvnq_u32(t2[0]), vmvnq_u32(t2[1])];
	{
		t3 = [vbslq_u32(t2[0], x4[0], t3[0]), vbslq_u32(t2[1], x4[1], t3[1])];
	}
	{
		let t5 = [vdupq_n_u32(4294967295u32); 2];
		t3 = [vbslq_u32(t4[0], t5[0], t3[0]), vbslq_u32(t4[1], t5[1], t3[1])];
	}
	res = t3;
	return res;
}

/// `clif_f16_next_in`.
#[inline(always)]
unsafe fn b21_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b22_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x4 = t0;
	let t1: [uint32x4_t; 2] = [vcltq_u32(x4[0], x3[0]), vcltq_u32(x4[1], x3[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		t2 = [vbslq_u32(t3[0], x4[0], t2[0]), vbslq_u32(t3[1], x4[1], t2[1])];
	}
	{
		let t5 = [vdupq_n_u32(4294967295u32); 2];
		t2 = [vbslq_u32(t4[0], t5[0], t2[0]), vbslq_u32(t4[1], t5[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 2] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b24_sss_f(l, lid0, x0, x1, x2);
	let t1 = b27_sssl_f(l, lid0, x0, x1, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b24_ssl_p(l, lid0, live, x0, x1, x2);
	let t1 = b27_ssll_p(l, lid0, live, x0, x1, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	let t1 = b27_lal136r0ll_p(l, lid0, live, x0, arg0, x1i, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_first_child`.
#[inline(always)]
unsafe fn b23_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	let t1 = [vdupq_n_u32(1u32); 2];
	let t2: [uint32x4_t; 2] = [vcgtq_u32(t0[0], t1[0]), vcgtq_u32(t0[1], t1[1])];
	let mut t3 = [vdupq_n_u32(0); 2];
	let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t2[0]), vandq_u32(live[1], t2[1])];
	let t5: [uint32x4_t; 2] = [vbicq_u32(live[0], t2[0]), vbicq_u32(live[1], t2[1])];
	{
		let t6: [uint32x4_t; 2] = [vaddq_u32(x2[0], t1[0]), vaddq_u32(x2[1], t1[1])];
		t3 = [vbslq_u32(t4[0], t6[0], t3[0]), vbslq_u32(t4[1], t6[1], t3[1])];
	}
	{
		let t7 = [vdupq_n_u32(4294967295u32); 2];
		t3 = [vbslq_u32(t5[0], t7[0], t3[0]), vbslq_u32(t5[1], t7[1], t3[1])];
	}
	res = t3;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b28_sss_f(l, lid0, x0, x1, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = [vdupq_n_u32((t1) as u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(x3[0], t2[0]), vandq_u32(x3[1], t2[1])];
	let t4: u64 = 0u64;
	let t5 = [vdupq_n_u32((t4) as u32); 2];
	let t6: [uint32x4_t; 2] = [vceqq_u32(t3[0], t5[0]), vceqq_u32(t3[1], t5[1])];
	let t7: [uint32x4_t; 2] = [vmvnq_u32(t6[0]), vmvnq_u32(t6[1])];
	let mut t8 = [vdupq_n_u32(0); 2];
	let t9: [uint32x4_t; 2] = [vmvnq_u32(t7[0]), vmvnq_u32(t7[1])];
	{
		let t10 = [vdupq_n_u32(1u32); 2];
		t8 = [vbslq_u32(t7[0], t10[0], t8[0]), vbslq_u32(t7[1], t10[1], t8[1])];
	}
	if any(&t9) {
		let t11: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<1>(x3[0]), vshlq_n_u32::<31>(x3[2])), vorrq_u32(vshrq_n_u32::<1>(x3[1]), vshlq_n_u32::<31>(x3[3])), vshrq_n_u32::<1>(x3[2]), vshrq_n_u32::<1>(x3[3])];
		let t12: [uint32x4_t; 2] = [t11[0], t11[1]];
		let t13: [uint32x4_t; 2];
		let t14 = bits(&t9);
		if t14.count_ones() <= 2 {
			let t15 = words2(t12);
			let mut t16 = [0u32; 8];
			let mut t17 = t14;
			while t17 != 0 {
				let j = t17.trailing_zeros() as usize;
				t17 &= t17 - 1;
				let t18 = b29_ss_f(l, lid0, t15[j], 1u32);
				t16[j] = t18;
			}
			t13 = regs2(t16);
		} else {
			let t19 = b29_ls_p(l, lid0, t9, t12, 1u32);
			t13 = t19;
		}
		t8 = [vbslq_u32(t9[0], t13[0], t8[0]), vbslq_u32(t9[1], t13[1], t8[1])];
	}
	res = t8;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b28_ssl_p(l, lid0, live, x0, x1, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = [vdupq_n_u32((t1) as u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(x3[0], t2[0]), vandq_u32(x3[1], t2[1])];
	let t4: u64 = 0u64;
	let t5 = [vdupq_n_u32((t4) as u32); 2];
	let t6: [uint32x4_t; 2] = [vceqq_u32(t3[0], t5[0]), vceqq_u32(t3[1], t5[1])];
	let t7: [uint32x4_t; 2] = [vmvnq_u32(t6[0]), vmvnq_u32(t6[1])];
	let mut t8 = [vdupq_n_u32(0); 2];
	let t9: [uint32x4_t; 2] = [vandq_u32(live[0], t7[0]), vandq_u32(live[1], t7[1])];
	let t10: [uint32x4_t; 2] = [vbicq_u32(live[0], t7[0]), vbicq_u32(live[1], t7[1])];
	{
		let t11 = [vdupq_n_u32(1u32); 2];
		t8 = [vbslq_u32(t9[0], t11[0], t8[0]), vbslq_u32(t9[1], t11[1], t8[1])];
	}
	if any(&t10) {
		let t12: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<1>(x3[0]), vshlq_n_u32::<31>(x3[2])), vorrq_u32(vshrq_n_u32::<1>(x3[1]), vshlq_n_u32::<31>(x3[3])), vshrq_n_u32::<1>(x3[2]), vshrq_n_u32::<1>(x3[3])];
		let t13: [uint32x4_t; 2] = [t12[0], t12[1]];
		let t14: [uint32x4_t; 2];
		let t15 = bits(&t10);
		if t15.count_ones() <= 2 {
			let t16 = words2(t13);
			let mut t17 = [0u32; 8];
			let mut t18 = t15;
			while t18 != 0 {
				let j = t18.trailing_zeros() as usize;
				t18 &= t18 - 1;
				let t19 = b29_ss_f(l, lid0, t16[j], 1u32);
				t17[j] = t19;
			}
			t14 = regs2(t17);
		} else {
			let t20 = b29_ls_p(l, lid0, t10, t13, 1u32);
			t14 = t20;
		}
		t8 = [vbslq_u32(t10[0], t14[0], t8[0]), vbslq_u32(t10[1], t14[1], t8[1])];
	}
	res = t8;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b28_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = [vdupq_n_u32((t1) as u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(x3[0], t2[0]), vandq_u32(x3[1], t2[1])];
	let t4: u64 = 0u64;
	let t5 = [vdupq_n_u32((t4) as u32); 2];
	let t6: [uint32x4_t; 2] = [vceqq_u32(t3[0], t5[0]), vceqq_u32(t3[1], t5[1])];
	let t7: [uint32x4_t; 2] = [vmvnq_u32(t6[0]), vmvnq_u32(t6[1])];
	let mut t8 = [vdupq_n_u32(0); 2];
	let t9: [uint32x4_t; 2] = [vandq_u32(live[0], t7[0]), vandq_u32(live[1], t7[1])];
	let t10: [uint32x4_t; 2] = [vbicq_u32(live[0], t7[0]), vbicq_u32(live[1], t7[1])];
	{
		let t11 = [vdupq_n_u32(1u32); 2];
		t8 = [vbslq_u32(t9[0], t11[0], t8[0]), vbslq_u32(t9[1], t11[1], t8[1])];
	}
	if any(&t10) {
		let t12: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<1>(x3[0]), vshlq_n_u32::<31>(x3[2])), vorrq_u32(vshrq_n_u32::<1>(x3[1]), vshlq_n_u32::<31>(x3[3])), vshrq_n_u32::<1>(x3[2]), vshrq_n_u32::<1>(x3[3])];
		let t13: [uint32x4_t; 2] = [t12[0], t12[1]];
		let t14: [uint32x4_t; 2];
		let t15 = bits(&t10);
		if t15.count_ones() <= 2 {
			let t16 = words2(t13);
			let mut t17 = [0u32; 8];
			let mut t18 = t15;
			while t18 != 0 {
				let j = t18.trailing_zeros() as usize;
				t18 &= t18 - 1;
				let t19 = b29_ss_f(l, lid0, t16[j], 1u32);
				t17[j] = t19;
			}
			t14 = regs2(t17);
		} else {
			let t20 = b29_ls_p(l, lid0, t10, t13, 1u32);
			t14 = t20;
		}
		t8 = [vbslq_u32(t10[0], t14[0], t8[0]), vbslq_u32(t10[1], t14[1], t8[1])];
	}
	res = t8;
	return res;
}

/// `clif_f16_number`.
#[inline(always)]
unsafe fn b25_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 2];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vshlq_u32(x2[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x2[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	let t1 = b31_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, t0);
	let t2 = [vdupq_n_u32(1023u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(x2[0], t2[0]), vandq_u32(x2[1], t2[1])];
	let t4 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, t1, t3, 1u32);
	let t5: [uint32x4_t; 2] = [t4[0], t4[1]];
	x3 = t5;
	let t6 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 76u32);
	x4 = t6;
	let t7: [uint32x4_t; 2] = [vcltq_u32(x3[0], x4[0]), vcltq_u32(x3[1], x4[1])];
	let t8: [uint32x4_t; 2] = [vandq_u32(live[0], t7[0]), vandq_u32(live[1], t7[1])];
	{
		res = [vbslq_u32(t8[0], x3[0], res[0]), vbslq_u32(t8[1], x3[1], res[1])];
		live = [vbicq_u32(live[0], t8[0]), vbicq_u32(live[1], t8[1])];
	}
	if any(&live) {
		let t9 = b33_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, x2);
		let t10: [uint32x4_t; 2] = [t9[0], t9[1]];
		x5 = t10;
		let t11: [uint32x4_t; 2] = [vsubq_u32(x3[0], x4[0]), vsubq_u32(x3[1], x4[1])];
		let t12 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 80u32);
		let t13: [uint32x4_t; 2] = [vmulq_u32(t11[0], t12[0]), vmulq_u32(t11[1], t12[1])];
		let t14: [uint32x4_t; 2] = [vaddq_u32(x4[0], t13[0]), vaddq_u32(x4[1], t13[1])];
		let t15: [uint32x4_t; 2] = [vaddq_u32(t14[0], x5[0]), vaddq_u32(t14[1], x5[1])];
		res = [vbslq_u32(live[0], t15[0], res[0]), vbslq_u32(live[1], t15[1], res[1])];
		live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
	}
	res
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b26_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b34_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let mut t1 = live;
	loop {
		// Lanes part: each its own rounds.
		let t2: u64 = (1u64).wrapping_shl((32u64) as u32);
		let t3 = [vdupq_n_u32((t2) as u32); 2];
		let t4 = [vdupq_n_u32(((t2) >> 32) as u32); 2];
		let t5: [uint32x4_t; 4] = [vandq_u32(x3[0], t3[0]), vandq_u32(x3[1], t3[1]), vandq_u32(x3[2], t4[0]), vandq_u32(x3[3], t4[1])];
		let t6: u64 = 0u64;
		let t7 = [vdupq_n_u32((t6) as u32); 2];
		let t8: [uint32x4_t; 2] = [veq64q_u32(t5[0], t5[2], t7[0], vdupq_n_u32(0)), veq64q_u32(t5[1], t5[3], t7[1], vdupq_n_u32(0))];
		let t9: [uint32x4_t; 2] = [vmvnq_u32(t8[0]), vmvnq_u32(t8[1])];
		t1 = [vandq_u32(t1[0], t9[0]), vandq_u32(t1[1], t9[1])];
		if !(any(&t1)) { break; }
		let t10: [uint32x4_t; 2] = [x3[0], x3[1]];
		let t11 = b34_lal136r0l_p(l, lid0, t1, x0, arg0, x1i, t10);
		x3 = [vbslq_u32(t1[0], t11[0], x3[0]), vbslq_u32(t1[1], t11[1], x3[1]), vbslq_u32(t1[0], t11[2], x3[2]), vbslq_u32(t1[1], t11[3], x3[3])];
	}
	let t12: [uint32x4_t; 2] = [x3[0], x3[1]];
	res = t12;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b27_sssl_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x2); 2];
	let t1: [uint32x4_t; 2] = [vaddq_u32(t0[0], x3[0]), vaddq_u32(t0[1], x3[1])];
	let t2 = b35_ls_f(l, lid0, t1, (x2).wrapping_add(1u32));
	let mut t3: bool = ((16u32) != (0u32));
	if t3 {
		t3 = ((4u32) == (12u32));
	}
	let t4: u32;
	if t3 {
		t4 = 16u32;
	} else {
		t4 = (x1.wrapping_add(((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
	}
	let t5 = b36_ls_f(l, lid0, t2, t4);
	let t6 = b35_ls_f(l, lid0, t5, (x2).wrapping_add(1u32));
	res = t6;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b27_ssll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(x2[0], x3[0]), vaddq_u32(x2[1], x3[1])];
	let t1 = [vdupq_n_u32(1u32); 2];
	let t2: [uint32x4_t; 2] = [vaddq_u32(x2[0], t1[0]), vaddq_u32(x2[1], t1[1])];
	let t3 = b35_ll_p(l, lid0, live, t0, t2);
	let mut t4: bool = ((16u32) != (0u32));
	if t4 {
		t4 = ((4u32) == (12u32));
	}
	let t5: u32;
	if t4 {
		t5 = 16u32;
	} else {
		t5 = (x1.wrapping_add(((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
	}
	let t6 = b36_ls_p(l, lid0, live, t3, t5);
	let t7: [uint32x4_t; 2] = [vaddq_u32(x2[0], t1[0]), vaddq_u32(x2[1], t1[1])];
	let t8 = b35_ll_p(l, lid0, live, t6, t7);
	res = t8;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b27_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(x2[0], x3[0]), vaddq_u32(x2[1], x3[1])];
	let t1 = [vdupq_n_u32(1u32); 2];
	let t2: [uint32x4_t; 2] = [vaddq_u32(x2[0], t1[0]), vaddq_u32(x2[1], t1[1])];
	let t3 = b35_ll_p(l, lid0, live, t0, t2);
	let mut t4: bool = ((16u32) != (0u32));
	if t4 {
		t4 = ((4u32) == (12u32));
	}
	let t5: [uint32x4_t; 2];
	if t4 {
		let t6 = [vdupq_n_u32(16u32); 2];
		t5 = t6;
	} else {
		let t7 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t8: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t7), vbslq_u32(live[1], x1i[1], t7)];
		let mut t9 = [vdupq_n_u32(0); 2];
		let t10 = minv(&t8);
		if t10 == maxv(&t8) {
			t9 = [vdupq_n_u32(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(t10 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
		} else {
			t9[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<0>(t8[0]) as usize * 136) as *const u32).read_unaligned() as u32), t9[0]);
			t9[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<1>(t8[0]) as usize * 136) as *const u32).read_unaligned() as u32), t9[0]);
			t9[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<2>(t8[0]) as usize * 136) as *const u32).read_unaligned() as u32), t9[0]);
			t9[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<3>(t8[0]) as usize * 136) as *const u32).read_unaligned() as u32), t9[0]);
			t9[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<0>(t8[1]) as usize * 136) as *const u32).read_unaligned() as u32), t9[1]);
			t9[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<1>(t8[1]) as usize * 136) as *const u32).read_unaligned() as u32), t9[1]);
			t9[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<2>(t8[1]) as usize * 136) as *const u32).read_unaligned() as u32), t9[1]);
			t9[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<3>(t8[1]) as usize * 136) as *const u32).read_unaligned() as u32), t9[1]);
		}
		t5 = t9;
	}
	let t11 = b36_ll_p(l, lid0, live, t3, t5);
	let t12: [uint32x4_t; 2] = [vaddq_u32(x2[0], t1[0]), vaddq_u32(x2[1], t1[1])];
	let t13 = b35_ll_p(l, lid0, live, t11, t12);
	res = t13;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 4] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 4];
	let t0: u64 = ((x0) as usize as u64);
	let t1 = [vdupq_n_u32((t0) as u32); 2];
	let t2 = [vdupq_n_u32(((t0) >> 32) as u32); 2];
	let t3: [uint32x4_t; 4] = [t1[0], t1[1], t2[0], t2[1]];
	let t4 = [vdupq_n_u32(x2); 2];
	let t5 = b37_lssl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], t3, x1, 0u32, t4);
	res = t5;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: u64 = ((x0) as usize as u64);
	let t1 = [vdupq_n_u32((t0) as u32); 2];
	let t2 = [vdupq_n_u32(((t0) >> 32) as u32); 2];
	let t3: [uint32x4_t; 4] = [t1[0], t1[1], t2[0], t2[1]];
	let t4 = b37_lssl_p(l, lid0, live, t3, x1, 0u32, x2);
	res = t4;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b37_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, x2);
	res = t0;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b29_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res: u32 = 0;
	let t0: u32;
	if ((x0) < (x1)) {
		t0 = x1;
	} else {
		t0 = x0;
	}
	res = t0;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b29_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x1); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(x0[0], t0[0]), vcltq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		let t5 = [vdupq_n_u32(x1); 2];
		t2 = [vbslq_u32(t3[0], t5[0], t2[0]), vbslq_u32(t3[1], t5[1], t2[1])];
	}
	{
		t2 = [vbslq_u32(t4[0], x0[0], t2[0]), vbslq_u32(t4[1], x0[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lssls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32, x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b38_lsls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lslls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b38_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lal136r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: [uint32x4_t; 2], x4: u32) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b38_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b31_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b32_lss_p(l, lid0, live, x0, x1, 40u32);
	let t1 = [vdupq_n_u32(t0); 2];
	let t2: [uint32x4_t; 2] = [vmulq_u32(x3[0], t1[0]), vmulq_u32(x3[1], t1[1])];
	let t3 = b39_lss_p(l, lid0, live, x0, x1, 0u32);
	let t4 = [vdupq_n_u32(t3); 2];
	let t5: [uint32x4_t; 2] = [vaddq_u32(t2[0], t4[0]), vaddq_u32(t2[1], t4[1])];
	x4 = t5;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t6 = b32_lss_p(l, lid0, live, x0, x1, 4u32);
	let t7: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
	let t8 = [vdupq_n_u32(t6); 2];
	let t9: [uint32x4_t; 2] = [vsubq_u32(t8[0], t7[0]), vsubq_u32(t8[1], t7[1])];
	let t10 = b36_ls_p(l, lid0, live, t9, 1024u32);
	x5 = t10;
	let t11 = b39_lss_p(l, lid0, live, x0, x1, x2);
	let t12 = [vdupq_n_u32(t11); 2];
	let t13: [uint32x4_t; 2] = [vmulq_u32(x5[0], t12[0]), vmulq_u32(x5[1], t12[1])];
	let t14: [uint32x4_t; 2] = [vaddq_u32(x4[0], t13[0]), vaddq_u32(x4[1], t13[1])];
	res = t14;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b31_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 40u32);
	let t1: [uint32x4_t; 2] = [vmulq_u32(x3[0], t0[0]), vmulq_u32(x3[1], t0[1])];
	let t2 = b39_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 0u32);
	let t3: [uint32x4_t; 2] = [vaddq_u32(t1[0], t2[0]), vaddq_u32(t1[1], t2[1])];
	x4 = t3;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t4 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t5: [uint32x4_t; 2] = [vshlq_u32(x3[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x3[1], vdupq_n_s32(((10u32) & 31) as i32))];
	let t6: [uint32x4_t; 2] = [vsubq_u32(t4[0], t5[0]), vsubq_u32(t4[1], t5[1])];
	let t7 = b36_ls_p(l, lid0, live, t6, 1024u32);
	x5 = t7;
	let t8 = b39_lal136r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	let t9: [uint32x4_t; 2] = [vmulq_u32(x5[0], t8[0]), vmulq_u32(x5[1], t8[1])];
	let t10: [uint32x4_t; 2] = [vaddq_u32(x4[0], t9[0]), vaddq_u32(x4[1], t9[1])];
	res = t10;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b32_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((x2) == (12u32));
	}
	let t1: u32;
	if t0 {
		t1 = 16u32;
	} else {
		t1 = (x1.wrapping_add(((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
	}
	res = t1;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b32_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((x2) == (12u32));
	}
	let t1: [uint32x4_t; 2];
	if t0 {
		let t2 = [vdupq_n_u32(16u32); 2];
		t1 = t2;
	} else {
		let t3 = vdupq_n_u32(maxv_on(&live, &x1i));
		let t4: [uint32x4_t; 2] = [vbslq_u32(live[0], x1i[0], t3), vbslq_u32(live[1], x1i[1], t3)];
		let mut t5 = [vdupq_n_u32(0); 2];
		let t6 = minv(&t4);
		if t6 == maxv(&t4) {
			t5 = [vdupq_n_u32(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(t6 as usize * 136) as *const u32).read_unaligned() as u32)); 2];
		} else {
			t5[0] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<0>(t4[0]) as usize * 136) as *const u32).read_unaligned() as u32), t5[0]);
			t5[0] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<1>(t4[0]) as usize * 136) as *const u32).read_unaligned() as u32), t5[0]);
			t5[0] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<2>(t4[0]) as usize * 136) as *const u32).read_unaligned() as u32), t5[0]);
			t5[0] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<3>(t4[0]) as usize * 136) as *const u32).read_unaligned() as u32), t5[0]);
			t5[1] = vsetq_lane_u32::<0>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<0>(t4[1]) as usize * 136) as *const u32).read_unaligned() as u32), t5[1]);
			t5[1] = vsetq_lane_u32::<1>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<1>(t4[1]) as usize * 136) as *const u32).read_unaligned() as u32), t5[1]);
			t5[1] = vsetq_lane_u32::<2>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<2>(t4[1]) as usize * 136) as *const u32).read_unaligned() as u32), t5[1]);
			t5[1] = vsetq_lane_u32::<3>(((((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8).add(vgetq_lane_u32::<3>(t4[1]) as usize * 136) as *const u32).read_unaligned() as u32), t5[1]);
		}
		t1 = t5;
	}
	res = t1;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b33_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4: u32 = 0;
	let mut x5: u32 = 0;
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut x8 = [vdupq_n_u32(0); 2];
	let mut x9: u32 = 0;
	let mut x10 = [vdupq_n_u32(0); 2];
	let mut x11 = [vdupq_n_u32(0); 2];
	let mut x12 = [vdupq_n_u32(0); 2];
	let mut x13 = [vdupq_n_u32(0); 2];
	let mut x14 = [vdupq_n_u32(0); 2];
	let mut x15 = [vdupq_n_u32(0); 2];
	let mut x16: u64 = 0;
	let mut x17 = [vdupq_n_u32(0); 4];
	let mut x18 = [vdupq_n_u32(0); 2];
	let mut x19 = [vdupq_n_u32(0); 2];
	let mut x20 = [vdupq_n_u32(0); 4];
	let mut x21 = [vdupq_n_u32(0); 4];
	let mut x22 = [vdupq_n_u32(0); 4];
	let mut x23 = [vdupq_n_u32(0); 2];
	let mut x24: u32 = 0;
	let mut x25 = [vdupq_n_u32(0); 2];
	let mut x26 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b32_lss_p(l, lid0, live, x0, x1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b32_lss_p(l, lid0, live, x0, x1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	x6 = t2;
	let t3 = [vdupq_n_u32(4u32); 2];
	let t4: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t5 = b30_lssls_p(l, lid0, live, x0, x1, x5, t4, 4u32);
	let t6: [uint32x4_t; 2] = [t5[0], t5[1]];
	x7 = t6;
	let t7: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t8: [uint32x4_t; 2] = [vaddq_u32(t7[0], t3[0]), vaddq_u32(t7[1], t3[1])];
	let t9 = b30_lssls_p(l, lid0, live, x0, x1, x5, t8, 4u32);
	let t10: [uint32x4_t; 2] = [t9[0], t9[1]];
	x8 = t10;
	let t11 = b32_lss_p(l, lid0, live, x0, x1, 4u32);
	x9 = ((t11).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
	let t12: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t13 = [vdupq_n_u32(x9); 2];
	let t14: [uint32x4_t; 2] = [vsubq_u32(t13[0], t12[0]), vsubq_u32(t13[1], t12[1])];
	let t15 = b1_sl_p(l, lid0, live, 4u32, t14);
	x10 = t15;
	let t16: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
	let t17 = [vdupq_n_u32(3u32); 2];
	let t18: [uint32x4_t; 2] = [vandq_u32(t16[0], t17[0]), vandq_u32(t16[1], t17[1])];
	x11 = t18;
	let t19 = [vdupq_n_u32(0u32); 2];
	let t20: [uint32x4_t; 2] = [vceqq_u32(x11[0], t19[0]), vceqq_u32(x11[1], t19[1])];
	let mut t21 = [vdupq_n_u32(0); 2];
	let t22: [uint32x4_t; 2] = [vandq_u32(live[0], t20[0]), vandq_u32(live[1], t20[1])];
	let t23: [uint32x4_t; 2] = [vbicq_u32(live[0], t20[0]), vbicq_u32(live[1], t20[1])];
	{
		t21 = [vbslq_u32(t22[0], t19[0], t21[0]), vbslq_u32(t22[1], t19[1], t21[1])];
	}
	if any(&t23) {
		let t24 = [vdupq_n_u32(1u32); 2];
		let t25: [uint32x4_t; 2] = [vsubq_u32(x11[0], t24[0]), vsubq_u32(x11[1], t24[1])];
		let t26 = [vdupq_n_u32(2u32); 2];
		let t27: [uint32x4_t; 2] = [vmulq_u32(t26[0], t25[0]), vmulq_u32(t26[1], t25[1])];
		let t28: [uint32x4_t; 2] = [vaddq_u32(x7[0], t27[0]), vaddq_u32(x7[1], t27[1])];
		let t29 = b36_ll_p(l, lid0, t23, t28, x8);
		let t30 = b30_lssls_p(l, lid0, t23, x0, x1, x4, t29, 2u32);
		let t31: [uint32x4_t; 2] = [t30[0], t30[1]];
		t21 = [vbslq_u32(t23[0], t31[0], t21[0]), vbslq_u32(t23[1], t31[1], t21[1])];
	}
	x12 = t21;
	let t32 = [vdupq_n_u32(2u32); 2];
	let t33: [uint32x4_t; 2] = [vmulq_u32(t32[0], x11[0]), vmulq_u32(t32[1], x11[1])];
	let t34: [uint32x4_t; 2] = [vaddq_u32(x7[0], t33[0]), vaddq_u32(x7[1], t33[1])];
	let t35 = b36_ll_p(l, lid0, live, t34, x8);
	let t36 = b30_lssls_p(l, lid0, live, x0, x1, x4, t35, 2u32);
	let t37: [uint32x4_t; 2] = [t36[0], t36[1]];
	x13 = t37;
	let t38: [uint32x4_t; 2] = [vmulq_u32(t32[0], x10[0]), vmulq_u32(t32[1], x10[1])];
	let t39: [uint32x4_t; 2] = [vaddq_u32(x7[0], t38[0]), vaddq_u32(x7[1], t38[1])];
	x14 = t39;
	let t40 = [vdupq_n_u32(255u32); 2];
	let t41: [uint32x4_t; 2] = [vandq_u32(x3[0], t40[0]), vandq_u32(x3[1], t40[1])];
	x15 = t41;
	x16 = 9259542123273814144u64;
	let t42: u64 = 72340172838076673u64;
	let t43 = [vdupq_n_u32((t42) as u32); 2];
	let t44 = [vdupq_n_u32(((t42) >> 32) as u32); 2];
	let t45: [uint32x4_t; 2] = [vmulhiq_u32(x15[0], t43[0]), vmulhiq_u32(x15[1], t43[1])];
	let t46: [uint32x4_t; 4] = [vmulq_u32(x15[0], t43[0]), vmulq_u32(x15[1], t43[1]), vmlaq_u32(t45[0], x15[0], t44[0]), vmlaq_u32(t45[1], x15[1], t44[1])];
	x17 = t46;
	x18 = x12;
	x19 = x12;
	let mut t47 = live;
	loop {
		// Lanes part: each its own rounds.
		let t48: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
		t47 = [vandq_u32(t47[0], t48[0]), vandq_u32(t47[1], t48[1])];
		if !(any(&t47)) { break; }
		let t49: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
		let t50 = b36_ll_p(l, lid0, t47, t49, x8);
		let t51 = b30_lssls_p(l, lid0, t47, x0, x1, x4, t50, 8u32);
		x20 = t51;
		let t52: u64 = x16;
		let t53 = [vdupq_n_u32((t52) as u32); 2];
		let t54 = [vdupq_n_u32(((t52) >> 32) as u32); 2];
		let t55: [uint32x4_t; 4] = [vorrq_u32(x20[0], t53[0]), vorrq_u32(x20[1], t53[1]), vorrq_u32(x20[2], t54[0]), vorrq_u32(x20[3], t54[1])];
		let t56: u64 = !(x16);
		let t57 = [vdupq_n_u32((t56) as u32); 2];
		let t58 = [vdupq_n_u32(((t56) >> 32) as u32); 2];
		let t59: [uint32x4_t; 4] = [vandq_u32(x17[0], t57[0]), vandq_u32(x17[1], t57[1]), vandq_u32(x17[2], t58[0]), vandq_u32(x17[3], t58[1])];
		let t60: [uint32x4_t; 2] = [vcltq_u32(t55[0], t59[0]), vcltq_u32(t55[1], t59[1])];
		let t61: [uint32x4_t; 4] = [vsubq_u32(t55[0], t59[0]), vsubq_u32(t55[1], t59[1]), vsbcq_u32(t55[2], t59[2], t60[0]), vsbcq_u32(t55[3], t59[3], t60[1])];
		x21 = t61;
		let t62: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t63: [uint32x4_t; 4] = [vandq_u32(t62[0], x17[0]), vandq_u32(t62[1], x17[1]), vandq_u32(t62[2], x17[2]), vandq_u32(t62[3], x17[3])];
		let t64: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
		let t65: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t64[r]));
		let t66: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
		let t67: [uint32x4_t; 4] = [vandq_u32(t65[0], t66[0]), vandq_u32(t65[1], t66[1]), vandq_u32(t65[2], t66[2]), vandq_u32(t65[3], t66[3])];
		let t68: [uint32x4_t; 4] = [vorrq_u32(t63[0], t67[0]), vorrq_u32(t63[1], t67[1]), vorrq_u32(t63[2], t67[2]), vorrq_u32(t63[3], t67[3])];
		let t69: u64 = x16;
		let t70 = [vdupq_n_u32((t69) as u32); 2];
		let t71 = [vdupq_n_u32(((t69) >> 32) as u32); 2];
		let t72: [uint32x4_t; 4] = [vandq_u32(t70[0], t68[0]), vandq_u32(t70[1], t68[1]), vandq_u32(t71[0], t68[2]), vandq_u32(t71[1], t68[3])];
		x22 = t72;
		let t73: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
		x23 = t73;
		let t74 = [vdupq_n_u32(8u32); 2];
		let t75: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t74[0]), vcgeq_u32(x23[1], t74[1])];
		let mut t76 = [vdupq_n_u32(0); 4];
		let t77: [uint32x4_t; 2] = [vandq_u32(t47[0], t75[0]), vandq_u32(t47[1], t75[1])];
		let t78: [uint32x4_t; 2] = [vbicq_u32(t47[0], t75[0]), vbicq_u32(t47[1], t75[1])];
		{
			t76 = [vbslq_u32(t77[0], x22[0], t76[0]), vbslq_u32(t77[1], x22[1], t76[1]), vbslq_u32(t77[0], x22[2], t76[2]), vbslq_u32(t77[1], x22[3], t76[3])];
		}
		{
			let t79: [uint32x4_t; 2] = [vmulq_u32(t74[0], x23[0]), vmulq_u32(t74[1], x23[1])];
			let t80: u64 = 1u64;
			let t81 = [vdupq_n_u32((t80) as u32); 2];
			let t82: [uint32x4_t; 2] = [t79[0], t79[1]];
			let t83: [uint32x4_t; 2] = [vandq_u32(t82[0], vdupq_n_u32(63)), vandq_u32(t82[1], vdupq_n_u32(63))];
			let t84: [uint32x4_t; 4] = [vshlq_u32(t81[0], vreinterpretq_s32_u32(t83[0])), vshlq_u32(t81[1], vreinterpretq_s32_u32(t83[1])), vshlq_u32(t81[0], vsubq_s32(vreinterpretq_s32_u32(t83[0]), vdupq_n_s32(32))), vshlq_u32(t81[1], vsubq_s32(vreinterpretq_s32_u32(t83[1]), vdupq_n_s32(32)))];
			let t85: u64 = 1u64;
			let t86 = [vdupq_n_u32((t85) as u32); 2];
			let t87: [uint32x4_t; 2] = [vcltq_u32(t84[0], t86[0]), vcltq_u32(t84[1], t86[1])];
			let t88: [uint32x4_t; 4] = [vsubq_u32(t84[0], t86[0]), vsubq_u32(t84[1], t86[1]), vsbcq_u32(t84[2], vdupq_n_u32(0), t87[0]), vsbcq_u32(t84[3], vdupq_n_u32(0), t87[1])];
			let t89: [uint32x4_t; 4] = [vandq_u32(x22[0], t88[0]), vandq_u32(x22[1], t88[1]), vandq_u32(x22[2], t88[2]), vandq_u32(x22[3], t88[3])];
			t76 = [vbslq_u32(t78[0], t89[0], t76[0]), vbslq_u32(t78[1], t89[1], t76[1]), vbslq_u32(t78[0], t89[2], t76[2]), vbslq_u32(t78[1], t89[3], t76[3])];
		}
		let t90 = b41_l_p(l, lid0, t47, t76);
		let t91: [uint32x4_t; 2] = [vaddq_u32(x18[0], t90[0]), vaddq_u32(x18[1], t90[1])];
		x18 = [vbslq_u32(t47[0], t91[0], x18[0]), vbslq_u32(t47[1], t91[1], x18[1])];
		let t92: [uint32x4_t; 2] = [vandq_u32(t47[0], live[0]), vandq_u32(t47[1], live[1])];
		let t93: [uint32x4_t; 2] = [vaddq_u32(x19[0], t74[0]), vaddq_u32(x19[1], t74[1])];
		x19 = t93;
	}
	x12 = x18;
	let t94 = b32_lss_p(l, lid0, live, x0, x1, 16u32);
	x24 = (((t94).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
	let t95 = [vdupq_n_u32(1u32); 2];
	let t96: [uint32x4_t; 2] = [vsubq_u32(x10[0], t95[0]), vsubq_u32(x10[1], t95[1])];
	let t97: [uint32x4_t; 2] = [vmulq_u32(t32[0], t96[0]), vmulq_u32(t32[1], t96[1])];
	let t98: [uint32x4_t; 2] = [vaddq_u32(x7[0], t97[0]), vaddq_u32(x7[1], t97[1])];
	let t99 = b36_ll_p(l, lid0, live, t98, x8);
	let t100 = b30_lssls_p(l, lid0, live, x0, x1, x4, t99, 2u32);
	let t101: [uint32x4_t; 2] = [t100[0], t100[1]];
	x25 = t101;
	let t102: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
	let t103 = [vdupq_n_u32(x24); 2];
	let t104: [uint32x4_t; 2] = [vmulq_u32(x12[0], t103[0]), vmulq_u32(x12[1], t103[1])];
	let t105: [uint32x4_t; 2] = [vaddq_u32(t102[0], t104[0]), vaddq_u32(t102[1], t104[1])];
	let t106 = b36_ll_p(l, lid0, live, t105, x8);
	x26 = t106;
	let t107 = b30_lssls_p(l, lid0, live, x0, x1, x4, x26, 8u32);
	let t108 = b42_s_p(l, lid0, live, x24);
	let t109: u64 = t108;
	let t110 = [vdupq_n_u32((t109) as u32); 2];
	let t111 = [vdupq_n_u32(((t109) >> 32) as u32); 2];
	let t112: [uint32x4_t; 4] = [vandq_u32(t107[0], t110[0]), vandq_u32(t107[1], t110[1]), vandq_u32(t107[2], t111[0]), vandq_u32(t107[3], t111[1])];
	res = t112;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b33_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut x7 = [vdupq_n_u32(0); 2];
	let mut x8 = [vdupq_n_u32(0); 2];
	let mut x9 = [vdupq_n_u32(0); 2];
	let mut x10 = [vdupq_n_u32(0); 2];
	let mut x11 = [vdupq_n_u32(0); 2];
	let mut x12 = [vdupq_n_u32(0); 2];
	let mut x13 = [vdupq_n_u32(0); 2];
	let mut x14 = [vdupq_n_u32(0); 2];
	let mut x15 = [vdupq_n_u32(0); 2];
	let mut x16: u64 = 0;
	let mut x17 = [vdupq_n_u32(0); 4];
	let mut x18 = [vdupq_n_u32(0); 2];
	let mut x19 = [vdupq_n_u32(0); 2];
	let mut x20 = [vdupq_n_u32(0); 4];
	let mut x21 = [vdupq_n_u32(0); 4];
	let mut x22 = [vdupq_n_u32(0); 4];
	let mut x23 = [vdupq_n_u32(0); 2];
	let mut x24 = [vdupq_n_u32(0); 2];
	let mut x25 = [vdupq_n_u32(0); 2];
	let mut x26 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	x6 = t2;
	let t3 = [vdupq_n_u32(4u32); 2];
	let t4: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t5 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t4, 4u32);
	let t6: [uint32x4_t; 2] = [t5[0], t5[1]];
	x7 = t6;
	let t7: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t8: [uint32x4_t; 2] = [vaddq_u32(t7[0], t3[0]), vaddq_u32(t7[1], t3[1])];
	let t9 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t8, 4u32);
	let t10: [uint32x4_t; 2] = [t9[0], t9[1]];
	x8 = t10;
	let t11 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t12 = [vdupq_n_u32(255u32); 2];
	let t13: [uint32x4_t; 2] = [vaddq_u32(t11[0], t12[0]), vaddq_u32(t11[1], t12[1])];
	let t14: [uint32x4_t; 2] = [vshlq_u32(t13[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t13[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
	x9 = t14;
	let t15: [uint32x4_t; 2] = [vmulq_u32(x6[0], t3[0]), vmulq_u32(x6[1], t3[1])];
	let t16: [uint32x4_t; 2] = [vsubq_u32(x9[0], t15[0]), vsubq_u32(x9[1], t15[1])];
	let t17 = b1_sl_p(l, lid0, live, 4u32, t16);
	x10 = t17;
	let t18: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
	let t19 = [vdupq_n_u32(3u32); 2];
	let t20: [uint32x4_t; 2] = [vandq_u32(t18[0], t19[0]), vandq_u32(t18[1], t19[1])];
	x11 = t20;
	let t21 = [vdupq_n_u32(0u32); 2];
	let t22: [uint32x4_t; 2] = [vceqq_u32(x11[0], t21[0]), vceqq_u32(x11[1], t21[1])];
	let mut t23 = [vdupq_n_u32(0); 2];
	let t24: [uint32x4_t; 2] = [vandq_u32(live[0], t22[0]), vandq_u32(live[1], t22[1])];
	let t25: [uint32x4_t; 2] = [vbicq_u32(live[0], t22[0]), vbicq_u32(live[1], t22[1])];
	{
		t23 = [vbslq_u32(t24[0], t21[0], t23[0]), vbslq_u32(t24[1], t21[1], t23[1])];
	}
	if any(&t25) {
		let t26 = [vdupq_n_u32(1u32); 2];
		let t27: [uint32x4_t; 2] = [vsubq_u32(x11[0], t26[0]), vsubq_u32(x11[1], t26[1])];
		let t28 = [vdupq_n_u32(2u32); 2];
		let t29: [uint32x4_t; 2] = [vmulq_u32(t28[0], t27[0]), vmulq_u32(t28[1], t27[1])];
		let t30: [uint32x4_t; 2] = [vaddq_u32(x7[0], t29[0]), vaddq_u32(x7[1], t29[1])];
		let t31 = b36_ll_p(l, lid0, t25, t30, x8);
		let t32 = b30_lal136r0lls_p(l, lid0, t25, x0, arg0, x1i, x4, t31, 2u32);
		let t33: [uint32x4_t; 2] = [t32[0], t32[1]];
		t23 = [vbslq_u32(t25[0], t33[0], t23[0]), vbslq_u32(t25[1], t33[1], t23[1])];
	}
	x12 = t23;
	let t34 = [vdupq_n_u32(2u32); 2];
	let t35: [uint32x4_t; 2] = [vmulq_u32(t34[0], x11[0]), vmulq_u32(t34[1], x11[1])];
	let t36: [uint32x4_t; 2] = [vaddq_u32(x7[0], t35[0]), vaddq_u32(x7[1], t35[1])];
	let t37 = b36_ll_p(l, lid0, live, t36, x8);
	let t38 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t37, 2u32);
	let t39: [uint32x4_t; 2] = [t38[0], t38[1]];
	x13 = t39;
	let t40: [uint32x4_t; 2] = [vmulq_u32(t34[0], x10[0]), vmulq_u32(t34[1], x10[1])];
	let t41: [uint32x4_t; 2] = [vaddq_u32(x7[0], t40[0]), vaddq_u32(x7[1], t40[1])];
	x14 = t41;
	let t42: [uint32x4_t; 2] = [vandq_u32(x3[0], t12[0]), vandq_u32(x3[1], t12[1])];
	x15 = t42;
	x16 = 9259542123273814144u64;
	let t43: u64 = 72340172838076673u64;
	let t44 = [vdupq_n_u32((t43) as u32); 2];
	let t45 = [vdupq_n_u32(((t43) >> 32) as u32); 2];
	let t46: [uint32x4_t; 2] = [vmulhiq_u32(x15[0], t44[0]), vmulhiq_u32(x15[1], t44[1])];
	let t47: [uint32x4_t; 4] = [vmulq_u32(x15[0], t44[0]), vmulq_u32(x15[1], t44[1]), vmlaq_u32(t46[0], x15[0], t45[0]), vmlaq_u32(t46[1], x15[1], t45[1])];
	x17 = t47;
	x18 = x12;
	x19 = x12;
	let mut t48 = live;
	loop {
		// Lanes part: each its own rounds.
		let t49: [uint32x4_t; 2] = [vcltq_u32(x19[0], x13[0]), vcltq_u32(x19[1], x13[1])];
		t48 = [vandq_u32(t48[0], t49[0]), vandq_u32(t48[1], t49[1])];
		if !(any(&t48)) { break; }
		let t50: [uint32x4_t; 2] = [vaddq_u32(x14[0], x19[0]), vaddq_u32(x14[1], x19[1])];
		let t51 = b36_ll_p(l, lid0, t48, t50, x8);
		let t52 = b30_lal136r0lls_p(l, lid0, t48, x0, arg0, x1i, x4, t51, 8u32);
		x20 = t52;
		let t53: u64 = x16;
		let t54 = [vdupq_n_u32((t53) as u32); 2];
		let t55 = [vdupq_n_u32(((t53) >> 32) as u32); 2];
		let t56: [uint32x4_t; 4] = [vorrq_u32(x20[0], t54[0]), vorrq_u32(x20[1], t54[1]), vorrq_u32(x20[2], t55[0]), vorrq_u32(x20[3], t55[1])];
		let t57: u64 = !(x16);
		let t58 = [vdupq_n_u32((t57) as u32); 2];
		let t59 = [vdupq_n_u32(((t57) >> 32) as u32); 2];
		let t60: [uint32x4_t; 4] = [vandq_u32(x17[0], t58[0]), vandq_u32(x17[1], t58[1]), vandq_u32(x17[2], t59[0]), vandq_u32(x17[3], t59[1])];
		let t61: [uint32x4_t; 2] = [vcltq_u32(t56[0], t60[0]), vcltq_u32(t56[1], t60[1])];
		let t62: [uint32x4_t; 4] = [vsubq_u32(t56[0], t60[0]), vsubq_u32(t56[1], t60[1]), vsbcq_u32(t56[2], t60[2], t61[0]), vsbcq_u32(t56[3], t60[3], t61[1])];
		x21 = t62;
		let t63: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t64: [uint32x4_t; 4] = [vandq_u32(t63[0], x17[0]), vandq_u32(t63[1], x17[1]), vandq_u32(t63[2], x17[2]), vandq_u32(t63[3], x17[3])];
		let t65: [uint32x4_t; 4] = [veorq_u32(x20[0], x17[0]), veorq_u32(x20[1], x17[1]), veorq_u32(x20[2], x17[2]), veorq_u32(x20[3], x17[3])];
		let t66: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t65[r]));
		let t67: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x21[r]));
		let t68: [uint32x4_t; 4] = [vandq_u32(t66[0], t67[0]), vandq_u32(t66[1], t67[1]), vandq_u32(t66[2], t67[2]), vandq_u32(t66[3], t67[3])];
		let t69: [uint32x4_t; 4] = [vorrq_u32(t64[0], t68[0]), vorrq_u32(t64[1], t68[1]), vorrq_u32(t64[2], t68[2]), vorrq_u32(t64[3], t68[3])];
		let t70: u64 = x16;
		let t71 = [vdupq_n_u32((t70) as u32); 2];
		let t72 = [vdupq_n_u32(((t70) >> 32) as u32); 2];
		let t73: [uint32x4_t; 4] = [vandq_u32(t71[0], t69[0]), vandq_u32(t71[1], t69[1]), vandq_u32(t72[0], t69[2]), vandq_u32(t72[1], t69[3])];
		x22 = t73;
		let t74: [uint32x4_t; 2] = [vsubq_u32(x13[0], x19[0]), vsubq_u32(x13[1], x19[1])];
		x23 = t74;
		let t75 = [vdupq_n_u32(8u32); 2];
		let t76: [uint32x4_t; 2] = [vcgeq_u32(x23[0], t75[0]), vcgeq_u32(x23[1], t75[1])];
		let mut t77 = [vdupq_n_u32(0); 4];
		let t78: [uint32x4_t; 2] = [vandq_u32(t48[0], t76[0]), vandq_u32(t48[1], t76[1])];
		let t79: [uint32x4_t; 2] = [vbicq_u32(t48[0], t76[0]), vbicq_u32(t48[1], t76[1])];
		{
			t77 = [vbslq_u32(t78[0], x22[0], t77[0]), vbslq_u32(t78[1], x22[1], t77[1]), vbslq_u32(t78[0], x22[2], t77[2]), vbslq_u32(t78[1], x22[3], t77[3])];
		}
		{
			let t80: [uint32x4_t; 2] = [vmulq_u32(t75[0], x23[0]), vmulq_u32(t75[1], x23[1])];
			let t81: u64 = 1u64;
			let t82 = [vdupq_n_u32((t81) as u32); 2];
			let t83: [uint32x4_t; 2] = [t80[0], t80[1]];
			let t84: [uint32x4_t; 2] = [vandq_u32(t83[0], vdupq_n_u32(63)), vandq_u32(t83[1], vdupq_n_u32(63))];
			let t85: [uint32x4_t; 4] = [vshlq_u32(t82[0], vreinterpretq_s32_u32(t84[0])), vshlq_u32(t82[1], vreinterpretq_s32_u32(t84[1])), vshlq_u32(t82[0], vsubq_s32(vreinterpretq_s32_u32(t84[0]), vdupq_n_s32(32))), vshlq_u32(t82[1], vsubq_s32(vreinterpretq_s32_u32(t84[1]), vdupq_n_s32(32)))];
			let t86: u64 = 1u64;
			let t87 = [vdupq_n_u32((t86) as u32); 2];
			let t88: [uint32x4_t; 2] = [vcltq_u32(t85[0], t87[0]), vcltq_u32(t85[1], t87[1])];
			let t89: [uint32x4_t; 4] = [vsubq_u32(t85[0], t87[0]), vsubq_u32(t85[1], t87[1]), vsbcq_u32(t85[2], vdupq_n_u32(0), t88[0]), vsbcq_u32(t85[3], vdupq_n_u32(0), t88[1])];
			let t90: [uint32x4_t; 4] = [vandq_u32(x22[0], t89[0]), vandq_u32(x22[1], t89[1]), vandq_u32(x22[2], t89[2]), vandq_u32(x22[3], t89[3])];
			t77 = [vbslq_u32(t79[0], t90[0], t77[0]), vbslq_u32(t79[1], t90[1], t77[1]), vbslq_u32(t79[0], t90[2], t77[2]), vbslq_u32(t79[1], t90[3], t77[3])];
		}
		let t91 = b41_l_p(l, lid0, t48, t77);
		let t92: [uint32x4_t; 2] = [vaddq_u32(x18[0], t91[0]), vaddq_u32(x18[1], t91[1])];
		x18 = [vbslq_u32(t48[0], t92[0], x18[0]), vbslq_u32(t48[1], t92[1], x18[1])];
		let t93: [uint32x4_t; 2] = [vandq_u32(t48[0], live[0]), vandq_u32(t48[1], live[1])];
		let t94: [uint32x4_t; 2] = [vaddq_u32(x19[0], t75[0]), vaddq_u32(x19[1], t75[1])];
		x19 = t94;
	}
	x12 = x18;
	let t95 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t96: [uint32x4_t; 2] = [vshlq_u32(t95[0], vnegq_s32(vdupq_n_s32(((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) & 31) as i32))), vshlq_u32(t95[1], vnegq_s32(vdupq_n_s32(((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) & 31) as i32)))];
	let t97 = [vdupq_n_u32(15u32); 2];
	let t98: [uint32x4_t; 2] = [vandq_u32(t96[0], t97[0]), vandq_u32(t96[1], t97[1])];
	x24 = t98;
	let t99 = [vdupq_n_u32(1u32); 2];
	let t100: [uint32x4_t; 2] = [vsubq_u32(x10[0], t99[0]), vsubq_u32(x10[1], t99[1])];
	let t101: [uint32x4_t; 2] = [vmulq_u32(t34[0], t100[0]), vmulq_u32(t34[1], t100[1])];
	let t102: [uint32x4_t; 2] = [vaddq_u32(x7[0], t101[0]), vaddq_u32(x7[1], t101[1])];
	let t103 = b36_ll_p(l, lid0, live, t102, x8);
	let t104 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t103, 2u32);
	let t105: [uint32x4_t; 2] = [t104[0], t104[1]];
	x25 = t105;
	let t106: [uint32x4_t; 2] = [vaddq_u32(x14[0], x25[0]), vaddq_u32(x14[1], x25[1])];
	let t107: [uint32x4_t; 2] = [vmulq_u32(x12[0], x24[0]), vmulq_u32(x12[1], x24[1])];
	let t108: [uint32x4_t; 2] = [vaddq_u32(t106[0], t107[0]), vaddq_u32(t106[1], t107[1])];
	let t109 = b36_ll_p(l, lid0, live, t108, x8);
	x26 = t109;
	let t110 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, x26, 8u32);
	let t111 = b42_l_p(l, lid0, live, x24);
	let t112: [uint32x4_t; 4] = [vandq_u32(t110[0], t111[0]), vandq_u32(t110[1], t111[1]), vandq_u32(t110[2], t111[2]), vandq_u32(t110[3], t111[3])];
	res = t112;
	return res;
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b34_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 2];
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 2];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = [vdupq_n_u32(0u32); 2];
	let t1: [uint32x4_t; 2] = [vceqq_u32(x2[0], t0[0]), vceqq_u32(x2[1], t0[1])];
	let t2: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	{
		let t3: u64 = 4294967295u64;
		let t4 = [vdupq_n_u32((t3) as u32); 2];
		let t5: [uint32x4_t; 4] = [t4[0], t4[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = [vbslq_u32(t2[0], t5[0], res[0]), vbslq_u32(t2[1], t5[1], res[1]), vbslq_u32(t2[0], t5[2], res[2]), vbslq_u32(t2[1], t5[3], res[3])];
		live = [vbicq_u32(live[0], t2[0]), vbicq_u32(live[1], t2[1])];
	}
	if any(&live) {
		let t6 = [vdupq_n_u32(1u32); 2];
		let t7: [uint32x4_t; 2] = [vsubq_u32(x2[0], t6[0]), vsubq_u32(x2[1], t6[1])];
		x3 = t7;
		let t8 = b28_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x3);
		x4 = t8;
		let t9: u64 = 1u64;
		let t10 = [vdupq_n_u32((t9) as u32); 2];
		let t11: [uint32x4_t; 2] = [vandq_u32(x4[0], t10[0]), vandq_u32(x4[1], t10[1])];
		let t12: u64 = 0u64;
		let t13 = [vdupq_n_u32((t12) as u32); 2];
		let t14: [uint32x4_t; 2] = [vceqq_u32(t11[0], t13[0]), vceqq_u32(t11[1], t13[1])];
		let t15: [uint32x4_t; 2] = [vandq_u32(live[0], t14[0]), vandq_u32(live[1], t14[1])];
		{
			let t16: [uint32x4_t; 4] = [x3[0], x3[1], vdupq_n_u32(0), vdupq_n_u32(0)];
			res = [vbslq_u32(t15[0], t16[0], res[0]), vbslq_u32(t15[1], t16[1], res[1]), vbslq_u32(t15[0], t16[2], res[2]), vbslq_u32(t15[1], t16[3], res[3])];
			live = [vbicq_u32(live[0], t15[0]), vbicq_u32(live[1], t15[1])];
		}
		if any(&live) {
			let t17: [uint32x4_t; 4] = [vorrq_u32(vshrq_n_u32::<2>(x4[0]), vshlq_n_u32::<30>(x4[2])), vorrq_u32(vshrq_n_u32::<2>(x4[1]), vshlq_n_u32::<30>(x4[3])), vshrq_n_u32::<2>(x4[2]), vshrq_n_u32::<2>(x4[3])];
			let t18: [uint32x4_t; 2] = [t17[0], t17[1]];
			x5 = t18;
			let t19: [uint32x4_t; 2] = [vsubq_u32(x5[0], t6[0]), vsubq_u32(x5[1], t6[1])];
			let t20: [uint32x4_t; 2] = [vcgeq_u32(t19[0], x2[0]), vcgeq_u32(t19[1], x2[1])];
			let t21: [uint32x4_t; 2] = [vandq_u32(live[0], t20[0]), vandq_u32(live[1], t20[1])];
			{
				let t22: u64 = 4294967295u64;
				let t23 = [vdupq_n_u32((t22) as u32); 2];
				let t24: [uint32x4_t; 4] = [t23[0], t23[1], vdupq_n_u32(0), vdupq_n_u32(0)];
				res = [vbslq_u32(t21[0], t24[0], res[0]), vbslq_u32(t21[1], t24[1], res[1]), vbslq_u32(t21[0], t24[2], res[2]), vbslq_u32(t21[1], t24[3], res[3])];
				live = [vbicq_u32(live[0], t21[0]), vbicq_u32(live[1], t21[1])];
			}
			if any(&live) {
				let t25: [uint32x4_t; 2] = [vsubq_u32(x2[0], x5[0]), vsubq_u32(x2[1], x5[1])];
				x6 = t25;
				let t26: u64 = 2u64;
				let t27 = [vdupq_n_u32((t26) as u32); 2];
				let t28: [uint32x4_t; 2] = [vandq_u32(x4[0], t27[0]), vandq_u32(x4[1], t27[1])];
				let t29: u64 = 0u64;
				let t30 = [vdupq_n_u32((t29) as u32); 2];
				let t31: [uint32x4_t; 2] = [vceqq_u32(t28[0], t30[0]), vceqq_u32(t28[1], t30[1])];
				let t32: [uint32x4_t; 2] = [vmvnq_u32(t31[0]), vmvnq_u32(t31[1])];
				let t33: [uint32x4_t; 2] = [vandq_u32(live[0], t32[0]), vandq_u32(live[1], t32[1])];
				{
					let t34: [uint32x4_t; 4] = [x6[0], x6[1], vdupq_n_u32(0), vdupq_n_u32(0)];
					res = [vbslq_u32(t33[0], t34[0], res[0]), vbslq_u32(t33[1], t34[1], res[1]), vbslq_u32(t33[0], t34[2], res[2]), vbslq_u32(t33[1], t34[3], res[3])];
					live = [vbicq_u32(live[0], t33[0]), vbicq_u32(live[1], t33[1])];
				}
				if any(&live) {
					let t35: [uint32x4_t; 2] = [vceqq_u32(x6[0], t0[0]), vceqq_u32(x6[1], t0[1])];
					let t36: [uint32x4_t; 2] = [vandq_u32(live[0], t35[0]), vandq_u32(live[1], t35[1])];
					{
						let t37: u64 = 4294967295u64;
						let t38 = [vdupq_n_u32((t37) as u32); 2];
						let t39: [uint32x4_t; 4] = [t38[0], t38[1], vdupq_n_u32(0), vdupq_n_u32(0)];
						res = [vbslq_u32(t36[0], t39[0], res[0]), vbslq_u32(t36[1], t39[1], res[1]), vbslq_u32(t36[0], t39[2], res[2]), vbslq_u32(t36[1], t39[3], res[3])];
						live = [vbicq_u32(live[0], t36[0]), vbicq_u32(live[1], t36[1])];
					}
					if any(&live) {
						let t40: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t41 = [vdupq_n_u32((t40) as u32); 2];
						let t42 = [vdupq_n_u32(((t40) >> 32) as u32); 2];
						let t43: [uint32x4_t; 4] = [vorrq_u32(t41[0], x6[0]), vorrq_u32(t41[1], x6[1]), t42[0], t42[1]];
						res = [vbslq_u32(live[0], t43[0], res[0]), vbslq_u32(live[1], t43[1], res[1]), vbslq_u32(live[0], t43[2], res[2]), vbslq_u32(live[1], t43[3], res[3])];
						live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
					}
				}
			}
		}
	}
	res
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b35_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x1); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(x0[0], t0[0]), vcltq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vmvnq_u32(t1[0]), vmvnq_u32(t1[1])];
	{
		let t4 = [vdupq_n_u32(x1); 2];
		t2 = [vbslq_u32(t1[0], t4[0], t2[0]), vbslq_u32(t1[1], t4[1], t2[1])];
	}
	{
		t2 = [vbslq_u32(t3[0], x0[0], t2[0]), vbslq_u32(t3[1], x0[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b35_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
	let mut t1 = [vdupq_n_u32(0); 2];
	let t2: [uint32x4_t; 2] = [vandq_u32(live[0], t0[0]), vandq_u32(live[1], t0[1])];
	let t3: [uint32x4_t; 2] = [vbicq_u32(live[0], t0[0]), vbicq_u32(live[1], t0[1])];
	{
		t1 = [vbslq_u32(t2[0], x1[0], t1[0]), vbslq_u32(t2[1], x1[1], t1[1])];
	}
	{
		t1 = [vbslq_u32(t3[0], x0[0], t1[0]), vbslq_u32(t3[1], x0[1], t1[1])];
	}
	res = t1;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x1); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(x0[0], t0[0]), vcltq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		t2 = [vbslq_u32(t3[0], x0[0], t2[0]), vbslq_u32(t3[1], x0[1], t2[1])];
	}
	{
		let t5 = [vdupq_n_u32(x1); 2];
		t2 = [vbslq_u32(t4[0], t5[0], t2[0]), vbslq_u32(t4[1], t5[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x1: u32) -> [uint32x4_t; 2] {
	let mut live = [vdupq_n_u32(u32::MAX); 2];
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = [vdupq_n_u32(x1); 2];
	let t1: [uint32x4_t; 2] = [vcltq_u32(x0[0], t0[0]), vcltq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 2];
	let t3: [uint32x4_t; 2] = [vmvnq_u32(t1[0]), vmvnq_u32(t1[1])];
	{
		t2 = [vbslq_u32(t1[0], x0[0], t2[0]), vbslq_u32(t1[1], x0[1], t2[1])];
	}
	{
		let t4 = [vdupq_n_u32(x1); 2];
		t2 = [vbslq_u32(t3[0], t4[0], t2[0]), vbslq_u32(t3[1], t4[1], t2[1])];
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x1: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vcltq_u32(x0[0], x1[0]), vcltq_u32(x0[1], x1[1])];
	let mut t1 = [vdupq_n_u32(0); 2];
	let t2: [uint32x4_t; 2] = [vandq_u32(live[0], t0[0]), vandq_u32(live[1], t0[1])];
	let t3: [uint32x4_t; 2] = [vbicq_u32(live[0], t0[0]), vbicq_u32(live[1], t0[1])];
	{
		t1 = [vbslq_u32(t2[0], x0[0], t1[0]), vbslq_u32(t2[1], x0[1], t1[1])];
	}
	{
		t1 = [vbslq_u32(t3[0], x1[0], t1[0]), vbslq_u32(t3[1], x1[1], t1[1])];
	}
	res = t1;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b37_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4: u32 = 0;
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6: u64 = 0;
	let mut x7 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b43_lss_p(l, lid0, live, x0, x1, x2);
	x4 = t0;
	let t1: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	let t2 = b31_lssl_p(l, lid0, live, x0, x1, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b42_s_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = [vdupq_n_u32(1023u32); 2];
	let t5: [uint32x4_t; 2] = [vandq_u32(x3[0], t4[0]), vandq_u32(x3[1], t4[1])];
	let t6 = [vdupq_n_u32(x4); 2];
	let t7: [uint32x4_t; 2] = [vmulq_u32(t5[0], t6[0]), vmulq_u32(t5[1], t6[1])];
	let t8 = b30_lslls_p(l, lid0, live, x0, x1, x5, t7, 8u32);
	let t9: u64 = x6;
	let t10 = [vdupq_n_u32((t9) as u32); 2];
	let t11 = [vdupq_n_u32(((t9) >> 32) as u32); 2];
	let t12: [uint32x4_t; 4] = [vandq_u32(t8[0], t10[0]), vandq_u32(t8[1], t10[1]), vandq_u32(t8[2], t11[0]), vandq_u32(t8[3], t11[1])];
	x7 = t12;
	let t13: u64 = x6;
	let t14 = [vdupq_n_u32((t13) as u32); 2];
	let t15 = [vdupq_n_u32(((t13) >> 32) as u32); 2];
	let t16: [uint32x4_t; 2] = [veq64q_u32(x7[0], x7[2], t14[0], t15[0]), veq64q_u32(x7[1], x7[3], t14[1], t15[1])];
	let mut t17 = [vdupq_n_u32(0); 4];
	let t18: [uint32x4_t; 2] = [vandq_u32(live[0], t16[0]), vandq_u32(live[1], t16[1])];
	let t19: [uint32x4_t; 2] = [vbicq_u32(live[0], t16[0]), vbicq_u32(live[1], t16[1])];
	if any(&t18) {
		let t20: [uint32x4_t; 4];
		let t21 = bits(&t18);
		if t21.count_ones() <= 2 {
			let t22 = words4(x0);
			let t23 = words2(x3);
			let mut t24 = [0u32; 16];
			let mut t25 = t21;
			while t25 != 0 {
				let j = t25.trailing_zeros() as usize;
				t25 &= t25 - 1;
				let t26: u64 = (((((t22[8 + j] as u64) << 32 | t22[j] as u64) as usize as *const u8)) as usize as u64);
				let t27 = [vdupq_n_u32((t26) as u32); 2];
				let t28 = [vdupq_n_u32(((t26) >> 32) as u32); 2];
				let t29: [uint32x4_t; 4] = [t27[0], t27[1], t28[0], t28[1]];
				let t30 = [vdupq_n_u32(t23[j]); 2];
				let t31 = b33_lssl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], t29, x1, (x2).wrapping_add(1u32), t30);
				t24[j] = vgetq_lane_u32::<0>(t31[0]);
				t24[8 + j] = vgetq_lane_u32::<0>(t31[2]);
			}
			t20 = regs4(t24);
		} else {
			let t32 = b33_lssl_p(l, lid0, t18, x0, x1, (x2).wrapping_add(1u32), x3);
			t20 = t32;
		}
		t17 = [vbslq_u32(t18[0], t20[0], t17[0]), vbslq_u32(t18[1], t20[1], t17[1]), vbslq_u32(t18[0], t20[2], t17[2]), vbslq_u32(t18[1], t20[3], t17[3])];
	}
	{
		t17 = [vbslq_u32(t19[0], x7[0], t17[0]), vbslq_u32(t19[1], x7[1], t17[1]), vbslq_u32(t19[0], x7[2], t17[2]), vbslq_u32(t19[1], x7[3], t17[3])];
	}
	res = t17;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b37_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32, x3: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 2];
	let mut x6 = [vdupq_n_u32(0); 4];
	let mut x7 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b43_lal136r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	x4 = t0;
	let t1: [uint32x4_t; 2] = [vshlq_u32(x3[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x3[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
	let t2 = b31_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b42_l_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = [vdupq_n_u32(1023u32); 2];
	let t5: [uint32x4_t; 2] = [vandq_u32(x3[0], t4[0]), vandq_u32(x3[1], t4[1])];
	let t6: [uint32x4_t; 2] = [vmulq_u32(t5[0], x4[0]), vmulq_u32(t5[1], x4[1])];
	let t7 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t6, 8u32);
	let t8: [uint32x4_t; 4] = [vandq_u32(t7[0], x6[0]), vandq_u32(t7[1], x6[1]), vandq_u32(t7[2], x6[2]), vandq_u32(t7[3], x6[3])];
	x7 = t8;
	let t9: [uint32x4_t; 2] = [veq64q_u32(x7[0], x7[2], x6[0], x6[2]), veq64q_u32(x7[1], x7[3], x6[1], x6[3])];
	let mut t10 = [vdupq_n_u32(0); 4];
	let t11: [uint32x4_t; 2] = [vandq_u32(live[0], t9[0]), vandq_u32(live[1], t9[1])];
	let t12: [uint32x4_t; 2] = [vbicq_u32(live[0], t9[0]), vbicq_u32(live[1], t9[1])];
	if any(&t11) {
		let t13: [uint32x4_t; 4];
		let t14 = bits(&t11);
		if t14.count_ones() <= 2 {
			let t15 = words4(x0);
			let t16 = words2(x1i);
			let t17 = words2(x3);
			let mut t18 = [0u32; 16];
			let mut t19 = t14;
			while t19 != 0 {
				let j = t19.trailing_zeros() as usize;
				t19 &= t19 - 1;
				let t20: u64 = (((((t15[8 + j] as u64) << 32 | t15[j] as u64) as usize as *const u8)) as usize as u64);
				let t21 = [vdupq_n_u32((t20) as u32); 2];
				let t22 = [vdupq_n_u32(((t20) >> 32) as u32); 2];
				let t23: [uint32x4_t; 4] = [t21[0], t21[1], t22[0], t22[1]];
				let t24 = [vdupq_n_u32(t17[j]); 2];
				let t25 = b33_lssl_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], t23, (arg0 as *const u8).wrapping_add(((t16[j]) as usize).wrapping_mul(136)), (x2).wrapping_add(1u32), t24);
				t18[j] = vgetq_lane_u32::<0>(t25[0]);
				t18[8 + j] = vgetq_lane_u32::<0>(t25[2]);
			}
			t13 = regs4(t18);
		} else {
			let t26 = b33_lal136r0sl_p(l, lid0, t11, x0, arg0, x1i, (x2).wrapping_add(1u32), x3);
			t13 = t26;
		}
		t10 = [vbslq_u32(t11[0], t13[0], t10[0]), vbslq_u32(t11[1], t13[1], t10[1]), vbslq_u32(t11[0], t13[2], t10[2]), vbslq_u32(t11[1], t13[3], t10[3])];
	}
	{
		t10 = [vbslq_u32(t12[0], x7[0], t10[0]), vbslq_u32(t12[1], x7[1], t10[1]), vbslq_u32(t12[0], x7[2], t10[2]), vbslq_u32(t12[1], x7[3], t10[3])];
	}
	res = t10;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b38_lsls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: u32, x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = b44_lal1s_p(l, lid0, live, x0, ((((x1) as u64)) as usize as *const u8), x2, x3);
	x4 = t0;
	let t1: [uint32x4_t; 4];
	if ((x3) >= (8u32)) {
		t1 = x4;
	} else {
		let t2 = b45_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t3: u64 = t2;
		let t4 = [vdupq_n_u32((t3) as u32); 2];
		let t5 = [vdupq_n_u32(((t3) >> 32) as u32); 2];
		let t6: [uint32x4_t; 4] = [vandq_u32(x4[0], t4[0]), vandq_u32(x4[1], t4[1]), vandq_u32(x4[2], t5[0]), vandq_u32(x4[3], t5[1])];
		t1 = t6;
	}
	res = t1;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b38_llls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 2], x2: [uint32x4_t; 2], x3: u32) -> [uint32x4_t; 4] {
	let mut x4 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0: [uint32x4_t; 2] = [vaddq_u32(x1[0], x2[0]), vaddq_u32(x1[1], x2[1])];
	let t1: [uint32x4_t; 2] = [vcltq_u32(t0[0], x1[0]), vcltq_u32(t0[1], x1[1])];
	let t2: [uint32x4_t; 4] = [t0[0], t0[1], vshrq_n_u32::<31>(t1[0]), vshrq_n_u32::<31>(t1[1])];
	let t3 = b44_lls_p(l, lid0, live, x0, t2, x3);
	x4 = t3;
	let t4: [uint32x4_t; 4];
	if ((x3) >= (8u32)) {
		t4 = x4;
	} else {
		let t5 = b45_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t6: u64 = t5;
		let t7 = [vdupq_n_u32((t6) as u32); 2];
		let t8 = [vdupq_n_u32(((t6) >> 32) as u32); 2];
		let t9: [uint32x4_t; 4] = [vandq_u32(x4[0], t7[0]), vandq_u32(x4[1], t7[1]), vandq_u32(x4[2], t8[0]), vandq_u32(x4[3], t8[1])];
		t4 = t9;
	}
	res = t4;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b39_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b32_lss_p(l, lid0, live, x0, x1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b39_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}


/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b41_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
	res = t0;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b42_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
	let mut live = k;
	let mut res: u64 = 0;
	let t0: u64;
	if ((x0) >= (8u32)) {
		t0 = !(0u64);
	} else {
		t0 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
	}
	res = t0;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b42_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	let t0 = [vdupq_n_u32(8u32); 2];
	let t1: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t0[0]), vcgeq_u32(x0[1], t0[1])];
	let mut t2 = [vdupq_n_u32(0); 4];
	let t3: [uint32x4_t; 2] = [vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1])];
	let t4: [uint32x4_t; 2] = [vbicq_u32(live[0], t1[0]), vbicq_u32(live[1], t1[1])];
	{
		let t5: u64 = !(0u64);
		let t6 = [vdupq_n_u32((t5) as u32); 2];
		let t7 = [vdupq_n_u32(((t5) >> 32) as u32); 2];
		let t8: [uint32x4_t; 4] = [t6[0], t6[1], t7[0], t7[1]];
		t2 = [vbslq_u32(t3[0], t8[0], t2[0]), vbslq_u32(t3[1], t8[1], t2[1]), vbslq_u32(t3[0], t8[2], t2[2]), vbslq_u32(t3[1], t8[3], t2[3])];
	}
	{
		let t9: [uint32x4_t; 2] = [vmulq_u32(x0[0], t0[0]), vmulq_u32(x0[1], t0[1])];
		let t10: u64 = 1u64;
		let t11 = [vdupq_n_u32((t10) as u32); 2];
		let t12: [uint32x4_t; 2] = [t9[0], t9[1]];
		let t13: [uint32x4_t; 2] = [vandq_u32(t12[0], vdupq_n_u32(63)), vandq_u32(t12[1], vdupq_n_u32(63))];
		let t14: [uint32x4_t; 4] = [vshlq_u32(t11[0], vreinterpretq_s32_u32(t13[0])), vshlq_u32(t11[1], vreinterpretq_s32_u32(t13[1])), vshlq_u32(t11[0], vsubq_s32(vreinterpretq_s32_u32(t13[0]), vdupq_n_s32(32))), vshlq_u32(t11[1], vsubq_s32(vreinterpretq_s32_u32(t13[1]), vdupq_n_s32(32)))];
		let t15: u64 = 1u64;
		let t16 = [vdupq_n_u32((t15) as u32); 2];
		let t17: [uint32x4_t; 2] = [vcltq_u32(t14[0], t16[0]), vcltq_u32(t14[1], t16[1])];
		let t18: [uint32x4_t; 4] = [vsubq_u32(t14[0], t16[0]), vsubq_u32(t14[1], t16[1]), vsbcq_u32(t14[2], vdupq_n_u32(0), t17[0]), vsbcq_u32(t14[3], vdupq_n_u32(0), t17[1])];
		t2 = [vbslq_u32(t4[0], t18[0], t2[0]), vbslq_u32(t4[1], t18[1], t2[1]), vbslq_u32(t4[0], t18[2], t2[2]), vbslq_u32(t4[1], t18[3], t2[3])];
	}
	res = t2;
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b43_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b32_lss_p(l, lid0, live, x0, x1, 16u32);
	res = (((t0).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b43_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], arg0: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 2] {
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 2];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t1: [uint32x4_t; 2] = [vshlq_u32(t0[0], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))), vshlq_u32(t0[1], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32)))];
	let t2 = [vdupq_n_u32(15u32); 2];
	let t3: [uint32x4_t; 2] = [vandq_u32(t1[0], t2[0]), vandq_u32(t1[1], t2[1])];
	res = t3;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b44_lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1: [uint32x4_t; 4], x2: u32) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
		let t1: [uint32x4_t; 2] = [vcltq_u32(t0[0], x0[0]), vcltq_u32(t0[1], x0[1])];
		let t2: [uint32x4_t; 4] = [t0[0], t0[1], vadcq_u32(x0[2], x1[2], t1[0]), vadcq_u32(x0[3], x1[3], t1[1])];
		let t3 = words2([t2[0], t2[1]]);
		let t4 = words2([t2[2], t2[3]]);
		let t5 = bits(&live);
		let t6 = t5.trailing_zeros() as usize % 8;
		let t7: [usize; 8] = if t5 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t5 >> 0 & 1 != 0 { 0 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 1 & 1 != 0 { 1 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 2 & 1 != 0 { 2 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 3 & 1 != 0 { 3 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 4 & 1 != 0 { 4 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 5 & 1 != 0 { 5 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 6 & 1 != 0 { 6 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }, { let j = if t5 >> 7 & 1 != 0 { 7 } else { t6 }; ((t4[j] as u64) << 32 | t3[j] as u64) as usize }] };
		let mut t8 = [vdupq_n_u32(0); 4];
		{ let v = (t7[0] as *const u64).read_unaligned(); t8[0] = vsetq_lane_u32::<0>(v as u32, t8[0]); t8[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t8[2]); }
		{ let v = (t7[1] as *const u64).read_unaligned(); t8[0] = vsetq_lane_u32::<1>(v as u32, t8[0]); t8[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t8[2]); }
		{ let v = (t7[2] as *const u64).read_unaligned(); t8[0] = vsetq_lane_u32::<2>(v as u32, t8[0]); t8[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t8[2]); }
		{ let v = (t7[3] as *const u64).read_unaligned(); t8[0] = vsetq_lane_u32::<3>(v as u32, t8[0]); t8[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t8[2]); }
		{ let v = (t7[4] as *const u64).read_unaligned(); t8[1] = vsetq_lane_u32::<0>(v as u32, t8[1]); t8[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t8[3]); }
		{ let v = (t7[5] as *const u64).read_unaligned(); t8[1] = vsetq_lane_u32::<1>(v as u32, t8[1]); t8[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t8[3]); }
		{ let v = (t7[6] as *const u64).read_unaligned(); t8[1] = vsetq_lane_u32::<2>(v as u32, t8[1]); t8[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t8[3]); }
		{ let v = (t7[7] as *const u64).read_unaligned(); t8[1] = vsetq_lane_u32::<3>(v as u32, t8[1]); t8[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t8[3]); }
		res = t8;
		return res;
	}
	if ((x2) == (4u32)) {
		let t9: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
		let t10: [uint32x4_t; 2] = [vcltq_u32(t9[0], x0[0]), vcltq_u32(t9[1], x0[1])];
		let t11: [uint32x4_t; 4] = [t9[0], t9[1], vadcq_u32(x0[2], x1[2], t10[0]), vadcq_u32(x0[3], x1[3], t10[1])];
		let t12 = words2([t11[0], t11[1]]);
		let t13 = words2([t11[2], t11[3]]);
		let t14 = bits(&live);
		let t15 = t14.trailing_zeros() as usize % 8;
		let t16: [usize; 8] = if t14 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t14 >> 0 & 1 != 0 { 0 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 1 & 1 != 0 { 1 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 2 & 1 != 0 { 2 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 3 & 1 != 0 { 3 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 4 & 1 != 0 { 4 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 5 & 1 != 0 { 5 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 6 & 1 != 0 { 6 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }, { let j = if t14 >> 7 & 1 != 0 { 7 } else { t15 }; ((t13[j] as u64) << 32 | t12[j] as u64) as usize }] };
		let mut t17 = [vdupq_n_u32(0); 2];
		t17[0] = vsetq_lane_u32::<0>((t16[0] as *const u32).read_unaligned() as u32, t17[0]);
		t17[0] = vsetq_lane_u32::<1>((t16[1] as *const u32).read_unaligned() as u32, t17[0]);
		t17[0] = vsetq_lane_u32::<2>((t16[2] as *const u32).read_unaligned() as u32, t17[0]);
		t17[0] = vsetq_lane_u32::<3>((t16[3] as *const u32).read_unaligned() as u32, t17[0]);
		t17[1] = vsetq_lane_u32::<0>((t16[4] as *const u32).read_unaligned() as u32, t17[1]);
		t17[1] = vsetq_lane_u32::<1>((t16[5] as *const u32).read_unaligned() as u32, t17[1]);
		t17[1] = vsetq_lane_u32::<2>((t16[6] as *const u32).read_unaligned() as u32, t17[1]);
		t17[1] = vsetq_lane_u32::<3>((t16[7] as *const u32).read_unaligned() as u32, t17[1]);
		let t18: [uint32x4_t; 4] = [t17[0], t17[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t18;
		return res;
	}
	if ((x2) == (2u32)) {
		let t19: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
		let t20: [uint32x4_t; 2] = [vcltq_u32(t19[0], x0[0]), vcltq_u32(t19[1], x0[1])];
		let t21: [uint32x4_t; 4] = [t19[0], t19[1], vadcq_u32(x0[2], x1[2], t20[0]), vadcq_u32(x0[3], x1[3], t20[1])];
		let t22 = words2([t21[0], t21[1]]);
		let t23 = words2([t21[2], t21[3]]);
		let t24 = bits(&live);
		let t25 = t24.trailing_zeros() as usize % 8;
		let t26: [usize; 8] = if t24 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t24 >> 0 & 1 != 0 { 0 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 1 & 1 != 0 { 1 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 2 & 1 != 0 { 2 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 3 & 1 != 0 { 3 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 4 & 1 != 0 { 4 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 5 & 1 != 0 { 5 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 6 & 1 != 0 { 6 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }, { let j = if t24 >> 7 & 1 != 0 { 7 } else { t25 }; ((t23[j] as u64) << 32 | t22[j] as u64) as usize }] };
		let mut t27 = [vdupq_n_u32(0); 2];
		t27[0] = vsetq_lane_u32::<0>((t26[0] as *const u16).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<1>((t26[1] as *const u16).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<2>((t26[2] as *const u16).read_unaligned() as u32, t27[0]);
		t27[0] = vsetq_lane_u32::<3>((t26[3] as *const u16).read_unaligned() as u32, t27[0]);
		t27[1] = vsetq_lane_u32::<0>((t26[4] as *const u16).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<1>((t26[5] as *const u16).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<2>((t26[6] as *const u16).read_unaligned() as u32, t27[1]);
		t27[1] = vsetq_lane_u32::<3>((t26[7] as *const u16).read_unaligned() as u32, t27[1]);
		let t28: [uint32x4_t; 4] = [t27[0], t27[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t28;
		return res;
	}
	if ((x2) == (1u32)) {
		let t29: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
		let t30: [uint32x4_t; 2] = [vcltq_u32(t29[0], x0[0]), vcltq_u32(t29[1], x0[1])];
		let t31: [uint32x4_t; 4] = [t29[0], t29[1], vadcq_u32(x0[2], x1[2], t30[0]), vadcq_u32(x0[3], x1[3], t30[1])];
		let t32 = words2([t31[0], t31[1]]);
		let t33 = words2([t31[2], t31[3]]);
		let t34 = bits(&live);
		let t35 = t34.trailing_zeros() as usize % 8;
		let t36: [usize; 8] = if t34 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t34 >> 0 & 1 != 0 { 0 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 1 & 1 != 0 { 1 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 2 & 1 != 0 { 2 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 3 & 1 != 0 { 3 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 4 & 1 != 0 { 4 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 5 & 1 != 0 { 5 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 6 & 1 != 0 { 6 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }, { let j = if t34 >> 7 & 1 != 0 { 7 } else { t35 }; ((t33[j] as u64) << 32 | t32[j] as u64) as usize }] };
		let mut t37 = [vdupq_n_u32(0); 2];
		t37[0] = vsetq_lane_u32::<0>((t36[0] as *const u8).read_unaligned() as u32, t37[0]);
		t37[0] = vsetq_lane_u32::<1>((t36[1] as *const u8).read_unaligned() as u32, t37[0]);
		t37[0] = vsetq_lane_u32::<2>((t36[2] as *const u8).read_unaligned() as u32, t37[0]);
		t37[0] = vsetq_lane_u32::<3>((t36[3] as *const u8).read_unaligned() as u32, t37[0]);
		t37[1] = vsetq_lane_u32::<0>((t36[4] as *const u8).read_unaligned() as u32, t37[1]);
		t37[1] = vsetq_lane_u32::<1>((t36[5] as *const u8).read_unaligned() as u32, t37[1]);
		t37[1] = vsetq_lane_u32::<2>((t36[6] as *const u8).read_unaligned() as u32, t37[1]);
		t37[1] = vsetq_lane_u32::<3>((t36[7] as *const u8).read_unaligned() as u32, t37[1]);
		let t38: [uint32x4_t; 4] = [t37[0], t37[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t38;
		return res;
	}
	let t39: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
	let t40: [uint32x4_t; 2] = [vcltq_u32(t39[0], x0[0]), vcltq_u32(t39[1], x0[1])];
	let t41: [uint32x4_t; 4] = [t39[0], t39[1], vadcq_u32(x0[2], x1[2], t40[0]), vadcq_u32(x0[3], x1[3], t40[1])];
	let t42: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
	let t43: [uint32x4_t; 2] = [vcltq_u32(t42[0], x0[0]), vcltq_u32(t42[1], x0[1])];
	let t44: [uint32x4_t; 4] = [t42[0], t42[1], vadcq_u32(x0[2], x1[2], t43[0]), vadcq_u32(x0[3], x1[3], t43[1])];
	let t45: u64 = 7u64;
	let t46 = [vdupq_n_u32((t45) as u32); 2];
	let t47: [uint32x4_t; 2] = [vandq_u32(t44[0], t46[0]), vandq_u32(t44[1], t46[1])];
	let t48: [uint32x4_t; 2] = [vcltq_u32(t41[0], t47[0]), vcltq_u32(t41[1], t47[1])];
	let t49: [uint32x4_t; 4] = [vsubq_u32(t41[0], t47[0]), vsubq_u32(t41[1], t47[1]), vsbcq_u32(t41[2], vdupq_n_u32(0), t48[0]), vsbcq_u32(t41[3], vdupq_n_u32(0), t48[1])];
	x3 = t49;
	let t50 = words2([x3[0], x3[1]]);
	let t51 = words2([x3[2], x3[3]]);
	let t52 = bits(&live);
	let t53 = t52.trailing_zeros() as usize % 8;
	let t54: [usize; 8] = if t52 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t52 >> 0 & 1 != 0 { 0 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 1 & 1 != 0 { 1 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 2 & 1 != 0 { 2 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 3 & 1 != 0 { 3 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 4 & 1 != 0 { 4 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 5 & 1 != 0 { 5 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 6 & 1 != 0 { 6 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }, { let j = if t52 >> 7 & 1 != 0 { 7 } else { t53 }; ((t51[j] as u64) << 32 | t50[j] as u64) as usize }] };
	let t55: [uint32x4_t; 2] = [vaddq_u32(x0[0], x1[0]), vaddq_u32(x0[1], x1[1])];
	let t56: [uint32x4_t; 2] = [vcltq_u32(t55[0], x0[0]), vcltq_u32(t55[1], x0[1])];
	let t57: [uint32x4_t; 4] = [t55[0], t55[1], vadcq_u32(x0[2], x1[2], t56[0]), vadcq_u32(x0[3], x1[3], t56[1])];
	let t58: u64 = 7u64;
	let t59 = [vdupq_n_u32((t58) as u32); 2];
	let t60: [uint32x4_t; 2] = [vandq_u32(t57[0], t59[0]), vandq_u32(t57[1], t59[1])];
	let t61 = [vdupq_n_u32(8u32); 2];
	let t62: [uint32x4_t; 2] = [vmulq_u32(t60[0], t61[0]), vmulq_u32(t60[1], t61[1])];
	x4 = t62;
	let mut t63 = [vdupq_n_u32(0); 4];
	{ let v = (t54[0] as *const u64).read_unaligned(); t63[0] = vsetq_lane_u32::<0>(v as u32, t63[0]); t63[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t63[2]); }
	{ let v = (t54[1] as *const u64).read_unaligned(); t63[0] = vsetq_lane_u32::<1>(v as u32, t63[0]); t63[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t63[2]); }
	{ let v = (t54[2] as *const u64).read_unaligned(); t63[0] = vsetq_lane_u32::<2>(v as u32, t63[0]); t63[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t63[2]); }
	{ let v = (t54[3] as *const u64).read_unaligned(); t63[0] = vsetq_lane_u32::<3>(v as u32, t63[0]); t63[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t63[2]); }
	{ let v = (t54[4] as *const u64).read_unaligned(); t63[1] = vsetq_lane_u32::<0>(v as u32, t63[1]); t63[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t63[3]); }
	{ let v = (t54[5] as *const u64).read_unaligned(); t63[1] = vsetq_lane_u32::<1>(v as u32, t63[1]); t63[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t63[3]); }
	{ let v = (t54[6] as *const u64).read_unaligned(); t63[1] = vsetq_lane_u32::<2>(v as u32, t63[1]); t63[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t63[3]); }
	{ let v = (t54[7] as *const u64).read_unaligned(); t63[1] = vsetq_lane_u32::<3>(v as u32, t63[1]); t63[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t63[3]); }
	let t64: [uint32x4_t; 2] = [x4[0], x4[1]];
	let t65: [uint32x4_t; 2] = [vandq_u32(t64[0], vdupq_n_u32(63)), vandq_u32(t64[1], vdupq_n_u32(63))];
	let t66: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t63[0], vnegq_s32(vreinterpretq_s32_u32(t65[0]))), vshlq_u32(t63[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t65[0])))), vorrq_u32(vshlq_u32(t63[1], vnegq_s32(vreinterpretq_s32_u32(t65[1]))), vshlq_u32(t63[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t65[1])))), vshlq_u32(t63[2], vnegq_s32(vreinterpretq_s32_u32(t65[0]))), vshlq_u32(t63[3], vnegq_s32(vreinterpretq_s32_u32(t65[1])))];
	x5 = t66;
	let t67 = [vdupq_n_u32(0u32); 2];
	let t68: [uint32x4_t; 2] = [vcneq_u32(x4[0], t67[0]), vcneq_u32(x4[1], t67[1])];
	let t69 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
	let t70: [uint32x4_t; 2] = [vaddq_u32(x4[0], t69[0]), vaddq_u32(x4[1], t69[1])];
	let t71 = [vdupq_n_u32(64u32); 2];
	let t72: [uint32x4_t; 2] = [vcgtq_u32(t70[0], t71[0]), vcgtq_u32(t70[1], t71[1])];
	let t73: [uint32x4_t; 2] = [vandq_u32(t68[0], t72[0]), vandq_u32(t68[1], t72[1])];
	let mut t74 = [vdupq_n_u32(0); 4];
	let t75: [uint32x4_t; 2] = [vandq_u32(live[0], t73[0]), vandq_u32(live[1], t73[1])];
	let t76: [uint32x4_t; 2] = [vbicq_u32(live[0], t73[0]), vbicq_u32(live[1], t73[1])];
	{
		let t77: u64 = (1u64).wrapping_mul(8u64);
		let t78 = [vdupq_n_u32((t77) as u32); 2];
		let t79 = [vdupq_n_u32(((t77) >> 32) as u32); 2];
		let t80: [uint32x4_t; 2] = [vaddq_u32(x3[0], t78[0]), vaddq_u32(x3[1], t78[1])];
		let t81: [uint32x4_t; 2] = [vcltq_u32(t80[0], x3[0]), vcltq_u32(t80[1], x3[1])];
		let t82: [uint32x4_t; 4] = [t80[0], t80[1], vadcq_u32(x3[2], t79[0], t81[0]), vadcq_u32(x3[3], t79[1], t81[1])];
		let t83: [usize; 8] = core::array::from_fn(|j| t54[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t84 = [vdupq_n_u32(0); 4];
		{ let v = (t83[0] as *const u64).read_unaligned(); t84[0] = vsetq_lane_u32::<0>(v as u32, t84[0]); t84[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t84[2]); }
		{ let v = (t83[1] as *const u64).read_unaligned(); t84[0] = vsetq_lane_u32::<1>(v as u32, t84[0]); t84[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t84[2]); }
		{ let v = (t83[2] as *const u64).read_unaligned(); t84[0] = vsetq_lane_u32::<2>(v as u32, t84[0]); t84[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t84[2]); }
		{ let v = (t83[3] as *const u64).read_unaligned(); t84[0] = vsetq_lane_u32::<3>(v as u32, t84[0]); t84[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t84[2]); }
		{ let v = (t83[4] as *const u64).read_unaligned(); t84[1] = vsetq_lane_u32::<0>(v as u32, t84[1]); t84[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t84[3]); }
		{ let v = (t83[5] as *const u64).read_unaligned(); t84[1] = vsetq_lane_u32::<1>(v as u32, t84[1]); t84[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t84[3]); }
		{ let v = (t83[6] as *const u64).read_unaligned(); t84[1] = vsetq_lane_u32::<2>(v as u32, t84[1]); t84[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t84[3]); }
		{ let v = (t83[7] as *const u64).read_unaligned(); t84[1] = vsetq_lane_u32::<3>(v as u32, t84[1]); t84[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t84[3]); }
		let t85: [uint32x4_t; 2] = [vsubq_u32(t71[0], x4[0]), vsubq_u32(t71[1], x4[1])];
		let t86: [uint32x4_t; 2] = [t85[0], t85[1]];
		let t87: [uint32x4_t; 2] = [vandq_u32(t86[0], vdupq_n_u32(63)), vandq_u32(t86[1], vdupq_n_u32(63))];
		let t88: [uint32x4_t; 4] = [vshlq_u32(t84[0], vreinterpretq_s32_u32(t87[0])), vshlq_u32(t84[1], vreinterpretq_s32_u32(t87[1])), vorrq_u32(vshlq_u32(t84[2], vreinterpretq_s32_u32(t87[0])), vshlq_u32(t84[0], vsubq_s32(vreinterpretq_s32_u32(t87[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t84[3], vreinterpretq_s32_u32(t87[1])), vshlq_u32(t84[1], vsubq_s32(vreinterpretq_s32_u32(t87[1]), vdupq_n_s32(32))))];
		let t89: [uint32x4_t; 4] = [vorrq_u32(x5[0], t88[0]), vorrq_u32(x5[1], t88[1]), vorrq_u32(x5[2], t88[2]), vorrq_u32(x5[3], t88[3])];
		t74 = [vbslq_u32(t75[0], t89[0], t74[0]), vbslq_u32(t75[1], t89[1], t74[1]), vbslq_u32(t75[0], t89[2], t74[2]), vbslq_u32(t75[1], t89[3], t74[3])];
	}
	{
		t74 = [vbslq_u32(t76[0], x5[0], t74[0]), vbslq_u32(t76[1], x5[1], t74[1]), vbslq_u32(t76[0], x5[2], t74[2]), vbslq_u32(t76[1], x5[3], t74[3])];
	}
	res = t74;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b44_lal1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4], x1b: *const u8, x1i: [uint32x4_t; 2], x2: u32) -> [uint32x4_t; 4] {
	let mut x3 = [vdupq_n_u32(0); 4];
	let mut x4 = [vdupq_n_u32(0); 2];
	let mut x5 = [vdupq_n_u32(0); 4];
	let mut live = k;
	let mut res = [vdupq_n_u32(0); 4];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 2] = [x1i[0], x1i[1]];
		let t1: u64 = ((x1b) as usize as u64);
		let t2 = [vdupq_n_u32((t1) as u32); 2];
		let t3 = [vdupq_n_u32(((t1) >> 32) as u32); 2];
		let t4: [uint32x4_t; 2] = [vaddq_u32(t2[0], t0[0]), vaddq_u32(t2[1], t0[1])];
		let t5: [uint32x4_t; 2] = [vcltq_u32(t4[0], t2[0]), vcltq_u32(t4[1], t2[1])];
		let t6: [uint32x4_t; 4] = [t4[0], t4[1], vadcq_u32(t3[0], vdupq_n_u32(0), t5[0]), vadcq_u32(t3[1], vdupq_n_u32(0), t5[1])];
		let t7: [uint32x4_t; 2] = [vaddq_u32(x0[0], t6[0]), vaddq_u32(x0[1], t6[1])];
		let t8: [uint32x4_t; 2] = [vcltq_u32(t7[0], x0[0]), vcltq_u32(t7[1], x0[1])];
		let t9: [uint32x4_t; 4] = [t7[0], t7[1], vadcq_u32(x0[2], t6[2], t8[0]), vadcq_u32(x0[3], t6[3], t8[1])];
		let t10 = words2([t9[0], t9[1]]);
		let t11 = words2([t9[2], t9[3]]);
		let t12 = bits(&live);
		let t13 = t12.trailing_zeros() as usize % 8;
		let t14: [usize; 8] = if t12 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t12 >> 0 & 1 != 0 { 0 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 1 & 1 != 0 { 1 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 2 & 1 != 0 { 2 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 3 & 1 != 0 { 3 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 4 & 1 != 0 { 4 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 5 & 1 != 0 { 5 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 6 & 1 != 0 { 6 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }, { let j = if t12 >> 7 & 1 != 0 { 7 } else { t13 }; ((t11[j] as u64) << 32 | t10[j] as u64) as usize }] };
		let mut t15 = [vdupq_n_u32(0); 4];
		{ let v = (t14[0] as *const u64).read_unaligned(); t15[0] = vsetq_lane_u32::<0>(v as u32, t15[0]); t15[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t15[2]); }
		{ let v = (t14[1] as *const u64).read_unaligned(); t15[0] = vsetq_lane_u32::<1>(v as u32, t15[0]); t15[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t15[2]); }
		{ let v = (t14[2] as *const u64).read_unaligned(); t15[0] = vsetq_lane_u32::<2>(v as u32, t15[0]); t15[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t15[2]); }
		{ let v = (t14[3] as *const u64).read_unaligned(); t15[0] = vsetq_lane_u32::<3>(v as u32, t15[0]); t15[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t15[2]); }
		{ let v = (t14[4] as *const u64).read_unaligned(); t15[1] = vsetq_lane_u32::<0>(v as u32, t15[1]); t15[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t15[3]); }
		{ let v = (t14[5] as *const u64).read_unaligned(); t15[1] = vsetq_lane_u32::<1>(v as u32, t15[1]); t15[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t15[3]); }
		{ let v = (t14[6] as *const u64).read_unaligned(); t15[1] = vsetq_lane_u32::<2>(v as u32, t15[1]); t15[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t15[3]); }
		{ let v = (t14[7] as *const u64).read_unaligned(); t15[1] = vsetq_lane_u32::<3>(v as u32, t15[1]); t15[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t15[3]); }
		res = t15;
		return res;
	}
	if ((x2) == (4u32)) {
		let t16: [uint32x4_t; 2] = [x1i[0], x1i[1]];
		let t17: u64 = ((x1b) as usize as u64);
		let t18 = [vdupq_n_u32((t17) as u32); 2];
		let t19 = [vdupq_n_u32(((t17) >> 32) as u32); 2];
		let t20: [uint32x4_t; 2] = [vaddq_u32(t18[0], t16[0]), vaddq_u32(t18[1], t16[1])];
		let t21: [uint32x4_t; 2] = [vcltq_u32(t20[0], t18[0]), vcltq_u32(t20[1], t18[1])];
		let t22: [uint32x4_t; 4] = [t20[0], t20[1], vadcq_u32(t19[0], vdupq_n_u32(0), t21[0]), vadcq_u32(t19[1], vdupq_n_u32(0), t21[1])];
		let t23: [uint32x4_t; 2] = [vaddq_u32(x0[0], t22[0]), vaddq_u32(x0[1], t22[1])];
		let t24: [uint32x4_t; 2] = [vcltq_u32(t23[0], x0[0]), vcltq_u32(t23[1], x0[1])];
		let t25: [uint32x4_t; 4] = [t23[0], t23[1], vadcq_u32(x0[2], t22[2], t24[0]), vadcq_u32(x0[3], t22[3], t24[1])];
		let t26 = words2([t25[0], t25[1]]);
		let t27 = words2([t25[2], t25[3]]);
		let t28 = bits(&live);
		let t29 = t28.trailing_zeros() as usize % 8;
		let t30: [usize; 8] = if t28 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t28 >> 0 & 1 != 0 { 0 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 1 & 1 != 0 { 1 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 2 & 1 != 0 { 2 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 3 & 1 != 0 { 3 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 4 & 1 != 0 { 4 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 5 & 1 != 0 { 5 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 6 & 1 != 0 { 6 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }, { let j = if t28 >> 7 & 1 != 0 { 7 } else { t29 }; ((t27[j] as u64) << 32 | t26[j] as u64) as usize }] };
		let mut t31 = [vdupq_n_u32(0); 2];
		t31[0] = vsetq_lane_u32::<0>((t30[0] as *const u32).read_unaligned() as u32, t31[0]);
		t31[0] = vsetq_lane_u32::<1>((t30[1] as *const u32).read_unaligned() as u32, t31[0]);
		t31[0] = vsetq_lane_u32::<2>((t30[2] as *const u32).read_unaligned() as u32, t31[0]);
		t31[0] = vsetq_lane_u32::<3>((t30[3] as *const u32).read_unaligned() as u32, t31[0]);
		t31[1] = vsetq_lane_u32::<0>((t30[4] as *const u32).read_unaligned() as u32, t31[1]);
		t31[1] = vsetq_lane_u32::<1>((t30[5] as *const u32).read_unaligned() as u32, t31[1]);
		t31[1] = vsetq_lane_u32::<2>((t30[6] as *const u32).read_unaligned() as u32, t31[1]);
		t31[1] = vsetq_lane_u32::<3>((t30[7] as *const u32).read_unaligned() as u32, t31[1]);
		let t32: [uint32x4_t; 4] = [t31[0], t31[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t32;
		return res;
	}
	if ((x2) == (2u32)) {
		let t33: [uint32x4_t; 2] = [x1i[0], x1i[1]];
		let t34: u64 = ((x1b) as usize as u64);
		let t35 = [vdupq_n_u32((t34) as u32); 2];
		let t36 = [vdupq_n_u32(((t34) >> 32) as u32); 2];
		let t37: [uint32x4_t; 2] = [vaddq_u32(t35[0], t33[0]), vaddq_u32(t35[1], t33[1])];
		let t38: [uint32x4_t; 2] = [vcltq_u32(t37[0], t35[0]), vcltq_u32(t37[1], t35[1])];
		let t39: [uint32x4_t; 4] = [t37[0], t37[1], vadcq_u32(t36[0], vdupq_n_u32(0), t38[0]), vadcq_u32(t36[1], vdupq_n_u32(0), t38[1])];
		let t40: [uint32x4_t; 2] = [vaddq_u32(x0[0], t39[0]), vaddq_u32(x0[1], t39[1])];
		let t41: [uint32x4_t; 2] = [vcltq_u32(t40[0], x0[0]), vcltq_u32(t40[1], x0[1])];
		let t42: [uint32x4_t; 4] = [t40[0], t40[1], vadcq_u32(x0[2], t39[2], t41[0]), vadcq_u32(x0[3], t39[3], t41[1])];
		let t43 = words2([t42[0], t42[1]]);
		let t44 = words2([t42[2], t42[3]]);
		let t45 = bits(&live);
		let t46 = t45.trailing_zeros() as usize % 8;
		let t47: [usize; 8] = if t45 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t45 >> 0 & 1 != 0 { 0 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 1 & 1 != 0 { 1 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 2 & 1 != 0 { 2 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 3 & 1 != 0 { 3 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 4 & 1 != 0 { 4 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 5 & 1 != 0 { 5 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 6 & 1 != 0 { 6 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }, { let j = if t45 >> 7 & 1 != 0 { 7 } else { t46 }; ((t44[j] as u64) << 32 | t43[j] as u64) as usize }] };
		let mut t48 = [vdupq_n_u32(0); 2];
		t48[0] = vsetq_lane_u32::<0>((t47[0] as *const u16).read_unaligned() as u32, t48[0]);
		t48[0] = vsetq_lane_u32::<1>((t47[1] as *const u16).read_unaligned() as u32, t48[0]);
		t48[0] = vsetq_lane_u32::<2>((t47[2] as *const u16).read_unaligned() as u32, t48[0]);
		t48[0] = vsetq_lane_u32::<3>((t47[3] as *const u16).read_unaligned() as u32, t48[0]);
		t48[1] = vsetq_lane_u32::<0>((t47[4] as *const u16).read_unaligned() as u32, t48[1]);
		t48[1] = vsetq_lane_u32::<1>((t47[5] as *const u16).read_unaligned() as u32, t48[1]);
		t48[1] = vsetq_lane_u32::<2>((t47[6] as *const u16).read_unaligned() as u32, t48[1]);
		t48[1] = vsetq_lane_u32::<3>((t47[7] as *const u16).read_unaligned() as u32, t48[1]);
		let t49: [uint32x4_t; 4] = [t48[0], t48[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t49;
		return res;
	}
	if ((x2) == (1u32)) {
		let t50: [uint32x4_t; 2] = [x1i[0], x1i[1]];
		let t51: u64 = ((x1b) as usize as u64);
		let t52 = [vdupq_n_u32((t51) as u32); 2];
		let t53 = [vdupq_n_u32(((t51) >> 32) as u32); 2];
		let t54: [uint32x4_t; 2] = [vaddq_u32(t52[0], t50[0]), vaddq_u32(t52[1], t50[1])];
		let t55: [uint32x4_t; 2] = [vcltq_u32(t54[0], t52[0]), vcltq_u32(t54[1], t52[1])];
		let t56: [uint32x4_t; 4] = [t54[0], t54[1], vadcq_u32(t53[0], vdupq_n_u32(0), t55[0]), vadcq_u32(t53[1], vdupq_n_u32(0), t55[1])];
		let t57: [uint32x4_t; 2] = [vaddq_u32(x0[0], t56[0]), vaddq_u32(x0[1], t56[1])];
		let t58: [uint32x4_t; 2] = [vcltq_u32(t57[0], x0[0]), vcltq_u32(t57[1], x0[1])];
		let t59: [uint32x4_t; 4] = [t57[0], t57[1], vadcq_u32(x0[2], t56[2], t58[0]), vadcq_u32(x0[3], t56[3], t58[1])];
		let t60 = words2([t59[0], t59[1]]);
		let t61 = words2([t59[2], t59[3]]);
		let t62 = bits(&live);
		let t63 = t62.trailing_zeros() as usize % 8;
		let t64: [usize; 8] = if t62 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t62 >> 0 & 1 != 0 { 0 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 1 & 1 != 0 { 1 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 2 & 1 != 0 { 2 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 3 & 1 != 0 { 3 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 4 & 1 != 0 { 4 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 5 & 1 != 0 { 5 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 6 & 1 != 0 { 6 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }, { let j = if t62 >> 7 & 1 != 0 { 7 } else { t63 }; ((t61[j] as u64) << 32 | t60[j] as u64) as usize }] };
		let mut t65 = [vdupq_n_u32(0); 2];
		t65[0] = vsetq_lane_u32::<0>((t64[0] as *const u8).read_unaligned() as u32, t65[0]);
		t65[0] = vsetq_lane_u32::<1>((t64[1] as *const u8).read_unaligned() as u32, t65[0]);
		t65[0] = vsetq_lane_u32::<2>((t64[2] as *const u8).read_unaligned() as u32, t65[0]);
		t65[0] = vsetq_lane_u32::<3>((t64[3] as *const u8).read_unaligned() as u32, t65[0]);
		t65[1] = vsetq_lane_u32::<0>((t64[4] as *const u8).read_unaligned() as u32, t65[1]);
		t65[1] = vsetq_lane_u32::<1>((t64[5] as *const u8).read_unaligned() as u32, t65[1]);
		t65[1] = vsetq_lane_u32::<2>((t64[6] as *const u8).read_unaligned() as u32, t65[1]);
		t65[1] = vsetq_lane_u32::<3>((t64[7] as *const u8).read_unaligned() as u32, t65[1]);
		let t66: [uint32x4_t; 4] = [t65[0], t65[1], vdupq_n_u32(0), vdupq_n_u32(0)];
		res = t66;
		return res;
	}
	let t67: [uint32x4_t; 2] = [x1i[0], x1i[1]];
	let t68: u64 = ((x1b) as usize as u64);
	let t69 = [vdupq_n_u32((t68) as u32); 2];
	let t70 = [vdupq_n_u32(((t68) >> 32) as u32); 2];
	let t71: [uint32x4_t; 2] = [vaddq_u32(t69[0], t67[0]), vaddq_u32(t69[1], t67[1])];
	let t72: [uint32x4_t; 2] = [vcltq_u32(t71[0], t69[0]), vcltq_u32(t71[1], t69[1])];
	let t73: [uint32x4_t; 4] = [t71[0], t71[1], vadcq_u32(t70[0], vdupq_n_u32(0), t72[0]), vadcq_u32(t70[1], vdupq_n_u32(0), t72[1])];
	let t74: [uint32x4_t; 2] = [vaddq_u32(x0[0], t73[0]), vaddq_u32(x0[1], t73[1])];
	let t75: [uint32x4_t; 2] = [vcltq_u32(t74[0], x0[0]), vcltq_u32(t74[1], x0[1])];
	let t76: [uint32x4_t; 4] = [t74[0], t74[1], vadcq_u32(x0[2], t73[2], t75[0]), vadcq_u32(x0[3], t73[3], t75[1])];
	let t77: [uint32x4_t; 2] = [x1i[0], x1i[1]];
	let t78: u64 = ((x1b) as usize as u64);
	let t79 = [vdupq_n_u32((t78) as u32); 2];
	let t80 = [vdupq_n_u32(((t78) >> 32) as u32); 2];
	let t81: [uint32x4_t; 2] = [vaddq_u32(t79[0], t77[0]), vaddq_u32(t79[1], t77[1])];
	let t82: [uint32x4_t; 2] = [vcltq_u32(t81[0], t79[0]), vcltq_u32(t81[1], t79[1])];
	let t83: [uint32x4_t; 4] = [t81[0], t81[1], vadcq_u32(t80[0], vdupq_n_u32(0), t82[0]), vadcq_u32(t80[1], vdupq_n_u32(0), t82[1])];
	let t84: [uint32x4_t; 2] = [vaddq_u32(x0[0], t83[0]), vaddq_u32(x0[1], t83[1])];
	let t85: [uint32x4_t; 2] = [vcltq_u32(t84[0], x0[0]), vcltq_u32(t84[1], x0[1])];
	let t86: [uint32x4_t; 4] = [t84[0], t84[1], vadcq_u32(x0[2], t83[2], t85[0]), vadcq_u32(x0[3], t83[3], t85[1])];
	let t87: u64 = 7u64;
	let t88 = [vdupq_n_u32((t87) as u32); 2];
	let t89: [uint32x4_t; 2] = [vandq_u32(t86[0], t88[0]), vandq_u32(t86[1], t88[1])];
	let t90: [uint32x4_t; 2] = [vcltq_u32(t76[0], t89[0]), vcltq_u32(t76[1], t89[1])];
	let t91: [uint32x4_t; 4] = [vsubq_u32(t76[0], t89[0]), vsubq_u32(t76[1], t89[1]), vsbcq_u32(t76[2], vdupq_n_u32(0), t90[0]), vsbcq_u32(t76[3], vdupq_n_u32(0), t90[1])];
	x3 = t91;
	let t92 = words2([x3[0], x3[1]]);
	let t93 = words2([x3[2], x3[3]]);
	let t94 = bits(&live);
	let t95 = t94.trailing_zeros() as usize % 8;
	let t96: [usize; 8] = if t94 == 0 { [NO_LANE.as_ptr() as usize; 8] } else { [{ let j = if t94 >> 0 & 1 != 0 { 0 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 1 & 1 != 0 { 1 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 2 & 1 != 0 { 2 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 3 & 1 != 0 { 3 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 4 & 1 != 0 { 4 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 5 & 1 != 0 { 5 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 6 & 1 != 0 { 6 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }, { let j = if t94 >> 7 & 1 != 0 { 7 } else { t95 }; ((t93[j] as u64) << 32 | t92[j] as u64) as usize }] };
	let t97: [uint32x4_t; 2] = [x1i[0], x1i[1]];
	let t98: u64 = ((x1b) as usize as u64);
	let t99 = [vdupq_n_u32((t98) as u32); 2];
	let t100 = [vdupq_n_u32(((t98) >> 32) as u32); 2];
	let t101: [uint32x4_t; 2] = [vaddq_u32(t99[0], t97[0]), vaddq_u32(t99[1], t97[1])];
	let t102: [uint32x4_t; 2] = [vcltq_u32(t101[0], t99[0]), vcltq_u32(t101[1], t99[1])];
	let t103: [uint32x4_t; 4] = [t101[0], t101[1], vadcq_u32(t100[0], vdupq_n_u32(0), t102[0]), vadcq_u32(t100[1], vdupq_n_u32(0), t102[1])];
	let t104: [uint32x4_t; 2] = [vaddq_u32(x0[0], t103[0]), vaddq_u32(x0[1], t103[1])];
	let t105: [uint32x4_t; 2] = [vcltq_u32(t104[0], x0[0]), vcltq_u32(t104[1], x0[1])];
	let t106: [uint32x4_t; 4] = [t104[0], t104[1], vadcq_u32(x0[2], t103[2], t105[0]), vadcq_u32(x0[3], t103[3], t105[1])];
	let t107: u64 = 7u64;
	let t108 = [vdupq_n_u32((t107) as u32); 2];
	let t109: [uint32x4_t; 2] = [vandq_u32(t106[0], t108[0]), vandq_u32(t106[1], t108[1])];
	let t110 = [vdupq_n_u32(8u32); 2];
	let t111: [uint32x4_t; 2] = [vmulq_u32(t109[0], t110[0]), vmulq_u32(t109[1], t110[1])];
	x4 = t111;
	let mut t112 = [vdupq_n_u32(0); 4];
	{ let v = (t96[0] as *const u64).read_unaligned(); t112[0] = vsetq_lane_u32::<0>(v as u32, t112[0]); t112[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t112[2]); }
	{ let v = (t96[1] as *const u64).read_unaligned(); t112[0] = vsetq_lane_u32::<1>(v as u32, t112[0]); t112[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t112[2]); }
	{ let v = (t96[2] as *const u64).read_unaligned(); t112[0] = vsetq_lane_u32::<2>(v as u32, t112[0]); t112[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t112[2]); }
	{ let v = (t96[3] as *const u64).read_unaligned(); t112[0] = vsetq_lane_u32::<3>(v as u32, t112[0]); t112[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t112[2]); }
	{ let v = (t96[4] as *const u64).read_unaligned(); t112[1] = vsetq_lane_u32::<0>(v as u32, t112[1]); t112[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t112[3]); }
	{ let v = (t96[5] as *const u64).read_unaligned(); t112[1] = vsetq_lane_u32::<1>(v as u32, t112[1]); t112[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t112[3]); }
	{ let v = (t96[6] as *const u64).read_unaligned(); t112[1] = vsetq_lane_u32::<2>(v as u32, t112[1]); t112[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t112[3]); }
	{ let v = (t96[7] as *const u64).read_unaligned(); t112[1] = vsetq_lane_u32::<3>(v as u32, t112[1]); t112[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t112[3]); }
	let t113: [uint32x4_t; 2] = [x4[0], x4[1]];
	let t114: [uint32x4_t; 2] = [vandq_u32(t113[0], vdupq_n_u32(63)), vandq_u32(t113[1], vdupq_n_u32(63))];
	let t115: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t112[0], vnegq_s32(vreinterpretq_s32_u32(t114[0]))), vshlq_u32(t112[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t114[0])))), vorrq_u32(vshlq_u32(t112[1], vnegq_s32(vreinterpretq_s32_u32(t114[1]))), vshlq_u32(t112[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t114[1])))), vshlq_u32(t112[2], vnegq_s32(vreinterpretq_s32_u32(t114[0]))), vshlq_u32(t112[3], vnegq_s32(vreinterpretq_s32_u32(t114[1])))];
	x5 = t115;
	let t116 = [vdupq_n_u32(0u32); 2];
	let t117: [uint32x4_t; 2] = [vcneq_u32(x4[0], t116[0]), vcneq_u32(x4[1], t116[1])];
	let t118 = [vdupq_n_u32((x2).wrapping_mul(8u32)); 2];
	let t119: [uint32x4_t; 2] = [vaddq_u32(x4[0], t118[0]), vaddq_u32(x4[1], t118[1])];
	let t120 = [vdupq_n_u32(64u32); 2];
	let t121: [uint32x4_t; 2] = [vcgtq_u32(t119[0], t120[0]), vcgtq_u32(t119[1], t120[1])];
	let t122: [uint32x4_t; 2] = [vandq_u32(t117[0], t121[0]), vandq_u32(t117[1], t121[1])];
	let mut t123 = [vdupq_n_u32(0); 4];
	let t124: [uint32x4_t; 2] = [vandq_u32(live[0], t122[0]), vandq_u32(live[1], t122[1])];
	let t125: [uint32x4_t; 2] = [vbicq_u32(live[0], t122[0]), vbicq_u32(live[1], t122[1])];
	{
		let t126: u64 = (1u64).wrapping_mul(8u64);
		let t127 = [vdupq_n_u32((t126) as u32); 2];
		let t128 = [vdupq_n_u32(((t126) >> 32) as u32); 2];
		let t129: [uint32x4_t; 2] = [vaddq_u32(x3[0], t127[0]), vaddq_u32(x3[1], t127[1])];
		let t130: [uint32x4_t; 2] = [vcltq_u32(t129[0], x3[0]), vcltq_u32(t129[1], x3[1])];
		let t131: [uint32x4_t; 4] = [t129[0], t129[1], vadcq_u32(x3[2], t128[0], t130[0]), vadcq_u32(x3[3], t128[1], t130[1])];
		let t132: [usize; 8] = core::array::from_fn(|j| t96[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t133 = [vdupq_n_u32(0); 4];
		{ let v = (t132[0] as *const u64).read_unaligned(); t133[0] = vsetq_lane_u32::<0>(v as u32, t133[0]); t133[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t133[2]); }
		{ let v = (t132[1] as *const u64).read_unaligned(); t133[0] = vsetq_lane_u32::<1>(v as u32, t133[0]); t133[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t133[2]); }
		{ let v = (t132[2] as *const u64).read_unaligned(); t133[0] = vsetq_lane_u32::<2>(v as u32, t133[0]); t133[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t133[2]); }
		{ let v = (t132[3] as *const u64).read_unaligned(); t133[0] = vsetq_lane_u32::<3>(v as u32, t133[0]); t133[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t133[2]); }
		{ let v = (t132[4] as *const u64).read_unaligned(); t133[1] = vsetq_lane_u32::<0>(v as u32, t133[1]); t133[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t133[3]); }
		{ let v = (t132[5] as *const u64).read_unaligned(); t133[1] = vsetq_lane_u32::<1>(v as u32, t133[1]); t133[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t133[3]); }
		{ let v = (t132[6] as *const u64).read_unaligned(); t133[1] = vsetq_lane_u32::<2>(v as u32, t133[1]); t133[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t133[3]); }
		{ let v = (t132[7] as *const u64).read_unaligned(); t133[1] = vsetq_lane_u32::<3>(v as u32, t133[1]); t133[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t133[3]); }
		let t134: [uint32x4_t; 2] = [vsubq_u32(t120[0], x4[0]), vsubq_u32(t120[1], x4[1])];
		let t135: [uint32x4_t; 2] = [t134[0], t134[1]];
		let t136: [uint32x4_t; 2] = [vandq_u32(t135[0], vdupq_n_u32(63)), vandq_u32(t135[1], vdupq_n_u32(63))];
		let t137: [uint32x4_t; 4] = [vshlq_u32(t133[0], vreinterpretq_s32_u32(t136[0])), vshlq_u32(t133[1], vreinterpretq_s32_u32(t136[1])), vorrq_u32(vshlq_u32(t133[2], vreinterpretq_s32_u32(t136[0])), vshlq_u32(t133[0], vsubq_s32(vreinterpretq_s32_u32(t136[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t133[3], vreinterpretq_s32_u32(t136[1])), vshlq_u32(t133[1], vsubq_s32(vreinterpretq_s32_u32(t136[1]), vdupq_n_s32(32))))];
		let t138: [uint32x4_t; 4] = [vorrq_u32(x5[0], t137[0]), vorrq_u32(x5[1], t137[1]), vorrq_u32(x5[2], t137[2]), vorrq_u32(x5[3], t137[3])];
		t123 = [vbslq_u32(t124[0], t138[0], t123[0]), vbslq_u32(t124[1], t138[1], t123[1]), vbslq_u32(t124[0], t138[2], t123[2]), vbslq_u32(t124[1], t138[3], t123[3])];
	}
	{
		t123 = [vbslq_u32(t125[0], x5[0], t123[0]), vbslq_u32(t125[1], x5[1], t123[1]), vbslq_u32(t125[0], x5[2], t123[2]), vbslq_u32(t125[1], x5[3], t123[3])];
	}
	res = t123;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b45_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
	let mut live = k;
	let mut res: u64 = 0;
	let t0: u64;
	if ((x0) >= (64u32)) {
		t0 = !(0u64);
	} else {
		t0 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
	}
	res = t0;
	return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn label_leaves_w8(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let x3: *const u8 = *argv.add(3);
	let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
	let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
	let x6: u32 = (*argv.add(6) as *const u32).read_unaligned();
	let mut t1187: bool = !(arg5).is_null();
	if t1187 {
		let mut t1188: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t1188 {
			t1188 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t1189: bool = t1188;
		if !t1189 {
			t1189 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t1187 = t1189;
	}
	if t1187 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 8 <= n {
		let mut live = [vdupq_n_u32(u32::MAX); 2];
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sr4g_p(l, lid0, [vdupq_n_u32(u32::MAX); 2], arg0, arg1, arg2, x3, arg4);
		}
		lid0 += 8;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t1190: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
		let mut live = t1190;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sr4g_p(l, lid0, live, arg0, arg1, arg2, x3, arg4);
		}
	}
}
