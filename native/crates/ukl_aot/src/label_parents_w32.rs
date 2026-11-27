//! `label_parents` at 32 lanes (8 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
/// Each lane's address of its halves `lo` and `hi`; a lane mask `k` has off, the first on's.
#[inline(always)]
unsafe fn addrs(lo: [uint32x4_t; 8], hi: [uint32x4_t; 8], k: [uint32x4_t; 8]) -> [usize; 32] {
	let (l, h, b) = (words8(lo), words8(hi), bits(&k));
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

/// `label_parents_inner`.
#[inline(always)]
unsafe fn b0_r0r1sr3r6o6gsi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, arg1: *const u8, x2: *const u8, arg3: *const u8, arg6: *const u8, arg6_x5: *const u8, x7: u32, x8: u32, phase: u32, st: *mut u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(40u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(32u64 as usize) as *const u32).read_unaligned();
	let mut x9: u32 = 0;
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13: u32 = 0;
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16 = [Z; 8];
	let mut x17 = [Z; 8];
	let mut x18 = [Z; 8];
	let mut x19: u32 = 0;
	let mut live = k;
	if phase > 0 {
		live = (st as *const [uint32x4_t; 8]).read_unaligned();
		x9 = (st.add(256) as *const u32).read_unaligned();
		x10 = (st.add(512) as *const [uint32x4_t; 8]).read_unaligned();
		x11 = (st.add(768) as *const [uint32x4_t; 8]).read_unaligned();
		x12 = (st.add(1024) as *const [uint32x4_t; 8]).read_unaligned();
		x13 = (st.add(1280) as *const u32).read_unaligned();
		x14 = (st.add(1536) as *const [uint32x4_t; 8]).read_unaligned();
		x15 = (st.add(1792) as *const [uint32x4_t; 8]).read_unaligned();
		x16 = (st.add(2048) as *const [uint32x4_t; 8]).read_unaligned();
		x17 = (st.add(2304) as *const [uint32x4_t; 8]).read_unaligned();
		x18 = (st.add(2560) as *const [uint32x4_t; 8]).read_unaligned();
		x19 = (st.add(2816) as *const u32).read_unaligned();
		if !any(&live) { return; }
	}
	if phase == 0 {
		'phase: {
			if ((a0) == (0u32)) {
				live = [Z; 8];
				break 'phase;
			}
			x9 = gid0;
			let t0 = [Z; 8];
			x10 = t0;
			let t1 = splat(a0);
			x11 = t1;
			let mut t2 = live;
			loop {
				let t3 = bits(&t2);
				if t3.count_ones() <= 2 {
					let mut t4 = t3;
					while t4 != 0 {
						let j = t4.trailing_zeros() as usize;
						t4 &= t4 - 1;
						let mut x9_j: u32 = (x9).wrapping_add(j as u32);
						let t5 = words8(x10);
						let mut x10_j: u32 = t5[j];
						let t6 = words8(x11);
						let mut x11_j: u32 = t6[j];
						let t7 = words8(x12);
						let mut x12_j: u32 = t7[j];
						loop {
							if !((((x11_j).wrapping_sub(x10_j)) > (1u32))) { break; }
							x12_j = ((x10_j).wrapping_add(x11_j)).checked_div(2u32).unwrap_or(0);
							if (((((arg0.wrapping_add((((x12_j) as u64)).wrapping_mul(128u64) as usize).wrapping_add(8u64 as usize) as *const u32).read_unaligned()).checked_div(32u32).unwrap_or(0)).wrapping_add((2u32).wrapping_mul(x12_j))) <= (x9_j)) {
								x10_j = x12_j;
							} else {
								x11_j = x12_j;
							}
						}
						let mut t8 = words8(x10);
						t8[j] = x10_j as u32;
						x10 = regs8(t8);
						let mut t9 = words8(x11);
						t9[j] = x11_j as u32;
						x11 = regs8(t9);
						let mut t10 = words8(x12);
						t10[j] = x12_j as u32;
						x12 = regs8(t10);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t11 = sub_r(x11, x10);
				let t12 = splat(1u32);
				let t13 = cgt_r(t11, t12);
				t2 = and_r(t2, t13);
				if !(any(&t2)) { break; }
				let t14 = add_r(x10, x11);
				let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t14[r], vdupq_n_s32(-1)));
				x12 = t15;
				let t16 = ix_on(t2, x12);
				let t17 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t16, 128);
				let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t17[r], vdupq_n_s32(-5)));
				let t19 = splat(2u32);
				let t20 = mul_r(t19, x12);
				let t21 = add_r(t18, t20);
				let t22 = iota(x9);
				let t23 = cle_r(t21, t22);
				let t24 = and_r(t2, t23);
				{
					x10 = bsl_r(t24, x12, x10);
				}
				let t25 = bic_r(t2, t23);
				{
					x11 = bsl_r(t25, x12, x11);
				}
			}
			let t26 = ix_on(live, x10);
			let t27 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t26, 128);
			let t28: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t27[r], vdupq_n_s32(-5)));
			let t29 = splat(2u32);
			let t30 = mul_r(t29, x10);
			let t31 = add_r(t28, t30);
			x14 = t31;
			let t32 = ix_on(live, x10);
			let t33 = ld_u32((arg0).wrapping_add((4u64) as usize) as *const u8, t32, 128);
			x15 = t33;
			let t34 = iota(x9);
			let t35 = sub_r(t34, x14);
			x16 = t35;
			let t36 = cge_r(t34, x14);
			let t37 = splat(a1);
			let t38 = clt_r(t34, t37);
			let t39 = and_r(t36, t38);
			let t40 = splat(32u32);
			let t41 = mul_r(x16, t40);
			let t42 = clt_r(t41, x15);
			let t43 = and_r(t39, t42);
			x17 = t43;
			let t44 = mvn_r(x17);
			let mut t45 = [Z; 8];
			let t46 = and_r(live, t44);
			let t47 = bic_r(live, t44);
			{
				t45 = bsl_r(t46, t0, t45);
			}
			{
				let t48 = mul_r(x16, t40);
				let t49 = add_r(t48, t40);
				let t50 = cle_r(t49, x15);
				let mut t51 = [Z; 8];
				let t52 = and_r(t47, t50);
				let t53 = bic_r(t47, t50);
				{
					let t54 = splat(4294967295u32);
					t51 = bsl_r(t52, t54, t51);
				}
				{
					let t55 = mul_r(x16, t40);
					let t56 = sub_r(x15, t55);
					let t57 = splat(1u32);
					let t58: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t57[r], vreinterpretq_s32_u32(vandq_u32(t56[r], vdupq_n_u32(31)))));
					let t59 = sub_r(t58, t57);
					t51 = bsl_r(t53, t59, t51);
				}
				t45 = bsl_r(t47, t51, t45);
			}
			x18 = t45;
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 0u32, st.add(3072));
		}
	}
	if phase == 1 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 1u32, st.add(3072));
		}
	}
	if phase == 2 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 2u32, st.add(3072));
		}
	}
	if phase == 3 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 3u32, st.add(3072));
		}
	}
	if phase == 4 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 4u32, st.add(3072));
		}
	}
	if phase == 5 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 5u32, st.add(3072));
		}
	}
	if phase == 6 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 6u32, st.add(3072));
			let t60 = iota(x9);
			let t61 = splat(a1);
			let t62 = clt_r(t60, t61);
			let t63 = and_r(live, t62);
			if any(&t63) {
				x19 = 0u32;
				loop {
					// Rounds of one condition: every lane goes on.
					if !(((x19) < (32u32))) { break; }
					let mut t64 = [Z; 8];
					if ((x8.wrapping_mul(32u32).wrapping_add(x19)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 && all(&t63) {
						let t65 = (arg6 as *const u8).add(((x8.wrapping_mul(32u32).wrapping_add(x19)) as usize) * 4);
						let t66 = ((32u32) as usize) * 4;
						t64 = ldj_u32(|j| t65.add(j * t66) as *const u8);
					} else {
						let t67: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x8.wrapping_mul(32u32).wrapping_add(x19)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
						let t68 = ix_on(t63, t67);
						let t69 = arg6 as *const u32;
						let mut t70 = [Z; 8];
						let t71 = words8(t68);
						t70 = ldj_u32(|j| t69.add(t71[j] as usize) as *const u8);
						t64 = t70;
					}
					if ((x9.wrapping_mul(32u32).wrapping_add(x19)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
						let t72 = (arg1 as *mut u32).add((x9.wrapping_mul(32u32).wrapping_add(x19)) as usize);
						let t73 = (32u32) as usize;
						stj_u32(t63, t64, |j| t72.add(j * t73) as *mut u8);
					} else {
						let t74: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x9.wrapping_mul(32u32).wrapping_add(x19)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
						let t75 = words8(t74);
						stj_u32(t63, t64, |j| (arg1 as *mut u32).add(t75[j] as usize) as *mut u8);
					}
					let t76 = x19;
					x19 = (x19).wrapping_add(1u32);
				}
			}
		}
	}
	if phase + 1 < 7 {
		(st as *mut [uint32x4_t; 8]).write_unaligned(live);
		(st.add(256) as *mut u32).write_unaligned(x9);
		(st.add(512) as *mut [uint32x4_t; 8]).write_unaligned(x10);
		(st.add(768) as *mut [uint32x4_t; 8]).write_unaligned(x11);
		(st.add(1024) as *mut [uint32x4_t; 8]).write_unaligned(x12);
		(st.add(1280) as *mut u32).write_unaligned(x13);
		(st.add(1536) as *mut [uint32x4_t; 8]).write_unaligned(x14);
		(st.add(1792) as *mut [uint32x4_t; 8]).write_unaligned(x15);
		(st.add(2048) as *mut [uint32x4_t; 8]).write_unaligned(x16);
		(st.add(2304) as *mut [uint32x4_t; 8]).write_unaligned(x17);
		(st.add(2560) as *mut [uint32x4_t; 8]).write_unaligned(x18);
		(st.add(2816) as *mut u32).write_unaligned(x19);
	}
}

