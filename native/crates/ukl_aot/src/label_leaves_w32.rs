//! `label_leaves` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec _kernels_rust_is` writes it again.

#![allow(unused, unused_mut, unused_parens, unused_unsafe, unreachable_code, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]

use crate::Launch;
use core::arch::aarch64::*;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.
const LANE: [[u32; 4]; 8] = [[0, 1, 2, 3], [4, 5, 6, 7], [8, 9, 10, 11], [12, 13, 14, 15], [16, 17, 18, 19], [20, 21, 22, 23], [24, 25, 26, 27], [28, 29, 30, 31]];

/// The program's constant memory, which its `__constant` variables are.
static DATA: [u8; 4133] = [16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 110, 111, 100, 101, 67, 111, 117, 110, 116, 44, 119, 111, 114, 100, 115, 44, 114, 101, 115, 117, 108, 116, 65, 114, 101, 110, 97, 44, 116, 97, 115, 107, 67, 111, 117, 110, 116, 44, 105, 116, 101, 109, 67, 111, 117, 110, 116, 44, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 44, 114, 111, 117, 110, 100, 44, 102, 105, 110, 105, 115, 104, 44, 114, 101, 99, 117, 114, 115, 105, 118, 101, 44, 108, 97, 115, 116, 76, 101, 118, 101, 108, 44, 102, 105, 108, 101, 67, 111, 117, 110, 116, 44, 115, 116, 101, 112, 115, 44, 112, 114, 111, 102, 105, 108, 101, 44, 113, 117, 101, 117, 101, 44, 98, 117, 100, 103, 101, 116, 44, 108, 97, 110, 101, 115, 44, 102, 114, 97, 109, 101, 115, 0, 0, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 79, 112, 44, 32, 111, 112, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 83, 116, 97, 116, 117, 115, 44, 32, 115, 116, 97, 116, 117, 115, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 115, 107, 67, 111, 117, 110, 116, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 114, 111, 111, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 115, 108, 111, 116, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 99, 111, 110, 116, 114, 111, 108, 44, 32, 34, 91, 53, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 113, 105, 116, 101, 109, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 32, 42, 32, 50, 48, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 115, 116, 97, 116, 115, 44, 32, 34, 91, 51, 32, 42, 32, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 32, 44, 32, 71, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 114, 105, 100, 44, 32, 44, 32, 76, 65, 78, 69, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 114, 111, 111, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 54, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 111, 117, 110, 116, 115, 44, 32, 34, 91, 115, 108, 111, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 110, 111, 100, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 71, 114, 111, 117, 112, 76, 101, 118, 101, 108, 44, 32, 108, 101, 118, 101, 108, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 79, 112, 44, 32, 111, 112, 115, 44, 32, 34, 91, 111, 112, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 77, 105, 99, 114, 111, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 104, 97, 110, 103, 101, 100, 44, 32, 49, 44, 32, 84, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 105, 108, 101, 44, 32, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 32, 44, 32, 76, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 105, 100, 116, 104, 44, 32, 44, 32, 76, 65, 78, 69, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 105, 109, 100, 76, 97, 110, 101, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 80, 97, 115, 115, 44, 32, 112, 97, 115, 115, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 73, 116, 101, 109, 44, 32, 105, 116, 101, 109, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 105, 116, 101, 109, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 101, 99, 111, 114, 100, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 32, 42, 32, 50, 48, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 99, 117, 114, 115, 111, 114, 44, 32, 34, 91, 50, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 97, 105, 108, 117, 114, 101, 115, 44, 32, 34, 91, 112, 97, 115, 115, 101, 115, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 82, 111, 111, 116, 70, 114, 97, 109, 101, 44, 32, 115, 116, 97, 99, 107, 115, 44, 32, 34, 91, 116, 104, 114, 101, 97, 100, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 102, 114, 97, 109, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 32, 44, 32, 71, 83, 90, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 114, 105, 100, 44, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 115, 116, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 101, 97, 118, 101, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 116, 97, 114, 103, 101, 116, 67, 111, 117, 110, 116, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 105, 100, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 101, 110, 116, 115, 44, 32, 34, 91, 51, 50, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 117, 112, 44, 32, 50, 48, 52, 56, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 115, 44, 32, 50, 53, 55, 44, 32, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 103, 105, 100, 44, 32, 44, 32, 84, 71, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 98, 108, 111, 99, 107, 44, 32, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 68, 111, 99, 86, 105, 101, 119, 44, 32, 100, 111, 99, 115, 44, 32, 34, 91, 115, 104, 97, 112, 101, 46, 102, 105, 108, 101, 67, 111, 117, 110, 116, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 104, 101, 97, 100, 44, 32, 34, 91, 50, 32, 43, 32, 108, 97, 98, 101, 108, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 70, 111, 114, 109, 117, 108, 97, 44, 32, 102, 111, 114, 109, 117, 108, 97, 115, 44, 32, 34, 91, 102, 111, 114, 109, 117, 108, 97, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 119, 111, 114, 100, 115, 44, 32, 34, 91, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 111, 111, 116, 115, 44, 32, 34, 91, 114, 117, 108, 101, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 115, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 102, 111, 114, 109, 117, 108, 97, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 113, 117, 101, 117, 101, 93, 34, 44, 32, 65, 82, 71, 83, 44, 32, 83, 104, 97, 112, 101, 44, 32, 115, 104, 97, 112, 101, 44, 32, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 101, 97, 118, 101, 115, 44, 32, 34, 91, 108, 101, 97, 118, 101, 115, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 114, 111, 103, 114, 97, 109, 44, 32, 34, 91, 112, 114, 111, 103, 114, 97, 109, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 70, 111, 114, 109, 117, 108, 97, 84, 97, 114, 103, 101, 116, 44, 32, 116, 97, 114, 103, 101, 116, 115, 44, 32, 34, 91, 116, 97, 114, 103, 101, 116, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 98, 101, 108, 83, 99, 114, 97, 116, 99, 104, 44, 32, 34, 91, 114, 111, 119, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 114, 117, 108, 101, 115, 44, 32, 34, 91, 114, 117, 108, 101, 115, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 101, 120, 116, 115, 44, 32, 34, 91, 116, 101, 120, 116, 115, 32, 42, 32, 119, 111, 114, 100, 115, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 102, 108, 97, 103, 115, 44, 32, 34, 91, 52, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 101, 110, 116, 115, 44, 32, 34, 91, 51, 50, 32, 42, 32, 115, 104, 97, 112, 101, 46, 114, 101, 99, 117, 114, 115, 105, 118, 101, 93, 34, 44, 32, 82, 79, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 56, 95, 116, 44, 32, 99, 104, 117, 110, 107, 44, 32, 34, 91, 98, 108, 111, 99, 107, 93, 34, 44, 32, 82, 87, 66, 85, 70, 70, 69, 82, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 104, 97, 115, 101, 113, 117, 101, 117, 101, 44, 32, 34, 91, 54, 32, 43, 32, 51, 32, 42, 32, 102, 111, 114, 109, 117, 108, 97, 115, 32, 43, 32, 99, 97, 112, 97, 99, 105, 116, 121, 93, 34, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 112, 97, 114, 116, 44, 32, 54, 52, 44, 32, 83, 72, 65, 82, 69, 68, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 116, 111, 111, 107, 44, 32, 50, 44, 32, 76, 73, 68, 48, 44, 32, 117, 105, 110, 116, 51, 50, 95, 116, 44, 32, 108, 97, 110, 101, 44, 0];

/// Whether a lane of mask `k` is on: a comparison of its bits, which the compiler folds where it knows `k`.
#[inline(always)]
unsafe fn any(k: &[uint32x4_t; 8]) -> bool {
    let mut x = k[0];
    for r in 1..8 {
        x = vorrq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) != 0
}

/// Whether every lane of mask `k` is on, the same way.
#[inline(always)]
unsafe fn all(k: &[uint32x4_t; 8]) -> bool {
    let mut x = k[0];
    for r in 1..8 {
        x = vandq_u32(x, k[r]);
    }
    core::mem::transmute::<uint32x4_t, u128>(x) == u128::MAX
}

/// Mask `k`'s lanes as bits, lane `j` bit `j`.
#[inline(always)]
unsafe fn bits(k: &[uint32x4_t; 8]) -> u32 {
    let w = vld1q_u32([1u32, 2, 4, 8].as_ptr());
    let mut b = 0u32;
    for r in 0..8 {
        b |= vaddvq_u32(vandq_u32(k[r], w)) << (4 * r);
    }
    b
}

/// The largest and the least of the lanes of `a`; the largest of those mask `k` has on (0 for one off).
#[inline(always)]
unsafe fn maxv(a: &[uint32x4_t; 8]) -> u32 {
    let mut m = a[0];
    for r in 1..8 {
        m = vmaxq_u32(m, a[r]);
    }
    vmaxvq_u32(m)
}
#[inline(always)]
unsafe fn minv(a: &[uint32x4_t; 8]) -> u32 {
    let mut m = a[0];
    for r in 1..8 {
        m = vminq_u32(m, a[r]);
    }
    vminvq_u32(m)
}
#[inline(always)]
unsafe fn maxv_on(k: &[uint32x4_t; 8], a: &[uint32x4_t; 8]) -> u32 {
    let mut m = vandq_u32(k[0], a[0]);
    for r in 1..8 {
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
unsafe fn splat(x: u32) -> [uint32x4_t; 8] {
	[vdupq_n_u32(x); 8]
}
/// A 64-bit value in every lane: its halves.
#[inline(always)]
unsafe fn splat64(v: u64) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| vdupq_n_u32(if r < 8 { v as u32 } else { (v >> 32) as u32 }))
}
/// Lanes `f + j`.
#[inline(always)]
unsafe fn iota(f: u32) -> [uint32x4_t; 8] {
	core::array::from_fn(|r| vaddq_u32(vdupq_n_u32(f), vld1q_u32(LANE[r].as_ptr())))
}
/// An array's first registers, and those from its 8th (a 64-bit value's halves).
#[inline(always)]
unsafe fn lo<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r])
}
#[inline(always)]
unsafe fn hi<const N: usize, const M: usize>(a: [uint32x4_t; M]) -> [uint32x4_t; N] {
	core::array::from_fn(|r| a[r + 8])
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
unsafe fn ix_on(k: [uint32x4_t; 8], a: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let s = vdupq_n_u32(maxv_on(&k, &a));
	core::array::from_fn(|r| vbslq_u32(k[r], a[r], s))
}
#[inline]
unsafe fn ld_u8(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u8).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_i8(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i8).read_unaligned() as i32 as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u16(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u16).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_i16(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const i16).read_unaligned() as i32 as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u32(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 8] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		return [vdupq_n_u32((b.add(u as usize * s) as *const u32).read_unaligned() as u32); 8];
	}
	let a = words8(ix);
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ld_u64(b: *const u8, ix: [uint32x4_t; 8], s: usize) -> [uint32x4_t; 16] {
	let u = minv(&ix);
	if u == maxv(&ix) {
		let v = (b.add(u as usize * s) as *const u64).read_unaligned();
		return core::array::from_fn(|r| vdupq_n_u32(if r < 8 { v as u32 } else { (v >> 32) as u32 }));
	}
	let a = words8(ix);
	let mut g = [0u32; 64];
	for j in 0..32 {
		let v = (b.add(a[j] as usize * s) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[32 + j] = (v >> 32) as u32;
	}
	regs16(g)
}
#[inline]
unsafe fn ldj_u8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u8).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u8(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn stjf_u8(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u8).write_unaligned(x[j] as u8);
	}
}
#[inline]
unsafe fn ldj_i8(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i8).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ldj_u16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u16).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u16(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn stjf_u16(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u16).write_unaligned(x[j] as u16);
	}
}
#[inline]
unsafe fn ldj_i16(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const i16).read_unaligned();
		g[j] = v as i32 as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn ldj_u32(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 8] {
	let mut g = [0u32; 32];
	for j in 0..32 {
		let v = (at(j) as *const u32).read_unaligned();
		g[j] = v as u32;
	}
	regs8(g)
}
#[inline]
unsafe fn stj_u32(k: [uint32x4_t; 8], v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words8(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn stjf_u32(v: [uint32x4_t; 8], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words8(v);
	for j in 0..32 {
		(at(j) as *mut u32).write_unaligned(x[j] as u32);
	}
}
#[inline]
unsafe fn ldj_u64(mut at: impl FnMut(usize) -> *const u8) -> [uint32x4_t; 16] {
	let mut g = [0u32; 64];
	for j in 0..32 {
		let v = (at(j) as *const u64).read_unaligned();
		g[j] = v as u32;
		g[32 + j] = (v >> 32) as u32;
	}
	regs16(g)
}
#[inline]
unsafe fn stj_u64(k: [uint32x4_t; 8], v: [uint32x4_t; 16], mut at: impl FnMut(usize) -> *mut u8) {
	let (m, x) = (words8(k), words16(v));
	let mut z = 0u64;
	let sink = core::hint::black_box(&raw mut z) as *mut u8;
	for j in 0..32 {
		(core::hint::select_unpredictable(m[j] != 0, at(j), sink) as *mut u64).write_unaligned((x[32 + j] as u64) << 32 | x[j] as u64);
	}
}
#[inline]
unsafe fn stjf_u64(v: [uint32x4_t; 16], mut at: impl FnMut(usize) -> *mut u8) {
	let x = words16(v);
	for j in 0..32 {
		(at(j) as *mut u64).write_unaligned((x[32 + j] as u64) << 32 | x[j] as u64);
	}
}
/// Where a load reads when no lane of its mask runs (a block entered under a uniform branch none of its lanes
/// takes): zeros, as a store's lanes that are off write to a sink. Its loads' constant offsets lie inside it.
static NO_LANE: [u8; 4096] = [0; 4096];
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's (none on: `NO_LANE`).
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 8], hi: [uint32x4_t; 8], k: [uint32x4_t; 8]) -> [usize; 32] {
	let (l, h, b) = (words8(lo), words8(hi), bits(&k));
	if b == 0 {
		return [NO_LANE.as_ptr() as usize; 32];
	}
	let f = b.trailing_zeros() as usize % 32;
	core::array::from_fn(|j| {
		let j = if b >> j & 1 != 0 { j } else { f };
		((h[j] as u64) << 32 | l[j] as u64) as usize
	})
}
#[inline(always)]
unsafe fn bsl_w(k: [uint32x4_t; 8], a: [uint32x4_t; 16], b: [uint32x4_t; 16]) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| vbslq_u32(k[r % 8], a[r], b[r]))
}
#[inline(always)]
unsafe fn cat_w(a: [uint32x4_t; 8], b: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	core::array::from_fn(|r| if r < 8 { a[r] } else { b[r - 8] })
}
/// Registers' lanes as words, and words as registers' lanes.
#[inline(always)]
unsafe fn words8(a: [uint32x4_t; 8]) -> [u32; 32] {
    core::mem::transmute::<[uint32x4_t; 8], [u32; 32]>(a)
}
#[inline(always)]
unsafe fn regs8(a: [u32; 32]) -> [uint32x4_t; 8] {
    core::mem::transmute::<[u32; 32], [uint32x4_t; 8]>(a)
}
#[inline(always)]
unsafe fn words16(a: [uint32x4_t; 16]) -> [u32; 64] {
    core::mem::transmute::<[uint32x4_t; 16], [u32; 64]>(a)
}
#[inline(always)]
unsafe fn regs16(a: [u32; 64]) -> [uint32x4_t; 16] {
    core::mem::transmute::<[u32; 64], [uint32x4_t; 16]>(a)
}

