//! Row programs (decision 0095b §2): block programs whose stream is a run's rows, written in
//! `rir`, checked on the CPU interpreter and Cranelift against a row-by-row reference that exists
//! only here.

use super::*;
use super::block::{CmpOp, RowBatch, lane};

/// A transducer program whose walk does nothing and whose block program is `rows`.
fn program(rows: &str) -> Program {
    let text = format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n");
    rir::assemble(&text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

/// A row program over three columns, a parameter and a set: `(c0 < c1 at 16 bits, or c2 = p0)`
/// and not in s0, and `c0 <= p0` alone.
const FILTER: &str = "  block 0 rows 3 1 1
  block 0 %0 = cmp lt c0 c1 16
  block 0 %1 = cmp eq c2 p0 64
  block 0 %2 = or %0, %1
  block 0 %3 = bit s0 c0
  block 0 %4 = andnot %2, %3
  block 0 %5 = cmp le c0 p0 32
  block 0 output %4
  block 0 output %5
  block 0 extract %4
";

/// Values spread over 16 and 32 bits and their signs, and negative ones (a set reads none).
fn column(seed: u64, len: usize) -> Vec<i64> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            match x % 4 {
                0 => (x % 7) as i64 - 3,
                1 => (x % 200) as i64,
                2 => (x as i64) >> 30,
                _ => 65_530 + (x % 12) as i64,
            }
        })
        .collect()
}

/// The reference: each row's bit, row by row.
fn reference(c: &[Vec<i64>], p: i64, set: &[u64], len: usize) -> [Vec<bool>; 2] {
    let member = |v: i64| usize::try_from(v).ok().and_then(|v| set.get(v / 64).map(|w| w >> (v % 64) & 1 == 1)).unwrap_or(false);
    let first = (0..len).map(|r| (CmpOp::Lt.holds(lane(c[0][r], 16), lane(c[1][r], 16)) || c[2][r] == p) && !member(c[0][r])).collect();
    let second = (0..len).map(|r| lane(c[0][r], 32) <= lane(p, 32)).collect();
    [first, second]
}

fn bits(words: &[u64], len: usize) -> Vec<bool> {
    (0..len).map(|r| words[r / 64] >> (r % 64) & 1 == 1).collect()
}

/// The interpreter and Cranelift give the reference's bits, at lengths around every word edge;
/// the extract pass gives the rows the first output sets.
#[test]
fn a_row_program_filters_rows_on_every_cpu_executor() {
    let p = program(FILTER);
    let block = &p.block_programs()[0];
    block.verify().expect("a valid row program");
    let jit = block.compile_rows().expect("compiles");
    assert!(!jit.clif.contains("call"), "no call in a row program:\n{}", jit.clif);
    let set: Vec<u64> = (0..3).map(|k| 0x8421_0842_1084_2108u64.rotate_left(k * 7)).collect();
    let mut values = Vec::new();
    let mut jit_out = Vec::new();
    for (seed, len) in [0usize, 1, 2, 63, 64, 65, 127, 128, 129, 300].into_iter().enumerate() {
        let cols: Vec<Vec<i64>> = (0..3).map(|k| column((seed * 3 + k) as u64, len)).collect();
        for param in [-1i64, 0, 5, 65_535, 1 << 33] {
            let refs: Vec<&[i64]> = cols.iter().map(Vec::as_slice).collect();
            let batch = RowBatch { columns: &refs, params: &[param], sets: &[&set], len };
            let want = reference(&cols, param, &set, len);
            let words = len.div_ceil(64);
            let got: Vec<Vec<u64>> = block.interpret_rows(&batch, &mut values).map(<[u64]>::to_vec).collect();
            for k in 0..2 {
                assert_eq!(bits(&got[k], len), want[k], "interpreter, output {k}, len {len}, p {param}");
                assert!(got[k].iter().enumerate().all(|(w, &x)| 64 * w + 64 <= len || x >> (len - 64 * w) == 0 || len % 64 == 0), "bits past the batch");
            }
            jit.run(&batch, &mut jit_out);
            for k in 0..2 {
                assert_eq!(&jit_out[k * words..(k + 1) * words], got[k].as_slice(), "Cranelift, output {k}, len {len}, p {param}");
            }
            let mut rows = Vec::new();
            block.extract_rows(&batch, 0, &mut values, &mut rows);
            let want_rows: Vec<u32> = (0..len).filter(|&r| want[0][r]).map(|r| r as u32).collect();
            assert_eq!(rows, want_rows);
        }
    }
}