/// `label_parents_inner`.
#[inline(always)]
unsafe fn b0_r0r1sr3r6o6gsi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8, arg3: *const u8, arg6: *const u8, arg6_x5: *const u8, x7: u32, x8: u32, phase: u32, st: *mut u8) {
	let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x2.wrapping_add(40u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x2.wrapping_add(32u64 as usize) as *const u32).read_unaligned();
	let mut x9: u32 = 0;
	let mut x10 = [Z; 8];
	let mut x11 = [Z; 8];
	let mut x12 = [Z; 8];
	let mut x13: u32 = 0;
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16 = [Z; 8];
	let mut x17 = [Z; 8];
	let mut x18 = [Z; 8];
	let mut x19: u32 = 0;
	let mut live = splat(u32::MAX);
	if phase > 0 {
		live = (st as *const [uint32x4_t; 8]).read_unaligned();
		x9 = (st.add(256) as *const u32).read_unaligned();
		x10 = (st.add(512) as *const [uint32x4_t; 8]).read_unaligned();
		x11 = (st.add(768) as *const [uint32x4_t; 8]).read_unaligned();
		x12 = (st.add(1024) as *const [uint32x4_t; 8]).read_unaligned();
		x13 = (st.add(1280) as *const u32).read_unaligned();
		x14 = (st.add(1536) as *const [uint32x4_t; 8]).read_unaligned();
		x15 = (st.add(1792) as *const [uint32x4_t; 8]).read_unaligned();
		x16 = (st.add(2048) as *const [uint32x4_t; 8]).read_unaligned();
		x17 = (st.add(2304) as *const [uint32x4_t; 8]).read_unaligned();
		x18 = (st.add(2560) as *const [uint32x4_t; 8]).read_unaligned();
		x19 = (st.add(2816) as *const u32).read_unaligned();
		if !any(&live) { return; }
	}
	if phase == 0 {
		'phase: {
			if ((a0) == (0u32)) {
				live = [Z; 8];
				break 'phase;
			}
			x9 = gid0;
			let t0 = [Z; 8];
			x10 = t0;
			let t1 = splat(a0);
			x11 = t1;
			let mut t2 = live;
			loop {
				let t3 = bits(&t2);
				if t3.count_ones() <= 2 {
					let mut t4 = t3;
					while t4 != 0 {
						let j = t4.trailing_zeros() as usize;
						t4 &= t4 - 1;
						let mut x9_j: u32 = (x9).wrapping_add(j as u32);
						let t5 = words8(x10);
						let mut x10_j: u32 = t5[j];
						let t6 = words8(x11);
						let mut x11_j: u32 = t6[j];
						let t7 = words8(x12);
						let mut x12_j: u32 = t7[j];
						loop {
							if !((((x11_j).wrapping_sub(x10_j)) > (1u32))) { break; }
							x12_j = ((x10_j).wrapping_add(x11_j)).checked_div(2u32).unwrap_or(0);
							if (((((arg0.wrapping_add((((x12_j) as u64)).wrapping_mul(128u64) as usize).wrapping_add(8u64 as usize) as *const u32).read_unaligned()).checked_div(32u32).unwrap_or(0)).wrapping_add((2u32).wrapping_mul(x12_j))) <= (x9_j)) {
								x10_j = x12_j;
							} else {
								x11_j = x12_j;
							}
						}
						let mut t8 = words8(x10);
						t8[j] = x10_j as u32;
						x10 = regs8(t8);
						let mut t9 = words8(x11);
						t9[j] = x11_j as u32;
						x11 = regs8(t9);
						let mut t10 = words8(x12);
						t10[j] = x12_j as u32;
						x12 = regs8(t10);
					}
					break;
				}
				// Lanes part: each its own rounds.
				let t11 = sub_r(x11, x10);
				let t12 = splat(1u32);
				let t13 = cgt_r(t11, t12);
				t2 = and_r(t2, t13);
				if !(any(&t2)) { break; }
				let t14 = add_r(x10, x11);
				let t15: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t14[r], vdupq_n_s32(-1)));
				x12 = t15;
				let t16 = ix_on(t2, x12);
				let t17 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t16, 128);
				let t18: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t17[r], vdupq_n_s32(-5)));
				let t19 = splat(2u32);
				let t20 = mul_r(t19, x12);
				let t21 = add_r(t18, t20);
				let t22 = iota(x9);
				let t23 = cle_r(t21, t22);
				let t24 = and_r(t2, t23);
				{
					x10 = bsl_r(t24, x12, x10);
				}
				let t25 = bic_r(t2, t23);
				{
					x11 = bsl_r(t25, x12, x11);
				}
			}
			let t26 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, x10, 128);
			let t27: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t26[r], vdupq_n_s32(-5)));
			let t28 = splat(2u32);
			let t29 = mul_r(t28, x10);
			let t30 = add_r(t27, t29);
			x14 = t30;
			let t31 = ld_u32((arg0).wrapping_add((4u64) as usize) as *const u8, x10, 128);
			x15 = t31;
			let t32 = iota(x9);
			let t33 = sub_r(t32, x14);
			x16 = t33;
			let t34 = cge_r(t32, x14);
			let t35 = splat(a1);
			let t36 = clt_r(t32, t35);
			let t37 = and_r(t34, t36);
			let t38 = splat(32u32);
			let t39 = mul_r(x16, t38);
			let t40 = clt_r(t39, x15);
			let t41 = and_r(t37, t40);
			x17 = t41;
			let t42 = mvn_r(x17);
			let mut t43 = [Z; 8];
			let t44 = mvn_r(t42);
			{
				t43 = bsl_r(t42, t0, t43);
			}
			{
				let t45 = mul_r(x16, t38);
				let t46 = add_r(t45, t38);
				let t47 = cle_r(t46, x15);
				let mut t48 = [Z; 8];
				let t49 = and_r(t44, t47);
				let t50 = bic_r(t44, t47);
				{
					let t51 = splat(4294967295u32);
					t48 = bsl_r(t49, t51, t48);
				}
				{
					let t52 = mul_r(x16, t38);
					let t53 = sub_r(x15, t52);
					let t54 = splat(1u32);
					let t55: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t54[r], vreinterpretq_s32_u32(vandq_u32(t53[r], vdupq_n_u32(31)))));
					let t56 = sub_r(t55, t54);
					t48 = bsl_r(t50, t56, t48);
				}
				t43 = bsl_r(t44, t48, t43);
			}
			x18 = t43;
			b1_r0sr3al128lllisr6o6_p(l, lid0, splat(u32::MAX), arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 0u32, st.add(3072));
		}
	}
	if phase == 1 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 1u32, st.add(3072));
		}
	}
	if phase == 2 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 2u32, st.add(3072));
		}
	}
	if phase == 3 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 3u32, st.add(3072));
		}
	}
	if phase == 4 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 4u32, st.add(3072));
		}
	}
	if phase == 5 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 5u32, st.add(3072));
		}
	}
	if phase == 6 {
		'phase: {
			b1_r0sr3al128lllisr6o6_p(l, lid0, live, arg0, a0, arg3, arg0, x10, x14, x16, x18, x8, (x7).wrapping_mul(64u32), arg6, arg6_x5, 6u32, st.add(3072));
			let t57 = iota(x9);
			let t58 = splat(a1);
			let t59 = clt_r(t57, t58);
			let t60 = and_r(live, t59);
			if any(&t60) {
				x19 = 0u32;
				loop {
					// Rounds of one condition: every lane goes on.
					if !(((x19) < (32u32))) { break; }
					let mut t61 = [Z; 8];
					if ((x8.wrapping_mul(32u32).wrapping_add(x19)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 && all(&t60) {
						let t62 = (arg6 as *const u8).add(((x8.wrapping_mul(32u32).wrapping_add(x19)) as usize) * 4);
						let t63 = ((32u32) as usize) * 4;
						t61 = ldj_u32(|j| t62.add(j * t63) as *const u8);
					} else {
						let t64: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x8.wrapping_mul(32u32).wrapping_add(x19)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
						let t65 = ix_on(t60, t64);
						let t66 = arg6 as *const u32;
						let mut t67 = [Z; 8];
						let t68 = words8(t65);
						t67 = ldj_u32(|j| t66.add(t68[j] as usize) as *const u8);
						t61 = t67;
					}
					if ((x9.wrapping_mul(32u32).wrapping_add(x19)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
						let t69 = (arg1 as *mut u32).add((x9.wrapping_mul(32u32).wrapping_add(x19)) as usize);
						let t70 = (32u32) as usize;
						stj_u32(t60, t61, |j| t69.add(j * t70) as *mut u8);
					} else {
						let t71: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x9.wrapping_mul(32u32).wrapping_add(x19)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
						let t72 = words8(t71);
						stj_u32(t60, t61, |j| (arg1 as *mut u32).add(t72[j] as usize) as *mut u8);
					}
					let t73 = x19;
					x19 = (x19).wrapping_add(1u32);
				}
			}
		}
	}
	if phase + 1 < 7 {
		(st as *mut [uint32x4_t; 8]).write_unaligned(live);
		(st.add(256) as *mut u32).write_unaligned(x9);
		(st.add(512) as *mut [uint32x4_t; 8]).write_unaligned(x10);
		(st.add(768) as *mut [uint32x4_t; 8]).write_unaligned(x11);
		(st.add(1024) as *mut [uint32x4_t; 8]).write_unaligned(x12);
		(st.add(1280) as *mut u32).write_unaligned(x13);
		(st.add(1536) as *mut [uint32x4_t; 8]).write_unaligned(x14);
		(st.add(1792) as *mut [uint32x4_t; 8]).write_unaligned(x15);
		(st.add(2048) as *mut [uint32x4_t; 8]).write_unaligned(x16);
		(st.add(2304) as *mut [uint32x4_t; 8]).write_unaligned(x17);
		(st.add(2560) as *mut [uint32x4_t; 8]).write_unaligned(x18);
		(st.add(2816) as *mut u32).write_unaligned(x19);
	}
}

/// `phase_parents`.
#[inline(always)]
unsafe fn b1_r0sr3al128lllisr6o6_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg0: *const u8, x1: u32, arg3: *const u8, x3b: *const u8, x3i: [uint32x4_t; 8], x4: [uint32x4_t; 8], x5: [uint32x4_t; 8], x6: [uint32x4_t; 8], x7: u32, x8: u32, arg6: *const u8, arg6_x10: *const u8, phase: u32, st: *mut u8) {
	let mut x11: u32 = 0;
	let mut x12 = [Z; 8];
	let mut x13 = [Z; 8];
	let mut x14 = [Z; 8];
	let mut x15 = [Z; 8];
	let mut x16: u32 = 0;
	let mut x17: u32 = 0;
	let mut x18 = [Z; 8];
	let mut x19 = [Z; 8];
	let mut x20 = [Z; 8];
	let mut x21 = [Z; 8];
	let mut x22 = [Z; 8];
	let mut x23: u32 = 0;
	let mut x24: u32 = 0;
	let mut x25 = [Z; 8];
	let mut x26 = [Z; 8];
	let mut x27 = [Z; 8];
	let mut live = k;
	if phase > 0 {
		live = (st as *const [uint32x4_t; 8]).read_unaligned();
		x11 = (st.add(256) as *const u32).read_unaligned();
		x12 = (st.add(512) as *const [uint32x4_t; 8]).read_unaligned();
		x13 = (st.add(768) as *const [uint32x4_t; 8]).read_unaligned();
		x14 = (st.add(1024) as *const [uint32x4_t; 8]).read_unaligned();
		x15 = (st.add(1280) as *const [uint32x4_t; 8]).read_unaligned();
		x16 = (st.add(1536) as *const u32).read_unaligned();
		x17 = (st.add(1792) as *const u32).read_unaligned();
		x18 = (st.add(2048) as *const [uint32x4_t; 8]).read_unaligned();
		x19 = (st.add(2304) as *const [uint32x4_t; 8]).read_unaligned();
		x20 = (st.add(2560) as *const [uint32x4_t; 8]).read_unaligned();
		x21 = (st.add(2816) as *const [uint32x4_t; 8]).read_unaligned();
		x22 = (st.add(3072) as *const [uint32x4_t; 8]).read_unaligned();
		x23 = (st.add(3328) as *const u32).read_unaligned();
		x24 = (st.add(3584) as *const u32).read_unaligned();
		x25 = (st.add(3840) as *const [uint32x4_t; 8]).read_unaligned();
		x26 = (st.add(4096) as *const [uint32x4_t; 8]).read_unaligned();
		x27 = (st.add(4352) as *const [uint32x4_t; 8]).read_unaligned();
		if !any(&live) { return; }
	}
	if phase == 0 {
		'phase: {
			let t0 = iota(x7);
			let t1 = [Z; 8];
			let t2 = ceq_r(t0, t1);
			let t3 = and_r(live, t2);
			{
				b2_o6s_p(l, lid0, t3, arg6_x10.wrapping_add((256u64).wrapping_mul(4u64) as usize), 0u32);
			}
		}
	}
	if phase == 1 {
		'phase: {
			x11 = 0u32;
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x11) < (32u32))) { break; }
				let t4 = splat(32u32);
				let t5 = mul_r(x5, t4);
				let t6 = splat(x11);
				let t7 = add_r(t5, t6);
				x12 = t7;
				let t8 = splat(4294967295u32);
				x13 = t8;
				let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x6[r], vnegq_s32(vdupq_n_s32(((x11) & 31) as i32))));
				let t10 = splat(1u32);
				let t11 = and_r(t9, t10);
				let t12 = [Z; 8];
				let t13 = cne_r(t11, t12);
				let t14 = cgt_r(x12, t12);
				let t15 = and_r(t13, t14);
				let t16 = and_r(live, t15);
				if any(&t16) {
					let t17 = b5_r3al128_p(l, lid0, t16, arg3, x3b, x3i);
					let t18 = sub_r(x12, t10);
					let t19 = b4_lal128l_p(l, lid0, t16, t17, x3b, x3i, t18);
					let t20: [uint32x4_t; 8] = lo(t19);
					x14 = t20;
					let t21 = and_r(x14, t10);
					let t22 = ceq_r(t21, t12);
					let mut t23 = [Z; 8];
					let t24 = and_r(t16, t22);
					let t25 = bic_r(t16, t22);
					{
						let t26 = sub_r(x12, t10);
						t23 = bsl_r(t24, t26, t23);
					}
					{
						let t27 = splat(2u32);
						let t28 = and_r(x14, t27);
						let t29 = cne_r(t28, t12);
						let mut t30 = [Z; 8];
						let t31 = and_r(t25, t29);
						let t32 = bic_r(t25, t29);
						{
							let t33: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x14[r], vnegq_s32(vdupq_n_s32(((2u32) & 31) as i32))));
							let t34 = sub_r(x12, t33);
							t30 = bsl_r(t31, t34, t30);
						}
						{
							let t35: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x14[r], vnegq_s32(vdupq_n_s32(((2u32) & 31) as i32))));
							let t36 = sub_r(x12, t35);
							let t37 = splat(2147483648u32);
							let t38 = orr_r(t37, t36);
							t30 = bsl_r(t32, t38, t30);
						}
						t23 = bsl_r(t25, t30, t23);
					}
					x13 = bsl_r(t16, t23, x13);
					let t39 = splat(2147483648u32);
					let t40 = and_r(x13, t39);
					let t41 = cne_r(t40, t12);
					let t42 = splat(!(2147483648u32));
					let t43 = and_r(x13, t42);
					let t44: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t43[r], vdupq_n_s32(-5)));
					let t45 = add_r(x4, t44);
					let t46 = splat(x8);
					let t47 = clt_r(t45, t46);
					let t48 = and_r(t41, t47);
					let t49 = and_r(t16, t48);
					if any(&t49) {
						let t50 = b6_o6s_p(l, lid0, t49, arg6_x10.wrapping_add((256u64).wrapping_mul(4u64) as usize), 1u32);
						x15 = t50;
						let t51 = splat(256u32);
						let t52 = clt_r(x15, t51);
						let t53 = and_r(t49, t52);
						{
							let t54: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x11)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
							let t55 = words8(x15);
							stj_u32(t53, t54, |j| (arg6_x10 as *mut u32).add(t55[j] as usize) as *mut u8);
						}
						let t56 = bic_r(t49, t52);
						if any(&t56) {
							let t57 = splat(!(2147483648u32));
							let t58 = and_r(x13, t57);
							let t59 = b7_r3al128l_p(l, lid0, t56, arg3, x3b, x3i, t58);
							x13 = bsl_r(t56, t59, x13);
						}
					}
				}
				if ((x7.wrapping_mul(32u32).wrapping_add(x11)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
					let t60 = (arg6 as *mut u32).add((x7.wrapping_mul(32u32).wrapping_add(x11)) as usize);
					let t61 = (32u32) as usize;
					stj_u32(live, x13, |j| t60.add(j * t61) as *mut u8);
				} else {
					let t62: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x11)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
					let t63 = words8(t62);
					stj_u32(live, x13, |j| (arg6 as *mut u32).add(t63[j] as usize) as *mut u8);
				}
				let t64 = x11;
				x11 = (x11).wrapping_add(1u32);
			}
		}
	}
	if phase == 2 {
		'phase: {
			let t65 = b9_o6_p(l, lid0, live, arg6_x10.wrapping_add((256u64).wrapping_mul(4u64) as usize));
			let t66 = b8_ss_p(l, lid0, live, t65, 256u32);
			x16 = t66;
			x17 = x7;
			let mut t67 = live;
			loop {
				// Lanes part: each its own rounds.
				let t68 = iota(x17);
				let t69 = splat(x16);
				let t70 = clt_r(t68, t69);
				t67 = and_r(t67, t70);
				if !(any(&t67)) { break; }
				let mut t71: [uint32x4_t; 8];
				if all(&t67) {
					let t72 = (arg6_x10 as *const u32).add((x17) as usize);
					t71 = [vld1q_u32(t72.add(0)), vld1q_u32(t72.add(4)), vld1q_u32(t72.add(8)), vld1q_u32(t72.add(12)), vld1q_u32(t72.add(16)), vld1q_u32(t72.add(20)), vld1q_u32(t72.add(24)), vld1q_u32(t72.add(28))];
				} else {
					let t73 = ix_on(t67, t68);
					let t74 = arg6_x10 as *const u32;
					let mut t75 = [Z; 8];
					let t76 = words8(t73);
					t75 = ldj_u32(|j| t74.add(t76[j] as usize) as *const u8);
					t71 = t75;
				}
				x18 = t71;
				let t77: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x18[r], vdupq_n_s32(-5)));
				let t78 = splat(x8);
				let t79 = add_r(t78, t77);
				x19 = t79;
				let t80 = [Z; 8];
				x20 = t80;
				let t81 = splat(x1);
				x21 = t81;
				let mut t82 = t67;
				loop {
					let t83 = bits(&t82);
					if t83.count_ones() <= 2 {
						let mut t84 = t83;
						while t84 != 0 {
							let j = t84.trailing_zeros() as usize;
							t84 &= t84 - 1;
							let t85 = words8(x19);
							let mut x19_j: u32 = t85[j];
							let t86 = words8(x20);
							let mut x20_j: u32 = t86[j];
							let t87 = words8(x21);
							let mut x21_j: u32 = t87[j];
							let t88 = words8(x22);
							let mut x22_j: u32 = t88[j];
							loop {
								if !((((x21_j).wrapping_sub(x20_j)) > (1u32))) { break; }
								x22_j = ((x20_j).wrapping_add(x21_j)).checked_div(2u32).unwrap_or(0);
								if (((((arg0.wrapping_add((((x22_j) as u64)).wrapping_mul(128u64) as usize).wrapping_add(8u64 as usize) as *const u32).read_unaligned()).checked_div(32u32).unwrap_or(0)).wrapping_add((2u32).wrapping_mul(x22_j))) <= (x19_j)) {
									x20_j = x22_j;
								} else {
									x21_j = x22_j;
								}
							}
							let mut t89 = words8(x20);
							t89[j] = x20_j as u32;
							x20 = regs8(t89);
							let mut t90 = words8(x21);
							t90[j] = x21_j as u32;
							x21 = regs8(t90);
							let mut t91 = words8(x22);
							t91[j] = x22_j as u32;
							x22 = regs8(t91);
						}
						break;
					}
					// Lanes part: each its own rounds.
					let t92 = sub_r(x21, x20);
					let t93 = splat(1u32);
					let t94 = cgt_r(t92, t93);
					t82 = and_r(t82, t94);
					if !(any(&t82)) { break; }
					let t95 = add_r(x20, x21);
					let t96: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t95[r], vdupq_n_s32(-1)));
					x22 = t96;
					let t97 = ix_on(t82, x22);
					let t98 = ld_u32((arg0).wrapping_add((8u64) as usize) as *const u8, t97, 128);
					let t99: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t98[r], vdupq_n_s32(-5)));
					let t100 = splat(2u32);
					let t101 = mul_r(t100, x22);
					let t102 = add_r(t99, t101);
					let t103 = cle_r(t102, x19);
					let t104 = and_r(t82, t103);
					{
						x20 = bsl_r(t104, x22, x20);
					}
					let t105 = bic_r(t82, t103);
					{
						x21 = bsl_r(t105, x22, x21);
					}
				}
				let t106 = ix_on(t67, x18);
				let t107 = arg6 as *const u32;
				let mut t108 = [Z; 8];
				let t109 = words8(t106);
				t108 = ldj_u32(|j| t107.add(t109[j] as usize) as *const u8);
				let t110 = splat(!(2147483648u32));
				let t111 = and_r(t108, t110);
				let t112 = b7_r3al128r0l_p(l, lid0, t67, arg3, arg0, x20, t111);
				let t113 = words8(x18);
				stj_u32(t67, t112, |j| (arg6 as *mut u32).add(t113[j] as usize) as *mut u8);
				let t114 = and_r(t67, live);
				x17 = x17.wrapping_add(64u32);
			}
		}
	}
	if phase == 3 {
		'phase: {
			x23 = 0u32;
			x24 = 0u32;
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x24) < (32u32))) { break; }
				let mut t115 = [Z; 8];
				let mut t116 = [Z; 8];
				if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 && all(&live) {
					let t117 = (arg6 as *const u8).add(((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize) * 4);
					let t118 = ((32u32) as usize) * 4;
					t116 = ldj_u32(|j| t117.add(j * t118) as *const u8);
				} else {
					let t119: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
					let t120 = ix_on(live, t119);
					let t121 = arg6 as *const u32;
					let mut t122 = [Z; 8];
					let t123 = words8(t120);
					t122 = ldj_u32(|j| t121.add(t123[j] as usize) as *const u8);
					t116 = t122;
				}
				x25 = t116;
				let t124 = splat(4294967295u32);
				let t125 = ceq_r(x25, t124);
				let t126 = splat(2147483648u32);
				let t127 = and_r(x25, t126);
				let t128 = [Z; 8];
				let t129 = ceq_r(t127, t128);
				let t130 = orr_r(t125, t129);
				let t131 = and_r(live, t130);
				if any(&t131) {
					t115 = orr_r(t115, t131);
				}
				let t132: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(live[r], live[r]), t115[r]));
				if any(&t132) {
					let t133 = splat(!(2147483648u32));
					let t134 = and_r(x25, t133);
					x26 = bsl_r(t132, t134, x26);
					let t135: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x26[r], vdupq_n_s32(-5)));
					let t136 = add_r(x4, t135);
					x27 = bsl_r(t132, t136, x27);
					let t137 = splat(x8);
					let t138 = cge_r(x27, t137);
					let t139 = splat((x8).wrapping_add(64u32));
					let t140 = clt_r(x27, t139);
					let t141 = and_r(t138, t140);
					let t142 = and_r(t132, t141);
					if any(&t142) {
						let t143 = splat(x8);
						let t144 = sub_r(x27, t143);
						let t145 = splat(32u32);
						let t146 = mul_r(t144, t145);
						let t147: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x26[r], vdupq_n_u32(31u32)));
						let t148 = add_r(t146, t147);
						let t149 = ix_on(t142, t148);
						let t150 = arg6 as *const u32;
						let mut t151 = [Z; 8];
						let t152 = words8(t149);
						t151 = ldj_u32(|j| t150.add(t152[j] as usize) as *const u8);
						if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
							let t153 = (arg6 as *mut u32).add((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize);
							let t154 = (32u32) as usize;
							stj_u32(t142, t151, |j| t153.add(j * t154) as *mut u8);
						} else {
							let t155: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
							let t156 = words8(t155);
							stj_u32(t142, t151, |j| (arg6 as *mut u32).add(t156[j] as usize) as *mut u8);
						}
					}
				}
				let t157 = x24;
				x24 = (x24).wrapping_add(1u32);
			}
		}
	}
	if phase == 4 {
		'phase: {
			x23 = 1u32;
			x24 = 0u32;
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x24) < (32u32))) { break; }
				let mut t158 = [Z; 8];
				let mut t159 = [Z; 8];
				if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 && all(&live) {
					let t160 = (arg6 as *const u8).add(((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize) * 4);
					let t161 = ((32u32) as usize) * 4;
					t159 = ldj_u32(|j| t160.add(j * t161) as *const u8);
				} else {
					let t162: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
					let t163 = ix_on(live, t162);
					let t164 = arg6 as *const u32;
					let mut t165 = [Z; 8];
					let t166 = words8(t163);
					t165 = ldj_u32(|j| t164.add(t166[j] as usize) as *const u8);
					t159 = t165;
				}
				x25 = t159;
				let t167 = splat(4294967295u32);
				let t168 = ceq_r(x25, t167);
				let t169 = splat(2147483648u32);
				let t170 = and_r(x25, t169);
				let t171 = [Z; 8];
				let t172 = ceq_r(t170, t171);
				let t173 = orr_r(t168, t172);
				let t174 = and_r(live, t173);
				if any(&t174) {
					t158 = orr_r(t158, t174);
				}
				let t175: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(live[r], live[r]), t158[r]));
				if any(&t175) {
					let t176 = splat(!(2147483648u32));
					let t177 = and_r(x25, t176);
					x26 = bsl_r(t175, t177, x26);
					let t178: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x26[r], vdupq_n_s32(-5)));
					let t179 = add_r(x4, t178);
					x27 = bsl_r(t175, t179, x27);
					let t180 = splat(x8);
					let t181 = cge_r(x27, t180);
					let t182 = splat((x8).wrapping_add(64u32));
					let t183 = clt_r(x27, t182);
					let t184 = and_r(t181, t183);
					let t185 = and_r(t175, t184);
					if any(&t185) {
						let t186 = splat(x8);
						let t187 = sub_r(x27, t186);
						let t188 = splat(32u32);
						let t189 = mul_r(t187, t188);
						let t190: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x26[r], vdupq_n_u32(31u32)));
						let t191 = add_r(t189, t190);
						let t192 = ix_on(t185, t191);
						let t193 = arg6 as *const u32;
						let mut t194 = [Z; 8];
						let t195 = words8(t192);
						t194 = ldj_u32(|j| t193.add(t195[j] as usize) as *const u8);
						if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
							let t196 = (arg6 as *mut u32).add((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize);
							let t197 = (32u32) as usize;
							stj_u32(t185, t194, |j| t196.add(j * t197) as *mut u8);
						} else {
							let t198: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
							let t199 = words8(t198);
							stj_u32(t185, t194, |j| (arg6 as *mut u32).add(t199[j] as usize) as *mut u8);
						}
					}
				}
				let t200 = x24;
				x24 = (x24).wrapping_add(1u32);
			}
		}
	}
	if phase == 5 {
		'phase: {
			x23 = 2u32;
			x24 = 0u32;
			loop {
				// Rounds of one condition: every lane goes on.
				if !(((x24) < (32u32))) { break; }
				let mut t201 = [Z; 8];
				let mut t202 = [Z; 8];
				if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 && all(&live) {
					let t203 = (arg6 as *const u8).add(((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize) * 4);
					let t204 = ((32u32) as usize) * 4;
					t202 = ldj_u32(|j| t203.add(j * t204) as *const u8);
				} else {
					let t205: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
					let t206 = ix_on(live, t205);
					let t207 = arg6 as *const u32;
					let mut t208 = [Z; 8];
					let t209 = words8(t206);
					t208 = ldj_u32(|j| t207.add(t209[j] as usize) as *const u8);
					t202 = t208;
				}
				x25 = t202;
				let t210 = splat(4294967295u32);
				let t211 = ceq_r(x25, t210);
				let t212 = splat(2147483648u32);
				let t213 = and_r(x25, t212);
				let t214 = [Z; 8];
				let t215 = ceq_r(t213, t214);
				let t216 = orr_r(t211, t215);
				let t217 = and_r(live, t216);
				if any(&t217) {
					t201 = orr_r(t201, t217);
				}
				let t218: [uint32x4_t; 8] = core::array::from_fn(|r| vbicq_u32(vandq_u32(live[r], live[r]), t201[r]));
				if any(&t218) {
					let t219 = splat(!(2147483648u32));
					let t220 = and_r(x25, t219);
					x26 = bsl_r(t218, t220, x26);
					let t221: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x26[r], vdupq_n_s32(-5)));
					let t222 = add_r(x4, t221);
					x27 = bsl_r(t218, t222, x27);
					let t223 = splat(x8);
					let t224 = cge_r(x27, t223);
					let t225 = splat((x8).wrapping_add(64u32));
					let t226 = clt_r(x27, t225);
					let t227 = and_r(t224, t226);
					let t228 = and_r(t218, t227);
					if any(&t228) {
						let t229 = splat(x8);
						let t230 = sub_r(x27, t229);
						let t231 = splat(32u32);
						let t232 = mul_r(t230, t231);
						let t233: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(x26[r], vdupq_n_u32(31u32)));
						let t234 = add_r(t232, t233);
						let t235 = ix_on(t228, t234);
						let t236 = arg6 as *const u32;
						let mut t237 = [Z; 8];
						let t238 = words8(t235);
						t237 = ldj_u32(|j| t236.add(t238[j] as usize) as *const u8);
						if ((x7.wrapping_mul(32u32).wrapping_add(x24)) as u64) + 31 * ((32u32) as u64) <= u32::MAX as u64 {
							let t239 = (arg6 as *mut u32).add((x7.wrapping_mul(32u32).wrapping_add(x24)) as usize);
							let t240 = (32u32) as usize;
							stj_u32(t228, t237, |j| t239.add(j * t240) as *mut u8);
						} else {
							let t241: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(x7.wrapping_mul(32u32).wrapping_add(x24)), vld1q_u32(LANE[r].as_ptr()), vdupq_n_u32(32u32)));
							let t242 = words8(t241);
							stj_u32(t228, t237, |j| (arg6 as *mut u32).add(t242[j] as usize) as *mut u8);
						}
					}
				}
				let t243 = x24;
				x24 = (x24).wrapping_add(1u32);
			}
		}
	}
	if phase == 6 {
		'phase: {
		}
	}
	if phase + 1 < 7 {
		(st as *mut [uint32x4_t; 8]).write_unaligned(live);
		(st.add(256) as *mut u32).write_unaligned(x11);
		(st.add(512) as *mut [uint32x4_t; 8]).write_unaligned(x12);
		(st.add(768) as *mut [uint32x4_t; 8]).write_unaligned(x13);
		(st.add(1024) as *mut [uint32x4_t; 8]).write_unaligned(x14);
		(st.add(1280) as *mut [uint32x4_t; 8]).write_unaligned(x15);
		(st.add(1536) as *mut u32).write_unaligned(x16);
		(st.add(1792) as *mut u32).write_unaligned(x17);
		(st.add(2048) as *mut [uint32x4_t; 8]).write_unaligned(x18);
		(st.add(2304) as *mut [uint32x4_t; 8]).write_unaligned(x19);
		(st.add(2560) as *mut [uint32x4_t; 8]).write_unaligned(x20);
		(st.add(2816) as *mut [uint32x4_t; 8]).write_unaligned(x21);
		(st.add(3072) as *mut [uint32x4_t; 8]).write_unaligned(x22);
		(st.add(3328) as *mut u32).write_unaligned(x23);
		(st.add(3584) as *mut u32).write_unaligned(x24);
		(st.add(3840) as *mut [uint32x4_t; 8]).write_unaligned(x25);
		(st.add(4096) as *mut [uint32x4_t; 8]).write_unaligned(x26);
		(st.add(4352) as *mut [uint32x4_t; 8]).write_unaligned(x27);
	}
}

