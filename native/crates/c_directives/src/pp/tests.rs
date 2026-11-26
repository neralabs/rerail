use super::*;

/// R4 of decision 0436 on `src`: the grammar's tape is the lean writer's, on Cranelift, the
/// interpreter and Metal, or every backend refuses the file. `Ok(false)`: refused.
fn check(src: &[u8], cpp: bool) -> Result<bool, String> {
    let c = compiled(cpp).unwrap_or_else(|e| {
        eprintln!("COMPILE: {e}");
        panic!("compiles: {e}")
    });
    let image = c.image(src);
    let interp = c.interpreter_image(src);
    if interp != image {
        return Err(match (&interp, &image) {
            (Some(i), Some(t)) => format!("the interpreter's tape differs: {}", crate::difference(i, t, c.lang())),
            _ => format!("the interpreter's tape differs ({} / {})", interp.is_some(), image.is_some()),
        });
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        let m = c.metal_images(&[src]).map_err(|e| format!("the device runs it: {e:?}"))?;
        if m[0] != image {
            return Err(match (&m[0], &image) {
                (Some(d), Some(t)) => format!("Metal's tape differs: {}", crate::difference(d, t, c.lang())),
                _ => format!("Metal's tape differs ({} / {})", m[0].is_some(), image.is_some()),
            });
        }
    }
    let Some(image) = image else { return Ok(false) };
    let lean = rerail_c_pp::tape::image(src, options(cpp));
    if image != lean {
        let lang = rerail_c_pp::tape::lang();
        let a = crate::outline(&crate::Nodes::of(&image, lang)?, KINDS, FIELDS, src)?;
        let b = crate::outline(&crate::Nodes::of(&lean, lang)?, KINDS, FIELDS, src)?;
        let k = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(a.lines().count().min(b.lines().count()));
        let around = |t: &str| t.lines().skip(k.saturating_sub(3)).take(8).collect::<Vec<_>>().join("\n");
        return Err(format!("the tape differs from the lean writer's at node {k}:\ngrammar:\n{}\nlean:\n{}", around(&a), around(&b)));
    }
    Ok(true)
}

fn must(src: &[u8], cpp: bool) {
    match check(src, cpp) {
        Ok(true) => {}
        Ok(false) => {
            let why = compiled(cpp).unwrap().tape(src).err();
            eprintln!("REFUSED: {:?}: {why:?}", String::from_utf8_lossy(src));
            panic!("refused");
        }
        Err(e) => {
            eprintln!("DIFFERS: {:?}\n{e}", String::from_utf8_lossy(src));
            panic!("differs");
        }
    }
}

/// R4 on the shapes the tape has: text lines, every directive kind and its operands, groups.
#[test]
fn the_grammars_pp_tape_is_the_lean_writers() {
    for src in [
        &b"int x = 1;\nf(a, b);\n"[..],
        b"#include <a.h>\n#include \"b.h\"\n#include M(x)\n#define F(a, ...) a + 1\n#define G (x)\n#define H\nint y = F(3);\n",
        b"#ifndef G\n#define G\n#if A\n#elif B\nx\n#else\n#define F(a, b...) a\n#endif\n#endif\n",
        b"#pragma once\n#pragma GCC system_header\n#line 3 \"f\"\n# 4 \"g\" 1\n#undef G\n#error don\n#\n#foo bar\n",
        b"a /* x\n y */ b\n  # define X(a) #a ## b\n",
        b"",
        b"// c\n",
        b"#define X 1",
        // A conditional's word past 16 bytes of blanks, after a comment, on the next line (no
        // conditional: a null directive, then text), and one only its start is (`ifx`).
        b"#                       if A\n#\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\telse\n#  endif\n",
        b"# /* c */ if A\n#/**/endif\n",
        b"#\nif (x) y;\n#ifx\n#endif_\na # if b\n",
    ] {
        must(src, true);
        must(src, false);
    }
}

