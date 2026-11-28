//! Bitlex's tiers printed as UKL (decision 0726), on the process's device: each pass's kernels compute what the tier's
//! Rust computes, chunk by chunk.

use crate::kernels::{alloc, buffer_of, launch, program, read};
use crate::Pod;

#[repr(C)]
#[derive(Clone, Copy)]
struct T1Args {
    n: u32,
    blocks: u32,
    chunks: u32,
    write: u32,
}
unsafe impl Pod for T1Args {}

/// JSON files for tier 1: lists of objects (several chunks), strings with escapes and multibyte text, a byte-order
/// mark, and the refusals (a control byte in a string, bad UTF-8, a string open at the end).
fn files() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Vec::new();
    let mut f = b"[".to_vec();
    for i in 0..400 {
        f.extend_from_slice(format!("{}{{\"id\": {i}, \"s\": \"a\\\"b\\\\c é中 {}\", \"v\": [1.5e3, true, null, -7]}}", if i == 0 { "" } else { ",\n" }, "x".repeat(i % 70)).as_bytes());
    }
    f.push(b']');
    v.push(f);
    v.push(b"\xef\xbb\xbf{\"a\": [1, 2, {\"b\": \"\\\\\\\\\\\"\"}]}".to_vec());
    v.push(b"[\"a\x01b\"]".to_vec());
    v.push(b"[\"\xc3\x28\"]".to_vec());
    v.push(b"[\"open".to_vec());
    v.push(b"{}".to_vec());
    v.push(format!("[{}]", (0..3000).map(|i| format!("\"{}\"", "\\\\".repeat(i % 5))).collect::<Vec<_>>().join(",")).into_bytes());
    v
}

/// Tier 1's passes as UKL (`json_t1_ends`, `json_t1_from`) on the device are its Rust's, chunk by chunk: each chunk's
/// end from each lane's entry, its records' and each type's counts and its verdict from its entry, then its records and
/// each type's records' numbers written at their places, which are `lex_typed`'s.
#[test]
fn json_tier1_as_ukl_is_its_rust() {
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 1 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let lanes = rerail_json_aot::LANES;
    let types = rerail_json_aot::TYPES;
    let (mut chunks_run, mut records) = (0usize, 0usize);
    for (k, f) in files().iter().enumerate() {
        let buf = rerail_json_aot::padded(f);
        let n = f.len();
        let off = if f.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
        let nw = (n - off).div_ceil(64);
        for blocks in [1usize, 3] {
            let chunks = nw.div_ceil(blocks).max(1);
            let args = |write: u32| T1Args { n: n as u32, blocks: blocks as u32, chunks: chunks as u32, write };
            // Pass 1.
            let want: Vec<[rerail_json_aot::Carries; 4]> = (0..chunks).map(|c| rerail_json_aot::stage1_ends_best(&buf, n, c * blocks, (c + 1) * blocks)).collect();
            let sb = buffer_of(&buf).unwrap();
            let eb = alloc::<u64>(chunks * lanes * 2).unwrap();
            launch(&q, p.function("json_t1_ends").unwrap()).value("args", &args(0)).buffer("src", &sb).buffer("ends", &eb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let got: Vec<u64> = read(&eb, chunks * lanes * 2);
            for (c, w) in want.iter().enumerate() {
                for (l, e) in w.iter().enumerate() {
                    let at = (c * lanes + l) * 2;
                    assert_eq!((got[at], got[at + 1]), (e.m_escaped, e.m_in_string), "{}: file {k}, chunks of {blocks}: chunk {c}, lane {l}'s end", crate::device::backend());
                }
            }
            // Pass 2, counting, from pass 3's entries.
            let ent = rerail_json_aot::entries(&want);
            let words: Vec<u64> = ent.iter().flat_map(|e| [e.m_escaped, e.m_in_string]).collect();
            let entb = buffer_of(&words).unwrap();
            let atb = alloc::<u32>(chunks * (types + 1)).unwrap();
            let (stb, tyb, rb) = (alloc::<u32>(1).unwrap(), alloc::<u32>(1).unwrap(), alloc::<u32>(chunks).unwrap());
            launch(&q, p.function("json_t1_from").unwrap()).value("args", &args(0)).buffer("src", &sb).buffer("entries", &entb).buffer("at", &atb).buffer("starts", &stb).buffer("types", &tyb).buffer("refused", &rb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let counts: Vec<u32> = read(&atb, chunks * (types + 1));
            let refused: Vec<u32> = read(&rb, chunks);
            let mut rust_refused = false;
            for c in 0..chunks {
                let mut at = [0usize; 4];
                // Counting writes nothing.
                let (r, _) = rerail_json_aot::stage1_from_best(&buf, n, c * blocks, (c + 1) * blocks, ent[c], &mut at, std::ptr::null_mut(), [std::ptr::null_mut(); 3], false);
                rust_refused |= r;
                let have: Vec<usize> = counts[c * (types + 1)..(c + 1) * (types + 1)].iter().map(|&x| x as usize).collect();
                assert_eq!(have, at.to_vec(), "{}: file {k}, chunks of {blocks}: chunk {c}'s counts", crate::device::backend());
                assert_eq!(refused[c] != 0, r, "{}: file {k}, chunks of {blocks}: chunk {c}'s verdict", crate::device::backend());
            }
            chunks_run += chunks;
            if rust_refused {
                continue;
            }
            // Pass 2, writing, at the places an exclusive scan of the counts gives: the records, then each type's list
            // after the types before it.
            let total: Vec<usize> = (0..=types).map(|t| (0..chunks).map(|c| counts[c * (types + 1) + t] as usize).sum()).collect();
            let mut places = vec![0u32; chunks * (types + 1)];
            let mut run = vec![0usize; types + 1];
            for t in 1..=types {
                run[t] = total[1..t].iter().sum();
            }
            for c in 0..chunks {
                for t in 0..=types {
                    places[c * (types + 1) + t] = run[t] as u32;
                    run[t] += counts[c * (types + 1) + t] as usize;
                }
            }
            let atb = buffer_of(&places).unwrap();
            let stb = alloc::<u32>(total[0].max(1)).unwrap();
            let tyb = alloc::<u32>(total[1..].iter().sum::<usize>().max(1)).unwrap();
            launch(&q, p.function("json_t1_from").unwrap()).value("args", &args(1)).buffer("src", &sb).buffer("entries", &entb).buffer("at", &atb).buffer("starts", &stb).buffer("types", &tyb).buffer("refused", &rb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let starts: Vec<u32> = read(&stb, total[0]);
            let lists: Vec<u32> = read(&tyb, total[1..].iter().sum());
            let typed = rerail_json_aot::lex_typed(&buf, n, 16);
            assert!(starts[..] == typed.lexed.starts[..total[0]], "{}: file {k}, chunks of {blocks}: the records", crate::device::backend());
            let mut at = 0;
            for t in 0..types {
                assert!(lists[at..at + total[1 + t]] == typed.types[t][..], "{}: file {k}, chunks of {blocks}: type {t}'s records", crate::device::backend());
                at += total[1 + t];
            }
            records += total[0];
        }
    }
    eprintln!("{}: tier 1 as UKL, {chunks_run} chunks in chunks of 1 and 3 blocks: each chunk's ends, counts and verdict its Rust's; {records} records and their types written as lex_typed's", crate::device::backend());
}