/// A row program round-trips through `rir`, and its digest covers its shape; the verifier
/// refuses a byte operation in a row program, a row operation in a byte program, and operands
/// past the shape.
#[test]
fn row_programs_are_verified_and_digested() {
    let p = program(FILTER);
    let again = rir::assemble(&rir::disassemble(&p, None)).expect("round-trips");
    assert_eq!(again.block_programs(), p.block_programs());
    let wider = program(&FILTER.replace("rows 3 1 1", "rows 4 1 1"));
    assert_ne!(wider.block_programs()[0].canonical_bytes(), p.block_programs()[0].canonical_bytes());
    for bad in [
        FILTER.replace("cmp eq c2 p0 64", "eq 0x41"),
        FILTER.replace("cmp eq c2 p0 64", "cmp eq c3 p0 64"),
        FILTER.replace("cmp eq c2 p0 64", "cmp eq c2 p1 64"),
        FILTER.replace("cmp eq c2 p0 64", "cmp eq c2 p0 8"),
        FILTER.replace("bit s0 c0", "bit s1 c0"),
        FILTER.replace("  block 0 rows 3 1 1\n", ""),
    ] {
        let text = format!("program -\n  mode set=0 query=0\n{bad}  0000 STOP 0\n");
        let refused = match rir::assemble(&text) {
            Err(_) => true,
            Ok(p) => p.block_programs()[0].verify().is_err(),
        };
        assert!(refused, "refused:\n{bad}");
    }
}

/// On Metal, over the 16-bit layout (`metal_rows`): node columns in a few documents (each a chunk
/// or more), a number column, comparisons of two node columns of a row's chunk, a node column
/// against parameters in and out of its chunk, and documents: the interpreter's rows.
#[cfg(target_os = "macos")]
#[test]
fn a_row_program_scans_rows_on_metal_in_the_16_bit_layout() {
    use super::metal_rows::{Form, Input, Resident, scan};
    let text = "  block 0 rows 4 3 0
  block 0 %0 = cmp lt c0 c1 64
  block 0 %1 = cmp le c0 p0 64
  block 0 %2 = cmp eq c3 p1 64
  block 0 %3 = not %2
  block 0 %4 = and %0, %1
  block 0 %5 = or %4, %3
  block 0 %6 = cmp ne c2 p2 64
  block 0 %7 = and %5, %6
  block 0 output %7
  block 0 extract %7
";
    let p = program(text);
    let block = &p.block_programs()[0];
    let node = |d: i64, i: i64| (d << 32) | i;
    let len = 5000;
    let x = column(7, len);
    let a: Vec<i64> = (0..len as i64).map(|r| node(1 + r % 3, (r * 37) % 70_000)).collect();
    // Column 1: the same chunk as column 0's mostly, so it is `Low16` only when every row is.
    let b: Vec<i64> = a.iter().zip(&x).map(|(&v, &y)| (v & !0xffff) | (y & 0xffff)).collect();
    let n: Vec<i64> = x.iter().map(|v| v % 9).collect();
    for params in [[node(2, 30_000), node(2, 0) >> 32, 4], [node(3, 69_999), 1, 0], [-5, 3, 8]] {
        // Program column 3 is column 0's document.
        let cols: Vec<&[i64]> = vec![&a, &b, &n];
        let docs: Vec<i64> = a.iter().map(|v| v >> 32).collect();
        let all: Vec<&[i64]> = vec![&a, &b, &n, &docs];
        let batch = RowBatch { columns: &all, params: &params, sets: &[], len };
        let mut values = Vec::new();
        let mut want = Vec::new();
        block.extract_rows(&batch, 0, &mut values, &mut want);
        let resident = Resident::new(&cols, len).expect("resident");
        assert_eq!(resident.forms, vec![Form::Low16, Form::Low16, Form::I32]);
        let inputs = [Input::Col(0), Input::Col(1), Input::Col(2), Input::Doc(0)];
        let mut got = scan(block, &inputs, &resident, &params).expect("runs").rows;
        got.sort_unstable();
        assert_eq!(got, want, "params {params:?}");
    }
}