/// `label_leaves_inner`.
#[inline(always)]
unsafe fn b0_r0r1r2sr4g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, arg2: *const u8, x3: *const u8, arg4: *const u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
	let mut x15: [[uint32x4_t; 8]; 8] = [[Z; 8]; 8];
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9: u32 = 0;
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13 = [Z; 8];
	let mut x14: u32 = 0;
	let mut x16: u32 = 0;
	let mut x17: u32 = 0;
	let mut x18 = [Z; 8];
	let mut x19 = [Z; 16];
	let mut x20: u32 = 0;
	let mut x21: u32 = 0;
	let mut x22: u32 = 0;
	let mut x23: u32 = 0;
	let mut x24: u32 = 0;
	let mut x25 = [Z; 8];
	let mut x26: u32 = 0;
	let mut x27: u32 = 0;
	let mut x28 = [Z; 8];
	let mut x29 = [Z; 8];
	let mut x30 = [Z; 8];
	let mut x31 = [Z; 8];
	let mut x32: u32 = 0;
	let mut x33: u32 = 0;
	let mut live = k;
	if (((x3.wrapping_add(40u64 as usize) as *const u32).read_unaligned()) == (0u32)) {
		return;
	}
	let t0 = [Z; 8];
	x6 = t0;
	let t1 = splat((x3.wrapping_add(40u64 as usize) as *const u32).read_unaligned());
	x7 = t1;
	let mut t2 = live;
	loop {
		let t3 = bits(&t2);
		if t3.count_ones() <= 2 {
			let mut t4 = t3;
			while t4 != 0 {
				let j = t4.trailing_zeros() as usize;
				t4 &= t4 - 1;
				let t5 = words8(x6);
				let mut x6_j: u32 = t5[j];
				let t6 = words8(x7);
				let mut x7_j: u32 = t6[j];
				let t7 = words8(x8);
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
				let mut t8 = words8(x6);
				t8[j] = x6_j as u32;
				x6 = regs8(t8);
				let mut t9 = words8(x7);
				t9[j] = x7_j as u32;
				x7 = regs8(t9);
				let mut t10 = words8(x8);
				t10[j] = x8_j as u32;
				x8 = regs8(t10);
			}
			break;
		}
		// Lanes part: each its own rounds.
		let t11 = sub_r(x7, x6);
		let t12 = splat(1u32);
		let t13 = cgt_r(t11, t12);
		t2 = and_r(t2, t13);
		if !(any(&t2)) { break; }
		let t14 = add_r(x6, x7);
		let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t14[r], vdupq_n_s32(-1)));
		x8 = t15;
		let t16 = ix_on(t2, x8);
		let t17 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t16, 136);
		let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t17[r], vdupq_n_s32(-5)));
		let t19 = splat(2u32);
		let t20 = mul_r(t19, x8);
		let t21 = add_r(t18, t20);
		let t22 = iota(gid0);
		let t23 = cle_r(t21, t22);
		let t24 = and_r(t2, t23);
		{
			x6 = bsl_r(t24, x8, x6);
		}
		let t25 = bic_r(t2, t23);
		{
			x7 = bsl_r(t25, x8, x7);
		}
	}
	let t26 = ix_on(live, x6);
	let t27 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t26, 136);
	let t28: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t27[r], vdupq_n_s32(-5)));
	let t29 = splat(2u32);
	let t30 = mul_r(t29, x6);
	let t31 = add_r(t28, t30);
	x10 = t31;
	let t32 = iota(gid0);
	let t33 = clt_r(t32, x10);
	let t34 = and_r(live, t33);
	{
		live = bic_r(live, t34);
	}
	if any(&live) {
		let t35 = sub_r(t32, x10);
		x11 = t35;
		let t36 = ix_on(live, x6);
		let t37 = ld_u32((arg0).wrapping_add((4u64) as usize) as *const u8, t36, 136);
		x12 = t37;
		let t38 = splat(32u32);
		let t39 = mul_r(x11, t38);
		let t40 = cge_r(t39, x12);
		let t41 = and_r(live, t40);
		{
			live = bic_r(live, t41);
		}
		if any(&live) {
			let t42 = mul_r(x11, t38);
			let t43 = sub_r(x12, t42);
			let t44 = b1_sl_p(l, lid0, live, 32u32, t43);
			x13 = t44;
			x14 = 0u32;
			let mut t45 = live;
			let mut t46 = [Z; 8];
			loop {
				// Lanes part: each its own rounds.
				let t47 = splat(if ((x14) < ((x3.wrapping_add(20u64 as usize) as *const u32).read_unaligned())) { u32::MAX } else { 0 });
				t45 = core::array::from_fn(|r| vandq_u32(vbicq_u32(t45[r], t46[r]), t47[r]));
				if !(any(&t45)) { break; }
				x16 = 0u32;
				loop {
					// Rounds of one condition: every lane goes on.
					if !(((x16) < (8u32))) { break; }
					let t48 = ((((x16) as u64)).wrapping_mul(4u64)) as usize / 4;
					x15[t48] = bsl_r(t45, t0, x15[t48]);
					let t49 = x16;
					x16 = (x16).wrapping_add(1u32);
				}
				x17 = 0u32;
				let mut t50 = t45;
				let mut t51 = [Z; 8];
				loop {
					// Lanes part: each its own rounds.
					let t52 = splat(x17);
					let t53 = clt_r(t52, x13);
					t50 = core::array::from_fn(|r| vandq_u32(vbicq_u32(t50[r], t51[r]), t53[r]));
					if !(any(&t50)) { break; }
					let t54 = mul_r(x11, t38);
					let t55 = splat(x17);
					let t56 = add_r(t54, t55);
					x18 = t56;
					let t57 = b2_r4al136r0l_p(l, lid0, t50, arg4, arg0, x6, x18);
					x19 = t57;
					let t58: [uint32x4_t; 8] = lo(x19);
					let t59: [uint32x4_t; 8] = hi(x19);
					let t60 = addrs(t58, t59, t50);
					x20 = 0u32;
					't61: loop {
						// Rounds of one condition: every lane goes on.
						if !(((x20) < (8u32))) { break; }
						x21 = (x14).wrapping_add(x20);
						if ((x21) >= ((x3.wrapping_add(20u64 as usize) as *const u32).read_unaligned())) {
							break 't61;
						}
						let t62 = and_r(t50, live);
						if any(&t62) {
							x22 = (arg1.wrapping_add((((x21) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
							x23 = ((x22) & (255u32));
							x24 = (x22).wrapping_shr((8u32) as u32);
							if ((x23) == (1u32)) {
								let mut t63 = [Z; 8];
								t63 = ldj_u16(|j| (t60[j] as *const u8) as *const u8);
								let t64 = splat(x24);
								let t65 = ceq_r(t63, t64);
								x25 = bsl_r(t62, t65, x25);
							} else {
								if ((x23) == (2u32)) {
									let t66: u64 = 2u64;
									let t67 = splat((t66) as u32);
									let t68 = add_r(lo(x19), t67);
									let t69 = clt_r(t68, lo(x19));
									let t70: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t69[r]));
									let t71 = cat_w(lo(t68), t70);
									let t72: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((2u64) as u64 as usize));
									let mut t73 = [Z; 8];
									t73 = ldj_u16(|j| (t72[j] as *const u8) as *const u8);
									let t74 = splat(x24);
									let t75 = ceq_r(t73, t74);
									x25 = bsl_r(t62, t75, x25);
								} else {
									if ((x23) == (3u32)) {
										x26 = ((x24) & (255u32));
										x27 = (x24).wrapping_shr((8u32) as u32);
										let t76: u64 = 4u64;
										let t77 = splat((t76) as u32);
										let t78 = add_r(lo(x19), t77);
										let t79 = clt_r(t78, lo(x19));
										let t80: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t79[r]));
										let t81 = cat_w(lo(t78), t80);
										let t82: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((4u64) as u64 as usize));
										let mut t83 = [Z; 8];
										t83 = ldj_u8(|j| (t82[j] as *const u8) as *const u8);
										let t84 = splat(x26);
										let t85 = and_r(t83, t84);
										let t86 = splat(x26);
										let t87 = ceq_r(t85, t86);
										let t88: u64 = 4u64;
										let t89 = splat((t88) as u32);
										let t90 = add_r(lo(x19), t89);
										let t91 = clt_r(t90, lo(x19));
										let t92: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t91[r]));
										let t93 = cat_w(lo(t90), t92);
										let t94: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((4u64) as u64 as usize));
										let mut t95 = [Z; 8];
										t95 = ldj_u8(|j| (t94[j] as *const u8) as *const u8);
										let t96 = splat(x27);
										let t97 = and_r(t95, t96);
										let t98 = ceq_r(t97, t0);
										let t99 = and_r(t87, t98);
										x25 = bsl_r(t62, t99, x25);
									} else {
										if ((x23) == (18u32)) {
											let t100 = b3_r4al136r0l_p(l, lid0, t62, arg4, arg0, x6, x18);
											let t101 = splat(4294967295u32);
											let t102 = cne_r(t100, t101);
											x25 = bsl_r(t62, t102, x25);
										} else {
											if ((x23) == (22u32)) {
												let t103 = splat(if ((0i32) != 0) { u32::MAX } else { 0 });
												x25 = bsl_r(t62, t103, x25);
												let t104 = b4_r4al136r0l_p(l, lid0, t62, arg4, arg0, x6, x18);
												x28 = bsl_r(t62, t104, x28);
												let t105 = b5_r4al136r0ll_p(l, lid0, t62, arg4, arg0, x6, x18, x28);
												x29 = bsl_r(t62, t105, x29);
												let mut t106 = t62;
												let mut t107 = [Z; 8];
												loop {
													// Lanes part: each its own rounds.
													let t108 = splat(4294967295u32);
													let t109 = cne_r(x29, t108);
													t106 = core::array::from_fn(|r| vandq_u32(vbicq_u32(t106[r], t107[r]), t109[r]));
													if !(any(&t106)) { break; }
													let t110 = b6_r4al136r0l_p(l, lid0, t106, arg4, arg0, x6, x29);
													let t111 = splat(1u32);
													let t112 = and_r(t110, t111);
													let t113 = cne_r(t112, t0);
													let t114 = and_r(t106, t113);
													if any(&t114) {
														let t115 = splat(if ((1i32) != 0) { u32::MAX } else { 0 });
														x25 = bsl_r(t114, t115, x25);
														t107 = orr_r(t107, t114);
													}
													let t116: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t106[r], live[r]), t107[r]));
													let t117 = b5_r4al136r0ll_p(l, lid0, t116, arg4, arg0, x6, x29, x28);
													x29 = bsl_r(t116, t117, x29);
												}
											} else {
												if ((x23) == (21u32)) {
													let t118 = splat(if ((0i32) != 0) { u32::MAX } else { 0 });
													x25 = bsl_r(t62, t118, x25);
													let t119 = b7_r4al136r0l_p(l, lid0, t62, arg4, arg0, x6, x18);
													x30 = bsl_r(t62, t119, x30);
													let t120 = b8_r4al136r0l_p(l, lid0, t62, arg4, arg0, x6, x18);
													x31 = bsl_r(t62, t120, x31);
													let mut t121 = t62;
													let mut t122 = [Z; 8];
													loop {
														// Lanes part: each its own rounds.
														let t123 = splat(4294967295u32);
														let t124 = cne_r(x31, t123);
														t121 = core::array::from_fn(|r| vandq_u32(vbicq_u32(t121[r], t122[r]), t124[r]));
														if !(any(&t121)) { break; }
														let t125 = b6_r4al136r0l_p(l, lid0, t121, arg4, arg0, x6, x31);
														let t126 = splat(1u32);
														let t127 = and_r(t125, t126);
														let t128 = cne_r(t127, t0);
														let t129 = and_r(t121, t128);
														if any(&t129) {
															let t130 = splat(if ((1i32) != 0) { u32::MAX } else { 0 });
															x25 = bsl_r(t129, t130, x25);
															t122 = orr_r(t122, t129);
														}
														let t131: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t121[r], live[r]), t122[r]));
														let t132 = b5_r4al136r0ll_p(l, lid0, t131, arg4, arg0, x6, x31, x30);
														x31 = bsl_r(t131, t132, x31);
													}
												} else {
													if ((x23) == (19u32)) {
														let t133 = b9_r4al136r0l_p(l, lid0, t62, arg4, arg0, x6, x18);
														let t134 = splat(1u32);
														let t135 = cgt_r(t133, t134);
														x25 = bsl_r(t62, t135, x25);
													} else {
														let t136 = splat(if ((0i32) != 0) { u32::MAX } else { 0 });
														x25 = bsl_r(t62, t136, x25);
														x32 = 0u32;
														let mut t137 = t62;
														let mut t138 = [Z; 8];
														loop {
															// Lanes part: each its own rounds.
															let t139: u64 = 5u64;
															let t140 = splat((t139) as u32);
															let t141 = add_r(lo(x19), t140);
															let t142 = clt_r(t141, lo(x19));
															let t143: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t142[r]));
															let t144 = cat_w(lo(t141), t143);
															let t145: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((5u64) as u64 as usize));
															let mut t146 = [Z; 8];
															t146 = ldj_u8(|j| (t145[j] as *const u8) as *const u8);
															let t147 = splat(x32);
															let t148 = clt_r(t147, t146);
															t137 = core::array::from_fn(|r| vandq_u32(vbicq_u32(t137[r], t138[r]), t148[r]));
															if !(any(&t137)) { break; }
															let t149: u64 = 8u64;
															let t150 = splat((t149) as u32);
															let t151 = add_r(lo(x19), t150);
															let t152 = clt_r(t151, lo(x19));
															let t153: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t152[r]));
															let t154 = cat_w(lo(t151), t153);
															let t155: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((8u64) as u64 as usize));
															let t156: u64 = (((x32) as u64)).wrapping_mul(2u64);
															let t157 = splat((t156) as u32);
															let t158 = splat(((t156) >> 32) as u32);
															let t159 = add_r(lo(t154), t157);
															let t160 = clt_r(t159, lo(t154));
															let t161 = adc_r(hi(t154), t158, t160);
															let t162 = cat_w(lo(t159), t161);
															let t163: [usize; 32] = core::array::from_fn(|j| t155[j].wrapping_add(((((x32) as u64)).wrapping_mul(2u64)) as u64 as usize));
															let mut t164 = [Z; 8];
															t164 = ldj_u16(|j| (t163[j] as *const u8) as *const u8);
															let t165 = splat(x24);
															let t166 = ceq_r(t164, t165);
															let t167 = and_r(t137, t166);
															if any(&t167) {
																let t168 = splat(if ((1i32) != 0) { u32::MAX } else { 0 });
																x25 = bsl_r(t167, t168, x25);
																t138 = orr_r(t138, t167);
															}
															let t169: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t137[r], live[r]), t138[r]));
															if any(&t169) {
																let t170: u64 = 8u64;
																let t171 = splat((t170) as u32);
																let t172 = add_r(lo(x19), t171);
																let t173 = clt_r(t172, lo(x19));
																let t174: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(x19[(r + 8)], Z, t173[r]));
																let t175 = cat_w(lo(t172), t174);
																let t176: [usize; 32] = core::array::from_fn(|j| t60[j].wrapping_add((8u64) as u64 as usize));
																let t177: u64 = (((x32) as u64)).wrapping_mul(2u64);
																let t178 = splat((t177) as u32);
																let t179 = splat(((t177) >> 32) as u32);
																let t180 = add_r(lo(t175), t178);
																let t181 = clt_r(t180, lo(t175));
																let t182 = adc_r(hi(t175), t179, t181);
																let t183 = cat_w(lo(t180), t182);
																let t184: [usize; 32] = core::array::from_fn(|j| t176[j].wrapping_add(((((x32) as u64)).wrapping_mul(2u64)) as u64 as usize));
																let mut t185 = [Z; 8];
																t185 = ldj_u16(|j| (t184[j] as *const u8) as *const u8);
																let t186 = splat(x24);
																let t187 = cgt_r(t185, t186);
																let t188 = and_r(t169, t187);
																if any(&t188) {
																	t138 = orr_r(t138, t188);
																}
															}
															let t189: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t137[r], live[r]), t138[r]));
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
							let t191 = and_r(t62, live);
							if any(&t191) {
								let t192 = ((((x20) as u64)).wrapping_mul(4u64)) as usize / 4;
								let t193: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(x25[r]));
								let t194: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t193[r], vdupq_n_s32(((x17) & 31) as i32)));
								let t195 = orr_r(x15[t192], t194);
								x15[t192] = bsl_r(t191, t195, x15[t192]);
							}
						}
						let t196 = x20;
						x20 = (x20).wrapping_add(1u32);
					}
					let t197: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t50[r], live[r]), t51[r]));
					let t198 = x17;
					x17 = (x17).wrapping_add(1u32);
				}
				let t199: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t45[r], live[r]), t46[r]));
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
						let t202 = splat(((x14).wrapping_add(x33)).wrapping_mul((x3.wrapping_add(32u64 as usize) as *const u32).read_unaligned()));
						let t203 = add_r(t202, x10);
						let t204 = add_r(t203, x11);
						let t205 = words8(t204);
						stj_u32(t199, x15[t201], |j| (arg2 as *mut u32).add(t205[j] as usize) as *mut u8);
						let t206 = x33;
						x33 = (x33).wrapping_add(1u32);
					}
				}
				let t207: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(t45[r], live[r]), t46[r]));
				x14 = (x14).wrapping_add(8u32);
			}
		}
	}
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b1_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32, x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(x0);
	let t1 = clt_r(t0, x1);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		let t5 = splat(x0);
		t2 = bsl_r(t3, t5, t2);
	}
	{
		t2 = bsl_r(t4, x1, t2);
	}
	res = t2;
	return res;
}

