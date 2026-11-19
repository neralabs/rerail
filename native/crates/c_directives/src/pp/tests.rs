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
        return Err(format!("the interpreter's tape differs ({} / {})", interp.is_some(), image.is_some()));
    }
    #[cfg(target_os = "macos")]
    if let Some(m) = c.metal_images(&[src]) {
        if m[0] != image {
            return Err(format!("Metal's tape differs ({} / {})", m[0].is_some(), image.is_some()));
        }
    }
    let Some(image) = image else { return Ok(false) };
    let lean = rerail_c_pp::tape::image(src, options(cpp));
    if image != lean {
        let a = crate::outline(&image, KINDS, FIELDS, src)?;
        let b = crate::outline(&lean, KINDS, FIELDS, src)?;
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
            let why = compiled(cpp).unwrap().parser.image(src).err();
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
    ] {
        must(src, true);
        must(src, false);
    }
}

/// R4 over every file of the C and C++ corpora: the same tape, or refused; none differ.
#[test]
#[ignore = "the corpora"]
fn the_corpus_pp_tapes() {
    let root = std::env::var("RERAIL_C_CORPUS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("orca/rerail-shared/lean-corpus"));
    let (mut same, mut refused, mut differ) = (0, 0, Vec::new());
    for (dir, cpp) in [("c", false), ("cpp", true)] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else { continue };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        let limit = std::env::var("RERAIL_PP_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
        for p in paths.into_iter().take(limit) {
            let Ok(src) = std::fs::read(&p) else { continue };
            match check(&src, cpp) {
                Ok(true) => same += 1,
                Ok(false) => refused += 1,
                Err(e) => differ.push(format!("{}: {e}", p.display())),
            }
        }
    }
    eprintln!("same {same}, refused {refused}, differ {}", differ.len());
    for d in differ.iter().take(5) {
        eprintln!("{d}\n");
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
        let why = compiled(true).unwrap().parser.image(src.as_bytes()).err();
        eprintln!("PROBE {src:?}: {r:?} {why:?}");
    }
}
