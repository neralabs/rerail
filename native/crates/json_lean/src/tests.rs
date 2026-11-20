//! The translation gives the C++ kernel's answers: stage 1's index and refusal, and the tape, byte
//! for byte. Over documents made to reach each node and check (those `rerail-simdjson`'s kernel
//! tests use), lengths over every block edge, and JSON's corpus and test suite where
//! `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE` name them.

use super::*;
use rerail_simdjson_native::Kernel;

const KINDS: Kinds = Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };

fn made() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = [
        &b"1"[..], b" -1.5e3 ", b"true", b"false", b"null", b"\"\"", b"\"a\\nb\"", b"\xef\xbb\xbf{\"a\":1}", b"{}", b"[]", b" [ ] ",
        br#"{"a": [1, true, "x\"y"], "b": null, "c": {}, "d": [], "e": {"f": [[], {}]}}"#,
        br#"["\u00e9", "\ud83d\ude00", "\\\\", "a\/b", "\b\f\n\r\t", "\u0041\u0042x"]"#,
        br#"{"k\"ey": "v\\", "": ""}"#,
        b"", b"{", b"[1,]", b"\"\\x\"", b"01", b"[\"\xff\"]", b"123456789012345678901234567890", br#"["\ud83d"]"#, br#"["\ude00"]"#,
        br#"["\ud83d\u0041"]"#, br#"["\u12g4"]"#, b"[tru]", b"[nul, 1]", b"{\"a\" 1}", b"[1 2]", b"1 2", b"[1.]", b"[-]",
        b"{\"a\":1,}", b"{\"a\"}", b"{\"a\":}", b"[1]]", b"{} {}", b"{\"a\":1 \"b\":2}", b"   ", b"[\"\\u", b"{1:2}", b"[,1]",
        b"[1,,2]", b"{\"a\":1]", b"[{]", b"[[]", b"{\"a\":[}]", b"[true false]", b"[nulll]", b"[-0, -0.0e-0, 1E+2, 0.5]", b"[\"a\"x]",
        b"  {  \"a\"  :  [ 1 , { } , [ ] , \"\" ]  }  ", b"\"x\" ", b"7 ", b" 7", b"-", b"[01]", b"[1e]", b"[18446744073709551615, -9223372036854775808]",
        b"[18446744073709551616]", b"[-9223372036854775809]", b"[1e400]", b"[1e308, 9.9e307, 1.7976931348623157e308, 1.8e308, 4.9e-324, 1e-400]",
        b"[0.1234567890123456789012345, 12345678901234567890.5, 123.456e-12, 1.00000000000000011102230246251565404236316680908203125]",
        b"[\"\x01\"]", b"[\"\xe2\x82\xac\", \"\xf0\x9f\x98\x80\", \"\xed\xa0\x80\", \"\xc0\xaf\", \"\xe2\x82\"]", b"\xef\xbb\xbf", b"\xef\xbb\xbf1",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect();
    let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n)).into_bytes();
    out.extend([1023, 1024, 1025].map(deep));
    for n in [0, 1, 15, 16, 30, 31, 32, 33, 63, 64, 65, 100] {
        for esc in [&b""[..], b"\\n", b"\\u00e9", b"\\\\"] {
            let mut s = b"[\"".to_vec();
            s.extend(std::iter::repeat_n(b'x', n));
            s.extend_from_slice(esc);
            s.extend(std::iter::repeat_n(b'y', n / 2));
            s.extend_from_slice(b"\", 1]");
            out.push(s);
        }
    }
    // Every length over the block edges, each kind of byte at the edge.
    for n in 0..200 {
        for fill in [&b" "[..], b"1,", b"\"ab\",", b"\"\\\"\",", b"\"\xc3\xa9\","] {
            let body: Vec<u8> = fill.iter().copied().cycle().take(n).collect();
            out.push([b"[".as_slice(), &body, b"0]"].concat());
        }
    }
    out.push(format!("[{}]", (0..30_000).map(|i| format!("{{\"k{i}\": [{i}, \"v\\n\"]}}")).collect::<Vec<_>>().join(",")).into_bytes());
    out.push(format!("[\"{}\", 1]", "x".repeat(70_000)).into_bytes());
    out
}

fn files() -> Vec<Vec<u8>> {
    let mut out = made();
    for var in ["RERAIL_JSON_SUITE", "RERAIL_JSON_CORPUS"] {
        let Ok(dir) = std::env::var(var) else { continue };
        let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the directory").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        out.extend(paths.iter().map(|p| std::fs::read(p).expect("reads")));
    }
    out
}