/// `tape_code`.
#[inline(always)]
unsafe fn b2_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	x3 = t0;
	let t1: [uint32x4_t; 8] = lo(x3);
	let t2: [uint32x4_t; 8] = hi(x3);
	let t3 = addrs(t1, t2, live);
	let t4 = ix_on(live, x1i);
	let t5 = ld_u32((arg0).wrapping_add((84u64) as usize) as *const u8, t4, 136);
	let t6: [uint32x4_t; 8] = core::array::from_fn(|r| vreinterpretq_u32_s32(vshrq_n_s32::<31>(vreinterpretq_s32_u32(t5[r]))));
	let t7 = cat_w(lo(t5), lo(t6));
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x3[r], t7[r]));
	let t9 = clt_r(t8, lo(x3));
	let t10 = adc_r(hi(x3), hi(t7), t9);
	let t11 = cat_w(lo(t8), t10);
	let t12 = b11_lal136r0l_p(l, lid0, live, x3, arg0, x1i, x2);
	let t13: [uint32x4_t; 8] = core::array::from_fn(|r| vmulq_u32(t12[r], vdupq_n_u32(24)));
	let t14: [uint32x4_t; 8] = core::array::from_fn(|r| vmulhiq_u32(t12[r], vdupq_n_u32(24)));
	let t15 = add_r(lo(t11), t13);
	let t16 = clt_r(t15, lo(t11));
	let t17 = adc_r(hi(t11), t14, t16);
	let t18 = cat_w(lo(t15), t17);
	res = t18;
	return res;
}