/// `ukl_atomic_store_local`.
#[inline(always)]
unsafe fn b2_o6s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg6_x0: *const u8, x1: u32) {
	let mut live = k;
	(arg6_x0 as *mut u32).write_unaligned(x1);
}

/// `clif_tape_slot`.
#[inline(always)]
unsafe fn b4_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x3 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 16];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((12u32) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((x1b).wrapping_add((((((12u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	x3 = t1;
	let t5 = b10_lal128l_p(l, lid0, live, x0, x1b, x1i, x2);
	res = t5;
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b5_r3al128_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg3: *const u8, x1b: *const u8, x1i: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = ix_on(live, x1i);
	let t1 = ld_u32((x1b).wrapping_add((124u64) as usize) as *const u8, t0, 128);
	let t2 = cat_w([Z; 8], lo(t1));
	let t3 = ix_on(live, x1i);
	let t4 = ld_u32(x1b as *const u8, t3, 128);
	let t5 = orr_r(lo(t2), t4);
	let t6 = cat_w(t5, hi(t2));
	let t7 = cat_w(live, live);
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[r], t7[r]));
	let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[(r + 8)], t7[(r + 8)]));
	let t10 = cat_w(t8, t9);
	let t11: u64 = ((arg3) as usize as u64);
	let t12 = splat((t11) as u32);
	let t13 = splat(((t11) >> 32) as u32);
	let t14 = add_r(t12, lo(t10));
	let t15 = clt_r(t14, t12);
	let t16 = adc_r(t13, hi(t10), t15);
	let t17 = cat_w(lo(t14), t16);
	res = t17;
	return res;
}

/// `tape_doc`.
#[inline(always)]
unsafe fn b5_r3al128r0_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg3: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = ix_on(live, x1i);
	let t1 = ld_u32((arg0).wrapping_add((124u64) as usize) as *const u8, t0, 128);
	let t2 = cat_w([Z; 8], lo(t1));
	let t3 = ix_on(live, x1i);
	let t4 = ld_u32(arg0 as *const u8, t3, 128);
	let t5 = orr_r(lo(t2), t4);
	let t6 = cat_w(t5, hi(t2));
	let t7 = cat_w(live, live);
	let t8: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[r], t7[r]));
	let t9: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t6[(r + 8)], t7[(r + 8)]));
	let t10 = cat_w(t8, t9);
	let t11: u64 = ((arg3) as usize as u64);
	let t12 = splat((t11) as u32);
	let t13 = splat(((t11) >> 32) as u32);
	let t14 = add_r(t12, lo(t10));
	let t15 = clt_r(t14, t12);
	let t16 = adc_r(t13, hi(t10), t15);
	let t17 = cat_w(lo(t14), t16);
	res = t17;
	return res;
}