/// Stage 1: the same count or refusal as simdjson's own (the kernel's `stage1_only`), and where
/// simdjson's parser accepts the file, the same index.
#[test]
fn stage1_is_simdjsons() {
    let kernel = Kernel::Arm64;
    let mut p = Parser::new();
    let files = files();
    let pmull = std::arch::is_aarch64_feature_detected!("pmull");
    for f in &files {
        let ours = p.stage1(f);
        let theirs = kernel.stage1(f, false);
        assert_eq!(ours, theirs, "stage 1 of {:?}", String::from_utf8_lossy(&f[..f.len().min(200)]));
        let index = if ours > 0 { p.indexes(ours as usize).to_vec() } else { Vec::new() };
        if pmull {
            for form in [1, 2] {
                let other = p.stage1_form(f, form);
                assert_eq!(other, ours, "stage 1 (form {form}) of {:?}", String::from_utf8_lossy(&f[..f.len().min(200)]));
                if ours > 0 {
                    assert_eq!(p.indexes(ours as usize), index, "the index (form {form}) of {:?}", String::from_utf8_lossy(&f[..f.len().min(200)]));
                }
            }
        }
        // simdjson's parser strips a byte order mark before its stage 1; `stage1_only` does not.
        if ours > 0 && !f.starts_with(b"\xef\xbb\xbf") {
            let ours = index;
            if let Ok(theirs) = rerail_simdjson_native::structurals(f, <[u32]>::to_vec) {
                assert_eq!(ours, theirs, "the index of {:?}", String::from_utf8_lossy(&f[..f.len().min(200)]));
            }
        }
    }
    eprintln!("{} files, stage 1 the same", files.len());
}

/// The tape: byte for byte the kernel's, and the same refusal.
#[test]
fn tape_is_the_kernels() {
    let kernel = Kernel::Arm64Sha;
    let mut p = Parser::new();
    let files = files();
    let mut accepted = 0;
    for f in &files {
        let ours = p.tape(f, &KINDS);
        let theirs = rerail_simdjson::tape_with(kernel, f, &KINDS);
        match (ours, theirs) {
            (Ok(a), Ok(b)) => {
                accepted += 1;
                assert_eq!((&a.code, &a.codes, &a.size, &a.base, &a.off, &a.tail, a.widths), (&b.code, &b.codes, &b.size, &b.base, &b.off, &b.tail, b.widths), "{:?}", String::from_utf8_lossy(&f[..f.len().min(200)]));
            }
            (Err(a), Err(b)) => assert_eq!(format!("{a}"), format!("{b}"), "{:?}", String::from_utf8_lossy(&f[..f.len().min(200)])),
            (a, b) => panic!("{:?}: {:?} against {:?}", String::from_utf8_lossy(&f[..f.len().min(200)]), a.map(|t| t.node_count()), b.map(|t| t.node_count())),
        }
    }
    eprintln!("{} files, {accepted} accepted; each tape and refusal the kernel's", files.len());
}

/// The process's instructions retired and cycles (macOS: `proc_pid_rusage`); zero elsewhere (a
/// Linux host builds and runs these tests too: its measurements read 0).
fn retired() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a `rusage_info_v4`, as the flavor says.
        if unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0 {
            return (info.ri_instructions, info.ri_cycles);
        }
    }
    (0, 0)
}

/// The benchmarks' inputs: JSON's corpus (`RERAIL_JSON_CORPUS`, the files simdjson accepts) and
/// inputs that part the masks from the tokens (those of `rr_parser`'s `json_tier1_by_pass`).
#[cfg(target_os = "macos")]
fn inputs(kernel: Kernel) -> Vec<(&'static str, Vec<Vec<u8>>)> {
    let mut corpus = Vec::new();
    if let Ok(dir) = std::env::var("RERAIL_JSON_CORPUS") {
        let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the directory").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        corpus.extend(paths.iter().map(|p| std::fs::read(p).expect("reads")).filter(|b| b.len() < 1 << 20 && rerail_simdjson::tape_with(kernel, b, &KINDS).is_ok()));
    }
    let n = 60_000;
    let one = |v: Vec<u8>| (0..16).map(|_| v.clone()).collect::<Vec<_>>();
    vec![
        ("corpus", corpus),
        ("long ASCII strings", one([b"[".as_slice(), &format!("\"{}\",", "a".repeat(5_000)).repeat(n / 5_000).into_bytes(), b"0]"].concat())),
        ("long UTF-8 strings", one([b"[".as_slice(), &format!("\"{}\",", "\u{e9}".repeat(2_500)).repeat(n / 5_000).into_bytes(), b"0]"].concat())),
        ("white space", one([b"[".as_slice(), &vec![b' '; n], b"]"].concat())),
        ("numbers", one([b"[".as_slice(), &"1234,".repeat(n / 5).into_bytes(), b"0]"].concat())),
        ("short strings", one([b"[".as_slice(), &"\"ab\",".repeat(n / 5).into_bytes(), b"0]"].concat())),
    ]
}