/// `tape_next`.
#[inline(always)]
unsafe fn b3_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b12_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_siblings_end`.
#[inline(always)]
unsafe fn b4_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b13_r4al136r0l_p(l, lid0, live, arg4, arg0, x1i, x2);
	x3 = t0;
	let t1 = splat(4294967295u32);
	let t2 = ceq_r(x3, t1);
	let mut t3 = [Z; 8];
	let t4 = and_r(live, t2);
	let t5 = bic_r(live, t2);
	{
		let t6 = [Z; 8];
		t3 = bsl_r(t4, t6, t3);
	}
	if any(&t5) {
		let t7: [uint32x4_t; 8];
		let t8 = bits(&t5);
		if t8.count_ones() <= 2 {
			let t9 = words8(x1i);
			let t10 = words8(x3);
			let mut t11 = [0u32; 32];
			let mut t12 = t8;
			while t12 != 0 {
				let j = t12.trailing_zeros() as usize;
				t12 &= t12 - 1;
				let t13 = splat(t10[j]);
				let t14 = b7_r4sl_p(l, lid0, splat(u32::MAX), arg4, (arg0 as *const u8).wrapping_add(((t9[j]) as usize).wrapping_mul(136)), t13);
				t11[j] = vgetq_lane_u32::<0>(t14[0]);
			}
			t7 = regs8(t11);
		} else {
			let t15 = b7_r4al136r0l_p(l, lid0, t5, arg4, arg0, x1i, x3);
			t7 = t15;
		}
		t3 = bsl_r(t5, t7, t3);
	}
	res = t3;
	return res;
}

/// `tape_next_in`.
#[inline(always)]
unsafe fn b5_r4al136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b14_lal136r0ll_p(l, lid0, live, t0, arg0, x1i, x2, x3);
	res = t1;
	return res;
}

/// `tape_flags`.
#[inline(always)]
unsafe fn b6_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b2_r4al136r0l_p(l, lid0, live, arg4, arg0, x1i, x2);
	let t1: u64 = 4u64;
	let t2 = splat((t1) as u32);
	let t3 = add_r(lo(t0), t2);
	let t4 = clt_r(t3, lo(t0));
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t0[(r + 8)], Z, t4[r]));
	let t6 = cat_w(lo(t3), t5);
	let t7: [uint32x4_t; 8] = lo(t6);
	let t8: [uint32x4_t; 8] = hi(t6);
	let t9 = addrs(t7, t8, live);
	let mut t10 = [Z; 8];
	t10 = ldj_u8(|j| (t9[j] as *const u8) as *const u8);
	res = t10;
	return res;
}

/// `tape_end_of`.
#[inline(always)]
unsafe fn b7_r4sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, x1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4s_p(l, lid0, live, arg4, x1);
	let t1 = b15_ssl_p(l, lid0, live, t0, x1, x2);
	res = t1;
	return res;
}

/// `tape_end_of`.
#[inline(always)]
unsafe fn b7_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b15_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_first_child`.
#[inline(always)]
unsafe fn b8_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b16_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_size`.
#[inline(always)]
unsafe fn b9_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b17_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b10_r4s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, x1: *const u8) -> *const u8 {
    let a0: u32 = (x1.wrapping_add(124u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x1 as *const u32).read_unaligned();
	let mut live = k;
	let mut res: *const u8 = core::ptr::null();
	res = arg4.wrapping_add((((((((((a0) as u64)).wrapping_shl((32u64) as u32)) | (((a1) as u64)))) as i64)) as u64) as usize);
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b10_r4al136r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = ix_on(live, x1i);
	let t1 = ld_u32((arg0).wrapping_add((124u64) as usize) as *const u8, t0, 136);
	let t2 = cat_w([Z; 8], lo(t1));
	let t3 = ix_on(live, x1i);
	let t4 = ld_u32(arg0 as *const u8, t3, 136);
	let t5 = orr_r(lo(t2), t4);
	let t6 = cat_w(t5, hi(t2));
	let t7 = cat_w(live, live);
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[r], t7[r]));
	let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[(r + 8)], t7[(r + 8)]));
	let t10 = cat_w(t8, t9);
	let t11: u64 = ((arg4) as usize as u64);
	let t12 = splat((t11) as u32);
	let t13 = splat(((t11) >> 32) as u32);
	let t14 = add_r(t12, lo(t10));
	let t15 = clt_r(t14, t12);
	let t16 = adc_r(t13, hi(t10), t15);
	let t17 = cat_w(lo(t14), t16);
	res = t17;
	return res;
}

/// `tape_number`.
#[inline(always)]
unsafe fn b11_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b18_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_next`.
#[inline(always)]
unsafe fn b12_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b19_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `tape_parent`.
#[inline(always)]
unsafe fn b13_r4al136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg4: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b10_r4al136r0_p(l, lid0, live, arg4, arg0, x1i);
	let t1 = b20_lal136r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `clif_tape_next_in`.
#[inline(always)]
unsafe fn b14_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b21_lal136r0ll_p(l, lid0, live, x0, arg0, x1i, x2, x3);
	res = t0;
	return res;
}

/// `clif_tape_end_of`.
#[inline(always)]
unsafe fn b15_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b22_ssl_p(l, lid0, live, x0, x1, x2);
	res = t0;
	return res;
}

/// `clif_tape_end_of`.
#[inline(always)]
unsafe fn b15_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b22_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_first_child`.
#[inline(always)]
unsafe fn b16_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_size`.
#[inline(always)]
unsafe fn b17_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_tape_number`.
#[inline(always)]
unsafe fn b18_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b25_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_f16_next`.
#[inline(always)]
unsafe fn b19_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b26_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let t1 = splat(4294967295u32);
	let t2 = ceq_r(x3, t1);
	let mut t3 = [Z; 8];
	let t4 = and_r(live, t2);
	let t5 = bic_r(live, t2);
	{
		t3 = bsl_r(t4, t1, t3);
	}
	if any(&t5) {
		let t6 = b22_lal136r0l_p(l, lid0, t5, x0, arg0, x1i, x3);
		let t7: [uint32x4_t; 8];
		let t8 = bits(&t5);
		if t8.count_ones() <= 2 {
			let t9 = words16(x0);
			let t10 = words8(x1i);
			let t11 = words8(x2);
			let t12 = words8(t6);
			let mut t13 = [0u32; 32];
			let mut t14 = t8;
			while t14 != 0 {
				let j = t14.trailing_zeros() as usize;
				t14 &= t14 - 1;
				let t15 = b21_ssss_f(l, lid0, (((t9[32 + j] as u64) << 32 | t9[j] as u64) as usize as *const u8), (arg0 as *const u8).wrapping_add(((t10[j]) as usize).wrapping_mul(136)), t11[j], t12[j]);
				t13[j] = vgetq_lane_u32::<0>(t15[0]);
			}
			t7 = regs8(t13);
		} else {
			let t16 = b21_lal136r0ll_p(l, lid0, t5, x0, arg0, x1i, x2, t6);
			t7 = t16;
		}
		t3 = bsl_r(t5, t7, t3);
	}
	res = t3;
	return res;
}

/// `clif_tape_parent`.
#[inline(always)]
unsafe fn b20_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b26_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	res = t0;
	return res;
}

/// `clif_f16_next_in`.
#[inline(always)]
unsafe fn b21_ssss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32, x3: u32) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = b22_sss_f(l, lid0, x0, x1, x2);
	x4 = t0;
	let t1 = splat(x3);
	let t2 = clt_r(x4, t1);
	let mut t3 = [Z; 8];
	let t4 = mvn_r(t2);
	{
		t3 = bsl_r(t2, x4, t3);
	}
	{
		let t5 = splat(4294967295u32);
		t3 = bsl_r(t4, t5, t3);
	}
	res = t3;
	return res;
}