/// `ukl_atomic_add_local`.
#[inline(always)]
unsafe fn b6_o6s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg6_x0: *const u8, x1: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0 = [0u32; 32];
	for r in 0..8 { vst1q_u32(t0.as_mut_ptr().add(4 * r), live[r]); }
	let mut t1 = [0u32; 32];
	let mut t2 = 0u32;
	for j in 0..32 { t1[j] = t2; t2 += t0[j] & 1; }
	let t3: [uint32x4_t; 8] = [vld1q_u32(t1.as_ptr().add(0)), vld1q_u32(t1.as_ptr().add(4)), vld1q_u32(t1.as_ptr().add(8)), vld1q_u32(t1.as_ptr().add(12)), vld1q_u32(t1.as_ptr().add(16)), vld1q_u32(t1.as_ptr().add(20)), vld1q_u32(t1.as_ptr().add(24)), vld1q_u32(t1.as_ptr().add(28))];
	let t4 = AtomicU32::from_ptr(arg6_x0 as *mut u32).fetch_add((x1).wrapping_mul(t2), Relaxed);
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vmlaq_u32(vdupq_n_u32(t4), t3[r], vdupq_n_u32(x1)));
	res = t5;
	return res;
}

/// `tape_parent`.
#[inline(always)]
unsafe fn b7_r3al128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg3: *const u8, x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b5_r3al128_p(l, lid0, live, arg3, x1b, x1i);
	let t1 = b11_lal128l_p(l, lid0, live, t0, x1b, x1i, x2);
	res = t1;
	return res;
}