/// The translation against simdjson, instructions and cycles a byte, each the least of
/// `RR_ROUNDS` rounds: stage 1 (simdjson's own, `stage1_only`) and the tape (the kernel, its
/// source not hashed), over JSON's corpus (`RERAIL_JSON_CORPUS`) and inputs that part the masks
/// from the tokens (those of `rr_parser`'s `json_tier1_by_pass`).
#[test]
#[cfg(target_os = "macos")]
#[ignore = "a benchmark: run by hand"]
fn against_simdjson() {
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(9);
    let kernel = Kernel::Arm64Sha;
    let inputs = inputs(kernel);
    let mut p = Parser::new();
    let least = |run: &mut dyn FnMut() -> usize| {
        let mut best = (u64::MAX, u64::MAX);
        for _ in 0..rounds {
            let (i0, c0) = retired();
            std::hint::black_box(run());
            let (i1, c1) = retired();
            best = (best.0.min(i1 - i0), best.1.min(c1 - c0));
        }
        best
    };
    eprintln!("least of {rounds} rounds, instructions / cycles a byte; simdjson's kernel {}", kernel.name());
    eprintln!("{:>20}  {:>15} {:>15}   {:>15} {:>15}", "", "stage 1 C++", "stage 1 Rust", "tape C++", "tape Rust");
    for (name, files) in &inputs {
        if files.is_empty() {
            continue;
        }
        let bytes: usize = files.iter().map(Vec::len).sum();
        let s_cpp = least(&mut || files.iter().map(|f| kernel.stage1(f, false) as usize).sum());
        let s_rs = least(&mut || files.iter().map(|f| p.stage1(f) as usize).sum());
        let t_cpp = least(&mut || files.iter().map(|f| rerail_simdjson::tape_unhashed_with(kernel, f, &KINDS).unwrap().node_count()).sum());
        let t_rs = least(&mut || files.iter().map(|f| p.tape(f, &KINDS).unwrap().node_count()).sum());
        let per = |(i, c): (u64, u64)| format!("{:6.2} / {:5.2}", i as f64 / bytes as f64, c as f64 / bytes as f64);
        eprintln!("{name:>20}  {} {}   {} {}", per(s_cpp), per(s_rs), per(t_cpp), per(t_rs));
    }
}

/// `prefix_xor` as simdjson's arm64 has it (six shift-XOR rounds) against the carry-less multiply
/// it leaves out (`vmull_p64`): stage 1 over the benchmarks' inputs, and the form alone, its
/// latency (each a function of the one before) and its throughput (none dependent), the least of
/// `RR_ROUNDS` rounds.
#[test]
#[cfg(target_os = "macos")]
#[ignore = "a benchmark: run by hand"]
fn prefix_xor_forms() {
    assert!(std::arch::is_aarch64_feature_detected!("pmull"));
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(9);
    let least = |run: &mut dyn FnMut() -> u64| {
        let mut best = (u64::MAX, u64::MAX);
        for _ in 0..rounds {
            let (i0, c0) = retired();
            std::hint::black_box(run());
            let (i1, c1) = retired();
            best = (best.0.min(i1 - i0), best.1.min(c1 - c0));
        }
        best
    };
    let mut p = Parser::new();
    eprintln!("least of {rounds} rounds; instructions / cycles a byte");
    eprintln!("{:>20}  {:>15} {:>15} {:>15}", "stage 1", "rounds", "vmull_p64", "vmull_p64 raw");
    for (name, files) in &inputs(Kernel::Arm64Sha) {
        if files.is_empty() {
            continue;
        }
        let bytes = files.iter().map(Vec::len).sum::<usize>() as f64;
        let r = least(&mut || files.iter().map(|f| p.stage1(f) as u64).sum());
        let m = least(&mut || files.iter().map(|f| p.stage1_form(f, 1) as u64).sum());
        let v = least(&mut || files.iter().map(|f| p.stage1_form(f, 2) as u64).sum());
        let per = |(i, c): (u64, u64)| format!("{:6.2} / {:5.2}", i as f64 / bytes, c as f64 / bytes);
        eprintln!("{name:>20}  {} {} {}", per(r), per(m), per(v));
    }
    // The form alone, over masks no compiler can know.
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let masks: Vec<u64> = (0..1 << 16)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        })
        .collect();
    let n = masks.len() as f64;
    let latency_r = least(&mut || masks.iter().fold(0, |a, &m| stage1::prefix_xor::<false>(a ^ m)));
    let latency_m = least(&mut || masks.iter().fold(0, |a, &m| stage1::prefix_xor::<true>(a ^ m)));
    let through_r = least(&mut || masks.iter().fold(0, |a, &m| a.wrapping_add(stage1::prefix_xor::<false>(m))));
    let through_m = least(&mut || masks.iter().fold(0, |a, &m| a.wrapping_add(stage1::prefix_xor::<true>(m))));
    eprintln!("{:>20}  {:>15} {:>15}", "the form alone", "rounds", "vmull_p64");
    for (what, r, m) in [("latency", latency_r, latency_m), ("throughput", through_r, through_m)] {
        eprintln!("{what:>20}  {:6.2} / {:5.2}  {:6.2} / {:5.2}   instructions / cycles a mask", r.0 as f64 / n, r.1 as f64 / n, m.0 as f64 / n, m.1 as f64 / n);
    }
}