/// R4 over every file of the C and C++ corpora: the same tape, or refused; none differ. Cranelift
/// against the lean writer, and Metal's tapes against Cranelift's, a batch at a time; the
/// interpreter too with `RERAIL_PP_INTERP` (its unit shapes are `the_grammars_…`). A chunk:
/// `RERAIL_PP_SKIP` files of each language, then `RERAIL_PP_COUNT`.
#[test]
#[ignore = "the corpora"]
fn the_corpus_pp_tapes() {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let n = |k: &str, d: usize| std::env::var(k).ok().and_then(|n| n.parse().ok()).unwrap_or(d);
    let (skip, limit, interp) = (n("RERAIL_PP_SKIP", 0), n("RERAIL_PP_COUNT", usize::MAX), std::env::var_os("RERAIL_PP_INTERP").is_some());
    let (mut same, mut differ) = (0, Vec::new());
    let mut refused = std::collections::BTreeMap::<String, usize>::new();
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        let c = compiled(cpp).unwrap();
        let files: Vec<(std::path::PathBuf, Vec<u8>)> = paths.into_iter().skip(skip).take(limit).filter_map(|p| std::fs::read(&p).ok().map(|s| (p, s))).collect();
        for batch in files.chunks(64) {
            let images: Vec<_> = batch.iter().map(|(_, src)| c.tape(src)).collect();
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            let metal = c.metal_images(&batch.iter().map(|(_, s)| s.as_slice()).collect::<Vec<_>>()).unwrap_or_else(|e| panic!("the device runs the batch: {e:?}"));
            for (k, ((p, src), image)) in batch.iter().zip(&images).enumerate() {
                #[cfg(any(target_os = "macos", target_os = "linux", windows))]
                if metal[k].as_ref() != image.as_ref().ok() {
                    differ.push(format!("{}: Metal's tape differs", p.display()));
                    continue;
                }
                let _ = k;
                if interp && c.interpreter_image(src) != image.as_ref().ok().cloned() {
                    differ.push(format!("{}: the interpreter's tape differs", p.display()));
                    continue;
                }
                match image {
                    Err(r) => *refused.entry(r.clone()).or_default() += 1,
                    Ok(image) => {
                        let lean = rerail_c_pp::tape::image(src, options(cpp));
                        if *image == lean {
                            same += 1;
                        } else {
                            let lang = rerail_c_pp::tape::lang();
                            let a = crate::outline(&crate::Nodes::of(image, lang).unwrap(), KINDS, FIELDS, src).unwrap();
                            let b = crate::outline(&crate::Nodes::of(&lean, lang).unwrap(), KINDS, FIELDS, src).unwrap();
                            let at = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
                            let around = |t: &str| t.lines().skip(at.saturating_sub(3)).take(7).collect::<Vec<_>>().join("\n");
                            differ.push(format!("{}: node {at}\ngrammar:\n{}\nlean:\n{}", p.display(), around(&a), around(&b)));
                        }
                    }
                }
            }
        }
    }
    eprintln!("same {same}, refused {}, differ {}", refused.values().sum::<usize>(), differ.len());
    for (why, k) in &refused {
        eprintln!("  refused {k}: {why}");
    }
    for d in differ.iter().take(6) {
        eprintln!("DIFF {d}\n");
    }
    assert!(differ.is_empty());
}

/// Each of `RERAIL_PP_PROBE`'s texts (`|||` between them, `\n` for a line break): the same, refused
/// (and why) or differs. For reading the grammar.
#[test]
#[ignore = "a probe"]
fn probe() {
    if let Ok(dir) = std::env::var("RERAIL_PP_DUMP") {
        let c = compiled(true).unwrap();
        std::fs::write(format!("{dir}/pp.machine"), &c.machine).unwrap();
        std::fs::write(format!("{dir}/pp.words"), &c.words).unwrap();
        std::fs::write(format!("{dir}/raw.machine"), &crate::compiled().unwrap().machine).unwrap();
    }
    let Ok(all) = std::env::var("RERAIL_PP_PROBE") else { return };
    for src in all.split("|||") {
        let src = src.replace("\\n", "\n");
        let r = check(src.as_bytes(), true);
        let why = compiled(true).unwrap().tape(src.as_bytes()).err();
        eprintln!("PROBE {src:?}: {r:?} {why:?}");
    }
}