/// `tape_parent`.
#[inline(always)]
unsafe fn b7_r3al128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg3: *const u8, arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b5_r3al128r0_p(l, lid0, live, arg3, arg0, x1i);
	let t1 = b11_lal128r0l_p(l, lid0, live, t0, arg0, x1i, x2);
	res = t1;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b8_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32, x1: u32) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	let t0: u32;
	if ((x0) < (x1)) {
		t0 = x0;
	} else {
		t0 = x1;
	}
	res = t0;
	return res;
}

/// `ukl_atomic_load_local`.
#[inline(always)]
unsafe fn b9_o6_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], arg6_x0: *const u8) -> u32 {
	let mut live = k;
	let mut res: u32 = 0;
	res = (arg6_x0 as *const u32).read_unaligned();
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b10_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b12_lal128sl_p(l, lid0, live, x0, x1b, x1i, 0u32, x2);
	res = t0;
	return res;
}

/// `clif_f16_slot`.
#[inline(always)]
unsafe fn b10_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b12_lal128r0sl_p(l, lid0, live, x0, arg0, x1i, 0u32, x2);
	res = t0;
	return res;
}

/// `clif_tape_parent`.
#[inline(always)]
unsafe fn b11_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((12u32) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((x1b).wrapping_add((((((12u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	x3 = t1;
	let t5 = splat(16u32);
	let t6 = ceq_r(x3, t5);
	let t7 = and_r(live, t6);
	if any(&t7) {
		let t8 = b13_lal128l_p(l, lid0, t7, x0, x1b, x1i, x2);
		res = bsl_r(t7, t8, res);
		live = bic_r(live, t7);
	}
	if any(&live) {
		let t9 = b14_lal128l_p(l, lid0, live, x0, x1b, x1i, x2);
		res = bsl_r(live, t9, res);
		live = bic_r(live, live);
	}
	res
}

/// `clif_tape_parent`.
#[inline(always)]
unsafe fn b11_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((12u32) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((arg0).wrapping_add((((((12u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	x3 = t1;
	let t5 = splat(16u32);
	let t6 = ceq_r(x3, t5);
	let t7 = and_r(live, t6);
	if any(&t7) {
		let t8 = b13_lal128r0l_p(l, lid0, t7, x0, arg0, x1i, x2);
		res = bsl_r(t7, t8, res);
		live = bic_r(live, t7);
	}
	if any(&live) {
		let t9 = b14_lal128r0l_p(l, lid0, live, x0, arg0, x1i, x2);
		res = bsl_r(live, t9, res);
		live = bic_r(live, live);
	}
	res
}

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b12_lal128sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 16];
	let mut x7 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b15_lal128s_p(l, lid0, live, x0, x1b, x1i, x2);
	x4 = t0;
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t2 = b16_lal128sl_p(l, lid0, live, x0, x1b, x1i, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b17_l_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = splat(1023u32);
	let t5 = and_r(x3, t4);
	let t6 = mul_r(t5, x4);
	let t7 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x5, t6, 8u32);
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
				let t24 = b19_lssl_p(l, lid0, splat(u32::MAX), t22, (x1b as *const u8).wrapping_add(((t18[j]) as usize).wrapping_mul(128)), (x2).wrapping_add(1u32), t23);
				t20[j] = vgetq_lane_u32::<0>(t24[0]);
				t20[32 + j] = vgetq_lane_u32::<0>(t24[8]);
			}
			t15 = regs16(t20);
		} else {
			let t25 = b19_lal128sl_p(l, lid0, t13, x0, x1b, x1i, (x2).wrapping_add(1u32), x3);
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

/// `clif_f16_get`.
#[inline(always)]
unsafe fn b12_lal128r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 16];
	let mut x7 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b15_lal128r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	x4 = t0;
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	let t2 = b16_lal128r0sl_p(l, lid0, live, x0, arg0, x1i, (1u32).wrapping_add(x2), t1);
	x5 = t2;
	let t3 = b17_l_p(l, lid0, live, x4);
	x6 = t3;
	let t4 = splat(1023u32);
	let t5 = and_r(x3, t4);
	let t6 = mul_r(t5, x4);
	let t7 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t6, 8u32);
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
				let t24 = b19_lssl_p(l, lid0, splat(u32::MAX), t22, (arg0 as *const u8).wrapping_add(((t18[j]) as usize).wrapping_mul(128)), (x2).wrapping_add(1u32), t23);
				t20[j] = vgetq_lane_u32::<0>(t24[0]);
				t20[32 + j] = vgetq_lane_u32::<0>(t24[8]);
			}
			t15 = regs16(t20);
		} else {
			let t25 = b19_lal128r0sl_p(l, lid0, t13, x0, arg0, x1i, (x2).wrapping_add(1u32), x3);
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

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b13_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b20_lal128l_p(l, lid0, live, x0, x1b, x1i, x2);
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
		let t13 = b20_lal128l_p(l, lid0, t1, x0, x1b, x1i, t12);
		x3 = bsl_w(t1, t13, x3);
	}
	let t14: [uint32x4_t; 8] = lo(x3);
	res = t14;
	return res;
}

/// `clif_f16_parent`.
#[inline(always)]
unsafe fn b13_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b20_lal128r0l_p(l, lid0, live, x0, arg0, x1i, x2);
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
		let t13 = b20_lal128r0l_p(l, lid0, t1, x0, arg0, x1i, t12);
		x3 = bsl_w(t1, t13, x3);
	}
	let t14: [uint32x4_t; 8] = lo(x3);
	res = t14;
	return res;
}

/// `clif_f15_parent`.
#[inline(always)]
unsafe fn b14_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut x4: u32 = 0;
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = [Z; 8];
	let t1 = ceq_r(x2, t0);
	let t2 = and_r(live, t1);
	{
		let t3 = splat(4294967295u32);
		res = bsl_r(t2, t3, res);
		live = bic_r(live, t2);
	}
	if any(&live) {
		x3 = x2;
		x4 = 0u32;
		let mut t4 = live;
		loop {
			// Lanes part: each its own rounds.
			let t5 = cne_r(x3, t0);
			let t6 = splat(if ((x4) < (64u32)) { u32::MAX } else { 0 });
			let t7 = and_r(t5, t6);
			t4 = and_r(t4, t7);
			if !(any(&t4)) { break; }
			let t8 = splat(1u32);
			let t9 = sub_r(x3, t8);
			x3 = t9;
			let t10 = b22_lal128l_p(l, lid0, t4, x0, x1b, x1i, x3);
			let t11 = b21_lal128ll_p(l, lid0, t4, x0, x1b, x1i, x3, t10);
			let t12 = cgt_r(t11, x2);
			let t13 = and_r(t4, t12);
			{
				res = bsl_r(t13, x3, res);
				live = bic_r(live, t13);
			}
			let t14 = and_r(t4, live);
			let t15 = x4;
			x4 = (x4).wrapping_add(1u32);
			t4 = and_r(t4, live);
		}
		if any(&live) {
			x5 = t0;
			let mut t16 = live;
			loop {
				// Lanes part: each its own rounds.
				let t17 = splat(if true { u32::MAX } else { 0 });
				t16 = and_r(t16, t17);
				if !(any(&t16)) { break; }
				let t18 = splat(1u32);
				let t19 = add_r(x5, t18);
				x6 = t19;
				let t20 = b22_lal128l_p(l, lid0, t16, x0, x1b, x1i, x6);
				let t21 = b21_lal128ll_p(l, lid0, t16, x0, x1b, x1i, x6, t20);
				x7 = t21;
				let mut t22 = t16;
				loop {
					// Lanes part: each its own rounds.
					let t23 = cle_r(x7, x2);
					t22 = and_r(t22, t23);
					if !(any(&t22)) { break; }
					x6 = bsl_r(t22, x7, x6);
					let t24 = b22_lal128l_p(l, lid0, t22, x0, x1b, x1i, x6);
					let t25 = b21_lal128ll_p(l, lid0, t22, x0, x1b, x1i, x6, t24);
					x7 = bsl_r(t22, t25, x7);
				}
				let t26 = ceq_r(x6, x2);
				let t27 = and_r(t16, t26);
				{
					res = bsl_r(t27, x5, res);
					live = bic_r(live, t27);
				}
				let t28 = and_r(t16, live);
				if any(&t28) {
					x5 = x6;
				}
				let t29 = and_r(t16, live);
				t16 = and_r(t16, live);
			}
		}
	}
	res
}

/// `clif_f15_parent`.
#[inline(always)]
unsafe fn b14_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x3 = [Z; 8];
	let mut x4: u32 = 0;
	let mut x5 = [Z; 8];
	let mut x6 = [Z; 8];
	let mut x7 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = [Z; 8];
	let t1 = ceq_r(x2, t0);
	let t2 = and_r(live, t1);
	{
		let t3 = splat(4294967295u32);
		res = bsl_r(t2, t3, res);
		live = bic_r(live, t2);
	}
	if any(&live) {
		x3 = x2;
		x4 = 0u32;
		let mut t4 = live;
		loop {
			// Lanes part: each its own rounds.
			let t5 = cne_r(x3, t0);
			let t6 = splat(if ((x4) < (64u32)) { u32::MAX } else { 0 });
			let t7 = and_r(t5, t6);
			t4 = and_r(t4, t7);
			if !(any(&t4)) { break; }
			let t8 = splat(1u32);
			let t9 = sub_r(x3, t8);
			x3 = t9;
			let t10 = b22_lal128r0l_p(l, lid0, t4, x0, arg0, x1i, x3);
			let t11 = b21_lal128r0ll_p(l, lid0, t4, x0, arg0, x1i, x3, t10);
			let t12 = cgt_r(t11, x2);
			let t13 = and_r(t4, t12);
			{
				res = bsl_r(t13, x3, res);
				live = bic_r(live, t13);
			}
			let t14 = and_r(t4, live);
			let t15 = x4;
			x4 = (x4).wrapping_add(1u32);
			t4 = and_r(t4, live);
		}
		if any(&live) {
			x5 = t0;
			let mut t16 = live;
			loop {
				// Lanes part: each its own rounds.
				let t17 = splat(if true { u32::MAX } else { 0 });
				t16 = and_r(t16, t17);
				if !(any(&t16)) { break; }
				let t18 = splat(1u32);
				let t19 = add_r(x5, t18);
				x6 = t19;
				let t20 = b22_lal128r0l_p(l, lid0, t16, x0, arg0, x1i, x6);
				let t21 = b21_lal128r0ll_p(l, lid0, t16, x0, arg0, x1i, x6, t20);
				x7 = t21;
				let mut t22 = t16;
				loop {
					// Lanes part: each its own rounds.
					let t23 = cle_r(x7, x2);
					t22 = and_r(t22, t23);
					if !(any(&t22)) { break; }
					x6 = bsl_r(t22, x7, x6);
					let t24 = b22_lal128r0l_p(l, lid0, t22, x0, arg0, x1i, x6);
					let t25 = b21_lal128r0ll_p(l, lid0, t22, x0, arg0, x1i, x6, t24);
					x7 = bsl_r(t22, t25, x7);
				}
				let t26 = ceq_r(x6, x2);
				let t27 = and_r(t16, t26);
				{
					res = bsl_r(t27, x5, res);
					live = bic_r(live, t27);
				}
				let t28 = and_r(t16, live);
				if any(&t28) {
					x5 = x6;
				}
				let t29 = and_r(t16, live);
				t16 = and_r(t16, live);
			}
		}
	}
	res
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b15_lal128s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, 16u32);
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t0[r], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))));
	let t2 = splat(15u32);
	let t3 = and_r(t1, t2);
	res = t3;
	return res;
}