/// `clif_f16_next_in`.
#[inline(always)]
unsafe fn b21_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b22_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x4 = t0;
	let t1 = clt_r(x4, x3);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		t2 = bsl_r(t3, x4, t2);
	}
	{
		let t5 = splat(4294967295u32);
		t2 = bsl_r(t4, t5, t2);
	}
	res = t2;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 8] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = b24_sss_f(l, lid0, x0, x1, x2);
	let t1 = b27_sssl_f(l, lid0, x0, x1, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b24_ssl_p(l, lid0, live, x0, x1, x2);
	let t1 = b27_ssll_p(l, lid0, live, x0, x1, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_end_of`.
#[inline(always)]
unsafe fn b22_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	let t1 = b27_lal136r0ll_p(l, lid0, live, x0, arg0, x1i, x2, t0);
	res = t1;
	return res;
}

/// `clif_f16_first_child`.
#[inline(always)]
unsafe fn b23_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b24_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	let t1 = splat(1u32);
	let t2 = cgt_r(t0, t1);
	let mut t3 = [Z; 8];
	let t4 = and_r(live, t2);
	let t5 = bic_r(live, t2);
	{
		let t6 = add_r(x2, t1);
		t3 = bsl_r(t4, t6, t3);
	}
	{
		let t7 = splat(4294967295u32);
		t3 = bsl_r(t5, t7, t3);
	}
	res = t3;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = b28_sss_f(l, lid0, x0, x1, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = splat((t1) as u32);
	let t3 = and_r(lo(x3), t2);
	let t4: u64 = 0u64;
	let t5 = splat((t4) as u32);
	let t6 = ceq_r(t3, t5);
	let t7 = mvn_r(t6);
	let mut t8 = [Z; 8];
	let t9 = mvn_r(t7);
	{
		let t10 = splat(1u32);
		t8 = bsl_r(t7, t10, t8);
	}
	if any(&t9) {
		let t11: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<1>(x3[r]), vshlq_n_u32::<31>(x3[(r + 8)])));
		let t12: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<1>(x3[(r + 8)]));
		let t13 = cat_w(t11, t12);
		let t14: [uint32x4_t; 8] = lo(t13);
		let t15: [uint32x4_t; 8];
		let t16 = bits(&t9);
		if t16.count_ones() <= 2 {
			let t17 = words8(t14);
			let mut t18 = [0u32; 32];
			let mut t19 = t16;
			while t19 != 0 {
				let j = t19.trailing_zeros() as usize;
				t19 &= t19 - 1;
				let t20 = b29_ss_f(l, lid0, t17[j], 1u32);
				t18[j] = t20;
			}
			t15 = regs8(t18);
		} else {
			let t21 = b29_ls_p(l, lid0, t9, t14, 1u32);
			t15 = t21;
		}
		t8 = bsl_r(t9, t15, t8);
	}
	res = t8;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b28_ssl_p(l, lid0, live, x0, x1, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = splat((t1) as u32);
	let t3 = and_r(lo(x3), t2);
	let t4: u64 = 0u64;
	let t5 = splat((t4) as u32);
	let t6 = ceq_r(t3, t5);
	let t7 = mvn_r(t6);
	let mut t8 = [Z; 8];
	let t9 = and_r(live, t7);
	let t10 = bic_r(live, t7);
	{
		let t11 = splat(1u32);
		t8 = bsl_r(t9, t11, t8);
	}
	if any(&t10) {
		let t12: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<1>(x3[r]), vshlq_n_u32::<31>(x3[(r + 8)])));
		let t13: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<1>(x3[(r + 8)]));
		let t14 = cat_w(t12, t13);
		let t15: [uint32x4_t; 8] = lo(t14);
		let t16: [uint32x4_t; 8];
		let t17 = bits(&t10);
		if t17.count_ones() <= 2 {
			let t18 = words8(t15);
			let mut t19 = [0u32; 32];
			let mut t20 = t17;
			while t20 != 0 {
				let j = t20.trailing_zeros() as usize;
				t20 &= t20 - 1;
				let t21 = b29_ss_f(l, lid0, t18[j], 1u32);
				t19[j] = t21;
			}
			t16 = regs8(t19);
		} else {
			let t22 = b29_ls_p(l, lid0, t10, t15, 1u32);
			t16 = t22;
		}
		t8 = bsl_r(t10, t16, t8);
	}
	res = t8;
	return res;
}

/// `clif_f16_size`.
#[inline(always)]
unsafe fn b24_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b28_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let t1: u64 = 1u64;
	let t2 = splat((t1) as u32);
	let t3 = and_r(lo(x3), t2);
	let t4: u64 = 0u64;
	let t5 = splat((t4) as u32);
	let t6 = ceq_r(t3, t5);
	let t7 = mvn_r(t6);
	let mut t8 = [Z; 8];
	let t9 = and_r(live, t7);
	let t10 = bic_r(live, t7);
	{
		let t11 = splat(1u32);
		t8 = bsl_r(t9, t11, t8);
	}
	if any(&t10) {
		let t12: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<1>(x3[r]), vshlq_n_u32::<31>(x3[(r + 8)])));
		let t13: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<1>(x3[(r + 8)]));
		let t14 = cat_w(t12, t13);
		let t15: [uint32x4_t; 8] = lo(t14);
		let t16: [uint32x4_t; 8];
		let t17 = bits(&t10);
		if t17.count_ones() <= 2 {
			let t18 = words8(t15);
			let mut t19 = [0u32; 32];
			let mut t20 = t17;
			while t20 != 0 {
				let j = t20.trailing_zeros() as usize;
				t20 &= t20 - 1;
				let t21 = b29_ss_f(l, lid0, t18[j], 1u32);
				t19[j] = t21;
			}
			t16 = regs8(t19);
		} else {
			let t22 = b29_ls_p(l, lid0, t10, t15, 1u32);
			t16 = t22;
		}
		t8 = bsl_r(t10, t16, t8);
	}
	res = t8;
	return res;
}

/// `clif_f16_number`.
#[inline(always)]
unsafe fn b25_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x2[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t1 = b31_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, t0);
	let t2 = splat(1023u32);
	let t3 = and_r(x2, t2);
	let t4 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, t1, t3, 1u32);
	let t5: [uint32x4_t; 8] = lo(t4);
	x3 = t5;
	let t6 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 76u32);
	x4 = t6;
	let t7 = clt_r(x3, x4);
	let t8 = and_r(live, t7);
	{
		res = bsl_r(t8, x3, res);
		live = bic_r(live, t8);
	}
	if any(&live) {
		let t9 = b33_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, x2);
		let t10: [uint32x4_t; 8] = lo(t9);
		x5 = t10;
		let t11 = sub_r(x3, x4);
		let t12 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 80u32);
		let t13 = mul_r(t11, t12);
		let t14 = add_r(x4, t13);
		let t15 = add_r(t14, x5);
		res = bsl_r(live, t15, res);
		live = bic_r(live, live);
	}
	res
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b26_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b34_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x2);
	x3 = t0;
	let mut t1 = live;
	loop {
		// Lanes part: each its own rounds.
		let t2: u64 = (1u64).wrapping_shl((32u64) as u32);
		let t3 = splat((t2) as u32);
		let t4 = splat(((t2) >> 32) as u32);
		let t5 = and_r(lo(x3), t3);
		let t6 = and_r(hi(x3), t4);
		let t7 = cat_w(t5, t6);
		let t8: u64 = 0u64;
		let t9 = splat((t8) as u32);
		let t10: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(t7[r], t7[(r + 8)], t9[r], Z));
		let t11 = mvn_r(t10);
		t1 = and_r(t1, t11);
		if !(any(&t1)) { break; }
		let t12: [uint32x4_t; 8] = lo(x3);
		let t13 = b34_lal136r0l_p(l, lid0, t1, x0, arg0, x1i, t12);
		x3 = bsl_w(t1, t13, x3);
	}
	let t14: [uint32x4_t; 8] = lo(x3);
	res = t14;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b27_sssl_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = splat(x2);
	let t1 = add_r(t0, x3);
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
unsafe fn b27_ssll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = add_r(x2, x3);
	let t1 = splat(1u32);
	let t2 = add_r(x2, t1);
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
	let t7 = add_r(x2, t1);
	let t8 = b35_ll_p(l, lid0, live, t6, t7);
	res = t8;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b27_lal136r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = add_r(x2, x3);
	let t1 = splat(1u32);
	let t2 = add_r(x2, t1);
	let t3 = b35_ll_p(l, lid0, live, t0, t2);
	let mut t4: bool = ((16u32) != (0u32));
	if t4 {
		t4 = ((4u32) == (12u32));
	}
	let t5: [uint32x4_t; 8];
	if t4 {
		let t6 = splat(16u32);
		t5 = t6;
	} else {
		let t7 = ix_on(live, x1i);
		let t8 = ld_u32((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t7, 136);
		t5 = t8;
	}
	let t9 = b36_ll_p(l, lid0, live, t3, t5);
	let t10 = add_r(x2, t1);
	let t11 = b35_ll_p(l, lid0, live, t9, t10);
	res = t11;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_sss_f(l: &Launch, lid0: u32, x0: *const u8, x1: *const u8, x2: u32) -> [uint32x4_t; 16] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 16];
	let t0 = splat64(((x0) as usize as u64));
	let t1 = splat(x2);
	let t2 = b37_lssl_p(l, lid0, splat(u32::MAX), t0, x1, 0u32, t1);
	res = t2;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_ssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: *const u8, x1: *const u8, x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = splat64(((x0) as usize as u64));
	let t1 = b37_lssl_p(l, lid0, live, t0, x1, 0u32, x2);
	res = t1;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b28_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b37_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, x2);
	res = t0;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b29_ss_f(l: &Launch, lid0: u32, x0: u32, x1: u32) -> u32 {
	let mut live = splat(u32::MAX);
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
unsafe fn b29_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(x1);
	let t1 = clt_r(x0, t0);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		let t5 = splat(x1);
		t2 = bsl_r(t3, t5, t2);
	}
	{
		t2 = bsl_r(t4, x0, t2);
	}
	res = t2;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lssls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b38_lsls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lslls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: [uint32x4_t; 8], x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b38_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b30_lal136r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b38_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b31_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b32_lss_p(l, lid0, live, x0, x1, 40u32);
	let t1 = splat(t0);
	let t2 = mul_r(x3, t1);
	let t3 = b39_lss_p(l, lid0, live, x0, x1, 0u32);
	let t4 = splat(t3);
	let t5 = add_r(t2, t4);
	x4 = t5;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t6 = b32_lss_p(l, lid0, live, x0, x1, 4u32);
	let t7: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vdupq_n_s32(((10u32) & 31) as i32)));
	let t8 = splat(t6);
	let t9 = sub_r(t8, t7);
	let t10 = b36_ls_p(l, lid0, live, t9, 1024u32);
	x5 = t10;
	let t11 = b39_lss_p(l, lid0, live, x0, x1, x2);
	let t12 = splat(t11);
	let t13 = mul_r(x5, t12);
	let t14 = add_r(x4, t13);
	res = t14;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b31_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 40u32);
	let t1 = mul_r(x3, t0);
	let t2 = b39_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 0u32);
	let t3 = add_r(t1, t2);
	x4 = t3;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t4 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vdupq_n_s32(((10u32) & 31) as i32)));
	let t6 = sub_r(t4, t5);
	let t7 = b36_ls_p(l, lid0, live, t6, 1024u32);
	x5 = t7;
	let t8 = b39_lal136r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	let t9 = mul_r(x5, t8);
	let t10 = add_r(x4, t9);
	res = t10;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b32_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32) -> u32 {
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
unsafe fn b32_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((x2) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 136);
		t1 = t4;
	}
	res = t1;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b33_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4: u32 = 0;
	let mut x5: u32 = 0;
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9: u32 = 0;
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13 = [Z; 8];
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16: u64 = 0;
	let mut x17 = [Z; 16];
	let mut x18 = [Z; 8];
	let mut x19 = [Z; 8];
	let mut x20 = [Z; 16];
	let mut x21 = [Z; 16];
	let mut x22 = [Z; 16];
	let mut x23 = [Z; 8];
	let mut x24: u32 = 0;
	let mut x25 = [Z; 8];
	let mut x26 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b32_lss_p(l, lid0, live, x0, x1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b32_lss_p(l, lid0, live, x0, x1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b30_lssls_p(l, lid0, live, x0, x1, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b30_lssls_p(l, lid0, live, x0, x1, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b32_lss_p(l, lid0, live, x0, x1, 4u32);
	x9 = ((t11).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
	let t12 = mul_r(x6, t3);
	let t13 = splat(x9);
	let t14 = sub_r(t13, t12);
	let t15 = b1_sl_p(l, lid0, live, 4u32, t14);
	x10 = t15;
	let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	let t17 = splat(3u32);
	let t18 = and_r(t16, t17);
	x11 = t18;
	let t19 = [Z; 8];
	let t20 = ceq_r(x11, t19);
	let mut t21 = [Z; 8];
	let t22 = and_r(live, t20);
	let t23 = bic_r(live, t20);
	{
		t21 = bsl_r(t22, t19, t21);
	}
	if any(&t23) {
		let t24 = splat(1u32);
		let t25 = sub_r(x11, t24);
		let t26 = splat(2u32);
		let t27 = mul_r(t26, t25);
		let t28 = add_r(x7, t27);
		let t29 = b36_ll_p(l, lid0, t23, t28, x8);
		let t30 = b30_lssls_p(l, lid0, t23, x0, x1, x4, t29, 2u32);
		let t31: [uint32x4_t; 8] = lo(t30);
		t21 = bsl_r(t23, t31, t21);
	}
	x12 = t21;
	let t32 = splat(2u32);
	let t33 = mul_r(t32, x11);
	let t34 = add_r(x7, t33);
	let t35 = b36_ll_p(l, lid0, live, t34, x8);
	let t36 = b30_lssls_p(l, lid0, live, x0, x1, x4, t35, 2u32);
	let t37: [uint32x4_t; 8] = lo(t36);
	x13 = t37;
	let t38 = mul_r(t32, x10);
	let t39 = add_r(x7, t38);
	x14 = t39;
	let t40 = splat(255u32);
	let t41 = and_r(x3, t40);
	x15 = t41;
	x16 = 9259542123273814144u64;
	let t42: u64 = 72340172838076673u64;
	let t43 = splat((t42) as u32);
	let t44 = splat(((t42) >> 32) as u32);
	let t45 = mulhi_r(x15, t43);
	let t46 = mul_r(x15, t43);
	let t47 = mla_r(t45, x15, t44);
	let t48 = cat_w(t46, t47);
	x17 = t48;
	x18 = x12;
	x19 = x12;
	let mut t49 = live;
	loop {
		// Lanes part: each its own rounds.
		let t50 = clt_r(x19, x13);
		t49 = and_r(t49, t50);
		if !(any(&t49)) { break; }
		let t51 = add_r(x14, x19);
		let t52 = b36_ll_p(l, lid0, t49, t51, x8);
		let t53 = b30_lssls_p(l, lid0, t49, x0, x1, x4, t52, 8u32);
		x20 = t53;
		let t54: u64 = x16;
		let t55 = splat((t54) as u32);
		let t56 = splat(((t54) >> 32) as u32);
		let t57 = orr_r(lo(x20), t55);
		let t58 = orr_r(hi(x20), t56);
		let t59 = cat_w(t57, t58);
		let t60: u64 = !(x16);
		let t61 = splat((t60) as u32);
		let t62 = splat(((t60) >> 32) as u32);
		let t63 = and_r(lo(x17), t61);
		let t64 = and_r(hi(x17), t62);
		let t65 = cat_w(t63, t64);
		let t66: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(t59[r], t65[r]));
		let t67: [uint32x4_t; 8] = core::array::from_fn(|r| vsubq_u32(t59[r], t65[r]));
		let t68 = sbc_r(hi(t59), hi(t65), t66);
		let t69 = cat_w(t67, t68);
		x21 = t69;
		let t70: [uint32x4_t; 16] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t71: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t70[r], x17[r]));
		let t72: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t70[(r + 8)], x17[(r + 8)]));
		let t73 = cat_w(t71, t72);
		let t74: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[r], x17[r]));
		let t75: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[(r + 8)], x17[(r + 8)]));
		let t76 = cat_w(t74, t75);
		let t77 = mvn_r(t76);
		let t78 = mvn_r(x21);
		let t79: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t77[r], t78[r]));
		let t80: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t77[(r + 8)], t78[(r + 8)]));
		let t81 = cat_w(t79, t80);
		let t82: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t73[r], t81[r]));
		let t83: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t73[(r + 8)], t81[(r + 8)]));
		let t84 = cat_w(t82, t83);
		let t85: u64 = x16;
		let t86 = splat((t85) as u32);
		let t87 = splat(((t85) >> 32) as u32);
		let t88 = and_r(t86, lo(t84));
		let t89 = and_r(t87, hi(t84));
		let t90 = cat_w(t88, t89);
		x22 = t90;
		let t91 = sub_r(x13, x19);
		x23 = t91;
		let t92 = splat(8u32);
		let t93 = cge_r(x23, t92);
		let mut t94 = [Z; 16];
		let t95 = and_r(t49, t93);
		let t96 = bic_r(t49, t93);
		{
			t94 = bsl_w(t95, x22, t94);
		}
		{
			let t97 = mul_r(t92, x23);
			let t98: u64 = 1u64;
			let t99 = splat((t98) as u32);
			let t100: [uint32x4_t; 8] = lo(t97);
			let t101: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t100[r], vdupq_n_u32(63)));
			let t102: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t99[r], vreinterpretq_s32_u32(t101[r])));
			let t103: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t99[r], vsubq_s32(vreinterpretq_s32_u32(t101[r]), vdupq_n_s32(32))));
			let t104 = cat_w(t102, t103);
			let t105: u64 = 1u64;
			let t106 = splat((t105) as u32);
			let t107 = clt_r(lo(t104), t106);
			let t108 = sub_r(lo(t104), t106);
			let t109: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t104[(r + 8)], Z, t107[r]));
			let t110 = cat_w(t108, t109);
			let t111: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[r], t110[r]));
			let t112: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[(r + 8)], t110[(r + 8)]));
			let t113 = cat_w(t111, t112);
			t94 = bsl_w(t96, t113, t94);
		}
		let t114 = b41_l_p(l, lid0, t49, t94);
		let t115 = add_r(x18, t114);
		x18 = bsl_r(t49, t115, x18);
		let t116 = and_r(t49, live);
		let t117 = add_r(x19, t92);
		x19 = t117;
	}
	x12 = x18;
	let t118 = b32_lss_p(l, lid0, live, x0, x1, 16u32);
	x24 = (((t118).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
	let t119 = splat(1u32);
	let t120 = sub_r(x10, t119);
	let t121 = mul_r(t32, t120);
	let t122 = add_r(x7, t121);
	let t123 = b36_ll_p(l, lid0, live, t122, x8);
	let t124 = b30_lssls_p(l, lid0, live, x0, x1, x4, t123, 2u32);
	let t125: [uint32x4_t; 8] = lo(t124);
	x25 = t125;
	let t126 = add_r(x14, x25);
	let t127 = splat(x24);
	let t128 = mul_r(x12, t127);
	let t129 = add_r(t126, t128);
	let t130 = b36_ll_p(l, lid0, live, t129, x8);
	x26 = t130;
	let t131 = b30_lssls_p(l, lid0, live, x0, x1, x4, x26, 8u32);
	let t132 = b42_s_p(l, lid0, live, x24);
	let t133: u64 = t132;
	let t134 = splat((t133) as u32);
	let t135 = splat(((t133) >> 32) as u32);
	let t136 = and_r(lo(t131), t134);
	let t137 = and_r(hi(t131), t135);
	let t138 = cat_w(t136, t137);
	res = t138;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b33_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut x8 = [Z; 8];
	let mut x9 = [Z; 8];
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13 = [Z; 8];
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16: u64 = 0;
	let mut x17 = [Z; 16];
	let mut x18 = [Z; 8];
	let mut x19 = [Z; 8];
	let mut x20 = [Z; 16];
	let mut x21 = [Z; 16];
	let mut x22 = [Z; 16];
	let mut x23 = [Z; 8];
	let mut x24 = [Z; 8];
	let mut x25 = [Z; 8];
	let mut x26 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t12 = splat(255u32);
	let t13 = add_r(t11, t12);
	let t14: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t13[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	x9 = t14;
	let t15 = mul_r(x6, t3);
	let t16 = sub_r(x9, t15);
	let t17 = b1_sl_p(l, lid0, live, 4u32, t16);
	x10 = t17;
	let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	let t19 = splat(3u32);
	let t20 = and_r(t18, t19);
	x11 = t20;
	let t21 = [Z; 8];
	let t22 = ceq_r(x11, t21);
	let mut t23 = [Z; 8];
	let t24 = and_r(live, t22);
	let t25 = bic_r(live, t22);
	{
		t23 = bsl_r(t24, t21, t23);
	}
	if any(&t25) {
		let t26 = splat(1u32);
		let t27 = sub_r(x11, t26);
		let t28 = splat(2u32);
		let t29 = mul_r(t28, t27);
		let t30 = add_r(x7, t29);
		let t31 = b36_ll_p(l, lid0, t25, t30, x8);
		let t32 = b30_lal136r0lls_p(l, lid0, t25, x0, arg0, x1i, x4, t31, 2u32);
		let t33: [uint32x4_t; 8] = lo(t32);
		t23 = bsl_r(t25, t33, t23);
	}
	x12 = t23;
	let t34 = splat(2u32);
	let t35 = mul_r(t34, x11);
	let t36 = add_r(x7, t35);
	let t37 = b36_ll_p(l, lid0, live, t36, x8);
	let t38 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t37, 2u32);
	let t39: [uint32x4_t; 8] = lo(t38);
	x13 = t39;
	let t40 = mul_r(t34, x10);
	let t41 = add_r(x7, t40);
	x14 = t41;
	let t42 = and_r(x3, t12);
	x15 = t42;
	x16 = 9259542123273814144u64;
	let t43: u64 = 72340172838076673u64;
	let t44 = splat((t43) as u32);
	let t45 = splat(((t43) >> 32) as u32);
	let t46 = mulhi_r(x15, t44);
	let t47 = mul_r(x15, t44);
	let t48 = mla_r(t46, x15, t45);
	let t49 = cat_w(t47, t48);
	x17 = t49;
	x18 = x12;
	x19 = x12;
	let mut t50 = live;
	loop {
		// Lanes part: each its own rounds.
		let t51 = clt_r(x19, x13);
		t50 = and_r(t50, t51);
		if !(any(&t50)) { break; }
		let t52 = add_r(x14, x19);
		let t53 = b36_ll_p(l, lid0, t50, t52, x8);
		let t54 = b30_lal136r0lls_p(l, lid0, t50, x0, arg0, x1i, x4, t53, 8u32);
		x20 = t54;
		let t55: u64 = x16;
		let t56 = splat((t55) as u32);
		let t57 = splat(((t55) >> 32) as u32);
		let t58 = orr_r(lo(x20), t56);
		let t59 = orr_r(hi(x20), t57);
		let t60 = cat_w(t58, t59);
		let t61: u64 = !(x16);
		let t62 = splat((t61) as u32);
		let t63 = splat(((t61) >> 32) as u32);
		let t64 = and_r(lo(x17), t62);
		let t65 = and_r(hi(x17), t63);
		let t66 = cat_w(t64, t65);
		let t67: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(t60[r], t66[r]));
		let t68: [uint32x4_t; 8] = core::array::from_fn(|r| vsubq_u32(t60[r], t66[r]));
		let t69 = sbc_r(hi(t60), hi(t66), t67);
		let t70 = cat_w(t68, t69);
		x21 = t70;
		let t71: [uint32x4_t; 16] = core::array::from_fn(|r| vmvnq_u32(x20[r]));
		let t72: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t71[r], x17[r]));
		let t73: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t71[(r + 8)], x17[(r + 8)]));
		let t74 = cat_w(t72, t73);
		let t75: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[r], x17[r]));
		let t76: [uint32x4_t; 8] = core::array::from_fn(|r| veorq_u32(x20[(r + 8)], x17[(r + 8)]));
		let t77 = cat_w(t75, t76);
		let t78 = mvn_r(t77);
		let t79 = mvn_r(x21);
		let t80: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t78[r], t79[r]));
		let t81: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t78[(r + 8)], t79[(r + 8)]));
		let t82 = cat_w(t80, t81);
		let t83: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t74[r], t82[r]));
		let t84: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(t74[(r + 8)], t82[(r + 8)]));
		let t85 = cat_w(t83, t84);
		let t86: u64 = x16;
		let t87 = splat((t86) as u32);
		let t88 = splat(((t86) >> 32) as u32);
		let t89 = and_r(t87, lo(t85));
		let t90 = and_r(t88, hi(t85));
		let t91 = cat_w(t89, t90);
		x22 = t91;
		let t92 = sub_r(x13, x19);
		x23 = t92;
		let t93 = splat(8u32);
		let t94 = cge_r(x23, t93);
		let mut t95 = [Z; 16];
		let t96 = and_r(t50, t94);
		let t97 = bic_r(t50, t94);
		{
			t95 = bsl_w(t96, x22, t95);
		}
		{
			let t98 = mul_r(t93, x23);
			let t99: u64 = 1u64;
			let t100 = splat((t99) as u32);
			let t101: [uint32x4_t; 8] = lo(t98);
			let t102: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t101[r], vdupq_n_u32(63)));
			let t103: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t100[r], vreinterpretq_s32_u32(t102[r])));
			let t104: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t100[r], vsubq_s32(vreinterpretq_s32_u32(t102[r]), vdupq_n_s32(32))));
			let t105 = cat_w(t103, t104);
			let t106: u64 = 1u64;
			let t107 = splat((t106) as u32);
			let t108 = clt_r(lo(t105), t107);
			let t109 = sub_r(lo(t105), t107);
			let t110: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t105[(r + 8)], Z, t108[r]));
			let t111 = cat_w(t109, t110);
			let t112: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[r], t111[r]));
			let t113: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x22[(r + 8)], t111[(r + 8)]));
			let t114 = cat_w(t112, t113);
			t95 = bsl_w(t97, t114, t95);
		}
		let t115 = b41_l_p(l, lid0, t50, t95);
		let t116 = add_r(x18, t115);
		x18 = bsl_r(t50, t116, x18);
		let t117 = and_r(t50, live);
		let t118 = add_r(x19, t93);
		x19 = t118;
	}
	x12 = x18;
	let t119 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t120: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t119[r], vnegq_s32(vdupq_n_s32(((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) & 31) as i32))));
	let t121 = splat(15u32);
	let t122 = and_r(t120, t121);
	x24 = t122;
	let t123 = splat(1u32);
	let t124 = sub_r(x10, t123);
	let t125 = mul_r(t34, t124);
	let t126 = add_r(x7, t125);
	let t127 = b36_ll_p(l, lid0, live, t126, x8);
	let t128 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t127, 2u32);
	let t129: [uint32x4_t; 8] = lo(t128);
	x25 = t129;
	let t130 = add_r(x14, x25);
	let t131 = mul_r(x12, x24);
	let t132 = add_r(t130, t131);
	let t133 = b36_ll_p(l, lid0, live, t132, x8);
	x26 = t133;
	let t134 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x4, x26, 8u32);
	let t135 = b42_l_p(l, lid0, live, x24);
	let t136: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[r], t135[r]));
	let t137: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[(r + 8)], t135[(r + 8)]));
	let t138 = cat_w(t136, t137);
	res = t138;
	return res;
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b34_lal136r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 8];
	let mut x4 = [Z; 16];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = [Z; 8];
	let t1 = ceq_r(x2, t0);
	let t2 = and_r(live, t1);
	{
		let t3 = splat64(4294967295u64);
		res = bsl_w(t2, t3, res);
		live = bic_r(live, t2);
	}
	if any(&live) {
		let t4 = splat(1u32);
		let t5 = sub_r(x2, t4);
		x3 = t5;
		let t6 = b28_lal136r0l_p(l, lid0, live, x0, arg0, x1i, x3);
		x4 = t6;
		let t7: u64 = 1u64;
		let t8 = splat((t7) as u32);
		let t9 = and_r(lo(x4), t8);
		let t10: u64 = 0u64;
		let t11 = splat((t10) as u32);
		let t12 = ceq_r(t9, t11);
		let t13 = and_r(live, t12);
		{
			let t14 = cat_w(lo(x3), [Z; 8]);
			res = bsl_w(t13, t14, res);
			live = bic_r(live, t13);
		}
		if any(&live) {
			let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshrq_n_u32::<2>(x4[r]), vshlq_n_u32::<30>(x4[(r + 8)])));
			let t16: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<2>(x4[(r + 8)]));
			let t17 = cat_w(t15, t16);
			let t18: [uint32x4_t; 8] = lo(t17);
			x5 = t18;
			let t19 = sub_r(x5, t4);
			let t20 = cge_r(t19, x2);
			let t21 = and_r(live, t20);
			{
				let t22 = splat64(4294967295u64);
				res = bsl_w(t21, t22, res);
				live = bic_r(live, t21);
			}
			if any(&live) {
				let t23 = sub_r(x2, x5);
				x6 = t23;
				let t24: u64 = 2u64;
				let t25 = splat((t24) as u32);
				let t26 = and_r(lo(x4), t25);
				let t27: u64 = 0u64;
				let t28 = splat((t27) as u32);
				let t29 = ceq_r(t26, t28);
				let t30 = mvn_r(t29);
				let t31 = and_r(live, t30);
				{
					let t32 = cat_w(lo(x6), [Z; 8]);
					res = bsl_w(t31, t32, res);
					live = bic_r(live, t31);
				}
				if any(&live) {
					let t33 = ceq_r(x6, t0);
					let t34 = and_r(live, t33);
					{
						let t35 = splat64(4294967295u64);
						res = bsl_w(t34, t35, res);
						live = bic_r(live, t34);
					}
					if any(&live) {
						let t36: u64 = (1u64).wrapping_shl((32u64) as u32);
						let t37 = splat((t36) as u32);
						let t38 = splat(((t36) >> 32) as u32);
						let t39 = orr_r(t37, x6);
						let t40 = cat_w(t39, lo(t38));
						res = bsl_w(live, t40, res);
						live = bic_r(live, live);
					}
				}
			}
		}
	}
	res
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b35_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = splat(x1);
	let t1 = clt_r(x0, t0);
	let mut t2 = [Z; 8];
	let t3 = mvn_r(t1);
	{
		let t4 = splat(x1);
		t2 = bsl_r(t1, t4, t2);
	}
	{
		t2 = bsl_r(t3, x0, t2);
	}
	res = t2;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b35_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = clt_r(x0, x1);
	let mut t1 = [Z; 8];
	let t2 = and_r(live, t0);
	let t3 = bic_r(live, t0);
	{
		t1 = bsl_r(t2, x1, t1);
	}
	{
		t1 = bsl_r(t3, x0, t1);
	}
	res = t1;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(x1);
	let t1 = clt_r(x0, t0);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		t2 = bsl_r(t3, x0, t2);
	}
	{
		let t5 = splat(x1);
		t2 = bsl_r(t4, t5, t2);
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
	let mut live = splat(u32::MAX);
	let mut res = [Z; 8];
	let t0 = splat(x1);
	let t1 = clt_r(x0, t0);
	let mut t2 = [Z; 8];
	let t3 = mvn_r(t1);
	{
		t2 = bsl_r(t1, x0, t2);
	}
	{
		let t4 = splat(x1);
		t2 = bsl_r(t3, t4, t2);
	}
	res = t2;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b36_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = clt_r(x0, x1);
	let mut t1 = [Z; 8];
	let t2 = and_r(live, t0);
	let t3 = bic_r(live, t0);
	{
		t1 = bsl_r(t2, x0, t1);
	}
	{
		t1 = bsl_r(t3, x1, t1);
	}
	res = t1;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b37_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4: u32 = 0;
	let mut x5 = [Z; 8];
	let mut x6: u64 = 0;
	let mut x7 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b43_lss_p(l, lid0, live, x0, x1, x2);
	x4 = t0;
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t2 = b31_lssl_p(l, lid0, live, x0, x1, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b42_s_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = splat(1023u32);
	let t5 = and_r(x3, t4);
	let t6 = splat(x4);
	let t7 = mul_r(t5, t6);
	let t8 = b30_lslls_p(l, lid0, live, x0, x1, x5, t7, 8u32);
	let t9: u64 = x6;
	let t10 = splat((t9) as u32);
	let t11 = splat(((t9) >> 32) as u32);
	let t12 = and_r(lo(t8), t10);
	let t13 = and_r(hi(t8), t11);
	let t14 = cat_w(t12, t13);
	x7 = t14;
	let t15: u64 = x6;
	let t16 = splat((t15) as u32);
	let t17 = splat(((t15) >> 32) as u32);
	let t18 = eq64_r(lo(x7), hi(x7), t16, t17);
	let mut t19 = [Z; 16];
	let t20 = and_r(live, t18);
	let t21 = bic_r(live, t18);
	if any(&t20) {
		let t22: [uint32x4_t; 16];
		let t23 = bits(&t20);
		if t23.count_ones() <= 2 {
			let t24 = words16(x0);
			let t25 = words8(x3);
			let mut t26 = [0u32; 64];
			let mut t27 = t23;
			while t27 != 0 {
				let j = t27.trailing_zeros() as usize;
				t27 &= t27 - 1;
				let t28 = splat64((((((t24[32 + j] as u64) << 32 | t24[j] as u64) as usize as *const u8)) as usize as u64));
				let t29 = splat(t25[j]);
				let t30 = b33_lssl_p(l, lid0, splat(u32::MAX), t28, x1, (x2).wrapping_add(1u32), t29);
				t26[j] = vgetq_lane_u32::<0>(t30[0]);
				t26[32 + j] = vgetq_lane_u32::<0>(t30[8]);
			}
			t22 = regs16(t26);
		} else {
			let t31 = b33_lssl_p(l, lid0, t20, x0, x1, (x2).wrapping_add(1u32), x3);
			t22 = t31;
		}
		t19 = bsl_w(t20, t22, t19);
	}
	{
		t19 = bsl_w(t21, x7, t19);
	}
	res = t19;
	return res;
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b37_lal136r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 16];
	let mut x7 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b43_lal136r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	x4 = t0;
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t2 = b31_lal136r0sl_p(l, lid0, live, x0, arg0, x1i, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b42_l_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = splat(1023u32);
	let t5 = and_r(x3, t4);
	let t6 = mul_r(t5, x4);
	let t7 = b30_lal136r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t6, 8u32);
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t7[r], x6[r]));
	let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t7[(r + 8)], x6[(r + 8)]));
	let t10 = cat_w(t8, t9);
	x7 = t10;
	let t11: [uint32x4_t; 8] = core::array::from_fn(|r| veq64q_u32(x7[r], x7[(r + 8)], x6[r], x6[(r + 8)]));
	let mut t12 = [Z; 16];
	let t13 = and_r(live, t11);
	let t14 = bic_r(live, t11);
	if any(&t13) {
		let t15: [uint32x4_t; 16];
		let t16 = bits(&t13);
		if t16.count_ones() <= 2 {
			let t17 = words16(x0);
			let t18 = words8(x1i);
			let t19 = words8(x3);
			let mut t20 = [0u32; 64];
			let mut t21 = t16;
			while t21 != 0 {
				let j = t21.trailing_zeros() as usize;
				t21 &= t21 - 1;
				let t22 = splat64((((((t17[32 + j] as u64) << 32 | t17[j] as u64) as usize as *const u8)) as usize as u64));
				let t23 = splat(t19[j]);
				let t24 = b33_lssl_p(l, lid0, splat(u32::MAX), t22, (arg0 as *const u8).wrapping_add(((t18[j]) as usize).wrapping_mul(136)), (x2).wrapping_add(1u32), t23);
				t20[j] = vgetq_lane_u32::<0>(t24[0]);
				t20[32 + j] = vgetq_lane_u32::<0>(t24[8]);
			}
			t15 = regs16(t20);
		} else {
			let t25 = b33_lal136r0sl_p(l, lid0, t13, x0, arg0, x1i, (x2).wrapping_add(1u32), x3);
			t15 = t25;
		}
		t12 = bsl_w(t13, t15, t12);
	}
	{
		t12 = bsl_w(t14, x7, t12);
	}
	res = t12;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b38_lsls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: u32, x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b44_lal1s_p(l, lid0, live, x0, ((((x1) as u64)) as usize as *const u8), x2, x3);
	x4 = t0;
	let t1: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t1 = x4;
	} else {
		let t2 = b45_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t3: u64 = t2;
		let t4 = splat((t3) as u32);
		let t5 = splat(((t3) >> 32) as u32);
		let t6 = and_r(lo(x4), t4);
		let t7 = and_r(hi(x4), t5);
		let t8 = cat_w(t6, t7);
		t1 = t8;
	}
	res = t1;
	return res;
}

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b38_llls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = add_r(x1, x2);
	let t1 = clt_r(t0, x1);
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t1[r]));
	let t3 = cat_w(lo(t0), t2);
	let t4 = b44_lls_p(l, lid0, live, x0, t3, x3);
	x4 = t4;
	let t5: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t5 = x4;
	} else {
		let t6 = b45_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
		let t7: u64 = t6;
		let t8 = splat((t7) as u32);
		let t9 = splat(((t7) >> 32) as u32);
		let t10 = and_r(lo(x4), t8);
		let t11 = and_r(hi(x4), t9);
		let t12 = cat_w(t10, t11);
		t5 = t12;
	}
	res = t5;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b39_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b32_lss_p(l, lid0, live, x0, x1, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b39_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}


/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b41_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[r])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[(r + 8)]))))));
	res = t0;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b42_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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
unsafe fn b42_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = splat(8u32);
	let t1 = cge_r(x0, t0);
	let mut t2 = [Z; 16];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	{
		let t5 = splat64(!(0u64));
		t2 = bsl_w(t3, t5, t2);
	}
	{
		let t6 = mul_r(x0, t0);
		let t7: u64 = 1u64;
		let t8 = splat((t7) as u32);
		let t9: [uint32x4_t; 8] = lo(t6);
		let t10: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t9[r], vdupq_n_u32(63)));
		let t11: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t8[r], vreinterpretq_s32_u32(t10[r])));
		let t12: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t8[r], vsubq_s32(vreinterpretq_s32_u32(t10[r]), vdupq_n_s32(32))));
		let t13 = cat_w(t11, t12);
		let t14: u64 = 1u64;
		let t15 = splat((t14) as u32);
		let t16 = clt_r(lo(t13), t15);
		let t17 = sub_r(lo(t13), t15);
		let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t13[(r + 8)], Z, t16[r]));
		let t19 = cat_w(t17, t18);
		t2 = bsl_w(t4, t19, t2);
	}
	res = t2;
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b43_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0 = b32_lss_p(l, lid0, live, x0, x1, 16u32);
	res = (((t0).wrapping_shr(((4u32).wrapping_mul(x2)) as u32)) & (15u32));
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b43_lal136r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b32_lal136r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t0[r], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))));
	let t2 = splat(15u32);
	let t3 = and_r(t1, t2);
	res = t3;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b44_lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 16], x2: u32) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
		let t1 = clt_r(t0, lo(x0));
		let t2 = adc_r(hi(x0), hi(x1), t1);
		let t3 = cat_w(lo(t0), t2);
		let t4: [uint32x4_t; 8] = lo(t3);
		let t5: [uint32x4_t; 8] = hi(t3);
		let t6 = addrs(t4, t5, live);
		let mut t7 = [Z; 16];
		t7 = ldj_u64(|j| (t6[j] as *const u8) as *const u8);
		res = t7;
		return res;
	}
	if ((x2) == (4u32)) {
		let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
		let t9 = clt_r(t8, lo(x0));
		let t10 = adc_r(hi(x0), hi(x1), t9);
		let t11 = cat_w(lo(t8), t10);
		let t12: [uint32x4_t; 8] = lo(t11);
		let t13: [uint32x4_t; 8] = hi(t11);
		let t14 = addrs(t12, t13, live);
		let mut t15 = [Z; 8];
		t15 = ldj_u32(|j| (t14[j] as *const u8) as *const u8);
		let t16 = cat_w(lo(t15), [Z; 8]);
		res = t16;
		return res;
	}
	if ((x2) == (2u32)) {
		let t17: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
		let t18 = clt_r(t17, lo(x0));
		let t19 = adc_r(hi(x0), hi(x1), t18);
		let t20 = cat_w(lo(t17), t19);
		let t21: [uint32x4_t; 8] = lo(t20);
		let t22: [uint32x4_t; 8] = hi(t20);
		let t23 = addrs(t21, t22, live);
		let mut t24 = [Z; 8];
		t24 = ldj_u16(|j| (t23[j] as *const u8) as *const u8);
		let t25 = cat_w(lo(t24), [Z; 8]);
		res = t25;
		return res;
	}
	if ((x2) == (1u32)) {
		let t26: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
		let t27 = clt_r(t26, lo(x0));
		let t28 = adc_r(hi(x0), hi(x1), t27);
		let t29 = cat_w(lo(t26), t28);
		let t30: [uint32x4_t; 8] = lo(t29);
		let t31: [uint32x4_t; 8] = hi(t29);
		let t32 = addrs(t30, t31, live);
		let mut t33 = [Z; 8];
		t33 = ldj_u8(|j| (t32[j] as *const u8) as *const u8);
		let t34 = cat_w(lo(t33), [Z; 8]);
		res = t34;
		return res;
	}
	let t35: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
	let t36 = clt_r(t35, lo(x0));
	let t37 = adc_r(hi(x0), hi(x1), t36);
	let t38 = cat_w(lo(t35), t37);
	let t39: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
	let t40 = clt_r(t39, lo(x0));
	let t41 = adc_r(hi(x0), hi(x1), t40);
	let t42 = cat_w(lo(t39), t41);
	let t43: u64 = 7u64;
	let t44 = splat((t43) as u32);
	let t45 = and_r(lo(t42), t44);
	let t46 = clt_r(lo(t38), t45);
	let t47 = sub_r(lo(t38), t45);
	let t48: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t38[(r + 8)], Z, t46[r]));
	let t49 = cat_w(t47, t48);
	x3 = t49;
	let t50: [uint32x4_t; 8] = lo(x3);
	let t51: [uint32x4_t; 8] = hi(x3);
	let t52 = addrs(t50, t51, live);
	let t53: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], x1[r]));
	let t54 = clt_r(t53, lo(x0));
	let t55 = adc_r(hi(x0), hi(x1), t54);
	let t56 = cat_w(lo(t53), t55);
	let t57: u64 = 7u64;
	let t58 = splat((t57) as u32);
	let t59 = and_r(lo(t56), t58);
	let t60 = splat(8u32);
	let t61 = mul_r(t59, t60);
	x4 = t61;
	let mut t62 = [Z; 16];
	t62 = ldj_u64(|j| (t52[j] as *const u8) as *const u8);
	let t63: [uint32x4_t; 8] = lo(x4);
	let t64: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t63[r], vdupq_n_u32(63)));
	let t65: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t62[r], vnegq_s32(vreinterpretq_s32_u32(t64[r]))), vshlq_u32(t62[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t64[r])))));
	let t66: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t62[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t64[r]))));
	let t67 = cat_w(t65, t66);
	x5 = t67;
	let t68 = [Z; 8];
	let t69 = cne_r(x4, t68);
	let t70 = splat((x2).wrapping_mul(8u32));
	let t71 = add_r(x4, t70);
	let t72 = splat(64u32);
	let t73 = cgt_r(t71, t72);
	let t74 = and_r(t69, t73);
	let mut t75 = [Z; 16];
	let t76 = and_r(live, t74);
	let t77 = bic_r(live, t74);
	{
		let t78: u64 = (1u64).wrapping_mul(8u64);
		let t79 = splat((t78) as u32);
		let t80 = splat(((t78) >> 32) as u32);
		let t81 = add_r(lo(x3), t79);
		let t82 = clt_r(t81, lo(x3));
		let t83 = adc_r(hi(x3), t80, t82);
		let t84 = cat_w(lo(t81), t83);
		let t85: [usize; 32] = core::array::from_fn(|j| t52[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t86 = [Z; 16];
		t86 = ldj_u64(|j| (t85[j] as *const u8) as *const u8);
		let t87 = sub_r(t72, x4);
		let t88: [uint32x4_t; 8] = lo(t87);
		let t89: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t88[r], vdupq_n_u32(63)));
		let t90: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t86[r], vreinterpretq_s32_u32(t89[r])));
		let t91: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t86[(r + 8)], vreinterpretq_s32_u32(t89[r])), vshlq_u32(t86[r], vsubq_s32(vreinterpretq_s32_u32(t89[r]), vdupq_n_s32(32)))));
		let t92 = cat_w(t90, t91);
		let t93: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t92[r]));
		let t94: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t92[(r + 8)]));
		let t95 = cat_w(t93, t94);
		t75 = bsl_w(t76, t95, t75);
	}
	{
		t75 = bsl_w(t77, x5, t75);
	}
	res = t75;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b44_lal1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 16];
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	if ((x2) == (8u32)) {
		let t0: [uint32x4_t; 8] = lo(x1i);
		let t1: u64 = ((x1b) as usize as u64);
		let t2 = splat((t1) as u32);
		let t3 = splat(((t1) >> 32) as u32);
		let t4 = add_r(t2, t0);
		let t5 = clt_r(t4, t2);
		let t6: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t3[r], Z, t5[r]));
		let t7 = cat_w(lo(t4), t6);
		let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t7[r]));
		let t9 = clt_r(t8, lo(x0));
		let t10 = adc_r(hi(x0), hi(t7), t9);
		let t11 = cat_w(lo(t8), t10);
		let t12: [uint32x4_t; 8] = lo(t11);
		let t13: [uint32x4_t; 8] = hi(t11);
		let t14 = addrs(t12, t13, live);
		let mut t15 = [Z; 16];
		t15 = ldj_u64(|j| (t14[j] as *const u8) as *const u8);
		res = t15;
		return res;
	}
	if ((x2) == (4u32)) {
		let t16: [uint32x4_t; 8] = lo(x1i);
		let t17: u64 = ((x1b) as usize as u64);
		let t18 = splat((t17) as u32);
		let t19 = splat(((t17) >> 32) as u32);
		let t20 = add_r(t18, t16);
		let t21 = clt_r(t20, t18);
		let t22: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t19[r], Z, t21[r]));
		let t23 = cat_w(lo(t20), t22);
		let t24: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t23[r]));
		let t25 = clt_r(t24, lo(x0));
		let t26 = adc_r(hi(x0), hi(t23), t25);
		let t27 = cat_w(lo(t24), t26);
		let t28: [uint32x4_t; 8] = lo(t27);
		let t29: [uint32x4_t; 8] = hi(t27);
		let t30 = addrs(t28, t29, live);
		let mut t31 = [Z; 8];
		t31 = ldj_u32(|j| (t30[j] as *const u8) as *const u8);
		let t32 = cat_w(lo(t31), [Z; 8]);
		res = t32;
		return res;
	}
	if ((x2) == (2u32)) {
		let t33: [uint32x4_t; 8] = lo(x1i);
		let t34: u64 = ((x1b) as usize as u64);
		let t35 = splat((t34) as u32);
		let t36 = splat(((t34) >> 32) as u32);
		let t37 = add_r(t35, t33);
		let t38 = clt_r(t37, t35);
		let t39: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t36[r], Z, t38[r]));
		let t40 = cat_w(lo(t37), t39);
		let t41: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t40[r]));
		let t42 = clt_r(t41, lo(x0));
		let t43 = adc_r(hi(x0), hi(t40), t42);
		let t44 = cat_w(lo(t41), t43);
		let t45: [uint32x4_t; 8] = lo(t44);
		let t46: [uint32x4_t; 8] = hi(t44);
		let t47 = addrs(t45, t46, live);
		let mut t48 = [Z; 8];
		t48 = ldj_u16(|j| (t47[j] as *const u8) as *const u8);
		let t49 = cat_w(lo(t48), [Z; 8]);
		res = t49;
		return res;
	}
	if ((x2) == (1u32)) {
		let t50: [uint32x4_t; 8] = lo(x1i);
		let t51: u64 = ((x1b) as usize as u64);
		let t52 = splat((t51) as u32);
		let t53 = splat(((t51) >> 32) as u32);
		let t54 = add_r(t52, t50);
		let t55 = clt_r(t54, t52);
		let t56: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t53[r], Z, t55[r]));
		let t57 = cat_w(lo(t54), t56);
		let t58: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t57[r]));
		let t59 = clt_r(t58, lo(x0));
		let t60 = adc_r(hi(x0), hi(t57), t59);
		let t61 = cat_w(lo(t58), t60);
		let t62: [uint32x4_t; 8] = lo(t61);
		let t63: [uint32x4_t; 8] = hi(t61);
		let t64 = addrs(t62, t63, live);
		let mut t65 = [Z; 8];
		t65 = ldj_u8(|j| (t64[j] as *const u8) as *const u8);
		let t66 = cat_w(lo(t65), [Z; 8]);
		res = t66;
		return res;
	}
	let t67: [uint32x4_t; 8] = lo(x1i);
	let t68: u64 = ((x1b) as usize as u64);
	let t69 = splat((t68) as u32);
	let t70 = splat(((t68) >> 32) as u32);
	let t71 = add_r(t69, t67);
	let t72 = clt_r(t71, t69);
	let t73: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t70[r], Z, t72[r]));
	let t74 = cat_w(lo(t71), t73);
	let t75: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t74[r]));
	let t76 = clt_r(t75, lo(x0));
	let t77 = adc_r(hi(x0), hi(t74), t76);
	let t78 = cat_w(lo(t75), t77);
	let t79: [uint32x4_t; 8] = lo(x1i);
	let t80: u64 = ((x1b) as usize as u64);
	let t81 = splat((t80) as u32);
	let t82 = splat(((t80) >> 32) as u32);
	let t83 = add_r(t81, t79);
	let t84 = clt_r(t83, t81);
	let t85: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t82[r], Z, t84[r]));
	let t86 = cat_w(lo(t83), t85);
	let t87: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t86[r]));
	let t88 = clt_r(t87, lo(x0));
	let t89 = adc_r(hi(x0), hi(t86), t88);
	let t90 = cat_w(lo(t87), t89);
	let t91: u64 = 7u64;
	let t92 = splat((t91) as u32);
	let t93 = and_r(lo(t90), t92);
	let t94 = clt_r(lo(t78), t93);
	let t95 = sub_r(lo(t78), t93);
	let t96: [uint32x4_t; 8] = core::array::from_fn(|r| vsbcq_u32(t78[(r + 8)], Z, t94[r]));
	let t97 = cat_w(t95, t96);
	x3 = t97;
	let t98: [uint32x4_t; 8] = lo(x3);
	let t99: [uint32x4_t; 8] = hi(x3);
	let t100 = addrs(t98, t99, live);
	let t101: [uint32x4_t; 8] = lo(x1i);
	let t102: u64 = ((x1b) as usize as u64);
	let t103 = splat((t102) as u32);
	let t104 = splat(((t102) >> 32) as u32);
	let t105 = add_r(t103, t101);
	let t106 = clt_r(t105, t103);
	let t107: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t104[r], Z, t106[r]));
	let t108 = cat_w(lo(t105), t107);
	let t109: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t108[r]));
	let t110 = clt_r(t109, lo(x0));
	let t111 = adc_r(hi(x0), hi(t108), t110);
	let t112 = cat_w(lo(t109), t111);
	let t113: u64 = 7u64;
	let t114 = splat((t113) as u32);
	let t115 = and_r(lo(t112), t114);
	let t116 = splat(8u32);
	let t117 = mul_r(t115, t116);
	x4 = t117;
	let mut t118 = [Z; 16];
	t118 = ldj_u64(|j| (t100[j] as *const u8) as *const u8);
	let t119: [uint32x4_t; 8] = lo(x4);
	let t120: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t119[r], vdupq_n_u32(63)));
	let t121: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t118[r], vnegq_s32(vreinterpretq_s32_u32(t120[r]))), vshlq_u32(t118[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t120[r])))));
	let t122: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t118[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t120[r]))));
	let t123 = cat_w(t121, t122);
	x5 = t123;
	let t124 = [Z; 8];
	let t125 = cne_r(x4, t124);
	let t126 = splat((x2).wrapping_mul(8u32));
	let t127 = add_r(x4, t126);
	let t128 = splat(64u32);
	let t129 = cgt_r(t127, t128);
	let t130 = and_r(t125, t129);
	let mut t131 = [Z; 16];
	let t132 = and_r(live, t130);
	let t133 = bic_r(live, t130);
	{
		let t134: u64 = (1u64).wrapping_mul(8u64);
		let t135 = splat((t134) as u32);
		let t136 = splat(((t134) >> 32) as u32);
		let t137 = add_r(lo(x3), t135);
		let t138 = clt_r(t137, lo(x3));
		let t139 = adc_r(hi(x3), t136, t138);
		let t140 = cat_w(lo(t137), t139);
		let t141: [usize; 32] = core::array::from_fn(|j| t100[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t142 = [Z; 16];
		t142 = ldj_u64(|j| (t141[j] as *const u8) as *const u8);
		let t143 = sub_r(t128, x4);
		let t144: [uint32x4_t; 8] = lo(t143);
		let t145: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t144[r], vdupq_n_u32(63)));
		let t146: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t142[r], vreinterpretq_s32_u32(t145[r])));
		let t147: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t142[(r + 8)], vreinterpretq_s32_u32(t145[r])), vshlq_u32(t142[r], vsubq_s32(vreinterpretq_s32_u32(t145[r]), vdupq_n_s32(32)))));
		let t148 = cat_w(t146, t147);
		let t149: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t148[r]));
		let t150: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t148[(r + 8)]));
		let t151 = cat_w(t149, t150);
		t131 = bsl_w(t132, t151, t131);
	}
	{
		t131 = bsl_w(t133, x5, t131);
	}
	res = t131;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b45_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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

/// The group `l.group`'s threads, 32 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn label_leaves_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
	let x3: *const u8 = *argv.add(3);
	let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
	let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
	let x6: u32 = (*argv.add(6) as *const u32).read_unaligned();
	let mut t1256: bool = !(arg5).is_null();
	if t1256 {
		let mut t1257: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x6) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t1257 {
			t1257 = ((l.group[1]) >= ((arg5.wrapping_add(((((x6).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t1258: bool = t1257;
		if !t1258 {
			t1258 = ((l.group[2]) >= ((arg5.wrapping_add(((((x6).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t1256 = t1258;
	}
	if t1256 {
		return;
	}
	let mut lid0 = 0u32;
	while lid0 + 32 <= n {
		let mut live = splat(u32::MAX);
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sr4g_p(l, lid0, splat(u32::MAX), arg0, arg1, arg2, x3, arg4);
		}
		lid0 += 32;
	}
	if lid0 < n {
		// Lanes part: the group's last threads, those under `n`.
		let t1259: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
		let mut live = t1259;
		let gid0 = l.group[0] * l.block[0] + lid0;
		'chunk: {
			b0_r0r1r2sr4g_p(l, lid0, live, arg0, arg1, arg2, x3, arg4);
		}
	}
}
