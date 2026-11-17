use super::*;

/// The lean scan's rows of `src`: its reference.
fn lean(src: &[u8], cpp: bool) -> Vec<Row> {
    let opts = if cpp { rerail_c_pp::Options::cpp(17, true) } else { rerail_c_pp::Options::c(17, true) };
    let d = rerail_c_pp::directives::scan(src, opts);
    d.all
        .iter()
        .map(|x| {
            let (start, end) = d.file(x.span);
            Row { kind: x.kind.name(), start, end }
        })
        .collect()
}

fn compiled_or_panic() -> &'static Compiled {
    compiled().unwrap_or_else(|e| panic!("the grammar compiles: {e}"))
}

/// Each case reads as the lean scan reads it: kinds and bytes.
#[test]
fn directives_read_as_the_lean_scan_reads_them() {
    let c = compiled_or_panic();
    let cases: &[&str] = &[
        "#define X 1\n",
        "#include <stdio.h>\nint main() { return 0; }\n",
        "  #  ifdef A  \n#endif\n",
        "#\n# 33 \"f.c\" 1\n#foo bar\n#!x\n",
        "/* a */ #define A /* b */ 2 // c\n",
        "#define LONG(a, b) \\\n    ((a) + \\\n     (b))\nint x;\n",
        "int s = \"#define not\"; // #define not\n/*\n#define not\n*/\n#pragma once\n",
        "#error \"don't\"\n#if 0\n#else\n#endif",
        "#include \"a.h\"\n#include_next <b.h>\n#import \"c.h\"\n#line 3 \"x\"\n#undef X\n",
        "x = a / b; /* c */\n#define Y\n",
        "#define S(x) #x\n#define C(a, b) a ## b\n",
        "\u{feff}#pragma once\n#include \"a.h\"\n",
    ];
    for src in cases {
        let got = c.rows(src.as_bytes()).unwrap_or_else(|| panic!("refused: {src:?}"));
        assert_eq!(got, lean(src.as_bytes(), false), "{src:?}\n{}", image_text(&c.image(src.as_bytes()).unwrap()).unwrap());
        assert_eq!(c.interpreter_rows(src.as_bytes()).as_ref(), Some(&got), "the interpreter, {src:?}");
    }
    #[cfg(target_os = "macos")]
    {
        let files: Vec<&[u8]> = cases.iter().map(|s| s.as_bytes()).collect();
        let metal = c.metal_rows(&files).expect("the device runs the batch");
        for (src, m) in cases.iter().zip(metal) {
            assert_eq!(m, c.rows(src.as_bytes()), "Metal, {src:?}");
        }
    }
}

/// What the step does not read is refused, never read wrong.
#[test]
fn what_the_step_does_not_read_is_refused() {
    let c = compiled_or_panic();
    for src in ["#define X 1\r\n", "auto s = R\"(x)\";\n", "int n = 1'000;\n", "#define X \"open\n", "/* open\n", "??=define X\n", "%:define X\n", "#error don't\n"] {
        assert!(c.rows(src.as_bytes()).is_none(), "{src:?}");
    }
}

/// The corpus (`~/orca/rerail-shared/lean-corpus/{c,cpp}`, or `RERAIL_C_CORPUS`): each file is
/// read the same, or refused; none differs. Ignored: it reads 3,000 files.
#[test]
#[ignore]
fn the_corpus_reads_the_same_or_is_refused() {
    let c = compiled_or_panic();
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same, mut refused, mut differ) = (0, 0, Vec::new());
    // The other backends', against Cranelift's: files whose rows differ.
    let (mut interp, mut metal) = (Vec::new(), Vec::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            let src = std::fs::read(&p).unwrap();
            let cl = c.rows(&src);
            if c.interpreter_rows(&src) != cl {
                interp.push(p.display().to_string());
            }
            #[cfg(target_os = "macos")]
            if c.metal_rows(&[&src]).is_some_and(|m| m[0] != cl) {
                metal.push(p.display().to_string());
            }
            match cl {
                None => refused += 1,
                Some(rows) if rows == lean(&src, cpp) => same += 1,
                Some(rows) => {
                    let want = lean(&src, cpp);
                    let at = rows.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(rows.len().min(want.len()));
                    differ.push(format!("{}: at {at}: {:?} against {:?}", p.display(), rows.get(at), want.get(at)));
                }
            }
        }
    }
    eprintln!("the same {same}, refused {refused}, differ {}; the interpreter differs on {}, Metal on {}", differ.len(), interp.len(), metal.len());
    for p in interp.iter().chain(&metal).take(10) {
        eprintln!("  backend: {p}");
    }
    for d in differ.iter().take(20) {
        eprintln!("  {d}");
    }
    assert!(differ.is_empty() && interp.is_empty() && metal.is_empty());
}

/// A file past 64 KiB: its tape needs 4-byte columns (the root's tail from its last directive to
/// the file's end, and offsets from a block's base), so the narrow parser refuses it and the wide
/// one reads it, on every backend.
#[test]
fn a_file_past_64_kib_is_read_at_wide_columns() {
    let c = compiled_or_panic();
    for (head, tail) in [("#define A 1\n", ""), ("", "#include <x.h>\n"), ("#if X\n", "#endif\n")] {
        let src = format!("{head}{}{tail}", "int x;\n".repeat(12_000));
        let got = c.rows(src.as_bytes()).unwrap_or_else(|| panic!("refused: {head:?} … {tail:?}"));
        assert_eq!(got, lean(src.as_bytes(), false));
        assert_eq!(c.interpreter_rows(src.as_bytes()), Some(got.clone()), "the interpreter");
        #[cfg(target_os = "macos")]
        assert_eq!(c.metal_rows(&[src.as_bytes()]).expect("the device runs it")[0], Some(got), "Metal");
    }
}