/// `clif_f16_width`.
#[inline(always)]
unsafe fn b15_lal128r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t1: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t0[r], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))));
	let t2 = splat(15u32);
	let t3 = and_r(t1, t2);
	res = t3;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_lal128sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, 40u32);
	let t1 = mul_r(x3, t0);
	let t2 = b24_lal128s_p(l, lid0, live, x0, x1b, x1i, 0u32);
	let t3 = add_r(t1, t2);
	x4 = t3;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t4 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, 4u32);
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vdupq_n_s32(((10u32) & 31) as i32)));
	let t6 = sub_r(t4, t5);
	let t7 = b25_ls_p(l, lid0, live, t6, 1024u32);
	x5 = t7;
	let t8 = b24_lal128s_p(l, lid0, live, x0, x1b, x1i, x2);
	let t9 = mul_r(x5, t8);
	let t10 = add_r(x4, t9);
	res = t10;
	return res;
}

/// `clif_f16_column`.
#[inline(always)]
unsafe fn b16_lal128r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut x4 = [Z; 8];
	let mut x5 = [Z; 8];
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 40u32);
	let t1 = mul_r(x3, t0);
	let t2 = b24_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 0u32);
	let t3 = add_r(t1, t2);
	x4 = t3;
	if ((x2) == (0u32)) {
		res = x4;
		return res;
	}
	let t4 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vdupq_n_s32(((10u32) & 31) as i32)));
	let t6 = sub_r(t4, t5);
	let t7 = b25_ls_p(l, lid0, live, t6, 1024u32);
	x5 = t7;
	let t8 = b24_lal128r0s_p(l, lid0, live, x0, arg0, x1i, x2);
	let t9 = mul_r(x5, t8);
	let t10 = add_r(x4, t9);
	res = t10;
	return res;
}

/// `clif_t_mask`.
#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_lssls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b26_lsls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_lal128lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b26_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_ld`.
#[inline(always)]
unsafe fn b18_lal128r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8], x4: u32) -> [uint32x4_t; 16] {
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b26_llls_p(l, lid0, live, x0, x2, x3, x4);
	res = t0;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_lssl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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
	let t0 = b23_lss_p(l, lid0, live, x0, x1, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b23_lss_p(l, lid0, live, x0, x1, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b18_lssls_p(l, lid0, live, x0, x1, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b18_lssls_p(l, lid0, live, x0, x1, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b23_lss_p(l, lid0, live, x0, x1, 4u32);
	x9 = ((t11).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
	let t12 = mul_r(x6, t3);
	let t13 = splat(x9);
	let t14 = sub_r(t13, t12);
	let t15 = b27_sl_p(l, lid0, live, 4u32, t14);
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
		let t29 = b28_ll_p(l, lid0, t23, t28, x8);
		let t30 = b18_lssls_p(l, lid0, t23, x0, x1, x4, t29, 2u32);
		let t31: [uint32x4_t; 8] = lo(t30);
		t21 = bsl_r(t23, t31, t21);
	}
	x12 = t21;
	let t32 = splat(2u32);
	let t33 = mul_r(t32, x11);
	let t34 = add_r(x7, t33);
	let t35 = b28_ll_p(l, lid0, live, t34, x8);
	let t36 = b18_lssls_p(l, lid0, live, x0, x1, x4, t35, 2u32);
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
		let t52 = b28_ll_p(l, lid0, t49, t51, x8);
		let t53 = b18_lssls_p(l, lid0, t49, x0, x1, x4, t52, 8u32);
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
		let t114 = b29_l_p(l, lid0, t49, t94);
		let t115 = add_r(x18, t114);
		x18 = bsl_r(t49, t115, x18);
		let t116 = and_r(t49, live);
		let t117 = add_r(x19, t92);
		x19 = t117;
	}
	x12 = x18;
	let t118 = b23_lss_p(l, lid0, live, x0, x1, 16u32);
	x24 = (((t118).wrapping_shr((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32));
	let t119 = splat(1u32);
	let t120 = sub_r(x10, t119);
	let t121 = mul_r(t32, t120);
	let t122 = add_r(x7, t121);
	let t123 = b28_ll_p(l, lid0, live, t122, x8);
	let t124 = b18_lssls_p(l, lid0, live, x0, x1, x4, t123, 2u32);
	let t125: [uint32x4_t; 8] = lo(t124);
	x25 = t125;
	let t126 = add_r(x14, x25);
	let t127 = splat(x24);
	let t128 = mul_r(x12, t127);
	let t129 = add_r(t126, t128);
	let t130 = b28_ll_p(l, lid0, live, t129, x8);
	x26 = t130;
	let t131 = b18_lssls_p(l, lid0, live, x0, x1, x4, x26, 8u32);
	let t132 = b17_s_p(l, lid0, live, x24);
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
unsafe fn b19_lal128sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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
	let t0 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, 4u32);
	let t12 = splat(255u32);
	let t13 = add_r(t11, t12);
	let t14: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t13[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	x9 = t14;
	let t15 = mul_r(x6, t3);
	let t16 = sub_r(x9, t15);
	let t17 = b27_sl_p(l, lid0, live, 4u32, t16);
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
		let t31 = b28_ll_p(l, lid0, t25, t30, x8);
		let t32 = b18_lal128lls_p(l, lid0, t25, x0, x1b, x1i, x4, t31, 2u32);
		let t33: [uint32x4_t; 8] = lo(t32);
		t23 = bsl_r(t25, t33, t23);
	}
	x12 = t23;
	let t34 = splat(2u32);
	let t35 = mul_r(t34, x11);
	let t36 = add_r(x7, t35);
	let t37 = b28_ll_p(l, lid0, live, t36, x8);
	let t38 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x4, t37, 2u32);
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
		let t53 = b28_ll_p(l, lid0, t50, t52, x8);
		let t54 = b18_lal128lls_p(l, lid0, t50, x0, x1b, x1i, x4, t53, 8u32);
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
		let t115 = b29_l_p(l, lid0, t50, t95);
		let t116 = add_r(x18, t115);
		x18 = bsl_r(t50, t116, x18);
		let t117 = and_r(t50, live);
		let t118 = add_r(x19, t93);
		x19 = t118;
	}
	x12 = x18;
	let t119 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, 16u32);
	let t120: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t119[r], vnegq_s32(vdupq_n_s32(((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) & 31) as i32))));
	let t121 = splat(15u32);
	let t122 = and_r(t120, t121);
	x24 = t122;
	let t123 = splat(1u32);
	let t124 = sub_r(x10, t123);
	let t125 = mul_r(t34, t124);
	let t126 = add_r(x7, t125);
	let t127 = b28_ll_p(l, lid0, live, t126, x8);
	let t128 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x4, t127, 2u32);
	let t129: [uint32x4_t; 8] = lo(t128);
	x25 = t129;
	let t130 = add_r(x14, x25);
	let t131 = mul_r(x12, x24);
	let t132 = add_r(t130, t131);
	let t133 = b28_ll_p(l, lid0, live, t132, x8);
	x26 = t133;
	let t134 = b18_lal128lls_p(l, lid0, live, x0, x1b, x1i, x4, x26, 8u32);
	let t135 = b17_l_p(l, lid0, live, x24);
	let t136: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[r], t135[r]));
	let t137: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[(r + 8)], t135[(r + 8)]));
	let t138 = cat_w(t136, t137);
	res = t138;
	return res;
}

/// `clif_f16_side`.
#[inline(always)]
unsafe fn b19_lal128r0sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32, x3: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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
	let t0 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, (44u32).wrapping_add((4u32).wrapping_mul(x2)));
	x4 = t0;
	let t1 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, (60u32).wrapping_add((4u32).wrapping_mul(x2)));
	x5 = t1;
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(x3[r], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))));
	x6 = t2;
	let t3 = splat(4u32);
	let t4 = mul_r(x6, t3);
	let t5 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t4, 4u32);
	let t6: [uint32x4_t; 8] = lo(t5);
	x7 = t6;
	let t7 = mul_r(x6, t3);
	let t8 = add_r(t7, t3);
	let t9 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x5, t8, 4u32);
	let t10: [uint32x4_t; 8] = lo(t9);
	x8 = t10;
	let t11 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 4u32);
	let t12 = splat(255u32);
	let t13 = add_r(t11, t12);
	let t14: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t13[r], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))));
	x9 = t14;
	let t15 = mul_r(x6, t3);
	let t16 = sub_r(x9, t15);
	let t17 = b27_sl_p(l, lid0, live, 4u32, t16);
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
		let t31 = b28_ll_p(l, lid0, t25, t30, x8);
		let t32 = b18_lal128r0lls_p(l, lid0, t25, x0, arg0, x1i, x4, t31, 2u32);
		let t33: [uint32x4_t; 8] = lo(t32);
		t23 = bsl_r(t25, t33, t23);
	}
	x12 = t23;
	let t34 = splat(2u32);
	let t35 = mul_r(t34, x11);
	let t36 = add_r(x7, t35);
	let t37 = b28_ll_p(l, lid0, live, t36, x8);
	let t38 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t37, 2u32);
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
		let t53 = b28_ll_p(l, lid0, t50, t52, x8);
		let t54 = b18_lal128r0lls_p(l, lid0, t50, x0, arg0, x1i, x4, t53, 8u32);
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
		let t115 = b29_l_p(l, lid0, t50, t95);
		let t116 = add_r(x18, t115);
		x18 = bsl_r(t50, t116, x18);
		let t117 = and_r(t50, live);
		let t118 = add_r(x19, t93);
		x19 = t118;
	}
	x12 = x18;
	let t119 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 16u32);
	let t120: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t119[r], vnegq_s32(vdupq_n_s32(((((((4u32).wrapping_mul(x2)).wrapping_add(16u32)) & (31u32))) & 31) as i32))));
	let t121 = splat(15u32);
	let t122 = and_r(t120, t121);
	x24 = t122;
	let t123 = splat(1u32);
	let t124 = sub_r(x10, t123);
	let t125 = mul_r(t34, t124);
	let t126 = add_r(x7, t125);
	let t127 = b28_ll_p(l, lid0, live, t126, x8);
	let t128 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x4, t127, 2u32);
	let t129: [uint32x4_t; 8] = lo(t128);
	x25 = t129;
	let t130 = add_r(x14, x25);
	let t131 = mul_r(x12, x24);
	let t132 = add_r(t130, t131);
	let t133 = b28_ll_p(l, lid0, live, t132, x8);
	x26 = t133;
	let t134 = b18_lal128r0lls_p(l, lid0, live, x0, arg0, x1i, x4, x26, 8u32);
	let t135 = b17_l_p(l, lid0, live, x24);
	let t136: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[r], t135[r]));
	let t137: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t134[(r + 8)], t135[(r + 8)]));
	let t138 = cat_w(t136, t137);
	res = t138;
	return res;
}

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b20_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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
		let t6 = b10_lal128l_p(l, lid0, live, x0, x1b, x1i, x3);
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

/// `clif_f16_parent_step`.
#[inline(always)]
unsafe fn b20_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 16] {
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
		let t6 = b10_lal128r0l_p(l, lid0, live, x0, arg0, x1i, x3);
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

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b21_lal128ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = add_r(x2, x3);
	let t1 = splat(1u32);
	let t2 = add_r(x2, t1);
	let t3 = b30_ll_p(l, lid0, live, t0, t2);
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
		let t8 = ld_u32((x1b).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t7, 128);
		t5 = t8;
	}
	let t9 = b28_ll_p(l, lid0, live, t3, t5);
	let t10 = add_r(x2, t1);
	let t11 = b30_ll_p(l, lid0, live, t9, t10);
	res = t11;
	return res;
}

/// `clif_t_hop`.
#[inline(always)]
unsafe fn b21_lal128r0ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = add_r(x2, x3);
	let t1 = splat(1u32);
	let t2 = add_r(x2, t1);
	let t3 = b30_ll_p(l, lid0, live, t0, t2);
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
		let t8 = ld_u32((arg0).wrapping_add((((((4u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t7, 128);
		t5 = t8;
	}
	let t9 = b28_ll_p(l, lid0, live, t3, t5);
	let t10 = add_r(x2, t1);
	let t11 = b30_ll_p(l, lid0, live, t9, t10);
	res = t11;
	return res;
}

/// `clif_f15_size`.
#[inline(always)]
unsafe fn b22_lal128l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b33_lal128s_p(l, lid0, live, x0, x1b, x1i, 1u32);
	let t1 = b34_lal128s_p(l, lid0, live, x0, x1b, x1i, 0u32);
	let t2 = b32_llll_p(l, lid0, live, x0, t0, t1, x2);
	let t3 = b31_ls_p(l, lid0, live, t2, 1u32);
	res = t3;
	return res;
}

/// `clif_f15_size`.
#[inline(always)]
unsafe fn b22_lal128r0l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b33_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 1u32);
	let t1 = b34_lal128r0s_p(l, lid0, live, x0, arg0, x1i, 0u32);
	let t2 = b32_llll_p(l, lid0, live, x0, t0, t1, x2);
	let t3 = b31_ls_p(l, lid0, live, t2, 1u32);
	res = t3;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b23_lss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: *const u8, x2: u32) -> u32 {
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
unsafe fn b23_lal128s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
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
		let t4 = ld_u32((x1b).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	res = t1;
	return res;
}

/// `clif_f16_view`.
#[inline(always)]
unsafe fn b23_lal128r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
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
		let t4 = ld_u32((arg0).wrapping_add((((((x2).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	res = t1;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b24_lal128s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128s_p(l, lid0, live, x0, x1b, x1i, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `clif_f16_columns`.
#[inline(always)]
unsafe fn b24_lal128r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = b23_lal128r0s_p(l, lid0, live, x0, arg0, x1i, (20u32).wrapping_add((4u32).wrapping_mul(x2)));
	res = t0;
	return res;
}

/// `ukl_min`.
#[inline(always)]
unsafe fn b25_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
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

/// `clif_t_ld`.
#[inline(always)]
unsafe fn b26_lsls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: u32, x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = b35_lal1s_p(l, lid0, live, x0, ((((x1) as u64)) as usize as *const u8), x2, x3);
	x4 = t0;
	let t1: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t1 = x4;
	} else {
		let t2 = b36_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
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
unsafe fn b26_llls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: u32) -> [uint32x4_t; 16] {
	let mut x4 = [Z; 16];
	let mut live = k;
	let mut res = [Z; 16];
	let t0 = add_r(x1, x2);
	let t1 = clt_r(t0, x1);
	let t2: [uint32x4_t; 8] = core::array::from_fn(|r| vshrq_n_u32::<31>(t1[r]));
	let t3 = cat_w(lo(t0), t2);
	let t4 = b35_lls_p(l, lid0, live, x0, t3, x3);
	x4 = t4;
	let t5: [uint32x4_t; 16];
	if ((x3) >= (8u32)) {
		t5 = x4;
	} else {
		let t6 = b36_s_p(l, lid0, live, (8u32).wrapping_mul(x3));
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

/// `ukl_min`.
#[inline(always)]
unsafe fn b27_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32, x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
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

/// `ukl_min`.
#[inline(always)]
unsafe fn b28_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
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

/// `ukl_popcount64`.
#[inline(always)]
unsafe fn b29_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[r])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[(r + 8)]))))));
	res = t0;
	return res;
}

/// `ukl_max`.
#[inline(always)]
unsafe fn b30_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
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

/// `ukl_max`.
#[inline(always)]
unsafe fn b31_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 8], x1: u32) -> [uint32x4_t; 8] {
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

/// `clif_f15_cell`.
#[inline(always)]
unsafe fn b32_llll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 8], x2: [uint32x4_t; 8], x3: [uint32x4_t; 8]) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let t0 = splat(2u32);
	let t1 = ceq_r(x2, t0);
	let mut t2 = [Z; 8];
	let t3 = and_r(live, t1);
	let t4 = bic_r(live, t1);
	if any(&t3) {
		let t5 = mul_r(x3, t0);
		let t6 = b26_llls_p(l, lid0, t3, x0, x1, t5, 2u32);
		let t7: [uint32x4_t; 8] = lo(t6);
		t2 = bsl_r(t3, t7, t2);
	}
	if any(&t4) {
		let t8 = splat(4u32);
		let t9 = mul_r(x3, t8);
		let t10 = b26_llls_p(l, lid0, t4, x0, x1, t9, 4u32);
		let t11: [uint32x4_t; 8] = lo(t10);
		t2 = bsl_r(t4, t11, t2);
	}
	res = t2;
	return res;
}

/// `clif_f15_column`.
#[inline(always)]
unsafe fn b33_lal128s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = (((20u32).wrapping_add((4u32).wrapping_mul(x2))) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((x1b).wrapping_add(((((((20u32).wrapping_add((4u32).wrapping_mul(x2))).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	res = t1;
	return res;
}

/// `clif_f15_column`.
#[inline(always)]
unsafe fn b33_lal128r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = (((20u32).wrapping_add((4u32).wrapping_mul(x2))) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((arg0).wrapping_add(((((((20u32).wrapping_add((4u32).wrapping_mul(x2))).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	res = t1;
	return res;
}

/// `clif_f15_width`.
#[inline(always)]
unsafe fn b34_lal128s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((16u32) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((x1b).wrapping_add((((((16u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t1[r], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))));
	let t6 = splat(15u32);
	let t7 = and_r(t5, t6);
	res = t7;
	return res;
}

/// `clif_f15_width`.
#[inline(always)]
unsafe fn b34_lal128r0s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], arg0: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 8] {
	let mut live = k;
	let mut res = [Z; 8];
	let mut t0: bool = ((16u32) != (0u32));
	if t0 {
		t0 = ((16u32) == (12u32));
	}
	let t1: [uint32x4_t; 8];
	if t0 {
		let t2 = splat(16u32);
		t1 = t2;
	} else {
		let t3 = ix_on(live, x1i);
		let t4 = ld_u32((arg0).wrapping_add((((((16u32).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64)) as usize) as *const u8, t3, 128);
		t1 = t4;
	}
	let t5: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t1[r], vnegq_s32(vdupq_n_s32((((4u32).wrapping_mul(x2)) & 31) as i32))));
	let t6 = splat(15u32);
	let t7 = and_r(t5, t6);
	res = t7;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b35_lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1: [uint32x4_t; 16], x2: u32) -> [uint32x4_t; 16] {
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
	let t35: u64 = !(7u64);
	let t36 = splat((t35) as u32);
	let t37 = splat(((t35) >> 32) as u32);
	let t38 = and_r(lo(x1), t36);
	let t39 = and_r(hi(x1), t37);
	let t40 = cat_w(t38, t39);
	let t41: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t40[r]));
	let t42 = clt_r(t41, lo(x0));
	let t43 = adc_r(hi(x0), hi(t40), t42);
	let t44 = cat_w(lo(t41), t43);
	x3 = t44;
	let t45: [uint32x4_t; 8] = lo(x3);
	let t46: [uint32x4_t; 8] = hi(x3);
	let t47 = addrs(t45, t46, live);
	let t48: u64 = 7u64;
	let t49 = splat((t48) as u32);
	let t50 = and_r(lo(x1), t49);
	let t51 = splat(8u32);
	let t52 = mul_r(t50, t51);
	x4 = t52;
	let mut t53 = [Z; 16];
	t53 = ldj_u64(|j| (t47[j] as *const u8) as *const u8);
	let t54: [uint32x4_t; 8] = lo(x4);
	let t55: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t54[r], vdupq_n_u32(63)));
	let t56: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t53[r], vnegq_s32(vreinterpretq_s32_u32(t55[r]))), vshlq_u32(t53[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t55[r])))));
	let t57: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t53[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t55[r]))));
	let t58 = cat_w(t56, t57);
	x5 = t58;
	let t59 = [Z; 8];
	let t60 = cne_r(x4, t59);
	let t61 = splat((x2).wrapping_mul(8u32));
	let t62 = add_r(x4, t61);
	let t63 = splat(64u32);
	let t64 = cgt_r(t62, t63);
	let t65 = and_r(t60, t64);
	let mut t66 = [Z; 16];
	let t67 = and_r(live, t65);
	let t68 = bic_r(live, t65);
	{
		let t69: u64 = (1u64).wrapping_mul(8u64);
		let t70 = splat((t69) as u32);
		let t71 = splat(((t69) >> 32) as u32);
		let t72 = add_r(lo(x3), t70);
		let t73 = clt_r(t72, lo(x3));
		let t74 = adc_r(hi(x3), t71, t73);
		let t75 = cat_w(lo(t72), t74);
		let t76: [usize; 32] = core::array::from_fn(|j| t47[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t77 = [Z; 16];
		t77 = ldj_u64(|j| (t76[j] as *const u8) as *const u8);
		let t78 = sub_r(t63, x4);
		let t79: [uint32x4_t; 8] = lo(t78);
		let t80: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t79[r], vdupq_n_u32(63)));
		let t81: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t77[r], vreinterpretq_s32_u32(t80[r])));
		let t82: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t77[(r + 8)], vreinterpretq_s32_u32(t80[r])), vshlq_u32(t77[r], vsubq_s32(vreinterpretq_s32_u32(t80[r]), vdupq_n_s32(32)))));
		let t83 = cat_w(t81, t82);
		let t84: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t83[r]));
		let t85: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t83[(r + 8)]));
		let t86 = cat_w(t84, t85);
		t66 = bsl_w(t67, t86, t66);
	}
	{
		t66 = bsl_w(t68, x5, t66);
	}
	res = t66;
	return res;
}

/// `ukl_ld_window`.
#[inline(always)]
unsafe fn b35_lal1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: [uint32x4_t; 16], x1b: *const u8, x1i: [uint32x4_t; 8], x2: u32) -> [uint32x4_t; 16] {
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
	let t75: u64 = !(7u64);
	let t76 = splat((t75) as u32);
	let t77 = splat(((t75) >> 32) as u32);
	let t78 = and_r(lo(t74), t76);
	let t79 = and_r(hi(t74), t77);
	let t80 = cat_w(t78, t79);
	let t81: [uint32x4_t; 8] = core::array::from_fn(|r| vaddq_u32(x0[r], t80[r]));
	let t82 = clt_r(t81, lo(x0));
	let t83 = adc_r(hi(x0), hi(t80), t82);
	let t84 = cat_w(lo(t81), t83);
	x3 = t84;
	let t85: [uint32x4_t; 8] = lo(x3);
	let t86: [uint32x4_t; 8] = hi(x3);
	let t87 = addrs(t85, t86, live);
	let t88: [uint32x4_t; 8] = lo(x1i);
	let t89: u64 = ((x1b) as usize as u64);
	let t90 = splat((t89) as u32);
	let t91 = splat(((t89) >> 32) as u32);
	let t92 = add_r(t90, t88);
	let t93 = clt_r(t92, t90);
	let t94: [uint32x4_t; 8] = core::array::from_fn(|r| vadcq_u32(t91[r], Z, t93[r]));
	let t95 = cat_w(lo(t92), t94);
	let t96: u64 = 7u64;
	let t97 = splat((t96) as u32);
	let t98 = and_r(lo(t95), t97);
	let t99 = splat(8u32);
	let t100 = mul_r(t98, t99);
	x4 = t100;
	let mut t101 = [Z; 16];
	t101 = ldj_u64(|j| (t87[j] as *const u8) as *const u8);
	let t102: [uint32x4_t; 8] = lo(x4);
	let t103: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t102[r], vdupq_n_u32(63)));
	let t104: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t101[r], vnegq_s32(vreinterpretq_s32_u32(t103[r]))), vshlq_u32(t101[(r + 8)], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t103[r])))));
	let t105: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t101[(r + 8)], vnegq_s32(vreinterpretq_s32_u32(t103[r]))));
	let t106 = cat_w(t104, t105);
	x5 = t106;
	let t107 = [Z; 8];
	let t108 = cne_r(x4, t107);
	let t109 = splat((x2).wrapping_mul(8u32));
	let t110 = add_r(x4, t109);
	let t111 = splat(64u32);
	let t112 = cgt_r(t110, t111);
	let t113 = and_r(t108, t112);
	let mut t114 = [Z; 16];
	let t115 = and_r(live, t113);
	let t116 = bic_r(live, t113);
	{
		let t117: u64 = (1u64).wrapping_mul(8u64);
		let t118 = splat((t117) as u32);
		let t119 = splat(((t117) >> 32) as u32);
		let t120 = add_r(lo(x3), t118);
		let t121 = clt_r(t120, lo(x3));
		let t122 = adc_r(hi(x3), t119, t121);
		let t123 = cat_w(lo(t120), t122);
		let t124: [usize; 32] = core::array::from_fn(|j| t87[j].wrapping_add(((1u64).wrapping_mul(8u64)) as u64 as usize));
		let mut t125 = [Z; 16];
		t125 = ldj_u64(|j| (t124[j] as *const u8) as *const u8);
		let t126 = sub_r(t111, x4);
		let t127: [uint32x4_t; 8] = lo(t126);
		let t128: [uint32x4_t; 8] = core::array::from_fn(|r| vandq_u32(t127[r], vdupq_n_u32(63)));
		let t129: [uint32x4_t; 8] = core::array::from_fn(|r| vshlq_u32(t125[r], vreinterpretq_s32_u32(t128[r])));
		let t130: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(vshlq_u32(t125[(r + 8)], vreinterpretq_s32_u32(t128[r])), vshlq_u32(t125[r], vsubq_s32(vreinterpretq_s32_u32(t128[r]), vdupq_n_s32(32)))));
		let t131 = cat_w(t129, t130);
		let t132: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[r], t131[r]));
		let t133: [uint32x4_t; 8] = core::array::from_fn(|r| vorrq_u32(x5[(r + 8)], t131[(r + 8)]));
		let t134 = cat_w(t132, t133);
		t114 = bsl_w(t115, t134, t114);
	}
	{
		t114 = bsl_w(t116, x5, t114);
	}
	res = t114;
	return res;
}

/// `ukl_mask64`.
#[inline(always)]
unsafe fn b36_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 8], x0: u32) -> u64 {
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
pub unsafe fn label_parents_w32(argv: *const *const u8, l: &Launch) {
	let n = l.block[0] * l.block[1] * l.block[2];
	let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
	let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
	let x2: *const u8 = *argv.add(2);
	let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
	let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
	let x5: u32 = (*argv.add(5) as *const u32).read_unaligned();
	let mut tg = [0u64; 1153];
	let arg6: *const u8 = tg.as_mut_ptr() as *const u8;
	let mut t1595: bool = !(arg4).is_null();
	if t1595 {
		let mut t1596: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x5) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		if !t1596 {
			t1596 = ((l.group[1]) >= ((arg4.wrapping_add(((((x5).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		let mut t1597: bool = t1596;
		if !t1597 {
			t1597 = ((l.group[2]) >= ((arg4.wrapping_add(((((x5).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
		}
		t1595 = t1597;
	}
	if t1595 {
		return;
	}
	// Phases: each chunk's, then the next phase's (its barriers).
	let mut state: Vec<u8> = Vec::with_capacity(n.div_ceil(32) as usize * 7680);
	let st: *mut u8 = state.as_mut_ptr();
	{
		let phase = 0u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1598: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1598;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 1u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1599: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1599;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 2u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1600: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1600;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 3u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1601: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1601;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 4u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1602: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1602;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 5u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1603: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1603;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
	{
		let phase = 6u32;
		let mut lid0 = 0u32;
		while lid0 + 32 <= n {
			let mut live = splat(u32::MAX);
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_f(l, lid0, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
			lid0 += 32;
		}
		if lid0 < n {
			// Lanes part: the group's last threads, those under `n`.
			let t1604: [uint32x4_t; 8] = core::array::from_fn(|r| vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[r].as_ptr())), vdupq_n_u32(n)));
			let mut live = t1604;
			let gid0 = l.group[0] * l.block[0] + lid0;
			'chunk: {
				b0_r0r1sr3r6o6gsi_p(l, lid0, live, arg0, arg1, x2, arg3, arg6, arg6.wrapping_add(8192), l.group[0], lid0, phase, st.add((lid0 / 32) as usize * 7680));
			}
		}
	}
}
